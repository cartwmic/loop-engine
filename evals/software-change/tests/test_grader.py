"""AC-3 grader proofs with scripted judge models only: evidence exclusion, redaction and leakage, pointer verification,
null reasons, the unstable rule, same-family refusal, usage as overhead, claims, controls and the owner-label rule.
No real model is called; the scripted grader is reached through real Inspect model dispatch.
"""
import asyncio
import json
from pathlib import Path

import pytest
from inspect_ai.model import ModelAPI, ModelOutput, ModelUsage, modelapi

from evalcore.target import ModelError
from scevals.arms import load_arm
from scevals.grader import (CLAIM_INSTRUCTION, FIDELITY_INSTRUCTION, GRADER_CONTRACT, build_evidence,
                            claim_sentences, grade, provider_family, redact, resolves)
from scevals.grader_cli import calibrate, load_owner_labels, owner_label_figures

TARGET = Path(__file__).resolve().parents[1]
EXAMPLE = TARGET / "example-case"
GRADER = "grader-scripted/gpt-scripted-judge"
DRIVER = "driver-scripted/claude-scripted-driver"
USAGE = ModelUsage(input_tokens=10, output_tokens=5, total_tokens=15)
OPENSPEC = load_arm({"arm": {"id": "openspec", "driver_command": ["drive"], "prompt_template": "openspec",
                             "workflow_owned_paths": ["openspec/**", ".pi/**"], "completion_rule": "driver-exit"}})
BRIEF = "# Brief\n## REQ-1: Trim\nTrim labels.\n## REQ-2: Fold\nFold case.\n"
REQS = ("REQ-1", "REQ-2")
BASE = {"labels.py": b"def key(label):\n    return label\n", "notes.txt": b"untouched\n"}
FINAL = {"labels.py": b"def key(label):\n    return label.strip().casefold()\n", "notes.txt": b"untouched\n"}
MESSAGE = "Trimmed and folded labels."


class Script:
    """Answers each scripted grader call through `respond(kind, payload)` and keeps what it was sent."""

    def __init__(self):
        self.respond = None
        self.usage = USAGE
        self.calls = []


ACTIVE = []


@modelapi(name="grader-scripted")
class ScriptedGrader(ModelAPI):
    async def generate(self, input, tools, tool_choice, config):
        script = ACTIVE[-1]
        kind = {FIDELITY_INSTRUCTION: "fidelity", CLAIM_INSTRUCTION: "claim"}[input[0].text]
        payload = json.loads(input[1].text)
        script.calls.append({"kind": kind, "payload": payload, "sent": input[0].text + input[1].text})
        reply = script.respond(kind, payload)
        output = ModelOutput.from_content(model=self.model_name,
                                          content=reply if isinstance(reply, str) else json.dumps(reply))
        output.usage = script.usage
        return output


@pytest.fixture
def scripted():
    script = Script()
    ACTIVE.append(script)
    yield script
    ACTIVE.remove(script)


def queued(fidelity=(), claims=()):
    """Replies in order for each call kind; one fidelity reply per sample."""
    queues = {"fidelity": list(fidelity), "claim": list(claims)}
    return lambda kind, payload: queues[kind].pop(0)


def req(rid, verdict="satisfied", probability=0.9, pointer=None, reason="nothing in the repository addresses it"):
    return {"id": rid, "verdict": verdict, "probability": probability, "pointer": pointer,
            "null_reason": None if pointer is not None else reason}


def fidelity(requirements, *, faithful=True, probability=0.9, unrequested=()):
    return {"requirements": list(requirements), "unrequested_changes": list(unrequested),
            "intent_faithful": faithful, "intent_probability": probability}


def pointer_at(file, start, end, quote, source="final"):
    return {"source": source, "file": file, "start_line": start, "end_line": end, "quote": quote}


def run_grade(scripted, *, final=FINAL, baseline=BASE, message=MESSAGE, hidden=None, visible=None, arm=None,
              grader=GRADER, driver=DRIVER):
    return asyncio.run(grade(brief=BRIEF, requirement_ids=REQS, baseline=baseline, final=final,
                             final_message=message, hidden_tests=hidden or {}, visible_tests=visible, arm=arm,
                             driver_model=driver, grader_model=grader))


