"""T9 registration proofs: evalctl lists and runs grader-calibrate, compare, stage-ledger and findings-check.

Subcommands run through the installed entry point (`uv run --project evals evalctl`) over fixtures: the shipped Tier 0
summaries (read only), the public example case and a case-check receipt written for it, and temporary ledger and
finding files. grader-calibrate runs in process because its judge is a scripted model registered in this module, reached
through real Inspect model dispatch. No paid model, network or Docker use.
"""
import hashlib
import json
from pathlib import Path
import subprocess

import pytest
from inspect_ai.model import ModelAPI, ModelOutput, modelapi

from evalcore.cli import main as evalctl_main
from scevals.arm_commands import no_op_failing_from_receipt
from scevals.cases import load_case
from scevals.grader import CLAIM_INSTRUCTION

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
EXAMPLE = ROOT / "example-case"
REFERENCE = (EXAMPLE / "hidden/reference/labels.py").read_text()
TIER0 = {
    "sonnet": ROOT / "results/tier-0-full-sonnet/20261018T132346.405605Z/summary.json",
    "sol": ROOT / "results/tier-0-full-sol/20261018T135956.603628Z/summary.json",
}
GRADER = "arm-judge/gpt-scripted-judge"
DRIVER = "arm-driver/claude-scripted-driver"
CASE_CHECK = {"id": "label-keys", "version": "1", "valid": True, "reasons": [],
              "no_op": {"passed": 0, "total": 6, "requirements": {
                  "REQ-1": {"passed": 0, "total": 2}, "REQ-2": {"passed": 0, "total": 2},
                  "REQ-3": {"passed": 0, "total": 2}}}}


def evalctl(*args):
    return subprocess.run(["uv", "run", "--project", "evals", "evalctl", *map(str, args)], cwd=REPO,
                          capture_output=True, text=True, timeout=240)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@modelapi(name="arm-judge")
class ScriptedJudge(ModelAPI):
    """Answers like a perfect grader of the example: faithful exactly when labels.py equals the reference overlay."""

    async def generate(self, input, tools, tool_choice, config):
        if input[0].text == CLAIM_INSTRUCTION:
            reply = {"claims": []}
        else:
            evidence = json.loads(input[1].text)
            faithful = evidence["files"].get("labels.py", {}).get("final") == REFERENCE
            reply = {"requirements": [
                {"id": rid, "verdict": "satisfied" if faithful else "not_addressed", "probability": 0.9,
                 "pointer": None, "null_reason": "scripted judge quotes no evidence"}
                for rid in evidence["requirement_ids"]],
                "unrequested_changes": [], "intent_faithful": faithful, "intent_probability": 0.9}
        return ModelOutput.from_content(model=self.model_name, content=json.dumps(reply))


def write_case_check(path: Path, receipt: dict = CASE_CHECK) -> Path:
    path.write_text("VALID: label-keys\nCASE_CHECK " + json.dumps(receipt, sort_keys=True) + "\n")
    return path


def calibration_args(tmp_path: Path, *, grader: str = GRADER, driver: str = DRIVER) -> list[str]:
    near_miss = tmp_path / "near-miss"
    near_miss.mkdir()
    (near_miss / "labels.py").write_text(REFERENCE.replace(' or "inbox"', ""))
    return ["grader-calibrate", str(EXAMPLE), "--grader-model", grader, "--driver-model", driver,
            "--case-check-receipt", str(write_case_check(tmp_path / "case-check.txt")),
            "--near-miss-workspace", str(near_miss), "--near-miss-requirement", "REQ-3",
            "--output", str(tmp_path / "calibration.json")]


def finding(**changes) -> dict:
    record = {"finding_id": "F-1", "source": "reviewer-a", "summary": "label key drops the fallback",
              "verification": {"status": "reproduced", "method": "test", "evidence": "REQ-3 test fails"},
              "material": True, "discovered_at": "2026-10-19T10:00:00+00:00", "already_known": False,
              "disposition": "fixed", "false_positive": False, "duplicate_of": None}
    return record | changes


