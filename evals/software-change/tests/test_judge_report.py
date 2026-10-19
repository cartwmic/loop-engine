"""AC-4 / AC-6 report outcome: per-model intent-judge scores and the owner-label calibration figures.

Black-box through the public evalctl CLI with scripted models and the real Docker trial container.
A synthetic calibration receipt (no owner data) is supplied through the target option the report already reads.
"missing receipt reports calibration pending" is already proved by test_scoring.py (ac5-demo); this test
adds the receipt-present case, the figures with their counts, and the judge's overhead accounting.
"""
import json
import os
from pathlib import Path
import subprocess
import tomllib
import uuid

from scevals.judge import JUDGE_CONTRACT

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
CAPTURES = ROOT / "captures/judge-report"

VERDICTS = [
    {"id": "a", "owner_label": "good", "judge_passed": True},
    {"id": "b", "owner_label": "good", "judge_passed": False},
    {"id": "c", "owner_label": "deficient", "judge_passed": True},
    {"id": "d", "owner_label": "deficient", "judge_passed": False},
]
# Known by hand: judge agrees on a and d; the one false pass is c, one of two owner-deficient intents.
AGREEMENT = {"numerator": 2, "denominator": 4, "rate": 0.5}
FALSE_PASS = {"numerator": 1, "denominator": 2, "rate": 0.5}
COUNTS = {"labelled": 4, "owner_good": 2, "owner_deficient": 2, "judge_passed": 2, "judge_failed": 2}


def run_with_receipt():
    CAPTURES.mkdir(parents=True, exist_ok=True)
    name = "jr-" + uuid.uuid4().hex
    receipt = CAPTURES / (name + "-receipt.json")
    receipt.write_text(json.dumps({
        "schema_version": 1, "judge_model": "sc-scripted/judge", "judge_contract": JUDGE_CONTRACT,
        "agreement": AGREEMENT, "false_pass_rate": FALSE_PASS, "counts": COUNTS, "verdicts": VERDICTS,
        "labels_sha256": "0" * 64, "unlabelled_count": 3,
        "evaluation_overhead": {"role": "judge", "model": "sc-scripted/judge",
                                "input_tokens": 12, "output_tokens": 8, "total_tokens": 20}}))
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings["campaign"] = name
    settings["target_options"] = {"scripted_driver": "asks-and-builds", "calibration_receipt": str(receipt)}
    config = CAPTURES / (name + ".json")
    config.write_text(json.dumps(settings))
    command = ["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)]
    result = subprocess.run(command, cwd=REPO, env=os.environ.copy(), text=True, capture_output=True, timeout=650)
    (CAPTURES / (name + ".log")).write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stdout + result.stderr
    result_dir = Path(result.stdout.split("Report written: ", 1)[1].strip())
    assert result_dir.parent == ROOT / "results" / name
    summary = json.loads((result_dir / "summary.json").read_text())
    markdown = (result_dir / "report.md").read_text()
    result_dir.rename(CAPTURES / name)
    result_dir.parent.rmdir()
    return summary, markdown


def test_report_shows_judge_scores_identity_overhead_and_calibration_figures():
    summary, markdown = run_with_receipt()
    trial = summary["trials"][0]
    comparison = summary["comparisons"][0]
    # Per-model intent-judge score with the pinned judge identity.
    assert summary["pinned_models"]["judge"] == "sc-scripted/judge"
    score = trial["target_metrics"]["intent_judge"]
    assert score["model"] == "sc-scripted/judge" and score["passed"] is True
    assert comparison["target_metrics"]["software_change_metrics"][0]["intent_judge"] == score
    # Calibration next to it: agreement and false-pass rate with the counts behind both.
    calibration = comparison["target_metrics"]["judge_calibration"]
    assert calibration["judge_model"] == "sc-scripted/judge"
    assert calibration["agreement"] == AGREEMENT
    assert calibration["false_pass_rate"] == FALSE_PASS
    assert calibration["counts"] == COUNTS
    assert calibration["unlabelled_count"] == 3
    for text in ('"judge_calibration"', '"agreement"', '"false_pass_rate"', '"counts"', '"intent_judge"'):
        assert text in markdown, text
    # Judge usage is evaluation overhead, not workflow cost (same scripted models and example case as
    # the asks-and-builds row proved in test_scoring.py: judge 30, overhead 120, workflow 120).
    assert trial["usage_by_role"]["judge"]["total_tokens"] == 30
    assert comparison["core"]["evaluation_overhead"]["total_tokens"] == 120
    assert comparison["core"]["workflow_cost"]["total_tokens"] == 120
