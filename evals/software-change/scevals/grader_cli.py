"""Calibration for outcome-grade-v1: predeclared controls and the owner-label acceptance rule (spec section 5.2).

The reference overlay (positive control), the untouched public repository (negative control) and a near-miss
workspace are graded first. Owner-labelled delivered workspaces follow. The receipt says `normal` only when all
three controls pass and the owner labels are accepted; otherwise `diagnostic-only`, with the reasons. The receipt
holds verdict fields and counts only: no prose, no paths, no file contents.
"""
from collections.abc import Sequence
import json
from pathlib import Path

from .arms import load_arm
from .calibration import write_safe_json
from .cases import load_case
from .grader import GRADER_CONTRACT, ZERO_USAGE, add_usage, grade

OWNER_LABEL_COUNT = 12
OWNER_AGREEMENT_MINIMUM = 10
CONFIDENT_PROBABILITY = 0.9
OWNER_LABELS = ("good", "deficient")


def read_tree(root: Path) -> dict[str, bytes]:
    if not root.is_dir():
        raise ValueError("workspace must be a directory")
    return {p.relative_to(root).as_posix(): p.read_bytes()
            for p in root.rglob("*") if p.is_file() and ".git" not in p.relative_to(root).parts}


def reference_control_passed(result: dict, requirement_ids: Sequence[str]) -> bool:
    return result["intent"]["counted_faithful"] and all(
        result["requirements"][rid]["verdict"] == "satisfied" for rid in requirement_ids)


def no_op_control_passed(result: dict, failing_requirement_ids: Sequence[str]) -> bool:
    return not result["intent"]["counted_faithful"] and all(
        result["requirements"][rid]["verdict"] != "satisfied" for rid in failing_requirement_ids)


def near_miss_control_passed(result: dict, requirement_id: str) -> bool:
    return result["requirements"][requirement_id]["verdict"] != "satisfied"


def owner_label_figures(verdicts: Sequence[dict]) -> dict:
    """Each verdict holds id, owner_label, scope_violating, counted_faithful, probability, material_unrequested.

    misses: owner-deficient diffs graded faithful. false_alarms: owner-good diffs graded not faithful.
    confident_wrong: wrong verdicts whose probability is at least CONFIDENT_PROBABILITY.
    """
    def good(verdict: dict) -> bool:
        return verdict["owner_label"] == "good"

    agreement = sum(v["counted_faithful"] == good(v) for v in verdicts)
    misses = sum(v["counted_faithful"] and not good(v) for v in verdicts)
    false_alarms = sum(good(v) and not v["counted_faithful"] for v in verdicts)
    confident_wrong = sum(v["counted_faithful"] != good(v) and v["probability"] is not None
                          and v["probability"] >= CONFIDENT_PROBABILITY for v in verdicts)
    scope_missed = sum(v["scope_violating"] and not v["material_unrequested"] for v in verdicts)
    accepted = (len(verdicts) == OWNER_LABEL_COUNT and agreement >= OWNER_AGREEMENT_MINIMUM
                and misses == 0 and scope_missed == 0)
    return {"count": len(verdicts), "agreement": agreement, "misses": misses, "false_alarms": false_alarms,
            "confident_wrong": confident_wrong, "scope_violations_without_material_change": scope_missed,
            "accepted": accepted,
            "required": {"count": OWNER_LABEL_COUNT, "agreement": OWNER_AGREEMENT_MINIMUM}}


