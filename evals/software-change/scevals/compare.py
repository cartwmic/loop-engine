"""Paired comparison of single-trial campaign summaries (G7).

`compare(paths, out)` reads each summary.json and never writes to it. A labelled summary
(settings.target_options.block, keys BLOCK_KEYS, optional attempt_reason) holds one trial that
belongs to the block named by its task and repetition, under the arm, realised order and attempt
the label records. An unlabelled summary, such as Tier 0, is shown under its case and combination.
The report pair (summary.json and report.md) is written to a new timestamped directory under `out`.
A missing usage, price or metric is "unavailable", never zero.

Target metrics read here, written by the software-change target when an arm is set:
  target_metrics.driver_elapsed_seconds   driver-only elapsed seconds (monotonic clock at the driver boundary)
  target_metrics.owner_interaction        {question_words, answer_words}, numbers only
  target_metrics.outcome_grade            {intent_faithful: true|false|null, unstable: true|false}
Owner question counts come from target_metrics.discipline.owner_question_count, which older summaries carry too.
"""
import json
import shutil
from collections.abc import Iterable
from pathlib import Path

from evalcore.report import protected_values, scan_results, utc_timestamp
from evalcore.settings import OVERHEAD_ROLES, WORKFLOW_ROLES

UNAVAILABLE = "unavailable"
BLOCK_KEYS = ("task", "repetition", "arm", "order", "attempt")
DRIVER_SECONDS_KEY = "driver_elapsed_seconds"
OWNER_INTERACTION_KEY = "owner_interaction"
OUTCOME_GRADE_KEY = "outcome_grade"

NOTES = (
    "Dollar figures are estimated from each campaign's recorded price table. \"unavailable\" means the value "
    "was not recorded, or a role used by the trial is unpriced; it is never zero.",
    "Workflow cost and tokens cover the driver, implementer and reviewer roles. Evaluation overhead (simulator "
    "and judge, which includes the intent grader) is shown apart and is never part of workflow cost.",
    "Driver wall s is the target-recorded driver-only elapsed time. Trial wall s is the whole trial (setup, "
    "driver, capture and scoring) and is shown beside it, not instead of it.",
    "Limit, infrastructure, resumed and rerun trials stay listed. A rerun is a separate attempt beside the "
    "original and never replaces it.",
    "Intent is the independent grader's verdict; an unstable verdict counts as not faithful. Campaigns without "
    "a grader verdict show unavailable.",
)


def _number(value):
    """A recorded number, or None. Booleans are not numbers here."""
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def load_summary(path: Path) -> dict:
    summary = json.loads(path.read_text())
    if summary.get("schema_version") != 1 or not isinstance(summary.get("trials"), list):
        raise ValueError(f"{path}: not a campaign summary.json (schema_version 1 with a trials list)")
    return summary


def block_label(summary: dict) -> dict | None:
    """The recorded target_options.block label, validated; None when the campaign is unlabelled."""
    campaign = summary["campaign"]
    options = (summary.get("settings") or {}).get("target_options") or {}
    if "block" not in options:
        return None
    label = options["block"]
    if not isinstance(label, dict) or not set(BLOCK_KEYS) <= set(label) <= set(BLOCK_KEYS) | {"attempt_reason"}:
        raise ValueError(f"{campaign}: target_options.block needs {', '.join(BLOCK_KEYS)} and may add attempt_reason")
    if not (isinstance(label["task"], str) and label["task"]) or not (isinstance(label["arm"], str) and label["arm"]):
        raise ValueError(f"{campaign}: block task and arm must be non-empty strings")
    for key in ("repetition", "order", "attempt"):
        if not (isinstance(label[key], int) and not isinstance(label[key], bool) and label[key] >= 1):
            raise ValueError(f"{campaign}: block {key} must be an integer of at least 1")
    reason = label.get("attempt_reason")
    if reason is not None and not isinstance(reason, str):
        raise ValueError(f"{campaign}: block attempt_reason must be a string")
    if label["attempt"] > 1 and not (reason or "").strip():
        raise ValueError(f"{campaign}: attempt {label['attempt']} needs a recorded attempt_reason")
    if len(summary["trials"]) != 1:
        raise ValueError(f"{campaign}: a labelled campaign must hold exactly one trial")
    return label


def _role_total(trial: dict, roles: tuple[str, ...], value_of) -> float | None:
    """Sum over the roles the trial used; None when none were used, any is unknown or any value is missing."""
    usage = trial.get("usage_by_role") or {}
    used = [role for role in roles if role in usage]
    unknown = set(trial.get("usage_unknown_roles") or [])
    if not used or unknown.intersection(used):
        return None
    values = [value_of(role) for role in used]
    return None if any(value is None for value in values) else sum(values)


