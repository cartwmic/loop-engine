"""Read-only inspection of the preserved first pilot (AC-8 revision 9 owner-scoped addendum).

Reads only the preserved result files; makes no model call, starts no campaign and writes nothing.
It states what the preserved evidence shows per assignment and keeps the unresolved AC-8 unattended and
AC-9 container/copy gaps visible as gaps. It does not claim that every original trial finished unattended.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PILOT = ROOT / "results/pilot"
ORIGINAL = "20261011T003331.702469Z"
CONTINUATION = "20261011T030831.922694Z"
ASSIGNMENTS = [f"m{m}-c{c}-t1" for m in (0, 1) for c in (0, 1, 2)]


def digest():
    files = sorted(p for p in PILOT.rglob("*") if p.is_file())
    return {str(p.relative_to(PILOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}


def load(run):
    path = PILOT / run / "summary.json"
    assert path.is_file(), f"preserved pilot evidence missing: {path}"
    return json.loads(path.read_text())


def coverage_map(summary):
    rows = {}
    for trial in summary["trials"]:
        metrics = trial["target_metrics"]
        hidden = metrics.get("hidden_test_outcome") or {}
        rows[trial["sample_id"]] = {
            "model": trial["combination_id"], "case": trial["case_id"],
            "outcome": trial["outcome"], "reason": trial["reason"], "terminal": trial["terminal"],
            "lifecycle": metrics.get("lifecycle"),
            "unattended_finish": (metrics.get("run_health") or {}).get("unattended_finish"),
            "container_recorded": metrics.get("container") is not None,
            "hidden": (hidden.get("passed_tests"), hidden.get("total_tests")),
        }
    return rows


def gaps(rows):
    """AC-8 unattended evidence unknown, or AC-9 container configuration not recorded."""
    return {sid: {"ac8_unattended_unknown": r["unattended_finish"] is None, "ac9_container_unrecorded": not r["container_recorded"]}
            for sid, r in rows.items() if r["unattended_finish"] is None or not r["container_recorded"]}


def test_pilot_inspection_is_read_only_and_matches_the_owner_scoped_addendum():
    before = digest()
    assert before, "no preserved pilot files"
    original = load(ORIGINAL)
    continuation = load(CONTINUATION)
    rows = coverage_map(original)

    # Six-assignment coverage map, one row per assignment.
    assert sorted(rows) == sorted(ASSIGNMENTS)
    assert {r["model"] for r in rows.values()} == {"codex/gpt-6.1-sol", "pi/claude-compat/claude-opus-5-5"}

    # Matched case/model assignment: m0-* is one model, m1-* the other, and m0-cN / m1-cN run the same case at the same version.
    models = {0: "codex/gpt-6.1-sol", 1: "pi/claude-compat/claude-opus-5-5"}
    for sample_id, row in rows.items():
        assert row["model"] == models[int(sample_id[1])], sample_id
    matched = {t["sample_id"]: (t["combination_id"], t["case_id"], t["case_version"]) for t in original["matched_assignment"]}
    assert sorted(matched) == sorted(ASSIGNMENTS)
    assert {s: (r["model"], r["case"]) for s, r in rows.items()} == {s: (m[0], m[1]) for s, m in matched.items()}
    for c in (0, 1, 2):
        assert matched[f"m0-c{c}-t1"][1:] == matched[f"m1-c{c}-t1"][1:], c  # same case_id and case_version across the two models
    assert [matched[f"m0-c{c}-t1"][1] for c in (0, 1, 2)] == ["followup-import", "recap-preview", "review-policy"]

    # Each original terminal / no-human evidence state is reported as recorded, not upgraded.
    assert (rows["m0-c0-t1"]["outcome"], rows["m0-c0-t1"]["unattended_finish"], rows["m0-c0-t1"]["hidden"]) == ("success", True, (6, 6))
    assert (rows["m0-c1-t1"]["outcome"], rows["m0-c1-t1"]["lifecycle"], rows["m0-c1-t1"]["unattended_finish"]) == ("failure", "terminated", None)
    assert rows["m0-c1-t1"]["hidden"] == (6, 6)  # recovered 6/6 stays a recorded failure (known historical scoring defect)
    assert (rows["m0-c2-t1"]["outcome"], rows["m0-c2-t1"]["hidden"], rows["m0-c2-t1"]["unattended_finish"]) == ("failure", (15, 16), True)
    assert (rows["m1-c0-t1"]["outcome"], rows["m1-c0-t1"]["unattended_finish"]) == ("success", True)
    assert (rows["m1-c1-t1"]["outcome"], rows["m1-c1-t1"]["unattended_finish"]) == ("success", None)

    # m1-c2-t1 original stays an unscored, nonterminal infrastructure outcome.
    original_opus = rows["m1-c2-t1"]
    assert (original_opus["outcome"], original_opus["reason"], original_opus["terminal"]) == ("infrastructure", "infrastructure_error", False)
    assert original_opus["hidden"] == (None, None) and original_opus["unattended_finish"] is False

    # The unresolved AC-8 unattended and AC-9 container/copy gaps are listed, never met.
    assert gaps(rows) == {"m0-c1-t1": {"ac8_unattended_unknown": True, "ac9_container_unrecorded": True},
                          "m1-c1-t1": {"ac8_unattended_unknown": True, "ac9_container_unrecorded": True}}

    # Headline rates exclude the infrastructure assignment and are unchanged by the continuation.
    by_model = {c["id"]: c["core"] for c in original["comparisons"]}
    assert by_model["codex/gpt-6.1-sol"]["success_rate"]["numerator"] == 1 and by_model["codex/gpt-6.1-sol"]["success_rate"]["denominator"] == 3
    opus = by_model["pi/claude-compat/claude-opus-5-5"]
    assert opus["success_rate"]["numerator"] == 2 and opus["success_rate"]["denominator"] == 2
    assert opus["pass_k"]["denominator"] == 2
    assert original["resumed_trials"] == []

    # The owner-authorized continuation is a separate, labelled run: m1-c2-t1, terminal end/final, 15/16 failure.
    assert [t["sample_id"] for t in continuation["trials"]] == ["m1-c2-t1"]
    resumed = continuation["trials"][0]
    assert (resumed["outcome"], resumed["terminal"]) == ("failure", True)
    assert resumed["target_metrics"]["lifecycle"] == "final" and resumed["target_metrics"]["terminal_state"] == "end"
    assert (resumed["target_metrics"]["hidden_test_outcome"]["passed_tests"], resumed["target_metrics"]["hidden_test_outcome"]["total_tests"]) == (15, 16)
    assert resumed["target_metrics"]["resumed"] is True
    assert [t["sample_id"] for t in continuation["resumed_trials"]] == ["m1-c2-t1"]
    for comparison in continuation["comparisons"]:  # excluded from headline success rate and pass^k
        assert comparison["core"]["success_rate"]["denominator"] == 0 and comparison["core"]["pass_k"]["denominator"] == 0

    # Model/profile provenance: the continuation ran the same combination, profile and model settings as the original attempt.
    assert resumed["combination_id"] == original_opus["model"] == "pi/claude-compat/claude-opus-5-5"
    assert (resumed["case_id"], resumed["case_version"]) == ("review-policy", "2")
    for key in ("profile", "models", "model_config", "cases_path", "loop_engine_version", "provider_version"):
        assert continuation["settings"][key] == original["settings"][key], key
    assert continuation["settings"]["profile"] == "standard-14"
    assert continuation["settings"]["model_config"]["pi/claude-compat/claude-opus-5-5"] == {"reasoning_effort": "medium"}
    assert continuation["versions"]["target"] == original["versions"]["target"]

    # Execution-limit provenance: the carried original records keep their limits; the continuation disclosed the deadline-free settings.
    assert continuation["settings"]["idle_limit_seconds"] is None and continuation["settings"]["hard_limit_seconds"] is None
    authorization = original["execution_continuation"]["authorization"]
    assert authorization["kind"] == "disable-outer-deadlines" and "Michael" in authorization["owner"]
    assert authorization["original_limits"] == {"hard_limit_seconds": 28800, "idle_limit_seconds": 1800}
    assert sorted(t["sample_id"] for t in original["carried_trials"]) == ["m0-c0-t1", "m0-c1-t1", "m0-c2-t1", "m1-c1-t1"]

    # The written outcome states the same provenance, the human (owner) intervention and keeps the original infrastructure segment.
    note = (PILOT / ORIGINAL / "continuation-outcome.md").read_text()
    for phrase in ("remains an infrastructure outcome", "owner-authorized", "15/16", "excluded from headline success rate and pass^k",
                   "retained, not erased or relabelled", "differing execution limits",
                   "Opus 5.5 via claude-compat, medium thinking", "without changing the request, model or workflow policy",
                   "1800-second idle and 28800-second hard limits", "outer limits disabled", "Owner hold"):
        assert phrase in note, phrase
    # Attributed owner usability is NOT recorded in the preserved result files; the inspection does not assert or imply it.
    assert "usable" not in note.lower()

    assert digest() == before  # read-only
