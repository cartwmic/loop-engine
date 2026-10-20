"""Paired comparison of single-trial campaign summaries (G7).

`compare(paths, out, calibration=None)` reads each summary.json and never writes to it. A labelled summary
(settings.target_options.block, keys BLOCK_KEYS, optional attempt_reason) holds one trial that
belongs to the block named by its task and repetition, under the arm, realised order and attempt
the label records. An unlabelled summary, such as Tier 0, is shown under its case and combination.
The report pair (summary.json and report.md) is written to a new timestamped directory under `out`.
A missing usage, price or metric is "unavailable", never zero. A resumed or rescored trial has no exact per-call
dollars, so its dollars are an upper bound from its recorded usage at the highest-tier price, labelled by
`workflow_cost_basis` and `overhead_cost_basis`.

The calibration receipt (grader-calibrate) decides how intent verdicts are shown. Only a receipt whose mode is
"normal" and whose grader_model is the campaign's own grader (settings.judge_model) shows them as the grader gave
them; otherwise each is labelled "diagnostic-only", campaign by campaign. A receipt without a grader_model is not
normal. A missing or unreadable receipt is diagnostic-only, never an error.

Each block with a native trial also shows paired deltas: each non-native attempt minus the native reference (the
original native attempt). They are descriptive differences, not significance tests.

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

from evalcore.accounting import usage_cost_usd
from evalcore.report import protected_values, scan_results, utc_timestamp
from evalcore.settings import OVERHEAD_ROLES, WORKFLOW_ROLES, Price

UNAVAILABLE = "unavailable"
EXACT = "exact"
UPPER_BOUND = "upper-bound"
BLOCK_KEYS = ("task", "repetition", "arm", "order", "attempt")
DRIVER_SECONDS_KEY = "driver_elapsed_seconds"
OWNER_INTERACTION_KEY = "owner_interaction"
OUTCOME_GRADE_KEY = "outcome_grade"
NATIVE_ARM = "native"
CALIBRATION_NORMAL = "normal"
CALIBRATION_DIAGNOSTIC = "diagnostic-only"
CALIBRATION_NOT_SUPPLIED = "not supplied"
UNREADABLE_RECEIPT = "receipt unreadable"
NO_GRADER_RECEIPT = "receipt has no grader model"
DELTA_FIELDS = ("workflow_tokens", "trial_wall_seconds", "driver_seconds")

NOTES = (
    "Dollar figures are estimated from each campaign's recorded price table. \"unavailable\" means the value "
    "was not recorded, or a role used by the trial is unpriced; it is never zero.",
    "\"<=\" marks an upper bound: a resumed or rescored trial has no exact per-call dollars, so its recorded usage is "
    "priced at the model's highest-tier rate. A paired delta with an upper-bound arm side over an exact native side is "
    "labelled upper-bound.",
    "Workflow cost and tokens cover the driver, implementer and reviewer roles. Evaluation overhead (simulator "
    "and judge, which includes the intent grader) is shown apart and is never part of workflow cost.",
    "Driver wall s is the target-recorded driver-only elapsed time. Trial wall s is the whole trial (setup, "
    "driver, capture and scoring) and is shown beside it, not instead of it.",
    "Limit, infrastructure, resumed and rerun trials stay listed. A rerun is a separate attempt beside the "
    "original and never replaces it.",
    "Intent is the independent grader's verdict; an unstable verdict counts as not faithful. Intent is labelled "
    "diagnostic-only unless the calibration receipt is normal for that campaign's own grader. Campaigns without a "
    "grader verdict show unavailable.",
    "Paired deltas are each non-native attempt minus the block's native trial (arm minus native). A delta is "
    "unavailable when either side is unavailable, never zero. A workflow-cost delta is also unavailable when the "
    "native side is an upper bound. Three repetitions make no significance claim.",
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


def _used_roles(trial: dict, roles: tuple[str, ...]) -> list[str] | None:
    """The roles of a group that the trial used; None when none were used or any role of the group is unknown.

    A role listed in usage_unknown_roles is unknown even when usage_by_role omits it, so its whole group is unavailable.
    """
    usage = trial.get("usage_by_role") or {}
    used = [role for role in roles if role in usage]
    unknown = set(trial.get("usage_unknown_roles") or [])
    if not used or unknown.intersection(roles):
        return None
    return used


def _role_total(trial: dict, roles: tuple[str, ...], value_of) -> float | None:
    """Sum over the roles the trial used; None when none were used, any is unknown or any value is missing."""
    used = _used_roles(trial, roles)
    if used is None:
        return None
    values = [value_of(role) for role in used]
    return None if any(value is None for value in values) else sum(values)


def _usd(trial: dict, roles: tuple[str, ...], role_models: dict, prices: dict[str, Price]) -> tuple[float | None, str]:
    """A role group's dollars and their basis.

    Each used role takes its exact per-call figure when the target recorded one. Otherwise (a resumed or rescored trial)
    it is an upper bound: its recorded usage at its model's highest-tier price. A role with neither is unavailable,
    which makes the group unavailable, never zero.
    """
    used = _used_roles(trial, roles)
    if used is None:
        return None, UNAVAILABLE
    priced = trial.get("priced_usd_by_role")
    priced = priced if isinstance(priced, dict) else {}
    usage = trial.get("usage_by_role") or {}
    total, basis = 0.0, EXACT
    for role in used:
        exact = _number(priced.get(role))
        if exact is not None:
            total += exact
            continue
        counts, price = usage[role], prices.get(role_models.get(role, ""))
        # No price, or no recorded input and output counts: an upper bound from them would read as zero.
        if price is None or _number(counts.get("input_tokens")) is None or _number(counts.get("output_tokens")) is None:
            return None, UNAVAILABLE
        total += usage_cost_usd(counts, price)
        basis = UPPER_BOUND
    return total, basis


def _role_models(summary: dict, combination_id: str) -> dict:
    """The role-to-model map cost_by_role priced the combination with, as the summary recorded it in comparisons."""
    for comparison in summary.get("comparisons") or []:
        if comparison.get("id") == combination_id:
            return comparison.get("role_models") or {}
    return {}


def _campaign_prices(summary: dict) -> dict[str, Price]:
    """The campaign's recorded price table, validated as the campaign validated it."""
    recorded = (summary.get("settings") or {}).get("prices") or {}
    return {model: Price.model_validate(price) for model, price in recorded.items()}


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