def _usd(trial: dict, roles: tuple[str, ...]) -> float | None:
    priced = trial.get("priced_usd_by_role")
    if not isinstance(priced, dict):
        return None
    return _role_total(trial, roles, lambda role: _number(priced.get(role)))


def _tokens(trial: dict, roles: tuple[str, ...]) -> int | None:
    usage = trial.get("usage_by_role") or {}
    return _role_total(trial, roles, lambda role: _number((usage.get(role) or {}).get("total_tokens")))


def _ratio(value) -> dict | None:
    if not isinstance(value, dict):
        return None
    numerator, denominator = _number(value.get("numerator")), _number(value.get("denominator"))
    if numerator is None or not denominator:
        return None
    return {"numerator": numerator, "denominator": denominator}


def _hidden_tests(value) -> dict | None:
    if not isinstance(value, dict) or not isinstance(value.get("passed"), bool):
        return None
    passed_tests, total_tests = _number(value.get("passed_tests")), _number(value.get("total_tests"))
    if passed_tests is None or total_tests is None:
        return None
    return {"passed": value["passed"], "passed_tests": passed_tests, "total_tests": total_tests}


def _intent(grade) -> str | None:
    if not isinstance(grade, dict):
        return None
    if grade.get("unstable") is True:
        return "unstable"
    if grade.get("intent_faithful") is True:
        return "faithful"
    if grade.get("intent_faithful") is False:
        return "not faithful"
    return None


def trial_row(summary: dict, trial: dict, path: Path, label: dict | None) -> dict:
    metrics = trial.get("target_metrics") or {}
    if label is None:
        identity = {"task": trial["case_id"], "repetition": trial["trial"], "arm": trial["combination_id"],
                    "order": None, "attempt": 1, "attempt_reason": None}
    else:
        identity = {key: label[key] for key in BLOCK_KEYS} | {"attempt_reason": label.get("attempt_reason")}
    owner = metrics.get(OWNER_INTERACTION_KEY)
    owner = owner if isinstance(owner, dict) else {}
    discipline = metrics.get("discipline") or {}
    return identity | {
        "labelled": label is not None,
        "campaign": summary["campaign"],
        "source": str(path),
        "case_id": trial["case_id"],
        "combination_id": trial["combination_id"],
        "trial": trial["trial"],
        "outcome": trial.get("outcome"),
        "reason": trial.get("reason"),
        "terminal": trial.get("terminal"),
        "limit_step": trial.get("limit_step"),
        "resumed": bool(trial.get("resumed")),
        "segments": len(trial.get("segments") or []),
        "carried_from": trial.get("carried_from"),
        "hidden_tests": _hidden_tests(metrics.get("hidden_test_outcome")),
        "delivery": _ratio(metrics.get("delivery_recall")),
        "elicitation": _ratio(metrics.get("elicitation_recall")),
        "intent": _intent(metrics.get(OUTCOME_GRADE_KEY)),
        "workflow_usd": _usd(trial, WORKFLOW_ROLES),
        "workflow_tokens": _tokens(trial, WORKFLOW_ROLES),
        "overhead_usd": _usd(trial, OVERHEAD_ROLES),
        "overhead_tokens": _tokens(trial, OVERHEAD_ROLES),
        "driver_seconds": _number(metrics.get(DRIVER_SECONDS_KEY)),
        "trial_wall_seconds": _number(trial.get("wall_seconds")),
        "owner_questions": _number(discipline.get("owner_question_count")),
        "owner_question_words": _number(owner.get("question_words")),
        "owner_answer_words": _number(owner.get("answer_words")),
    }


def group_blocks(rows: list[dict]) -> list[dict]:
    """Blocks by (task, repetition); attempts of one arm sit together, original first. Duplicate attempts are refused."""
    identities = set()
    blocks: dict[tuple, list[dict]] = {}
    for row in rows:
        identity = (row["task"], row["repetition"], row["arm"], row["attempt"])
        if identity in identities:
            raise ValueError(f"duplicate attempt {identity} in {row['source']}; each block arm attempt must be unique")
        identities.add(identity)
        blocks.setdefault((row["task"], row["repetition"]), []).append(row)
    result = []
    for (task, repetition), members in sorted(blocks.items()):
        members.sort(key=lambda r: (r["order"] is None, r["order"] or 0, r["arm"], r["attempt"]))
        realised = []
        for row in members:
            entry = {"order": row["order"], "arm": row["arm"]}
            if entry not in realised:
                realised.append(entry)
        result.append({"task": task, "repetition": repetition, "realised_order": realised, "attempts": members})
    return result


def _cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def _show(value, render) -> str:
    return UNAVAILABLE if value is None else render(value)


def _tests_text(row: dict) -> str:
    tests = row["hidden_tests"]
    if tests is None:
        return UNAVAILABLE
    state = "pass" if tests["passed"] else "fail"
    return f"{state} {tests['passed_tests']:g}/{tests['total_tests']:g}"


