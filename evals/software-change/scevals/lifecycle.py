"""Public replay and durable trial continuation commands (host private inputs)."""
import json
from pathlib import Path
import re
import shutil

from evalcore.cli import read_settings
from evalcore.continuation import (load_run, settings_from_recorded, record_digest,
                                   validate_execution_exception)
from evalcore.provenance import check_same_cohort
from evalcore.report import utc_timestamp
from evalcore.runner import run_campaign
from evalcore.settings import Assignment, CampaignSettings, matched_assignment
from .capture import captured_lifecycle, read_capture
from .target import ROOT, SoftwareChangeTarget


def checked_sources(target, settings, sources):
    cases = {c.id: c for c in target.load_cases(settings.cases_path)}
    combos = {c.id: c for c in settings.resolved_combinations()}
    assignments = []
    for sample_id, source in sources.items():
        archive, meta = read_capture(source)
        if not archive.is_file():
            raise FileNotFoundError("workspace snapshot missing")
        assignment = Assignment.model_validate(meta["assignment"])
        if assignment.sample_id != sample_id or assignment.combination_id not in combos:
            raise ValueError("capture assignment does not match settings")
        case = cases.get(assignment.case_id)
        if case is None or case.version != assignment.case_version:
            raise ValueError("capture case ID/version mismatch")
        assignments.append(assignment)
    if not assignments:
        raise ValueError("no captured trials")
    return assignments


