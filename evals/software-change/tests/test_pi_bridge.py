"""pi/ trial role through the existing public bridge and real sandbox Pi."""
import json
import subprocess
import sys
from pathlib import Path

import pytest

from inspect_ai import Task, eval
from inspect_ai.agent import sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.model import GenerateConfig, get_model
from inspect_ai.solver import solver
from inspect_ai.util import sandbox

from evalcore import pi
from scevals.image import DEFAULT_IMAGE


@pytest.mark.parametrize("transient", [False, True])
def test_pi_scripted_helper_through_real_pi_trial_role(tmp_path, monkeypatch, transient):
    monkeypatch.setattr(pi, "RETRY_BASE_DELAY_SECONDS", 0)
    helper = Path(__file__).resolve().parents[2] / "core/tests/scripted_pi_helper.py"
    requests = tmp_path / "requests.jsonl"

    @solver
    def probe():
        async def solve(state, generate):
            env = sandbox()
            model = get_model("pi/scripted/transient" if transient else "pi/scripted/model", role="role-driver", memoize=False,
                              helper_command=[sys.executable, str(helper), str(requests)],
                              config=GenerateConfig(reasoning_effort="medium", max_retries=0))
            try:
                async with sandbox_agent_bridge(model_aliases={"role-driver": model}, port=14141):
                    result = await env.exec(["sc-driver", "Run the scripted Pi-provider probe."],
                                            env={"SC_BRIDGE_URL": "http://localhost:14141/v1"}, timeout=90)
                    assert result.success, "real Pi scripted provider bridge failed"
                    assert "pi-scripted-completed" in result.stdout
                absent = await env.exec(["sh", "-c", 'test ! -e "$HOME/.pi/agent/auth.json"'], timeout=10)
                assert absent.success, "host credentials must not enter trial containers"
                connection = await env.connection()
                container = json.loads(subprocess.check_output(["docker", "inspect", connection.container], text=True))[0]
                assert container["Mounts"] == []
                assert not container["HostConfig"].get("Binds")
                state.metadata["pi_scripted_bridge"] = {"tool_loop": True, "host_auth_absent": True, "mounts": []}
                state.completed = True
                return state
            finally:
                await model.api.aclose()
        return solve

    compose = tmp_path / "compose.yaml"
    compose.write_text(f"services:\n  default:\n    image: {DEFAULT_IMAGE}\n    platform: linux/arm64\n"
                       "    working_dir: /workspace\n    command: [sleep, infinity]\n")
    capture = Path(__file__).resolve().parents[1] / "captures/F3/pi-bridge"
    capture.mkdir(parents=True, exist_ok=True)
    # Root evaluation model is the same scripted helper, not a live provider.
    root_model = get_model("pi/scripted/model", memoize=False,
                           helper_command=[sys.executable, str(helper)])
    task = Task(name="pi-scripted-real-pi", dataset=[Sample(input="scripted Pi bridge")], solver=probe(),
                sandbox=("docker", str(compose)), model=root_model, time_limit=180)
    logs = eval(task, log_dir=str(capture / "logs"), display="none", retry_on_error=0,
                log_model_api=False, sandbox_cleanup=True)
    assert logs[0].status == "success", "Inspect/Pi scripted integration failed"
    sample = logs[0].samples[0]
    assert sample.error is None, "scripted sandbox sample failed"
    calls = [json.loads(line) for line in requests.read_text().splitlines()]
    assert len(calls) == (3 if transient else 2)
    if transient:
        assert calls[0] == calls[1]  # same host request resent, agent sees no error
        calls = calls[1:]
    assert all(c["options"]["reasoning"] == "medium" for c in calls)
    assert not any(m["role"] == "tool" for m in calls[0]["messages"])
    assert any(m["role"] == "tool" and "pi-tool-proof" in str(m["content"]) for m in calls[1]["messages"])
    usage = sample.role_usage["role-driver"]
    assert usage.total_tokens == 48 and usage.input_tokens_cache_read == 14 and usage.input_tokens_cache_write == 4
    receipt = {**sample.metadata["pi_scripted_bridge"], "backend_calls": 3 if transient else 2, "thinking_level": "medium",
               "role_usage": usage.model_dump()}
    (capture / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
