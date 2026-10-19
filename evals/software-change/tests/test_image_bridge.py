"""Black-box real Pi -> public Inspect bridge -> scripted backends, no paid calls."""
import json
import subprocess
from pathlib import Path

import anyio
from inspect_ai import Task, eval
from inspect_ai.agent import sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.model import (ChatCompletionChoice, ChatMessageAssistant, ModelAPI,
                              ModelOutput, ModelUsage, get_model, modelapi)
from inspect_ai.solver import solver
from inspect_ai.tool import ToolCall
from inspect_ai.util import sandbox_service, sandbox

from scevals.image import DEFAULT_IMAGE

CAPTURES = Path(__file__).resolve().parents[1] / "captures" / "bridge"
CALLS = []
ANSWERS = []


@modelapi(name="t2-scripted")
class ScriptedPiAPI(ModelAPI):
    async def generate(self, input, tools, tool_choice, config):
        tool_messages = [message for message in input if message.role == "tool"]
        CALLS.append({"model": self.model_name, "tool_results": [message.text for message in tool_messages]})
        if not tool_messages:
            assert any(tool.name == "bash" for tool in tools), "real Pi must expose bash"
            command = 'ask-owner "What is required?"' if self.model_name == "driver" else f"printf 'tool-{self.model_name}'"
            return ModelOutput(model=self.model_name, choices=[ChatCompletionChoice(
                message=ChatMessageAssistant(content="", tool_calls=[ToolCall(
                    id="t2_bash", function="bash", arguments={"command": command})]),
                stop_reason="tool_calls")],
                usage=ModelUsage(input_tokens=10, output_tokens=5, total_tokens=15))
        expected = "owner-answer" if self.model_name == "driver" else f"tool-{self.model_name}"
        assert expected in tool_messages[-1].text, "Pi must actually execute the tool"
        result = ModelOutput.from_content(model=self.model_name, content=f"completed-{self.model_name}")
        result.usage = ModelUsage(input_tokens=12, output_tokens=6, total_tokens=18)
        return result


@solver
def pi_contract_probe():
    async def solve(state, generate):
        env = sandbox()
        done = anyio.Event()
        started = anyio.Event()
        async def owner(question: str) -> str:
            ANSWERS.append(question)
            if question == "timeout-question":
                await anyio.sleep(3)
            if question == "error-question":
                raise RuntimeError("scripted-private-error-canary")
            if question == "malformed-question":
                return {"unexpected": "not text"}
            return "owner-answer"
        async def service():
            await sandbox_service("sc_owner", {"ask_owner": owner},
                                  until=done.is_set, sandbox=env, started=started, instance="t2-proof")
        async with anyio.create_task_group() as group:
            group.start_soon(service)
            await started.wait()
            try:
                outputs = {}
                # Real bridge aliases point to independently identifiable scripted Models.
                for swapped in (False, True):
                    identities = {"role-driver": "driver", "role-implementer": "b" if swapped else "a",
                                  "role-reviewer": "a" if swapped else "b"}
                    models = {role: get_model(f"t2-scripted/{name}", role=role, memoize=False)
                              for role, name in identities.items()}
                    async with sandbox_agent_bridge(model_aliases=models, port=14141):
                        for command, role in (("sc-driver", "role-driver"), ("sc-implement", "role-implementer"),
                                              ("sc-review", "role-reviewer")):
                            result = await env.exec([command, "Execute the assigned probe."],
                                                    env={"SC_BRIDGE_URL": "http://localhost:14141/v1",
                                                         "SC_OWNER_INSTANCE": "t2-proof"}, timeout=90)
                            assert result.success, f"{command} failed: {result.stderr}"
                            assert f"completed-{identities[role]}" in result.stdout
                            outputs[f"{swapped}/{role}"] = result.stdout.strip()
                # A prompt may not replace the harness-owned model selection.
                invalid = await env.exec(["sc-implement", "--model", "role-reviewer"], timeout=10)
                assert invalid.returncode == 2
                empty = await env.exec(["ask-owner", ""], timeout=10)
                assert empty.returncode == 2
                unavailable = await env.exec(["ask-owner", "probe"],
                                             env={"SC_OWNER_INSTANCE": "not-started"}, timeout=10)
                assert unavailable.returncode == 1
                stdin = await env.exec(["sh", "-c", "printf 'stdin-question' | ask-owner"],
                                       env={"SC_OWNER_INSTANCE": "t2-proof"}, timeout=20)
                assert stdin.success and stdin.stdout.strip() == "owner-answer"
                timeout = await env.exec(["ask-owner", "probe"],
                                         env={"SC_OWNER_INSTANCE": "t2-proof", "SC_OWNER_TIMEOUT_SECONDS": "0"}, timeout=10)
                assert timeout.returncode == 1
                for question in ("timeout-question", "error-question", "malformed-question"):
                    bad = await env.exec(["ask-owner", question],
                                         env={"SC_OWNER_INSTANCE": "t2-proof", "SC_OWNER_TIMEOUT_SECONDS": "1"},
                                         timeout=10)
                    assert bad.returncode == 1
                    assert "scripted-private-error-canary" not in bad.stdout + bad.stderr
                connection = await env.connection()
                config = json.loads(subprocess.check_output(
                    ["docker", "inspect", connection.container], text=True))[0]
                assert config["Mounts"] == []
                assert not config["HostConfig"].get("Binds")
                assert not config["HostConfig"].get("Mounts")
                state.metadata["mounts"] = config["Mounts"]
                state.metadata["probe_outputs"] = outputs
                state.completed = True
            finally:
                done.set()
        return state
    return solve