def test_evalctl_help_lists_the_arm_subcommands_beside_the_existing_ones():
    result = evalctl("--help")
    assert result.returncode == 0
    for name in ("grader-calibrate", "compare", "stage-ledger", "findings-check", "campaign", "check-results"):
        assert name in result.stdout


@pytest.mark.parametrize("command", [
    ["grader-calibrate"], ["compare"], ["stage-ledger"], ["stage-ledger", "init"], ["stage-ledger", "admit"],
    ["stage-ledger", "record"], ["stage-ledger", "status"], ["findings-check"],
])
def test_each_subcommand_help_runs(command):
    result = evalctl(*command, "--help")
    assert result.returncode == 0
    assert f"usage: evalctl {' '.join(command)}" in result.stdout


def test_stage_ledger_admits_refuses_and_reports_overshoot(tmp_path):
    ledger = tmp_path / "stage.json"
    assert evalctl("stage-ledger", "init", ledger, "--stage-dollars", "30", "--stage-hours", "4").returncode == 0
    admitted = evalctl("stage-ledger", "admit", ledger, "--command", "T1 trial a", "--class", "T1")
    assert admitted.returncode == 0
    assert json.loads(admitted.stdout)["handed_cap"] == {"dollars": 12.0, "hours": 1.25}
    recorded = evalctl("stage-ledger", "record", ledger, "1", "--outcome", "success", "--hours", "0.5", "--dollars", "9")
    assert recorded.returncode == 0
    second = evalctl("stage-ledger", "admit", ledger, "--command", "T1 trial b", "--class", "T1")
    assert second.returncode == 0 and json.loads(second.stdout)["id"] == 2
    assert evalctl("stage-ledger", "record", ledger, "2", "--outcome", "success", "--hours", "0.6",
                   "--dollars", "25").returncode == 0
    before = sha256(ledger)
    refused = evalctl("stage-ledger", "admit", ledger, "--command", "T1 trial c", "--class", "T1")
    assert refused.returncode == 2 and "refused: remaining" in refused.stderr
    assert sha256(ledger) == before
    status = json.loads(evalctl("stage-ledger", "status", ledger).stdout)
    assert status["spent"]["dollars"] == 34.0 and status["overshoot"]["dollars"] == 4.0


def test_findings_check_accepts_a_complete_record_and_refuses_missing_or_unknown_fields(tmp_path):
    ledger = tmp_path / "findings.json"
    ledger.write_text(json.dumps([finding()]))
    valid = evalctl("findings-check", ledger)
    assert valid.returncode == 0 and "VALID: 1 finding record(s)" in valid.stdout
    missing = dict(finding())
    del missing["disposition"]
    ledger.write_text(json.dumps([missing]))
    result = evalctl("findings-check", ledger)
    assert result.returncode == 1 and "missing field(s): disposition" in result.stdout
    ledger.write_text(json.dumps([finding(reviewer_score=3)]))
    result = evalctl("findings-check", ledger)
    assert result.returncode == 1 and "unknown field(s): reviewer_score" in result.stdout


def test_compare_reads_the_shipped_tier0_summaries_and_leaves_them_unchanged(tmp_path):
    before = {name: sha256(path) for name, path in TIER0.items()}
    out = tmp_path / "reports"
    result = evalctl("compare", *TIER0.values(), "--out", out)
    assert result.returncode == 0, result.stderr
    assert result.stdout.startswith("Report written: ")
    (destination,) = out.iterdir()
    assert (destination / "summary.json").is_file() and (destination / "report.md").is_file()
    assert "## Task" in (destination / "report.md").read_text()
    assert {name: sha256(path) for name, path in TIER0.items()} == before


