"""Intent check-in end to end: one scripted superpowers-arm trial through evalctl campaign, in the real image.

The scripted arm driver asks the owner once, writes a design note, sends it to confirm_intent.py twice and builds the
labels. The harness's own client crosses the real sandbox service to the scripted simulator. The first reply is the
scripted owner's; the second must be the refusal. No paid model, no network. Mechanics proof only.
"""
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import uuid


REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
DRIVER_SOURCE = (ROOT / "scevals/scripted_arm_driver.py").read_text()
ARM = {"id": "superpowers", "driver_command": ["python3", "-c", DRIVER_SOURCE, "--checkin"],
       "prompt_template": "superpowers", "workflow_owned_paths": ["docs/superpowers/**"],
       "completion_rule": "driver-exit"}


def test_the_checkin_client_reaches_the_scripted_owner_once_and_is_recorded_apart():
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings["target_options"] = {"arm": ARM, "intent_checkin": True}
    settings["idle_limit_seconds"] = 60
    name = f"t-checkin-{uuid.uuid4().hex[:8]}"
    settings["campaign"] = name
    with tempfile.TemporaryDirectory(prefix="t-checkin-") as scratch:
        config = Path(scratch) / (name + ".json")
        config.write_text(json.dumps(settings))
        result = subprocess.run(["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)],
                                cwd=REPO, text=True, capture_output=True, timeout=650)
    assert result.returncode == 0, result.stdout + result.stderr
    result_dir = Path(result.stdout.split("Report written: ", 1)[1].strip())
    summary = json.loads((result_dir / "summary.json").read_text())
    (trial,) = summary["trials"]
    metrics = trial["target_metrics"]
    assert (trial["outcome"], trial["reason"]) == ("success", "passed"), metrics
    assert metrics["intent_checkin"]["count"] == 1 and metrics["intent_checkin"]["revealed_requirement_ids"] == ["REQ-3"]
    assert metrics["discipline"]["owner_question_count"] == 1
    capture = Path(summary["capture_run"]) / trial["sample_id"]
    with tarfile.open(capture / "workspace.tar") as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith("trial/checkin-replies.txt"))
        first, second = archive.extractfile(member).read().decode().split("\n---\n")
    assert first == "Scripted check-in: also handle empty labels."
    assert second.startswith("The owner has already confirmed")
