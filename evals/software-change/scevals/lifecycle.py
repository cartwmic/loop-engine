"""Public replay and durable trial continuation commands (host private inputs)."""
import json
from pathlib import Path
import re
import shutil

from evalcore.cli import read_settings
from evalcore.continuation import load_run, settings_from_recorded
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
