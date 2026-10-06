"""Explicit matched campaign continuation; never infer categories from error text."""
import json
from pathlib import Path

from .accounting import TrialRecord
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


def resume_campaign(target, settings, source, *, resume_interrupted=False, provider_config_paths=()):
    from .runner import run_campaign
    summary, captures = load_run(source)
    original = settings_from_recorded(summary["settings"])
    _, assignments = matched_assignment(settings, target.load_cases(settings.cases_path))
    current_values = settings.model_dump(mode="json", by_alias=True)
    current_values["cases_path"] = str(settings.cases_path.resolve())
    original_values = original.model_dump(mode="json", by_alias=True)
    original_values["cases_path"] = str(original.cases_path.resolve())
    if (current_values != original_values or
            [a.model_dump() for a in assignments] != summary["matched_assignment"]):
        raise ValueError("resume requires identical settings and matched assignment")
    carried = []
    carried_sources = {}
    sources = {}
    for data in summary["trials"]:
        clean = data["outcome"] == "success" or (data["outcome"] == "failure" and data["reason"] != "unclassified_error")
        if clean:
            record = TrialRecord(**data)
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
    target.sources = sources
    target.carried_sources = carried_sources
    return run_campaign(target, settings, carried_records=carried, provider_config_paths=provider_config_paths)