def test_workflow_owned_paths_leave_all_three_file_inputs(scripted):
    baseline = {"labels.py": b"A = 1\n", "openspec/changes/plan.md": b"PLAN-MARKER v1\n", ".pi/skill.md": b"PI-MARKER\n"}
    final = {"labels.py": b"A = 2\n", "openspec/changes/plan.md": b"PLAN-MARKER v2\n", ".pi/skill.md": b"PI-MARKER v2\n"}
    owned = build_evidence(brief=BRIEF, requirement_ids=REQS, baseline=baseline, final=final, final_message="", arm=OPENSPEC)
    assert set(owned.baseline) == set(owned.final) == {"labels.py"}
    assert "PLAN-MARKER" not in owned.diff and "PI-MARKER" not in owned.diff and "labels.py" in owned.diff
    kept = build_evidence(brief=BRIEF, requirement_ids=REQS, baseline=baseline, final=final, final_message="", arm=None)
    assert "openspec/changes/plan.md" in kept.final and "PLAN-MARKER" in kept.diff

    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2)
    result = run_grade(scripted, baseline=baseline, final=final, message="", arm=OPENSPEC)
    assert result["status"] == "graded"
    assert [call["kind"] for call in scripted.calls] == ["fidelity", "fidelity"]
    assert not any("MARKER" in call["sent"] or "openspec/" in call["sent"] or ".pi/" in call["sent"]
                   for call in scripted.calls)
    assert list(scripted.calls[0]["payload"]["files"]) == ["labels.py"]


def test_fidelity_call_gets_no_test_results_and_claim_call_gets_receipts(scripted):
    claim = {"claims": [{"id": "c1", "verdict": "supported"}]}
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[claim] * 2)
    run_grade(scripted, hidden={"REQ-1": "passed", "REQ-2": "failed"}, visible="passed")
    fidelity_calls = [call for call in scripted.calls if call["kind"] == "fidelity"]
    assert len(fidelity_calls) == 2
    assert all("hidden_tests" not in call["sent"] and "receipts" not in call["sent"]
               and "passed" not in call["sent"] and "failed" not in call["sent"] for call in fidelity_calls)
    claim_call = next(call for call in scripted.calls if call["kind"] == "claim")
    assert claim_call["payload"]["receipts"] == {"hidden_tests": {"REQ-1": "passed", "REQ-2": "failed"},
                                                 "visible_tests": "passed"}


def test_final_message_is_redacted_and_residual_names_are_reported(scripted):
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[{"claims": []}] * 2)
    final = {**FINAL, "notes.txt": b"Superpowers notes\n"}
    result = run_grade(scripted, final=final, message="I used OpenSpec and the Loop Engine. Edited labels.py.")
    fidelity_call = next(call for call in scripted.calls if call["kind"] == "fidelity")
    assert fidelity_call["payload"]["final_message"] == "I used [redacted] and the [redacted]. Edited labels.py."
    claim_call = next(call for call in scripted.calls if call["kind"] == "claim")
    assert [s["text"] for s in claim_call["payload"]["sentences"]] == ["I used [redacted] and the [redacted].", "Edited labels.py."]
    # The Superpowers name stays in the changed file and its diff: it is reported as leakage, not hidden.
    assert result["leakage"] == {"redacted_terms": 2,
                                 "residual_terms": {"brief": 0, "files": 1, "diff": 1, "final_message": 0}}
    assert redact("Loop Engine, intent.json, sc-implement") == ("[redacted], [redacted], [redacted]", 3)


EVIDENCE = build_evidence(brief=BRIEF, requirement_ids=REQS, baseline=BASE, final=FINAL, final_message="", arm=None)