def test_real_pi_role_bridge_and_owner_service(tmp_path):
    CALLS.clear()
    ANSWERS.clear()
    compose = tmp_path / "compose.yaml"
    compose.write_text(f"services:\n  default:\n    image: {DEFAULT_IMAGE}\n    platform: linux/arm64\n"
                       "    working_dir: /workspace\n    command: [sleep, infinity]\n")
    CAPTURES.mkdir(parents=True, exist_ok=True)
    task = Task(name="t2-image-public-pi", dataset=[Sample(input="scripted image contract")],
                solver=pi_contract_probe(), sandbox=("docker", str(compose)),
                model="t2-scripted/driver", time_limit=300)
    logs = eval(task, log_dir=str(CAPTURES / "logs"), display="none", retry_on_error=0,
                log_model_api=False, sandbox_cleanup=True)
    assert len(logs) == 1
    assert logs[0].status == "success", logs[0].error
    sample = logs[0].samples[0]
    assert sample.error is None, sample.error
    assert len(CALLS) == 12, CALLS  # six real Pi sessions, two generations/tool loop each
    assert ANSWERS == ["What is required?", "What is required?", "stdin-question",
                       "timeout-question", "error-question", "malformed-question"]
    assert {call["model"] for call in CALLS} == {"driver", "a", "b"}
    assert set(sample.role_usage) >= {"role-driver", "role-implementer", "role-reviewer"}
    receipt = {"outputs": sample.metadata["probe_outputs"], "mounts": sample.metadata["mounts"],
               "backend_calls": CALLS,
               "owner_questions": ANSWERS, "role_usage": {role: usage.model_dump()
                                                           for role, usage in sample.role_usage.items()}}
    (CAPTURES / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print("Real Pi role aliases, swapped mappings, bash tool loops and host owner RPC: passed")



def test_codex_scripted_stream_through_real_pi_bridge(tmp_path, monkeypatch):
    """Codex item-only SSE -> Inspect bridge -> actual Pi bash/tool-result loop."""
    import time
    import httpx
    from evalcore import codex

    auth = tmp_path / "auth.json"
    auth.write_text(json.dumps({"openai-codex": {
        "access": "synthetic-bridge-access", "refresh": "synthetic-bridge-refresh",
        "accountId": "synthetic-bridge-account", "expires": time.time() * 1000 + 600000}}))
    monkeypatch.setattr(codex, "codex_auth_path", lambda: auth)
    calls = []

    def backend(request):
        body = json.loads(request.content)
        assert str(request.url) == codex.CODEX_BASE_URL + "/responses"
        assert body["stream"] is True and body["store"] is False
        assert any(tool["name"] == "bash" for tool in body["tools"])
        results = [item for item in body["input"] if item.get("type") == "function_call_output"]
        calls.append({"tool_result_seen": bool(results)})
        if not results:
            item = {"type": "function_call", "id": "fc_bridge", "call_id": "call_bridge",
                    "name": "bash", "arguments": json.dumps({"command": "printf codex-tool-proof"}),
                    "status": "completed"}
        else:
            assert "codex-tool-proof" in results[-1]["output"]
            item = {"type": "message", "id": "msg_bridge", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": "codex-pi-completed", "annotations": []}]}
        response = {"id": "resp_bridge", "object": "response", "created_at": 1,
                    "model": "gpt-6.1-sol", "status": "completed", "output": [],
                    "parallel_tool_calls": True, "tool_choice": "auto", "tools": [],
                    "usage": {"input_tokens": 12, "output_tokens": 3, "total_tokens": 15,
                              "input_tokens_details": {"cached_tokens": 7},
                              "output_tokens_details": {"reasoning_tokens": 0}}}
        events = [{"type": "response.output_item.done", "output_index": 0, "item": item, "sequence_number": 0},
                  {"type": "response.completed", "response": response, "sequence_number": 1}]
        content = "".join("data: " + json.dumps(event) + "\n\n" for event in events)
        return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=content)

    @solver
    def probe():
        async def solve(state, generate):
            env = sandbox()
            model = get_model("codex/gpt-6.1-sol", role="role-driver", memoize=False,
                              http_client=httpx.AsyncClient(transport=httpx.MockTransport(backend)))
            try:
                async with sandbox_agent_bridge(model_aliases={"role-driver": model}, port=14141):
                    result = await env.exec(["sc-driver", "Run the scripted probe."],
                                            env={"SC_BRIDGE_URL": "http://localhost:14141/v1"}, timeout=90)
                    assert result.success, "real Pi Codex bridge probe failed"
                    assert "codex-pi-completed" in result.stdout
                absent = await env.exec(["sh", "-c", 'test ! -e "$HOME/.pi/agent/auth.json"'], timeout=10)
                assert absent.success, "host auth must not enter the container"
                connection = await env.connection()
                container = json.loads(subprocess.check_output(["docker", "inspect", connection.container], text=True))[0]
                assert container["Mounts"] == []
                state.metadata["codex_scripted_pi"] = {"tool_loop": True, "host_auth_absent": True, "mounts": []}
                state.completed = True
                return state
            finally:
                await model.api.aclose()
        return solve

    compose = tmp_path / "compose.yaml"
    compose.write_text(f"services:\n  default:\n    image: {DEFAULT_IMAGE}\n    platform: linux/arm64\n"
                       "    working_dir: /workspace\n    command: [sleep, infinity]\n")
    capture = CAPTURES / "codex"
    capture.mkdir(parents=True, exist_ok=True)
    task = Task(name="codex-scripted-real-pi", dataset=[Sample(input="scripted Codex bridge")],
                solver=probe(), sandbox=("docker", str(compose)), model="t2-scripted/driver", time_limit=180)
    logs = eval(task, log_dir=str(capture / "logs"), display="none", retry_on_error=0,
                log_model_api=False, sandbox_cleanup=True)
    assert logs[0].status == "success", "Inspect Codex/Pi scripted integration failed"
    sample = logs[0].samples[0]
    assert sample.error is None
    assert calls == [{"tool_result_seen": False}, {"tool_result_seen": True}]
    usage = sample.role_usage["role-driver"]
    assert usage.total_tokens == 30 and usage.input_tokens_cache_read == 14
    receipt = {**sample.metadata["codex_scripted_pi"], "backend_calls": calls, "usage": usage.model_dump()}
    (capture / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
