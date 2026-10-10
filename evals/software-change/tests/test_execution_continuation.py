"""Owner-authorized limit changes: same records/assignment, no silent relaxation."""
import asyncio
import copy
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import uuid

import pytest

from evalcore.continuation import validate_execution_exception, normalized_settings, record_digest
from evalcore.settings import CampaignSettings
from test_lifecycle import settings, invoke, PROOFS, ROOT, REPO


def authorization(original):
    return {"kind": "disable-outer-deadlines", "owner": "scripted owner",
            "authority": "scripted acceptance of new deadlines only",
            "reason": "retain progressing trials", "original_limits":
            {k: original[k] for k in ("hard_limit_seconds", "idle_limit_seconds")},
            "new_limits": {"hard_limit_seconds": None, "idle_limit_seconds": None}}


def test_exception_is_narrow_and_explicit(tmp_path):
    data, _ = settings()
    original = CampaignSettings.model_validate(data)
    changed = dict(data, hard_limit_seconds=None, idle_limit_seconds=None)
    new = CampaignSettings.model_validate(changed)
    auth = authorization(normalized_settings(original))
    assert validate_execution_exception(auth, original, new) == auth
    for key, value in [("profile", "minimal-14"), ("judge_model", "sc-scripted/broken-judge"),
                       ("trials_per_case", 2), ("provider_version", "v0.24.1"),
                       ("model_config", {"sc-scripted/driver": {"reasoning_effort": "low"}})]:
        with pytest.raises(ValueError):
            validate_execution_exception(auth, original, CampaignSettings.model_validate(dict(changed, **{key: value})))
    with pytest.raises(ValueError):
        validate_execution_exception(auth, original, original)
    with pytest.raises(ValueError):
        validate_execution_exception(dict(auth, new_limits={"hard_limit_seconds": 999999, "idle_limit_seconds": None}),
                                     original, new)


def test_deadline_free_public_campaign_survives_old_limits():
    data, config = settings(scripted_driver="asks-and-builds",
                            driver_command=["sh", "-c", 'sleep 3; exec sc-driver "$@"', "sh"])
    # This command succeeds despite the configuration file's 1s outer budgets,
    # proving the CLI flag and core/Inspect/target driver/idle layers together.
    data.update(hard_limit_seconds=1, idle_limit_seconds=1)
    config.write_text(json.dumps(data))
    result, _, _ = invoke("campaign", "--settings", config, "--no-outer-deadlines")
    assert result["settings"]["hard_limit_seconds"] is result["settings"]["idle_limit_seconds"] is None
    assert result["trials"][0]["outcome"] == "success"
    assert result["trials"][0]["wall_seconds"] > 3
    assert result["comparisons"][0]["core"]["timeouts"] == 0