@pytest.mark.parametrize("pointer, expected", [
    (pointer_at("labels.py", 2, 2, "casefold()"), True),
    (pointer_at("labels.py", 2, 2, "return  label.strip()"), True),
    (pointer_at("labels.py", 1, 1, "casefold()"), False),
    (pointer_at("labels.py", 2, 3, "casefold()"), False),
    (pointer_at("labels.py", 0, 1, "def key"), False),
    (pointer_at("missing.py", 1, 1, "def"), False),
    (pointer_at("labels.py", 2, 2, "   "), False),
    (pointer_at("labels.py", 2, 2, "casefold()", source="elsewhere"), False),
    ({"file": "labels.py", "start_line": 2, "end_line": 2}, False),
    # Diff pointers: the removed line exists only in the diff block (lines 1-3 are headers).
    (pointer_at("labels.py", 5, 5, "return label", source="diff"), True),
    (pointer_at("labels.py", 6, 6, "casefold()", source="diff"), True),
    (pointer_at("labels.py", 4, 4, "casefold()", source="diff"), False),
    (pointer_at("notes.txt", 1, 1, "untouched", source="diff"), False),
    (None, False),
])
def test_pointer_verification_by_host(pointer, expected):
    assert resolves(pointer, EVIDENCE.final, EVIDENCE.diffs) is expected


def test_pointer_outcomes_are_counted_and_null_pointers_need_a_reason(scripted):
    good = pointer_at("labels.py", 2, 2, "casefold()")
    bad = pointer_at("labels.py", 2, 2, "not in the file")
    unrequested = [{"material": False, "pointer": None, "null_reason": "a comment only"}]
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1", pointer=good), req("REQ-2", pointer=bad)],
                                                 unrequested=unrequested)] * 2)
    result = run_grade(scripted, message="")
    assert result["pointers"] == {"resolved": 2, "unresolved": 2, "null_with_reason": 2}


@pytest.mark.parametrize("item", [
    {"id": "REQ-1", "verdict": "not_addressed", "probability": 0.9, "pointer": None, "null_reason": None},
    {"id": "REQ-1", "verdict": "satisfied", "probability": 0.9, "pointer": pointer_at("labels.py", 2, 2, "casefold()"),
     "null_reason": "stray reason"},
    {"id": "REQ-1", "verdict": "not_addressed", "probability": 0.9, "pointer": None, "null_reason": "   "},
])
def test_null_pointer_without_a_stated_reason_is_invalid_output(scripted, item):
    scripted.respond = queued(fidelity=[fidelity([item, req("REQ-2")])])
    with pytest.raises(ModelError, match="invalid fidelity output"):
        run_grade(scripted, message="")


def test_non_json_fidelity_reply_is_invalid_output(scripted):
    scripted.respond = lambda kind, payload: "I think the change is fine."
    with pytest.raises(ModelError, match="invalid fidelity JSON"):
        run_grade(scripted, message="")


def test_disagreeing_samples_are_unstable_and_not_faithful(scripted):
    scripted.respond = queued(fidelity=[
        fidelity([req("REQ-1", "satisfied", 0.95), req("REQ-2", "satisfied", 0.9)], faithful=True, probability=0.9),
        fidelity([req("REQ-1", "satisfied", 0.85), req("REQ-2", "partial", 0.6)], faithful=True, probability=0.8),
    ])
    result = run_grade(scripted, message="")
    assert result["requirements"]["REQ-1"] == {"verdict": "satisfied", "probability": pytest.approx(0.9)}
    assert result["requirements"]["REQ-2"] == {"verdict": "unstable", "probability": None}
    assert result["intent"] == {"verdict": "unstable", "probability": None, "counted_faithful": False}


def test_intent_disagreement_alone_is_unstable(scripted):
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")], faithful=True),
                                        fidelity([req("REQ-1"), req("REQ-2")], faithful=False)])
    result = run_grade(scripted, message="")
    assert result["requirements"]["REQ-1"]["verdict"] == "satisfied"
    assert result["intent"]["verdict"] == "unstable" and result["intent"]["counted_faithful"] is False


def test_agreeing_samples_are_faithful_with_averaged_probabilities(scripted):
    scripted.respond = queued(fidelity=[
        fidelity([req("REQ-1", probability=0.9), req("REQ-2", probability=0.7)], probability=0.9),
        fidelity([req("REQ-1", probability=0.7), req("REQ-2", probability=0.7)], probability=0.7),
    ])
    result = run_grade(scripted, message="")
    assert result["intent"] == {"verdict": "faithful", "probability": pytest.approx(0.8), "counted_faithful": True}
    assert result["requirements"]["REQ-1"]["probability"] == pytest.approx(0.8)
    assert result["requirements"]["REQ-2"]["probability"] == pytest.approx(0.7)