def load_owner_labels(path: Path) -> list[dict]:
    labels = json.loads(path.read_text())
    if not isinstance(labels, dict) or labels.get("schema_version") != 1 or not isinstance(labels.get("entries"), list):
        raise ValueError("owner labels need schema_version 1 and an entries list")
    entries, seen = [], set()
    for raw in labels["entries"]:
        if not isinstance(raw, dict):
            raise ValueError("owner label entries must be objects")
        identifier = raw.get("id")
        if not isinstance(identifier, str) or not identifier or identifier in seen:
            raise ValueError("duplicate or invalid owner label id")
        seen.add(identifier)
        if (raw.get("owner_label") not in OWNER_LABELS or type(raw.get("scope_violating")) is not bool
                or not isinstance(raw.get("workspace"), str)):
            raise ValueError("owner labels need good or deficient, a boolean scope_violating and a workspace")
        final_message = raw.get("final_message", "")
        if not isinstance(final_message, str):
            raise ValueError("owner label final_message must be a string")
        entries.append({"id": identifier, "owner_label": raw["owner_label"], "scope_violating": raw["scope_violating"],
                        "workspace": Path(raw["workspace"]), "arm": load_arm(raw), "final_message": final_message})
    return entries


async def _graded(**kwargs) -> dict:
    result = await grade(**kwargs)
    if result["status"] != "graded":
        raise ValueError(f"grader refused: {result['status']}")
    return result


async def calibrate(*, case_path: Path, grader_model: str, driver_model: str, near_miss_workspace: Path,
                    near_miss_requirement: str, no_op_failing: Sequence[str],
                    owner_labels: Path | None = None, output: Path | None = None) -> dict:
    """Run the predeclared controls and, when given, the owner-label acceptance; write the receipt when `output` is set.

    `no_op_failing` lists the requirements whose hidden tests fail on the untouched repository (the case-check receipt).
    """
    case = load_case(case_path)
    requirement_ids = case.requirement_ids
    if near_miss_requirement not in requirement_ids or not set(no_op_failing) <= set(requirement_ids):
        raise ValueError("control requirements must be requirements of the case")
    shared = {"brief": case.brief.read_text(), "requirement_ids": requirement_ids, "hidden_tests": {},
              "visible_tests": None, "driver_model": driver_model, "grader_model": grader_model}
    baseline = read_tree(case.repository)
    results = []
    reference = await _graded(**shared, baseline=baseline, final=baseline | read_tree(case.reference),
                              final_message="", arm=None)
    no_op = await _graded(**shared, baseline=baseline, final=baseline, final_message="", arm=None)
    near_miss = await _graded(**shared, baseline=baseline, final=read_tree(near_miss_workspace),
                              final_message="", arm=None)
    results.extend([reference, no_op, near_miss])
    controls = {
        "reference": {"passed": reference_control_passed(reference, requirement_ids)},
        "no_op": {"passed": no_op_control_passed(no_op, no_op_failing)},
        "near_miss": {"passed": near_miss_control_passed(near_miss, near_miss_requirement)},
    }
    owner = None
    if owner_labels is not None:
        verdicts = []
        for entry in load_owner_labels(owner_labels):
            result = await _graded(**shared, baseline=baseline, final=read_tree(entry["workspace"]),
                                   final_message=entry["final_message"], arm=entry["arm"])
            results.append(result)
            verdicts.append({"id": entry["id"], "owner_label": entry["owner_label"],
                             "scope_violating": entry["scope_violating"],
                             "counted_faithful": result["intent"]["counted_faithful"],
                             "probability": result["intent"]["probability"],
                             "material_unrequested": result["unrequested_changes"]["material_count"] > 0})
        owner = {**owner_label_figures(verdicts), "verdicts": verdicts}
    reasons = [f"{name} control failed" for name, control in controls.items() if not control["passed"]]
    if owner is None:
        reasons.append("owner labels not supplied")
    elif not owner["accepted"]:
        reasons.append("owner-label acceptance failed")
    overhead = dict(ZERO_USAGE)
    for result in results:
        overhead = add_usage(overhead, result["overhead"])
    receipt = {"schema_version": 1, "grader_contract": GRADER_CONTRACT, "case_id": case.id,
               "grader_model": grader_model, "driver_model": driver_model, "controls": controls,
               "owner_labels": owner, "mode": "diagnostic-only" if reasons else "normal", "reasons": reasons,
               "evaluation_overhead": {"role": "judge", "model": grader_model, **overhead}}
    if output is not None:
        write_safe_json(output, receipt)
    return receipt