def _ratio_text(ratio: dict | None) -> str:
    return UNAVAILABLE if ratio is None else f"{ratio['numerator']:g}/{ratio['denominator']:g}"


def _notes(row: dict) -> str:
    notes = []
    if not row["labelled"]:
        notes.append(f"unlabelled: case {row['case_id']}, trial {row['trial']}")
    if row["attempt"] > 1:
        notes.append(f"attempt {row['attempt']}: {row['attempt_reason']}")
    if row["outcome"] == "limit":
        notes.append("limit" + (f" at {row['limit_step']}" if row["limit_step"] else ""))
    if row["outcome"] == "infrastructure":
        notes.append("infrastructure, excluded from success rate")
    if row["resumed"]:
        notes.append(f"resumed, {row['segments']} segments")
    if row["carried_from"]:
        notes.append(f"carried from {row['carried_from']}")
    if row["terminal"] is False:
        notes.append("not terminal")
    return "; ".join(notes)


def render_markdown(report: dict) -> str:
    lines = ["# Paired comparison", "", "Sources:", ""]
    lines += [f"- {source['campaign']}: {source['summary']}" for source in report["sources"]]
    lines += ["", *NOTES, ""]
    header = ("| Arm | Order | Attempt | Outcome | Hidden tests | Delivery | Elicitation | Intent | Workflow cost "
              "| Workflow tokens | Overhead cost | Overhead tokens | Driver wall s | Trial wall s | Owner questions "
              "| Owner words (questions / answers) | Notes |")
    for block in report["blocks"]:
        order = ", ".join(f"{e['order']}: {e['arm']}" for e in block["realised_order"] if e["order"] is not None)
        order = order or "not recorded (unlabelled campaigns)"
        lines += [f"## Task {block['task']}, repetition {block['repetition']}", "",
                  f"Realised order: {order}", "", header, "|" + "---|" * 17]
        for row in block["attempts"]:
            cells = [
                row["arm"],
                "-" if row["order"] is None else str(row["order"]),
                str(row["attempt"]),
                f"{row['outcome']} ({row['reason']})",
                _tests_text(row),
                _ratio_text(row["delivery"]),
                _ratio_text(row["elicitation"]),
                _show(row["intent"], str),
                _show(row["workflow_usd"], lambda x: f"${x:.4f}"),
                _show(row["workflow_tokens"], lambda x: f"{x:,}"),
                _show(row["overhead_usd"], lambda x: f"${x:.4f}"),
                _show(row["overhead_tokens"], lambda x: f"{x:,}"),
                _show(row["driver_seconds"], lambda x: f"{x:.1f}"),
                _show(row["trial_wall_seconds"], lambda x: f"{x:.1f}"),
                _show(row["owner_questions"], lambda x: f"{x:g}"),
                f"{_show(row['owner_question_words'], lambda x: f'{x:g}')} / "
                f"{_show(row['owner_answer_words'], lambda x: f'{x:g}')}",
                _notes(row) or "-",
            ]
            lines.append("| " + " | ".join(_cell(cell) for cell in cells) + " |")
        lines.append("")
    return "\n".join(lines)


def _reserve_directory(out: Path, sources: list[Path]) -> Path:
    """A fresh timestamped directory under out. Checked before anything is created, so an input tree is never touched."""
    while True:
        destination = out / utc_timestamp()
        if any(destination.resolve().is_relative_to(source.parent.resolve()) for source in sources):
            raise ValueError("report directory would be inside an input campaign directory; choose another output root")
        try:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.mkdir()
            return destination
        except FileExistsError:
            continue


def compare(paths: Iterable[Path | str], out: Path | str) -> Path:
    """Write one new paired report pair (summary.json, report.md) under out and return its directory."""
    sources = [Path(path) for path in paths]
    if not sources:
        raise ValueError("compare needs at least one summary.json")
    loaded = [(path, load_summary(path)) for path in sources]
    rows = []
    for path, summary in loaded:
        label = block_label(summary)
        rows += [trial_row(summary, trial, path, label) for trial in summary["trials"]]
    report = {"schema_version": 1, "report": "paired-comparison",
              "sources": [{"campaign": summary["campaign"], "summary": str(path)} for path, summary in loaded],
              "blocks": group_blocks(rows)}
    json_text = json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
    markdown = render_markdown(report)
    configured = [p for _, summary in loaded for p in (summary.get("settings") or {}).get("provider_config_paths") or []]
    provider_configs = [Path(p) for p in configured if Path(p).is_file()]
    scan_results((json_text, markdown), protected_values(provider_config_paths=provider_configs))
    destination = _reserve_directory(Path(out), sources)
    try:
        (destination / "summary.json").write_text(json_text)
        (destination / "report.md").write_text(markdown)
    except BaseException:
        shutil.rmtree(destination)
        raise
    return destination