def _intent(grade, calibrated: bool) -> str | None:
    """The grader's verdict; labelled diagnostic-only unless the calibration receipt is normal. No verdict is unavailable."""
    if not isinstance(grade, dict):
        return None
    if grade.get("unstable") is True:
        verdict = "unstable"
    elif grade.get("intent_faithful") is True:
        verdict = "faithful"
    elif grade.get("intent_faithful") is False:
        verdict = "not faithful"
    else:
        return None
    return verdict if calibrated else f"{verdict} ({CALIBRATION_DIAGNOSTIC})"


def read_calibration(path: Path | str | None) -> dict:
    """The calibration state the report states: mode normal, diagnostic-only or not supplied, with reasons.

    A normal receipt names the grader it calibrated (grader_model); one without a grader model is diagnostic-only.
    A missing or unreadable receipt, or one whose mode is neither normal nor diagnostic-only, is diagnostic-only.
    """
    if path is None:
        return {"mode": CALIBRATION_NOT_SUPPLIED, "reasons": []}
    try:
        receipt = json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return {"mode": CALIBRATION_DIAGNOSTIC, "reasons": [UNREADABLE_RECEIPT]}
    mode = receipt.get("mode") if isinstance(receipt, dict) else None
    if mode == CALIBRATION_NORMAL:
        grader = receipt.get("grader_model")
        if isinstance(grader, str) and grader:
            return {"mode": mode, "reasons": [], "grader_model": grader}
        return {"mode": CALIBRATION_DIAGNOSTIC, "reasons": [NO_GRADER_RECEIPT]}
    if mode == CALIBRATION_DIAGNOSTIC:
        reasons = receipt.get("reasons")
        return {"mode": mode, "reasons": [r for r in reasons if isinstance(r, str)] if isinstance(reasons, list) else []}
    return {"mode": CALIBRATION_DIAGNOSTIC, "reasons": [UNREADABLE_RECEIPT]}