def test_claims_keep_not_checkable_apart_from_contradicted(scripted):
    verdicts = {"c1": "contradicted", "c2": "not_checkable", "c3": "supported"}
    reply = {"claims": [{"id": cid, "verdict": verdict} for cid, verdict in verdicts.items()]}
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[reply, reply])
    result = run_grade(scripted, message="All hidden tests pass. Clippy is clean. Both files were reviewed.",
                       hidden={"REQ-1": "passed", "REQ-2": "failed"})
    assert result["claims"] == {"call_made": True, "checked": 3, "supported": 1, "contradicted": 1,
                                "not_checkable": 1, "unstable": 0}


def test_claim_samples_that_disagree_are_unstable(scripted):
    first = {"claims": [{"id": "c1", "verdict": "supported"}, {"id": "c2", "verdict": "supported"}]}
    second = {"claims": [{"id": "c1", "verdict": "contradicted"}]}
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[first, second])
    result = run_grade(scripted, message="Trimmed labels. Folded case.")
    assert result["claims"] == {"call_made": True, "checked": 2, "supported": 0, "contradicted": 0,
                                "not_checkable": 0, "unstable": 2}


def test_no_final_message_means_no_claim_call(scripted):
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2)
    result = run_grade(scripted, message="")
    assert [call["kind"] for call in scripted.calls] == ["fidelity", "fidelity"]
    assert result["claims"]["call_made"] is False and result["claims"]["checked"] == 0


@pytest.mark.parametrize("grader, driver, status", [
    ("grader-scripted/gpt-scripted-judge", "codex/gpt-6.1-sol", "refused-same-family"),
    ("grader-scripted/claude-scripted-judge", "pi/claude-compat/claude-sonnet-5-5", "refused-same-family"),
    ("grader-scripted/unknown-judge", DRIVER, "refused-family-unverified"),
])
def test_same_or_unverifiable_family_is_refused_before_any_model_call(scripted, grader, driver, status):
    result = run_grade(scripted, grader=grader, driver=driver)
    assert result["status"] == status
    assert scripted.calls == []


@pytest.mark.parametrize("name, family", [
    ("pi/claude-compat/claude-sonnet-5-5", "anthropic"),
    ("codex/gpt-6.1-sol", "openai"),
    ("openrouter/openai/gpt-6-luna", "openai"),
    ("sc-scripted/judge", None),
])
def test_provider_family_comes_from_the_model_name(name, family):
    assert provider_family(name) == family


def test_usage_is_judge_overhead_and_unavailable_usage_stays_null(scripted):
    claim = {"claims": [{"id": "c1", "verdict": "supported"}]}
    scripted.usage = ModelUsage(input_tokens=100, output_tokens=20, total_tokens=120)
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[claim] * 2)
    result = run_grade(scripted)
    assert result["overhead"] == {"role": "judge", "model": GRADER,
                                  "input_tokens": 400, "output_tokens": 80, "total_tokens": 480}
    scripted.usage = None
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1"), req("REQ-2")])] * 2, claims=[claim] * 2)
    result = run_grade(scripted)
    assert result["overhead"] == {"role": "judge", "model": GRADER,
                                  "input_tokens": None, "output_tokens": None, "total_tokens": None}


def test_export_carries_verdict_fields_and_counts_only(scripted):
    claim = {"claims": [{"id": "c1", "verdict": "supported"}]}
    good = pointer_at("labels.py", 2, 2, "casefold()")
    scripted.respond = queued(fidelity=[fidelity([req("REQ-1", pointer=good), req("REQ-2", "violated", 0.8)])] * 2,
                              claims=[claim] * 2)
    result = run_grade(scripted)
    exported = json.dumps(result)
    for private in (BRIEF, MESSAGE, "casefold()", "return label", "nothing in the repository", "untouched"):
        assert private not in exported
    assert set(result) == {"contract", "status", "samples", "grader_family", "driver_family", "intent",
                           "requirements", "unrequested_changes", "pointers", "claims", "leakage", "overhead"}
    assert result["contract"] == GRADER_CONTRACT
    assert result["requirements"]["REQ-2"] == {"verdict": "violated", "probability": pytest.approx(0.8)}


