"""Public evalctl extraction/scoring proofs with deterministic Inspect backends."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import uuid

import pytest
from inspect_ai import Task, eval as inspect_eval
from inspect_ai.dataset import Sample
from inspect_ai.model import get_model
from inspect_ai.solver import solver

from scevals.calibration import calibration_figures, calibration_for_report
from scevals.judge import judge_intent

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).parent / "fixtures"
CAPTURES = Path(__file__).parent / "captures" / "calibration"


def invoke(*args, environment=None):
    folder = CAPTURES / uuid.uuid4().hex
    folder.mkdir(parents=True)
    env = dict(os.environ)
    env["PYTHONPATH"] = str(FIXTURES) + os.pathsep + env.get("PYTHONPATH", "")
    if environment:
        env.update(environment)
    command = [str(ROOT / "evals/.venv/bin/evalctl"), "--plugin", "calibration_backend", "calibrate", *map(str, args)]
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, timeout=60)
    (folder / "command.json").write_text(json.dumps(command))
    (folder / "stdout.txt").write_text(result.stdout)
    (folder / "stderr.txt").write_text(result.stderr)
    return result


def database_fixture(path, artifact_root):
    with sqlite3.connect(path) as connection:
        connection.executescript("""
            CREATE TABLE runs (id TEXT PRIMARY KEY, workflow_id TEXT, artifact_root TEXT, created_at INTEGER);
            CREATE TABLE context_records (run_id TEXT, record_id TEXT, sequence INTEGER, kind TEXT, data_json TEXT,
                                          PRIMARY KEY(run_id, record_id), UNIQUE(run_id, sequence));
        """)
        for i in range(12):
            run_id = f"run-{i:02}"
            root = artifact_root / run_id
            root.mkdir(parents=True)
            (root / "intent.json").write_text(json.dumps({"revision": str(i), "outcome": f"Outcome {i}"}))
            connection.execute("INSERT INTO runs VALUES (?,?,?,?)", (run_id, "software-change", str(root), i))
            for sequence, instruction in ((1, "The original owner request."), (2, "The owner's clarification.")):
                connection.execute("INSERT INTO context_records VALUES (?,?,?,?,?)",
                                   (run_id, f"owner-{sequence}", sequence, "user-steering", json.dumps({"instruction": instruction})))
            connection.execute("INSERT INTO context_records VALUES (?,?,?,?,?)",
                               (run_id, "noise", 3, "review-evidence", '{"irrelevant": true}'))
        connection.execute("INSERT INTO runs VALUES ('missing','software-change',NULL,99)")
        connection.execute("INSERT INTO runs VALUES ('other','research',NULL,100)")


def test_extract_cli_read_only_unlabelled(tmp_path):
    database = tmp_path / "loop.db"
    database_fixture(database, tmp_path / "artifacts")
    before = hashlib.sha256(database.read_bytes()).hexdigest()
    labels = tmp_path / "labels.json"
    result = invoke("extract", "--database", database, "--output", labels, "--limit", 10)
    assert result.returncode == 0, result.stderr
    assert "Extracted 10 candidates" in result.stdout
    assert "read-only" in result.stdout
    data = json.loads(labels.read_text())
    assert len(data["entries"]) == 10
    assert data["extraction"]["skipped"] == 1
    for entry in data["entries"]:
        assert entry["owner_label"] is None and entry["owner_confirmed"] is False
        assert "original owner request" in entry["owner_request"] and "clarification" in entry["owner_request"]
        assert entry["source"]["request_record_ids"] == ["owner-1", "owner-2"]
        assert "irrelevant" not in json.dumps(entry)
    assert data["judge_model"] is None
    assert hashlib.sha256(database.read_bytes()).hexdigest() == before
    original = labels.read_bytes()
    assert invoke("extract", "--database", database, "--output", labels).returncode == 2
    assert labels.read_bytes() == original


def test_missing_database_not_created(tmp_path):
    path = tmp_path / "missing.db"
    result = invoke("extract", "--database", path, "--output", tmp_path / "labels.json")
    assert result.returncode == 2
    assert not path.exists() and not (tmp_path / "labels.json").exists()


def test_score_cli_known_outputs(tmp_path):
    output = tmp_path / "captures" / "calibration.json"
    result = invoke("score", "--labels", FIXTURES / "calibration-labels.json", "--output", output)
    assert result.returncode == 0, result.stderr
    assert "Agreement: 2/4 (0.5)" in result.stdout
    assert "False-pass rate: 1/2 owner-deficient (0.5)" in result.stdout
    data = json.loads(output.read_text())
    assert data["agreement"] == {"numerator": 2, "denominator": 4, "rate": .5}
    assert data["false_pass_rate"] == {"numerator": 1, "denominator": 2, "rate": .5}
    assert data["counts"] == {"labelled": 4, "owner_good": 2, "owner_deficient": 2, "judge_passed": 2, "judge_failed": 2}
    assert data["unlabelled_count"] == 1
    assert data["evaluation_overhead"] == {"role": "judge", "model": "calibration-fixture/pinned-v1", "input_tokens": 40, "output_tokens": 8, "total_tokens": 48}
    assert "owner_request" not in output.read_text() and "intent\"" not in output.read_text()
    assert data["labels_sha256"] == hashlib.sha256((FIXTURES / "calibration-labels.json").read_bytes()).hexdigest()
    report = calibration_for_report(output, judge_model=data["judge_model"])
    assert report["agreement"] == data["agreement"]
    assert "verdicts" not in report
    with pytest.raises(ValueError):
        calibration_for_report(output, judge_model="calibration-fixture/another-pin")
    tampered = dict(data, agreement={"numerator": 4, "denominator": 4, "rate": 1})
    (tmp_path / "tampered.json").write_text(json.dumps(tampered))
    with pytest.raises(ValueError):
        calibration_for_report(tmp_path / "tampered.json", judge_model=data["judge_model"])
    assert invoke("score", "--labels", FIXTURES / "calibration-labels.json", "--output", output).returncode == 2


@pytest.mark.parametrize("failure", ["empty", "unconfirmed", "invalid", "missing-model", "duplicate"])
def test_score_cli_owner_gate(tmp_path, failure):
    labels = json.loads((FIXTURES / "calibration-labels.json").read_text())
    labels["judge_model"] = "calibration-fixture/must-not-call"
    if failure == "empty":
        labels["entries"] = []
    elif failure == "unconfirmed":
        labels["entries"][0]["owner_confirmed"] = False
    elif failure == "invalid":
        labels["entries"][0]["owner_label"] = "approved"
    elif failure == "missing-model":
        labels["judge_model"] = None
    else:
        labels["entries"][1]["id"] = labels["entries"][0]["id"]
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(labels))
    output = tmp_path / "out.json"
    result = invoke("score", "--labels", path, "--output", output)
    assert result.returncode == 2
    assert "Calibration pending" in result.stdout
    assert "no judge calls made" in result.stdout
    assert not output.exists()


def test_score_cli_empty_extracted_labels(tmp_path):
    database = tmp_path / "loop.db"
    database_fixture(database, tmp_path / "artifacts")
    path = tmp_path / "labels.json"
    assert invoke("extract", "--database", database, "--output", path).returncode == 0
    result = invoke("score", "--labels", path)
    assert result.returncode == 2 and "Calibration pending" in result.stdout
    assert not (tmp_path / "captures/calibration.json").exists()


def test_extract_secret_refusal(tmp_path):
    database = tmp_path / "loop.db"
    database_fixture(database, tmp_path / "artifacts")
    secret = "fixture-not-a-credential-unique-phrase"
    (tmp_path / "artifacts/run-11/intent.json").write_text(json.dumps({"revision": "1", "outcome": secret}))
    labels = tmp_path / "labels.json"
    result = invoke("extract", "--database", database, "--output", labels, environment={"FAKE_EVAL_API_KEY": secret})
    assert result.returncode == 2 and "Result refused" in result.stderr
    assert not labels.exists()
    assert secret not in result.stdout + result.stderr


def test_score_invalid_judge_has_no_receipt(tmp_path):
    output = tmp_path / "out.json"
    result = invoke("score", "--labels", FIXTURES / "calibration-labels.json", "--judge-model", "calibration-fixture/invalid", "--output", output)
    assert result.returncode == 2 and "ModelError" in result.stderr
    assert not output.exists()


def test_zero_denominators():
    assert calibration_figures([])["agreement"]["rate"] is None
    assert calibration_figures([])["false_pass_rate"]["rate"] is None
    only_good = [{"owner_label": "good", "judge_passed": False}]
    assert calibration_figures(only_good)["false_pass_rate"] == {"numerator": 0, "denominator": 0, "rate": None}


def test_judge_bound_role_is_evaluation_overhead(monkeypatch):
    # Real Inspect Task and bound judge role, with the same model available to
    # workflow roles: accounting must separate by role rather than model identity.
    monkeypatch.syspath_prepend(str(FIXTURES))
    __import__("calibration_backend")
    folder = CAPTURES / uuid.uuid4().hex
    folder.mkdir(parents=True)

    @solver
    def judge_solver():
        async def solve(state, generate):
            await get_model(role="role-driver").generate(json.dumps({"owner_brief": "public fixture", "intent": {"fixture_id": "good-pass"}}))
            result = await judge_intent({"fixture_id": "good-pass"}, "Hidden private owner brief.")
            assert result.passed
            assert result.metrics()["contract"] == "intent-judge-v1"
            return state
        return solve

    logs = inspect_eval(Task(dataset=[Sample(input="public request")], solver=judge_solver()),
                        model="calibration-fixture/pinned-v1",
                        model_roles={"judge": "calibration-fixture/pinned-v1", "role-driver": "calibration-fixture/pinned-v1"},
                        log_dir=str(folder), display="none")
    assert logs[0].status == "success"
    sample = logs[0].samples[0]
    assert sample.role_usage["judge"].total_tokens == 12
    assert sample.role_usage["role-driver"].total_tokens == 12