def trial_row(summary: dict, trial: dict, path: Path, label: dict | None, calibrated: bool) -> dict:
    metrics = trial.get("target_metrics") or {}
    if label is None:
        identity = {"task": trial["case_id"], "repetition": trial["trial"], "arm": trial["combination_id"],
                    "order": None, "attempt": 1, "attempt_reason": None}
    else:
        identity = {key: label[key] for key in BLOCK_KEYS} | {"attempt_reason": label.get("attempt_reason")}
    owner = metrics.get(OWNER_INTERACTION_KEY)
    owner = owner if isinstance(owner, dict) else {}
    discipline = metrics.get("discipline") or {}
    models = _role_models(summary, trial["combination_id"])
    prices = _campaign_prices(summary)
    workflow_usd, workflow_basis = _usd(trial, WORKFLOW_ROLES, models, prices)
    overhead_usd, overhead_basis = _usd(trial, OVERHEAD_ROLES, models, prices)
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
        "intent": _intent(metrics.get(OUTCOME_GRADE_KEY), calibrated),
        "workflow_usd": workflow_usd,
        "workflow_cost_basis": workflow_basis,
        "workflow_tokens": _tokens(trial, WORKFLOW_ROLES),
        "overhead_usd": overhead_usd,
        "overhead_cost_basis": overhead_basis,
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
        result.append({"task": task, "repetition": repetition, "realised_order": realised, "attempts": members,
                       "paired_deltas": paired_deltas(members)})
    return result


def _difference(value, reference):
    """arm minus native; unavailable (None) when either side is unavailable, never zero."""
    return None if value is None or reference is None else value - reference


def paired_deltas(members: list[dict]) -> dict:
    """Each non-native attempt minus the block's native reference, the original (lowest) native attempt.

    A block without a native trial has no reference and no deltas. Values are null where unavailable.
    """
    natives = [row for row in members if row["arm"] == NATIVE_ARM]
    if not natives:
        return {"reference": None, "deltas": []}
    reference = min(natives, key=lambda row: row["attempt"])
    deltas = [{"arm": row["arm"], "attempt": row["attempt"]} |
              {field: _difference(row[field], reference[field]) for field in DELTA_FIELDS} |
              _usd_delta(row, reference)
              for row in members if row["arm"] != NATIVE_ARM]
    return {"reference": {"arm": NATIVE_ARM, "attempt": reference["attempt"]}, "deltas": deltas}


def _cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def _show(value, render) -> str:
    return UNAVAILABLE if value is None else render(value)


def _usd_delta(row: dict, reference: dict) -> dict:
    """arm minus native workflow dollars, with their basis. An upper-bound native side may sit above its true cost,
    so the difference bounds nothing and is unavailable. An upper-bound arm over an exact native side bounds the
    difference from above."""
    usd = _difference(row["workflow_usd"], reference["workflow_usd"])
    if usd is None or reference["workflow_cost_basis"] != EXACT:
        return {"workflow_usd": None, "workflow_cost_basis": UNAVAILABLE}
    return {"workflow_usd": usd, "workflow_cost_basis": row["workflow_cost_basis"]}


def _usd_text(value: float | None, basis: str, render) -> str:
    """A dollar figure marked by its basis: "<=" for an upper bound. Unavailable stays unavailable, never zero."""
    if value is None or basis == UNAVAILABLE:
        return UNAVAILABLE
    return f"<={render(value)}" if basis == UPPER_BOUND else render(value)


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


def _signed_usd(value: float) -> str:
    return f"{'-' if value < 0 else '+'}${abs(value):.4f}"


def _delta_usd_text(delta: dict) -> str:
    """A workflow-cost delta; an upper-bound delta is labelled as one, unavailable stays unavailable."""
    if delta["workflow_usd"] is None:
        return UNAVAILABLE
    text = _signed_usd(delta["workflow_usd"])
    return f"{text} ({UPPER_BOUND})" if delta["workflow_cost_basis"] == UPPER_BOUND else text


def campaign_grader(summary: dict) -> str | None:
    """The judge model the campaign's grader ran with, as its summary records it."""
    return (summary.get("settings") or {}).get("judge_model")


def intent_calibrated(calibration: dict, summary: dict) -> bool:
    """A campaign's intent verdicts are normal only under a normal receipt for that campaign's own grader."""
    return calibration["mode"] == CALIBRATION_NORMAL and calibration["grader_model"] == campaign_grader(summary)


def intent_calibration(calibration: dict, summary: dict) -> str:
    """One line for a campaign: which receipt applies to its intent verdicts, and why they are diagnostic-only."""
    if calibration["mode"] == CALIBRATION_NORMAL:
        verdict = "intent as graded" if intent_calibrated(calibration, summary) else "diagnostic-only"
        grader = campaign_grader(summary) or "not recorded"
        return (f"receipt normal for grader {calibration['grader_model']}; this campaign's grader is {grader}: "
                f"{verdict}.")
    if calibration["mode"] == CALIBRATION_NOT_SUPPLIED:
        return "no calibration receipt supplied: intent is diagnostic-only."
    reasons = "; ".join(calibration["reasons"]) or "no reason recorded"
    return f"receipt diagnostic-only (reason: {reasons}): intent is diagnostic-only."


