"""Read-only past-run extraction and owner-confirmed judge calibration."""
import argparse
import asyncio
import hashlib
import json
import sqlite3
import re
from collections import Counter
from contextlib import closing
from pathlib import Path

from evalcore.report import protected_values, scan_results

from .judge import JUDGE_CONTRACT, judge_intent

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LABELS = ROOT / "calibration" / "labels.json"


def write_safe_json(path: Path, data: dict) -> None:
    """Scan before creating anything; never overwrite owner labels or receipts."""
    text = json.dumps(data, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
    scan_results((text,), protected_values())
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as stream:
        stream.write(text)


def session_intents(directory: Path, revisions: dict[str, str]) -> dict:
    """Two streaming passes per relevant file; retain only matching intent objects.

    First identify all runs mentioned anywhere in a transcript (reviewer tool
    output may not repeat the ID next to the intent). Then decode message text.
    No session contents or paths are printed or modified.
    """
    if not directory.is_dir():
        raise ValueError("sessions directory is missing")
    if not revisions:
        return {}
    mentions = re.compile("|".join(re.escape(run_id) for run_id in revisions))
    counts = {run_id: Counter() for run_id in revisions}
    file_counts = Counter()
    decoder = json.JSONDecoder()
    for path in sorted(directory.rglob("*.jsonl")):
        runs = set()
        with path.open(errors="replace") as stream:
            for line in stream:
                runs.update(mentions.findall(line))
        if not runs:
            continue
        file_counts.update(runs)
        with path.open(errors="replace") as stream:
            for line in stream:
                if 'acceptance' not in line or 'outcome' not in line:
                    continue
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                message = row.get("message", row) if isinstance(row, dict) else None
                if not isinstance(message, dict):
                    continue
                content = message.get("content", [])
                texts = [content] if isinstance(content, str) else [
                    block.get("text", "") for block in content
                    if isinstance(block, dict) and block.get("type") == "text"
                ] if isinstance(content, list) else []
                for text in texts:
                    if not isinstance(text, str):
                        continue
                    index = text.find("{")
                    while index != -1:
                        try:
                            obj, end = decoder.raw_decode(text, index)
                        except ValueError:
                            index = text.find("{", index + 1)
                            continue
                        if isinstance(obj, dict) and {"revision", "outcome", "acceptance"} <= obj.keys():
                            canonical = json.dumps(obj, sort_keys=True, ensure_ascii=False)
                            for run_id in runs:
                                if str(obj["revision"]) == revisions[run_id]:
                                    counts[run_id][canonical] += 1
                        index = text.find("{", end)
    result = {}
    for run_id, matches in counts.items():
        if not matches:
            continue
        # Stable lexical tie-break, while explicitly recording any ambiguity.
        selected = min(matches, key=lambda obj: (-matches[obj], obj))
        result[run_id] = (json.loads(selected), {
            "reconstructed": True, "session_file_count": file_counts[run_id],
            "match_count": sum(matches.values()), "selected_match_count": matches[selected],
            "ambiguous": len(matches) > 1, "distinct_match_count": len(matches),
        })
    return result


def extract_candidates(database: Path, *, limit: int = 20, scan_limit: int = 200,
                       sessions: Path | None = None) -> dict:
    if limit < 1 or scan_limit < limit:
        raise ValueError("invalid extraction limits")
    entries = []
    examined = 0
    skipped_reasons = {"artifact_folder_missing": 0, "no_owner_request": 0,
                       "intent_unreadable": 0}
    recovery = {"recovered": 0, "not_found": 0, "ambiguous": 0, "skipped_no_request": 0}
    pending = []
    # mode=ro preserves SQLite's WAL view (do not use immutable=1 on a live DB).
    with closing(sqlite3.connect(database.expanduser().resolve().as_uri() + "?mode=ro", uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        runs = connection.execute(
            "SELECT id, artifact_root FROM runs WHERE workflow_id='software-change' "
            "AND lifecycle='final' AND current_state='end' "
            "ORDER BY created_at DESC, id DESC LIMIT ?", (scan_limit,)
        ).fetchall()
        for run_id, artifact_root in runs:
            root = Path(artifact_root) if artifact_root else None
            missing = root is None or not root.is_dir()
            if missing and sessions is None:
                pending.append((run_id, None, "artifact_folder_missing"))
                continue
            records = connection.execute(
                "SELECT record_id, data_json FROM context_records "
                "WHERE run_id=? AND kind='user-steering' ORDER BY sequence", (run_id,)
            ).fetchall()
            instructions, source_ids = [], []
            for record_id, raw in records:
                try:
                    record = json.loads(raw)
                    instruction = record.get("instruction") if isinstance(record, dict) else None
                except (ValueError, TypeError):
                    instruction = None
                if isinstance(instruction, str) and instruction.strip():
                    instructions.append(instruction)
                    source_ids.append(record_id)
            if not instructions:
                pending.append((run_id, None, "no_owner_request"))
                continue
            entry = {"id": run_id, "source": {"run_id": run_id, "request_record_ids": source_ids},
                     "owner_request": "\n\n".join(instructions),
                     "owner_label": None, "owner_confirmed": False}
            if missing:
                revision = None
                # Inspected catalog schema: subject and subject_revision live in
                # review-evidence data_json, ordered by context record sequence.
                for (raw,) in connection.execute(
                    "SELECT data_json FROM context_records WHERE run_id=? "
                    "AND kind='review-evidence' ORDER BY sequence DESC", (run_id,)
                ):
                    try:
                        record = json.loads(raw)
                    except ValueError:
                        continue
                    if isinstance(record, dict) and record.get("subject") == "intent.json":
                        revision = record.get("subject_revision")
                        break
                entry["source"]["intent_revision"] = str(revision) if revision is not None else None
                pending.append((run_id, entry, "reconstruct"))
                continue
            try:
                content = (root / "intent.json").read_bytes()
                intent = json.loads(content)
                if not isinstance(intent, dict) or not intent.get("revision"):
                    raise ValueError()
            except (OSError, ValueError, TypeError):
                pending.append((run_id, None, "intent_unreadable"))
                continue
            entry.update(intent=intent)
            entry["source"].update(intent_revision=intent["revision"], intent_sha256=hashlib.sha256(content).hexdigest())
            pending.append((run_id, entry, None))
    revisions = {run_id: entry["source"]["intent_revision"] for run_id, entry, reason in pending
                 if reason == "reconstruct" and entry["source"]["intent_revision"] is not None}
    reconstructed = session_intents(sessions.expanduser(), revisions) if sessions is not None else {}
    for run_id, entry, reason in pending:
        if len(entries) >= limit:
            break
        examined += 1
        if reason == "reconstruct":
            if run_id not in reconstructed:
                recovery["not_found"] += 1
                reason = "artifact_folder_missing"
            else:
                intent, metadata = reconstructed[run_id]
                entry["intent"] = intent
                entry["source"].update(metadata, intent_sha256=hashlib.sha256(
                    json.dumps(intent, sort_keys=True, ensure_ascii=False).encode()).hexdigest())
                recovery["recovered"] += 1
                recovery["ambiguous"] += int(metadata["ambiguous"])
                reason = None
        if reason:
            skipped_reasons[reason] += 1
            if reason == "no_owner_request":
                recovery["skipped_no_request"] += 1
        else:
            entries.append(entry)
    extraction = {"candidates": len(entries), "completed_runs_examined": examined,
                  "skipped": sum(skipped_reasons.values()), "skipped_reasons": skipped_reasons,
                  "scan_limit": scan_limit}
    if sessions is not None:
        extraction.update(recovery)
    return {"schema_version": 1, "judge_model": None, "judge_contract": JUDGE_CONTRACT,
            "entries": entries, "extraction": extraction}


def confirmed_entries(labels: dict) -> list[dict]:
    if labels.get("schema_version") != 1 or labels.get("judge_contract") != JUDGE_CONTRACT:
        raise ValueError("unsupported label contract")
    entries = labels.get("entries")
    if not isinstance(entries, list) or not entries:
        raise ValueError("no labelled entries")
    seen = set()
    confirmed = []
    for entry in entries:
        identifier = entry["id"]
        if not isinstance(identifier, str) or not identifier or identifier in seen:
            raise ValueError("duplicate or invalid label id")
        seen.add(identifier)
        label = entry.get("owner_label")
        if label is None and entry.get("owner_confirmed") is False:
            continue
        if label not in ("good", "deficient") or entry.get("owner_confirmed") is not True:
            raise ValueError("labels must be owner-confirmed good or deficient")
        if not isinstance(entry.get("intent"), dict) or not isinstance(entry.get("owner_request"), str) or not entry["owner_request"].strip():
            raise ValueError("invalid candidate pair")
        confirmed.append(entry)
    if not confirmed:
        raise ValueError("no owner-confirmed labels")
    return confirmed


def calibration_figures(verdicts: list[dict]) -> dict:
    """False pass denominator is owner-deficient, not all judge passes."""
    count = len(verdicts)
    good = sum(v["owner_label"] == "good" for v in verdicts)
    deficient = count - good
    agree = sum(v["judge_passed"] == (v["owner_label"] == "good") for v in verdicts)
    false_passes = sum(v["judge_passed"] and v["owner_label"] == "deficient" for v in verdicts)
    return {"agreement": {"numerator": agree, "denominator": count, "rate": agree / count if count else None},
            "false_pass_rate": {"numerator": false_passes, "denominator": deficient,
                                "rate": false_passes / deficient if deficient else None},
            "counts": {"labelled": count, "owner_good": good, "owner_deficient": deficient,
                       "judge_passed": sum(v["judge_passed"] for v in verdicts),
                       "judge_failed": sum(not v["judge_passed"] for v in verdicts)}}


async def score_labels(labels: dict, judge_model: str) -> dict:
    entries = confirmed_entries(labels)
    if not judge_model or "/" not in judge_model:
        raise ValueError("explicit pinned Inspect judge model is required")
    verdicts = []
    usage = {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    for entry in entries:
        judgment = await judge_intent(entry["intent"], entry["owner_request"], judge_model=judge_model)
        verdicts.append({"id": entry["id"], "owner_label": entry["owner_label"], "judge_passed": judgment.passed})
        for key in usage:
            value = judgment.usage[key]
            usage[key] = usage[key] + value if usage[key] is not None and value is not None else None
    return {"schema_version": 1, "judge_model": judge_model, "judge_contract": JUDGE_CONTRACT,
            **calibration_figures(verdicts),
            "unlabelled_count": len(labels["entries"]) - len(entries),
            "evaluation_overhead": {"role": "judge", "model": judge_model, **usage},
            "verdicts": verdicts}


def calibration_for_report(path: Path, *, judge_model: str) -> dict:
    """Load export-safe figures only when they calibrate this campaign's judge.

    Targets attach this object beside intent_judge scores in report_metrics.
    A missing receipt is not a zero: the caller should report calibration pending.
    """
    receipt = json.loads(path.read_text())
    if receipt.get("schema_version") != 1 or receipt.get("judge_contract") != JUDGE_CONTRACT or receipt.get("judge_model") != judge_model:
        raise ValueError("calibration receipt does not match pinned judge")
    figures = calibration_figures(receipt["verdicts"])
    if any(receipt[key] != figures[key] for key in figures):
        raise ValueError("inconsistent calibration figures")
    result = {"judge_model": judge_model, "judge_contract": JUDGE_CONTRACT,
              "labels_sha256": receipt["labels_sha256"], **figures,
              "unlabelled_count": receipt["unlabelled_count"],
              "evaluation_overhead": receipt["evaluation_overhead"]}
    scan_results((json.dumps(result),), protected_values())
    return result


def extract_command(args):
    labels = extract_candidates(args.database, limit=args.limit, scan_limit=args.scan_limit, sessions=args.sessions)
    print("Extraction summary: " + json.dumps(labels["extraction"], sort_keys=True))
    if not labels["entries"]:
        print("No usable candidate pairs found; no labelling file written.")
        return 2
    if args.sessions is not None and args.output.exists():
        existing = json.loads(args.output.read_text())
        by_id = {entry["id"]: entry for entry in existing["entries"]}
        for entry in labels["entries"]:
            previous = by_id.get(entry["id"])
            if previous is not None:
                entry["owner_label"] = previous.get("owner_label")
                entry["owner_confirmed"] = previous.get("owner_confirmed", False)
            by_id[entry["id"]] = entry
        labels["entries"] = list(by_id.values())
        labels["judge_model"] = existing.get("judge_model")
        text = json.dumps(labels, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
        scan_results((text,), protected_values())
        # Atomic replacement only in explicit reconstruction mode; scoring
        # receipts and legacy extraction retain their no-overwrite contract.
        temporary = args.output.with_name(args.output.name + ".tmp")
        with temporary.open("x") as stream:
            stream.write(text)
        temporary.replace(args.output)
    else:
        write_safe_json(args.output, labels)
    print(f"Extracted {len(labels['entries'])} candidates; new owner labels remain empty and existing labels are preserved. Database opened read-only.")
    return 0


def score_command(args):
    # Validate the entire label set and model pin before any model request.
    if not args.labels.is_file():
        print("Calibration pending: extract candidates, obtain owner-confirmed labels and pin judge_model; no judge calls made.")
        return 2
    labels_content = args.labels.read_bytes()
    labels = json.loads(labels_content)
    model = args.judge_model or labels.get("judge_model")
    try:
        confirmed_entries(labels)
        if not isinstance(model, str) or "/" not in model:
            raise ValueError()
    except (KeyError, TypeError, ValueError):
        print("Calibration pending: obtain valid owner-confirmed good/deficient labels and pin judge_model; no judge calls made.")
        return 2
    destination = args.output or args.labels.parent / "captures" / "calibration.json"
    if destination.exists():
        raise FileExistsError("calibration receipt exists")
    # Labels must pass the secret guard before being sent to a judge as well.
    scan_results((json.dumps(labels),), protected_values())
    result = asyncio.run(score_labels(labels, model))
    result["labels_sha256"] = hashlib.sha256(labels_content).hexdigest()
    write_safe_json(destination, result)
    a, f = result["agreement"], result["false_pass_rate"]
    print(f"Agreement: {a['numerator']}/{a['denominator']} ({a['rate']})")
    print(f"False-pass rate: {f['numerator']}/{f['denominator']} owner-deficient ({f['rate']})")
    print("Counts: " + json.dumps(result["counts"], sort_keys=True))
    return 0


def register_subcommands(subparsers):
    parser = subparsers.add_parser("calibrate", help="owner-confirmed intent judge calibration")
    commands = parser.add_subparsers(dest="calibration_command", required=True)
    extract = commands.add_parser("extract", help="read-only candidate extraction; never assign labels")
    extract.add_argument("--database", type=Path, required=True)
    extract.add_argument("--sessions", type=Path, help="read-only Pi transcripts for missing artifact roots; merge existing labels by id")
    extract.add_argument("--output", type=Path, default=DEFAULT_LABELS)
    extract.add_argument("--limit", type=int, default=20)
    extract.add_argument("--scan-limit", type=int, default=200)
    extract.set_defaults(handler=extract_command)
    score = commands.add_parser("score", help="score only owner-confirmed entries")
    score.add_argument("--labels", type=Path, required=True)
    score.add_argument("--judge-model", help="pinned Inspect model ID; otherwise labels.judge_model")
    score.add_argument("--output", type=Path, help="safe metric receipt; default labels-parent/captures/calibration.json")
    score.set_defaults(handler=score_command)