def rescore(args):
    target = SoftwareChangeTarget()
    source = args.path.resolve()
    if source.is_file():
        if not args.sample or not args.case or not args.model:
            raise ValueError("bare snapshot requires --sample, --case and --model")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", args.sample):
            raise ValueError("invalid sample ID")
        if args.settings:
            settings = CampaignSettings.model_validate(read_settings(args.settings))
        else:
            settings = CampaignSettings(campaign="external-recovery", cases_path=args.case, models=(args.model,),
                                        simulator_model=args.model, judge_model=args.model, case_count=1, trials_per_case=1)
        cases = target.load_cases(settings.cases_path)
        case = next((c for c in cases if c.id == str(args.case) or c.path.resolve() == args.case.resolve()), None)
        if case is None:
            raise ValueError("external case not found")
        combo = next((c for c in settings.resolved_combinations() if c.driver == args.model), None)
        if combo is None:
            raise ValueError("external model not in settings")
        assignment = Assignment(sample_id=args.sample, case_id=case.id, case_version=case.version,
                                combination_id=combo.id, trial=1)
        # Keep the supplied external evidence durably and privately too.
        capture = ROOT / "captures" / settings.campaign / utc_timestamp() / args.sample
        capture.mkdir(mode=0o700, parents=True)
        shutil.copyfile(source, capture / "workspace.tar")
        meta = {"format_version": 1, "settings": settings.recorded(), "assignment": assignment.model_dump(),
                "evidence": {"run_id": args.run_id}, "usage_by_role": {}, "segments": [],
                "original_usage_available": False,
                "usage_source": "unavailable: external snapshot has no metadata or Inspect log"}
        (capture / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
        sources = {args.sample: capture}
    else:
        manifest, captures = load_run(source)
        settings = (CampaignSettings.model_validate(read_settings(args.settings)) if args.settings
                    else settings_from_recorded(manifest["settings"]))
        sources = {p.parent.name: p.parent for p in sorted(captures.glob("*/meta.json"))}
        sources = {**{k: Path(v) for k, v in manifest.get("sources", {}).items()}, **sources}
    assignments = checked_sources(target, settings, sources)
    target.sources = sources
    target.operation = "rescore"
    destination = run_campaign(target, settings, assignments_override=assignments)
    print(f"Report written: {destination}")
    return 0


def merge_captures(args):
    """Assemble private snapshots and scored records without scoring or model calls."""
    target = SoftwareChangeTarget()
    settings = CampaignSettings.model_validate(read_settings(args.settings))
    _, assignments = matched_assignment(settings, target.load_cases(settings.cases_path))
    expected = {a.sample_id: a.model_dump() for a in assignments}

    def scoring_settings(value):
        data = value.model_dump(mode="json", by_alias=True)
        data["cases_path"] = str(value.cases_path.resolve())
        # Execution/report label only; all model/limit/price/target options match.
        data.pop("campaign")
        return data

    destination = args.out_dir.resolve()
    if destination.exists():
        raise FileExistsError("merge output directory already exists")
    records, captures = {}, {}
    exception_path = getattr(args, "execution_exception", None)
    authorization = json.loads(exception_path.read_text()) if exception_path else None
    original_settings = None
    original_recorded_settings = None
    source_receipts = []
    for source in args.src_dirs:
        summary, root = load_run(source)
        # Explicit before the existing settings-equality refusal: never merge across projection cohorts.
        check_same_cohort(summary["settings"], settings, action="merge")
        original = settings_from_recorded(summary["settings"])
        if authorization:
            validate_execution_exception(authorization, original, settings)
            if original_settings is not None and scoring_settings(original_settings) != scoring_settings(original):
                raise ValueError("source execution settings differ")
            original_settings = original
            if original_recorded_settings is None:
                original_recorded_settings = summary["settings"]
            if summary["matched_assignment"] != list(expected.values()):
                raise ValueError("source matched assignment differs")
        elif scoring_settings(original) != scoring_settings(settings):
            raise ValueError("merge requires matching scoring settings")
        source_receipts.append({"path": str(Path(source).resolve()),
                                "settings": summary["settings"],
                                "records": {d["sample_id"]: record_digest(d) for d in summary["trials"]}})
        for data in summary["trials"]:
            sample = data["sample_id"]
            if sample in records:
                raise ValueError(f"duplicate sample_id: {sample}")
            if sample not in expected or any(data.get(k) != v for k, v in expected[sample].items()):
                raise ValueError("merge capture assignment does not match target settings")
            capture = root / sample
            if not (capture / "meta.json").is_file():
                capture = Path(summary.get("sources", {}).get(sample, capture))
            archive, meta = read_capture(capture)
            if not archive.is_file() or meta["assignment"] != expected[sample]:
                raise ValueError("merge snapshot missing or assignment mismatch")
            if authorization:
                if scoring_settings(settings_from_recorded(meta["settings"])) != scoring_settings(original):
                    raise ValueError("capture settings differ from its source")
                if meta.get("record") != data:
                    raise ValueError("source and capture scored records differ")
                if data["outcome"] != "success" and not (
                        data["outcome"] == "failure" and data["reason"] != "unclassified_error"):
                    raise ValueError("only clean scored records may be carried in this continuation")
            records[sample], captures[sample] = data, capture
    if not records:
        raise ValueError("no scored captures to merge")
    # Validate all inputs before creating the destination; no partial output on
    # duplicate/settings errors. A copy failure removes only this new directory.
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    try:
        for sample, capture in captures.items():
            copied = destination / sample
            shutil.copytree(capture, copied)
            copied.chmod(0o700)
            meta = json.loads((copied / "meta.json").read_text())
            if not authorization:
                meta.update(settings=settings.recorded(), record=records[sample],
                            usage_by_role=records[sample]["usage_by_role"], source_capture=str(capture.resolve()))
                (copied / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
            # Exception merges preserve original metadata and records byte-for-byte.
        manifest = {"settings": settings.recorded(), "matched_assignment": list(expected.values()),
                    "sources": {}, "merged_from": [str(p.resolve()) for p in args.src_dirs]}
        if authorization:
            manifest["execution_continuation"] = {
                "authorization": authorization,
                "original_settings": original_recorded_settings,
                "new_settings": settings.recorded(),
                "matched_assignment": list(expected.values()),
                "carried_record_sha256": {k: record_digest(v) for k, v in records.items()},
                "new_assignment_ids": [k for k in expected if k not in records],
                "sources": source_receipts,
                "comparison_note": "Historical trials retain their original outer limits. Only new assignments have hard and idle limits disabled; this is not a uniform-limit comparison.",
            }
        (destination / "run.json").write_text(json.dumps(manifest, indent=2) + "\n")
        (destination / "records.json").write_text(json.dumps({**manifest, "trials": list(records.values())}, indent=2) + "\n")
    except BaseException:
        shutil.rmtree(destination)
        raise
    print(f"Merged {len(records)} captures: {destination}")
    return 0


def resume_trial(args):
    archive, meta = read_capture(args.path)
    settings = settings_from_recorded(meta["settings"])
    evidence = meta.get("evidence", {})
    record = meta.get("record", {})
    if (captured_lifecycle(archive) in ("final", "terminated") or
            evidence.get("lifecycle") in ("final", "terminated") or record.get("terminal")):
        raise ValueError("only non-terminal trials can resume")
    if record and not (record.get("outcome") == "infrastructure" or record.get("reason") == "unclassified_error"):
        raise ValueError("resume-trial requires an infrastructure or harness interruption")
    target = SoftwareChangeTarget()
    target.operation = "resume-trial"
    assignment = Assignment.model_validate(meta["assignment"])
    sources = {assignment.sample_id: args.path.resolve()}
    checked_sources(target, settings, sources)
    target.sources = sources
    destination = run_campaign(target, settings, assignments_override=[assignment])
    print(f"Report written: {destination}")
    return 0


def register_subcommands(subparsers):
    parser = subparsers.add_parser("merge-captures", help="merge disjoint scored capture runs for matched campaign resume")
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("src_dirs", type=Path, nargs="+")
    parser.add_argument("--settings", required=True)
    parser.add_argument("--execution-exception", type=Path, help="explicit owner-authorized disabling of outer deadlines; preserves original capture settings")
    parser.set_defaults(handler=merge_captures)
    parser = subparsers.add_parser("rescore", help="score private workspace captures with the current harness")
    parser.add_argument("path", type=Path, help="capture run directory or bare /workspace tar")
    parser.add_argument("--settings")
    parser.add_argument("--sample")
    parser.add_argument("--case", type=Path)
    parser.add_argument("--model")
    parser.add_argument("--run-id", default="eval-trial")
    parser.set_defaults(handler=rescore)
    parser = subparsers.add_parser("resume-trial", help="continue a captured non-terminal infrastructure/harness interruption")
    parser.add_argument("path", type=Path)
    parser.set_defaults(handler=resume_trial)