@pytest.mark.parametrize(("receipt_text", "reasons"), [
    pytest.param(json.dumps({"schema_version": 1, "mode": "diagnostic-only", "reasons": ["owner labels not supplied"]}),
                 ["owner labels not supplied"], id="diagnostic-only-receipt"),
    pytest.param("{not json", ["receipt unreadable"], id="unreadable-receipt"),
])
def test_compare_takes_a_calibration_receipt_and_states_it_in_the_report(tmp_path, receipt_text, reasons):
    receipt = tmp_path / "grader-receipt.json"
    receipt.write_text(receipt_text)
    out = tmp_path / "reports"
    result = evalctl("compare", *TIER0.values(), "--out", out, "--calibration", receipt)
    assert result.returncode == 0, result.stderr
    (destination,) = out.iterdir()
    summary = json.loads((destination / "summary.json").read_text())
    assert summary["calibration"] == {"mode": "diagnostic-only", "reasons": reasons}
    assert f"Calibration receipt: diagnostic-only (reason: {'; '.join(reasons)})." in (destination / "report.md").read_text()


def test_grader_calibrate_writes_the_example_controls_receipt_through_evalctl(tmp_path, capsys):
    assert evalctl_main(calibration_args(tmp_path)) == 0
    receipt = json.loads((tmp_path / "calibration.json").read_text())
    assert receipt["controls"] == {"reference": {"passed": True}, "no_op": {"passed": True},
                                   "near_miss": {"passed": True}}
    assert receipt["mode"] == "diagnostic-only" and receipt["reasons"] == ["owner labels not supplied"]
    out = capsys.readouterr().out
    assert "Calibration mode: diagnostic-only" in out and "reason: owner labels not supplied" in out


def test_grader_calibrate_passes_owner_labels_to_the_acceptance_rule(tmp_path, capsys):
    arguments = calibration_args(tmp_path)
    workspace = tmp_path / "owner-good"
    workspace.mkdir()
    (workspace / "labels.py").write_text(REFERENCE)
    labels = tmp_path / "owner-labels.json"
    labels.write_text(json.dumps({"schema_version": 1, "entries": [
        {"id": "good-0", "owner_label": "good", "scope_violating": False, "workspace": str(workspace)}]}))
    assert evalctl_main([*arguments, "--owner-labels", str(labels)]) == 0
    receipt = json.loads((tmp_path / "calibration.json").read_text())
    assert receipt["owner_labels"]["count"] == 1 and receipt["owner_labels"]["accepted"] is False
    assert receipt["mode"] == "diagnostic-only" and receipt["reasons"] == ["owner-label acceptance failed"]


def test_grader_calibrate_refuses_a_same_family_grader_and_writes_nothing(tmp_path, capsys):
    arguments = calibration_args(tmp_path, grader=GRADER, driver="codex/gpt-6.1-sol")
    assert evalctl_main(arguments) == 2
    assert "refused-same-family" in capsys.readouterr().err
    assert not (tmp_path / "calibration.json").exists()


def test_no_op_failing_comes_from_the_case_check_receipt_requirements_that_do_not_all_pass(tmp_path):
    case = load_case(EXAMPLE)
    receipt = dict(CASE_CHECK, no_op={"passed": 2, "total": 6, "requirements": {
        "REQ-1": {"passed": 0, "total": 2}, "REQ-2": {"passed": 2, "total": 2}, "REQ-3": {"passed": 0, "total": 2}}})
    assert no_op_failing_from_receipt(write_case_check(tmp_path / "receipt.txt", receipt), case) == ["REQ-1", "REQ-3"]


@pytest.mark.parametrize("change", [
    {"id": "other-case"},
    {"version": "2"},
    {"valid": False},
    {"no_op": {"passed": 0, "total": 2, "requirements": {"REQ-1": {"passed": 0, "total": 2}}}},
])
def test_case_check_receipt_must_be_valid_and_describe_this_case(tmp_path, change):
    path = write_case_check(tmp_path / "receipt.txt", CASE_CHECK | change)
    with pytest.raises(ValueError):
        no_op_failing_from_receipt(path, load_case(EXAMPLE))


def test_case_check_receipt_needs_exactly_one_case_check_line(tmp_path):
    path = write_case_check(tmp_path / "receipt.txt")
    path.write_text(path.read_text() * 2)
    with pytest.raises(ValueError, match="exactly one CASE_CHECK line"):
        no_op_failing_from_receipt(path, load_case(EXAMPLE))
