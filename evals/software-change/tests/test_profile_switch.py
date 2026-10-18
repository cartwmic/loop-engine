"""AC-3: choosing another shipped profile changes what actually runs.

Black-box through the public evalctl CLI with scripted models, the real engine/provider and the real
Docker trial container, like test_target.py. Needs the arm64 trial image and a reachable Docker daemon.
The scripted driver itself asserts that the run's frozen review policies equal the chosen profile's;
this test adds what the report and trial record say about it.
"""
import json
import os
from pathlib import Path
import subprocess
import tomllib
import uuid

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
CAPTURES = ROOT / "captures/profile-switch"


def campaign(settings_name):
    CAPTURES.mkdir(parents=True, exist_ok=True)
    settings = tomllib.loads((ROOT / "campaigns" / settings_name).read_text())
    name = "ps-" + uuid.uuid4().hex
    settings["campaign"] = name
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
    # Keep test-generated reports as local receipts, not publishable extra results.
    result_dir.rename(CAPTURES / name)
    result_dir.parent.rmdir()
    return summary, markdown


def test_default_and_high_rigor_profiles_change_what_actually_runs():
    seen = {}
    for settings_name, expected in (("scripted-example.toml", "standard-16"), ("scripted-high-rigor.toml", "high-rigor-16")):
        summary, markdown = campaign(settings_name)
        trial = summary["trials"][0]
        metrics = trial["target_metrics"]
        assert trial["terminal"] is True and metrics["lifecycle"] == "final" and metrics["terminal_state"] == "end"
        # The report records the settings used ...
        assert summary["settings"]["profile"] == expected
        assert f'"profile": "{expected}"' in markdown
        # ... and the run's own frozen start input names the profile that actually ran.
        assert metrics["profile_config_version"] == expected
        assert f'"profile_config_version": "{expected}"' in markdown
        assert summary["comparisons"][0]["target_metrics"]["software_change_metrics"][0]["profile_config_version"] == expected
        seen[expected] = metrics["profile_config_version"]
    assert seen == {"standard-16": "standard-16", "high-rigor-16": "high-rigor-16"}