def test_four_carried_failures_and_exactly_two_new_assignments():
    data, config = settings(scripted_driver="asks-but-does-not-build")
    data.update(trials_per_case=6)
    config.write_text(json.dumps(data))
    original, _, _ = invoke("campaign", "--settings", config)
    assert len(original["trials"]) == 6 and all(r["outcome"] == "failure" for r in original["trials"])
    source_root = Path(original["capture_run"])
    subset = PROOFS / (uuid.uuid4().hex + "-four")
    subset.mkdir()
    import shutil
    records = original["trials"][:4]
    manifest = json.loads((source_root / "run.json").read_text())
    for r in records:
        shutil.copytree(source_root / r["sample_id"], subset / r["sample_id"])
    (subset / "records.json").write_text(json.dumps({**manifest, "trials": records}))
    (subset / "run.json").write_text(json.dumps(manifest))
    auth_path = PROOFS / (uuid.uuid4().hex + "-authorization.json")
    auth_path.write_text(json.dumps(authorization(original["settings"])))
    changed = dict(data, hard_limit_seconds=None, idle_limit_seconds=None)
    new_config = PROOFS / (uuid.uuid4().hex + "-new.json")
    new_config.write_text(json.dumps(changed))
    output = PROOFS / (uuid.uuid4().hex + "-merged")

    def merge(dest, cfg=new_config, exception=True, src=subset):
        command = ["uv", "run", "--project", str(REPO / "evals"), "evalctl", "merge-captures",
                   str(dest), str(src), "--settings", str(cfg)]
        if exception:
            command += ["--execution-exception", str(auth_path)]
        return subprocess.run(command, cwd=REPO, capture_output=True, text=True)

    assert merge(output, exception=False).returncode != 0
    assert not output.exists()
    bad_config = PROOFS / (uuid.uuid4().hex + "-bad.json")
    bad_config.write_text(json.dumps(dict(changed, profile="minimal-14")))
    assert merge(output, cfg=bad_config).returncode != 0
    assert not output.exists()
    # A changed case/assignment is never covered by the deadline exception.
    bad_source = PROOFS / (uuid.uuid4().hex + "-wrong-assignment")
    shutil.copytree(subset, bad_source)
    m = json.loads((bad_source / "records.json").read_text())
    m["matched_assignment"][0]["case_version"] = "999"
    (bad_source / "records.json").write_text(json.dumps(m))
    assert merge(output, src=bad_source).returncode != 0
    assert not output.exists()
    assert merge(output).returncode == 0
    index = json.loads((output / "records.json").read_text())
    assert index["trials"] == records
    for r in records:
        assert (output / r["sample_id"] / "meta.json").read_bytes() == (subset / r["sample_id"] / "meta.json").read_bytes()
    pending = [r["sample_id"] for r in original["trials"][4:]]
    assert index["execution_continuation"]["new_assignment_ids"] == pending
    result, _, markdown = invoke("campaign", "--settings", new_config, "--resume", output)
    assert result["trials"][:4] == records  # NO carried_from mutation, no rescoring
    assert result["execution_continuation"] == index["execution_continuation"]
    assert "not a uniform-limit comparison" in markdown
    launched = [p.name for p in Path(result["capture_run"]).iterdir() if p.is_dir()]
    assert sorted(launched) == sorted(pending)
    assert result["comparisons"][0]["core"]["trial_count"] == 6
    # Historical row edits fail before any new campaign starts.
    index["trials"][0]["outcome"] = "success"
    (output / "records.json").write_text(json.dumps(index))
    invoke("campaign", "--settings", new_config, "--resume", output, code=2)
    # An ordinary resume matcher also remains exact.
    invoke("campaign", "--settings", config, "--resume", output, code=2)


def test_deadline_free_pi_exchange_still_cancels_and_cleans_helper(monkeypatch, tmp_path):
    from evalcore.pi import PiAPI
    import sys
    helper = tmp_path / "sleep.py"
    helper.write_text("import sys,time\nfor line in sys.stdin: time.sleep(3600)\n")
    model = PiAPI("scripted/model", helper_command=[sys.executable, str(helper)])

    async def proof():
        task = asyncio.create_task(model._exchange({}, None))
        while model.process is None:
            await asyncio.sleep(0.01)
        proc = model.process
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert proc.returncode is not None and model.process is None
        await model.aclose()
    asyncio.run(proof())


def test_driver_launcher_cancellation_kills_descendants(tmp_path):
    # Production launcher in a real mount-free container. Stop its group explicitly;
    # neither parent nor progressing descendants may remain.
    from scevals.image import trial_container
    with trial_container() as (container, _):
        def docker(*args, **kw):
            return subprocess.run(["docker", "exec", container, *args], capture_output=True, text=True, **kw)
        subprocess.run(["docker", "cp", str(ROOT / "scevals/driver_process.py"), container + ":/tmp/driver.py"], check=True, capture_output=True)
        proc = subprocess.Popen(["docker", "exec", container, "sh", "-c",
                                 "mkdir -p /workspace/trial; python3 /tmp/driver.py sh -c 'sleep 3600 & wait'"],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                reading = docker("cat", "/workspace/trial/driver.pid")
                if reading.returncode == 0:
                    break
                time.sleep(0.05)
            else:
                pytest.fail("launcher never started")
            wrapper = docker("python3", "-c",
                             "from pathlib import Path; p=Path('/proc') / '" + reading.stdout.strip() +
                             "' / 'status'; print(next(l.split(':')[1].strip() for l in p.read_text().splitlines() if l.startswith('PPid:')))").stdout.strip()
            assert wrapper.isdigit()
            docker("kill", "-TERM", wrapper, check=True)
            proc.wait(timeout=10)
            assert docker("sh", "-c", "ps -eo args | grep '[s]leep 3600'").returncode != 0
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=10)
