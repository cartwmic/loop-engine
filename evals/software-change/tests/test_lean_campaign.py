"""Lean-change arms through the public campaign path (evalctl campaign): one real single-trial campaign per profile in the
real lean image, with the scripted lean driver (no model, no network). Mechanics proof only: the harness launches the arm,
the provider runs in the container, the engine records are read back into target_metrics.lean_change.

Needs the lean image: set SC_LEAN_IMAGE to its tag (the test is skipped without it)."""
import json
import os
import shlex
from pathlib import Path
import subprocess
import tempfile
import tomllib
import uuid

import pytest

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
IMAGE = os.environ.get("SC_LEAN_IMAGE")
BASE_COMMIT = os.environ.get("SC_LEAN_BASE_COMMIT", "8aeedb5ee4ef58d4a0fcbab8fef98b2a195e0d46")
DRIVER = (ROOT / "scevals/scripted_lean_driver.py").read_text()

pytestmark = pytest.mark.skipif(not IMAGE, reason="SC_LEAN_IMAGE names the lean image")


def campaign(profile):
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings["profile"] = f"lean-{profile}"
    settings["loop_engine_version"] = settings["provider_version"] = BASE_COMMIT
    settings["target_options"] = {"arm": {"id": f"lean-{profile}", "driver_command": ["python3", "-c", DRIVER, "--profile", profile],
                                          "prompt_template": f"lean-{profile}", "workflow_owned_paths": [],
                                          "completion_rule": "driver-exit", "image": IMAGE},
                                "scripted_driver": "asks-and-builds", "intent_checkin": True}
    settings["campaign"] = f"lean-proof-{profile}-{uuid.uuid4().hex[:8]}"
    with tempfile.TemporaryDirectory(prefix="lean-proof-") as scratch:
        config = Path(scratch) / "settings.json"
        config.write_text(json.dumps(settings))
        result = subprocess.run([*shlex.split(os.environ.get("SC_EVALCTL", "uv run --project evals evalctl")), "campaign", "--settings", str(config)],
                                cwd=REPO, text=True, capture_output=True, timeout=900)
    assert result.returncode == 0, result.stdout + result.stderr
    report = Path(result.stdout.split("Report written: ", 1)[1].strip())
    return json.loads((report / "summary.json").read_text())["trials"][0]


@pytest.mark.parametrize("profile", ["b", "c"])
def test_scripted_lean_run_completes_and_its_records_reach_the_metrics(profile):
    trial = campaign(profile)
    metrics = trial["target_metrics"]
    assert trial["outcome"] == "success", metrics
    lean = metrics["lean_change"]
    assert (lean["lifecycle"], lean["final_state"], lean["review_enabled"]) == ("final", "end", profile == "b")
    assert lean["denials"] == 1 and lean["criteria"] == 1 and lean["verifications"] == 1
    if profile == "b":
        assert (lean["reviews_appended"], lean["review_findings"], lean["principles_findings"], lean["findings_rejected"]) == (1, 1, 1, 1)
    else:
        assert lean["reviews_appended"] == 0 and lean["review_findings"] == 0
