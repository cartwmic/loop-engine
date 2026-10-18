"""Explicit matched campaign continuation; never infer categories from error text."""
import hashlib
import json
from pathlib import Path

from .accounting import TrialRecord
from .provenance import check_same_cohort
from .settings import CampaignSettings, matched_assignment


def settings_from_recorded(data):
    # Preserve explicit/default provenance when recovering original settings.
    fields = {field.alias or key for key, field in CampaignSettings.model_fields.items()}
    sources = data.get("setting_sources", {})
    values = {k: v for k, v in data.items() if k in fields and sources.get(k, "explicit") == "explicit"}
    if "resolved_cases_path" in data:
        values["cases_path"] = data["resolved_cases_path"]
    return CampaignSettings.model_validate(values)


def load_run(path):
    path = Path(path).resolve()
    summary_file = path / "summary.json"
    if not summary_file.is_file():
        summary_file = path / "records.json"
    if summary_file.is_file():
        summary = json.loads(summary_file.read_text())
        return summary, Path(summary.get("capture_run") or path)
    manifest = json.loads((path / "run.json").read_text())
    trials = []
    for file in sorted(path.glob("*/meta.json")):
        meta = json.loads(file.read_text())
        if meta.get("record"):
            trials.append(meta["record"])
    return {**manifest, "trials": trials}, path


OUTER_LIMITS = ("hard_limit_seconds", "idle_limit_seconds")


def normalized_settings(settings):
    values = settings.model_dump(mode="json", by_alias=True)
    values["cases_path"] = str(settings.cases_path.resolve())
    return values


def record_digest(record):
    return hashlib.sha256(json.dumps(record, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=False).encode()).hexdigest()


def validate_execution_exception(authorization, original, current):
    """Only an explicit, durable disabling of both outer limits is permitted."""
    required = {"kind", "owner", "authority", "reason", "original_limits", "new_limits"}
    if set(authorization) != required or authorization["kind"] != "disable-outer-deadlines":
        raise ValueError("invalid execution exception")
    if any(not isinstance(authorization[k], str) or not authorization[k].strip()
           for k in ("owner", "authority", "reason")):
        raise ValueError("execution exception needs owner, authority and reason")
    before, after = normalized_settings(original), normalized_settings(current)
    if authorization["original_limits"] != {k: before[k] for k in OUTER_LIMITS}:
        raise ValueError("exception does not match original execution limits")
    if authorization["new_limits"] != {k: None for k in OUTER_LIMITS}:
        raise ValueError("exception must disable both outer deadlines")
    if any(after[k] is not None for k in OUTER_LIMITS):
        raise ValueError("new execution settings still impose outer deadlines")
    if any(before[k] != after[k] for k in before if k not in OUTER_LIMITS):
        raise ValueError("unrelated settings differ; exception only changes outer deadlines")
    return authorization


def validate_continuation(summary, current, assignments):
    """Validate the new index, including original records, without rescoring."""
    continuation = summary.get("execution_continuation")
    if continuation is None:
        return
    original = settings_from_recorded(continuation["original_settings"])
    validate_execution_exception(continuation["authorization"], original, current)
    if normalized_settings(settings_from_recorded(continuation["new_settings"])) != normalized_settings(current):
        raise ValueError("continuation new settings do not match")
    expected = [a.model_dump() for a in assignments]
    if continuation["matched_assignment"] != expected:
        raise ValueError("continuation assignment mismatch")
    digests = continuation["carried_record_sha256"]
    records = {r["sample_id"]: r for r in summary["trials"]}
    if not set(digests) <= set(records) or any(record_digest(records[k]) != v for k, v in digests.items()):
        raise ValueError("historical scored records changed")
    if not set(records) <= {a["sample_id"] for a in expected}:
        raise ValueError("continuation includes unrelated scored assignments")
    pending = [a["sample_id"] for a in expected if a["sample_id"] not in digests]
    if continuation["new_assignment_ids"] != pending:
        raise ValueError("continuation would execute unintended assignments")


def resume_campaign(target, settings, source, *, resume_interrupted=False, provider_config_paths=(), clock_start=None):
    from .runner import run_campaign
    summary, captures = load_run(source)
    original = settings_from_recorded(summary["settings"])
    # Explicit before the general settings-equality refusal: never resume across projection cohorts.
    check_same_cohort(summary["settings"], settings, action="resume")
    _, assignments = matched_assignment(settings, target.load_cases(settings.cases_path))
    current_values = settings.model_dump(mode="json", by_alias=True)
    current_values["cases_path"] = str(settings.cases_path.resolve())
    original_values = original.model_dump(mode="json", by_alias=True)
    original_values["cases_path"] = str(original.cases_path.resolve())
    if (current_values != original_values or
            [a.model_dump() for a in assignments] != summary["matched_assignment"]):
        raise ValueError("resume requires identical settings and matched assignment")
    validate_continuation(summary, settings, assignments)
    target.execution_continuation = summary.get("execution_continuation")
    if target.execution_continuation and resume_interrupted:
        raise ValueError("deadline exception carries scored records; it does not restart interrupted runs")
    carried = []
    carried_sources = {}
    sources = {}
    for data in summary["trials"]:
        clean = data["outcome"] == "success" or (data["outcome"] == "failure" and data["reason"] != "unclassified_error")
        if clean:
            record = TrialRecord(**data)
            if not target.execution_continuation:
                record.carried_from = str(Path(source).resolve())
            carried.append(record)
            capture = captures / data["sample_id"]
            if not (capture / "meta.json").is_file():
                capture = Path(summary.get("sources", {}).get(data["sample_id"], capture))
            carried_sources[data["sample_id"]] = capture
        elif resume_interrupted:
            capture = captures / data["sample_id"]
            if not (capture / "meta.json").is_file():
                capture = Path(summary.get("sources", {}).get(data["sample_id"], capture))
            if ((capture / "meta.json").is_file() and not data.get("terminal") and
                    getattr(target, "can_resume_capture", lambda p: True)(capture)):
                sources[data["sample_id"]] = capture
    if resume_interrupted:
        known = {data["sample_id"] for data in summary["trials"]}
        for assignment in assignments:
            if assignment.sample_id in known:
                continue
            capture = captures / assignment.sample_id
            if getattr(target, "can_resume_capture", lambda p: False)(capture):
                sources[assignment.sample_id] = capture
    if target.execution_continuation and len(carried) != len(summary["trials"]):
        raise ValueError("deadline continuation cannot rerun a carried scored record")
    target.sources = sources
    target.carried_sources = carried_sources
    return run_campaign(target, settings, carried_records=carried, provider_config_paths=provider_config_paths,
                        clock_start=clock_start)