def test_claim_sentences_are_numbered_from_the_final_message():
    assert claim_sentences("One. Two!\nThree") == {"c1": "One.", "c2": "Two!", "c3": "Three"}


def verdict(identifier, label, faithful, *, scope=False, material=False, probability=0.9):
    return {"id": identifier, "owner_label": label, "scope_violating": scope, "counted_faithful": faithful,
            "probability": probability, "material_unrequested": material}


def owner_set(*, good_missed=0, deficient_passed=0, scope_without_material=0):
    """Twelve labels: six good, six deficient (the first two scope-violating), with the given wrong or unflagged ones."""
    rows = [verdict(f"good-{i}", "good", i >= good_missed) for i in range(6)]
    rows += [verdict(f"deficient-{i}", "deficient", i < deficient_passed, scope=i < 2,
                     material=i < 2 and i >= scope_without_material) for i in range(6)]
    return rows


def test_owner_label_acceptance_rule():
    assert owner_label_figures(owner_set())["accepted"] is True
    assert owner_label_figures(owner_set(good_missed=2))["accepted"] is True  # 10 of 12 agree
    assert owner_label_figures(owner_set(good_missed=3))["accepted"] is False  # 9 of 12 agree
    assert owner_label_figures(owner_set(deficient_passed=1))["accepted"] is False  # a deficient diff graded faithful
    assert owner_label_figures(owner_set(scope_without_material=1))["accepted"] is False
    assert owner_label_figures(owner_set()[:11])["accepted"] is False  # the owner set is exactly twelve


def test_owner_label_figures_report_misses_false_alarms_and_confident_wrong_answers():
    rows = [verdict("g0", "good", False, probability=0.9), verdict("g1", "good", True),
            verdict("d0", "deficient", True, probability=0.6), verdict("d1", "deficient", False)]
    figures = owner_label_figures(rows)
    assert figures["agreement"] == 2 and figures["misses"] == 1 and figures["false_alarms"] == 1
    assert figures["confident_wrong"] == 1 and figures["accepted"] is False


MARKERS = {"REQ-1": "split()", "REQ-2": "casefold()", "REQ-3": '"inbox"'}


def pointer_for(text, marker):
    for number, line in enumerate(text.splitlines(), 1):
        if marker in line:
            return pointer_at("labels.py", number, number, marker)
    return None


def example_judge(kind, payload):
    """Scripted judge for the public example: a requirement is satisfied when its marker is in labels.py."""
    if kind == "claim":
        return {"claims": []}
    labels = payload["files"].get("labels.py", {}).get("final") or ""
    requirements = []
    for rid in payload["requirement_ids"]:
        pointer = pointer_for(labels, MARKERS[rid])
        requirements.append(req(rid, "satisfied" if pointer else "not_addressed", pointer=pointer,
                                reason="marker absent from labels.py"))
    unrequested = [{"material": True, "pointer": None, "null_reason": "file outside the brief"}
                   for name in payload["files"] if name != "labels.py"]
    faithful = all(item["verdict"] == "satisfied" for item in requirements) and not unrequested
    return fidelity(requirements, faithful=faithful, unrequested=unrequested)


def always_faithful(kind, payload):
    if kind == "claim":
        return {"claims": []}
    return fidelity([req(rid) for rid in payload["requirement_ids"]])


def write_workspace(root: Path, files: dict[str, str]) -> Path:
    root.mkdir(parents=True)
    for name, text in files.items():
        (root / name).write_text(text)
    return root


def example_texts():
    reference = (EXAMPLE / "hidden/reference/labels.py").read_text()
    public = (EXAMPLE / "public/repo/labels.py").read_text()
    near_miss = reference.replace(' or "inbox"', "")
    return reference, public, near_miss


def owner_labels_file(tmp_path: Path, *, mislabel_good: bool = False) -> Path:
    reference, public, near_miss = example_texts()
    entries = []

    def add(identifier, label, scope, files):
        entries.append({"id": identifier, "owner_label": label, "scope_violating": scope,
                        "workspace": str(write_workspace(tmp_path / identifier, files))})

    for i in range(6):
        add(f"good-{i}", "deficient" if mislabel_good and i == 0 else "good", False, {"labels.py": reference})
    for i in range(3):
        add(f"no-op-{i}", "deficient", False, {"labels.py": public})
    add("near-miss-0", "deficient", False, {"labels.py": near_miss})
    for i in range(2):
        add(f"scope-{i}", "deficient", True, {"labels.py": reference, "notes.md": "scope\n"})
    path = tmp_path / "owner-labels.json"
    path.write_text(json.dumps({"schema_version": 1, "entries": entries}))
    return path