def calibration_statement(calibration: dict) -> str:
    """One sentence on how intent verdicts are to be read: the receipt's mode, with its reasons when diagnostic-only."""
    if calibration["mode"] == CALIBRATION_NORMAL:
        return (f"Calibration receipt: normal. It calibrates grader {calibration['grader_model']} only: intent "
                "verdicts are shown as the grader gave them in campaigns whose judge model is that grader, and are "
                "diagnostic-only in the others (see each campaign below).")
    if calibration["mode"] == CALIBRATION_NOT_SUPPLIED:
        return ("Calibration receipt: not supplied. Every intent verdict is diagnostic-only and is excluded from "
                "decisions.")
    reasons = "; ".join(calibration["reasons"]) or "no reason recorded"
    return (f"Calibration receipt: diagnostic-only (reason: {reasons}). Every intent verdict is diagnostic-only and is "
            "excluded from decisions.")


def delta_lines(block: dict) -> list[str]:
    """The paired-delta table of one block, or a line saying why it has none."""
    deltas = block["paired_deltas"]
    if deltas["reference"] is None:
        return ["No native trial in this block, so no paired deltas.", ""]
    lines = ["### Paired deltas vs native (arm minus native)", "",
             f"Native reference: attempt {deltas['reference']['attempt']}. A delta is unavailable where either side "
             "is unavailable, and a workflow-cost delta where the native side is an upper bound.", ""]
    if not deltas["deltas"]:
        return lines + ["No non-native arm trials in this block.", ""]
    lines += ["| Arm | Attempt | Workflow cost | Workflow tokens | Trial wall s | Driver wall s |", "|---|---|---|---|---|---|"]
    for delta in deltas["deltas"]:
        cells = [delta["arm"], str(delta["attempt"]),
                 _delta_usd_text(delta),
                 _show(delta["workflow_tokens"], lambda x: f"{x:+,}"),
                 _show(delta["trial_wall_seconds"], lambda x: f"{x:+.1f}"),
                 _show(delta["driver_seconds"], lambda x: f"{x:+.1f}")]
        lines.append("| " + " | ".join(_cell(cell) for cell in cells) + " |")
    return lines + [""]


def render_markdown(report: dict) -> str:
    lines = ["# Paired comparison", "", "Sources:", ""]
    lines += [f"- {source['campaign']}: {source['summary']}" for source in report["sources"]]
    lines += ["", *NOTES, "", calibration_statement(report["calibration"]), ""]
    lines += ["Intent calibration by campaign:", ""]
    lines += [f"- {source['campaign']}: {source['intent_calibration']}" for source in report["sources"]]
    lines += [""]
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
                _usd_text(row["workflow_usd"], row["workflow_cost_basis"], lambda x: f"${x:.4f}"),
                _show(row["workflow_tokens"], lambda x: f"{x:,}"),
                _usd_text(row["overhead_usd"], row["overhead_cost_basis"], lambda x: f"${x:.4f}"),
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
        lines += delta_lines(block)
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


def compare(paths: Iterable[Path | str], out: Path | str, calibration: Path | str | None = None) -> Path:
    """Write one new paired report pair (summary.json, report.md) under out and return its directory.

    `calibration` is an optional grader-calibrate receipt. Its intent verdicts are normal only for the campaigns whose
    grader is the receipt's grader_model; the others are diagnostic-only.
    """
    sources = [Path(path) for path in paths]
    if not sources:
        raise ValueError("compare needs at least one summary.json")
    loaded = [(path, load_summary(path)) for path in sources]
    state = read_calibration(calibration)
    rows, source_records = [], []
    for path, summary in loaded:
        label = block_label(summary)
        calibrated = intent_calibrated(state, summary)
        rows += [trial_row(summary, trial, path, label, calibrated) for trial in summary["trials"]]
        source_records.append({"campaign": summary["campaign"], "summary": str(path),
                               "intent_calibration": intent_calibration(state, summary)})
    report = {"schema_version": 1, "report": "paired-comparison", "sources": source_records,
              "calibration": state, "blocks": group_blocks(rows)}
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