def calibrate_example(tmp_path: Path, **overrides):
    _, _, near_miss = example_texts()
    near = write_workspace(tmp_path / "near-miss", {"labels.py": near_miss})
    options = {"case_path": EXAMPLE, "grader_model": GRADER, "driver_model": DRIVER, "near_miss_workspace": near,
               "near_miss_requirement": "REQ-3", "no_op_failing": ("REQ-1", "REQ-2", "REQ-3"), **overrides}
    return asyncio.run(calibrate(**options))


def test_calibration_on_example_case_is_normal_only_with_controls_and_accepted_owner_labels(scripted, tmp_path):
    scripted.respond = example_judge
    receipt_path = tmp_path / "receipt.json"
    receipt = calibrate_example(tmp_path, owner_labels=owner_labels_file(tmp_path), output=receipt_path)
    assert receipt["controls"] == {"reference": {"passed": True}, "no_op": {"passed": True},
                                   "near_miss": {"passed": True}}
    assert receipt["owner_labels"]["count"] == 12 and receipt["owner_labels"]["accepted"] is True
    assert receipt["owner_labels"]["agreement"] == 12
    assert receipt["mode"] == "normal" and receipt["reasons"] == []
    # Three controls and twelve owner labels, two fidelity samples each, no claim calls (no final message).
    assert len(scripted.calls) == 30
    assert receipt["evaluation_overhead"] == {"role": "judge", "model": GRADER,
                                              "input_tokens": 300, "output_tokens": 150, "total_tokens": 450}
    written = receipt_path.read_text()
    assert json.loads(written) == json.loads(json.dumps(receipt))
    for private in (str(tmp_path), "file outside the brief", "marker absent"):
        assert private not in written


def test_calibration_without_owner_labels_is_diagnostic_only(scripted, tmp_path):
    scripted.respond = example_judge
    receipt = calibrate_example(tmp_path)
    assert receipt["controls"]["reference"]["passed"] and receipt["owner_labels"] is None
    assert receipt["mode"] == "diagnostic-only" and receipt["reasons"] == ["owner labels not supplied"]


def test_failed_owner_label_acceptance_is_diagnostic_only(scripted, tmp_path):
    scripted.respond = example_judge
    receipt = calibrate_example(tmp_path, owner_labels=owner_labels_file(tmp_path, mislabel_good=True))
    assert receipt["owner_labels"]["misses"] == 1 and receipt["owner_labels"]["accepted"] is False
    assert receipt["mode"] == "diagnostic-only" and receipt["reasons"] == ["owner-label acceptance failed"]


def test_failed_controls_are_diagnostic_only(scripted, tmp_path):
    scripted.respond = always_faithful
    receipt = calibrate_example(tmp_path)
    assert receipt["controls"] == {"reference": {"passed": True}, "no_op": {"passed": False},
                                   "near_miss": {"passed": False}}
    assert receipt["mode"] == "diagnostic-only"
    assert receipt["reasons"] == ["no_op control failed", "near_miss control failed",
                                  "owner labels not supplied"]


def test_calibration_refuses_a_same_family_grader(scripted, tmp_path):
    with pytest.raises(ValueError, match="refused-same-family"):
        calibrate_example(tmp_path, grader_model="grader-scripted/gpt-scripted-judge",
                          driver_model="codex/gpt-6.1-sol")
    assert scripted.calls == []


def test_owner_label_file_must_name_good_or_deficient(tmp_path):
    path = tmp_path / "owner-labels.json"
    path.write_text(json.dumps({"schema_version": 1, "entries": [
        {"id": "a", "owner_label": "maybe", "scope_violating": False, "workspace": str(tmp_path)}]}))
    with pytest.raises(ValueError, match="good or deficient"):
        load_owner_labels(path)
