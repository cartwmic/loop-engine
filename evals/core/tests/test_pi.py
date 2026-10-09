"""Public Inspect calls via scripted JSONL and actual Pi SDK; no live calls."""
import asyncio
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from inspect_ai.model import (ChatMessageSystem, ChatMessageTool, ChatMessageUser,
                              GenerateConfig, get_model)
from inspect_ai.tool import ToolFunction, ToolInfo

from evalcore import pi
from evalcore.report import UnsafeReportError, protected_values, scan_results
from evalcore.settings import CampaignSettings

SCRIPT = Path(__file__).with_name("scripted_pi_helper.py")
TOOL = ToolInfo(name="lookup", description="Find a key", parameters={
    "type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]})


def model(capture, name="model"):
    return get_model(f"pi/scripted/{name}", memoize=False,
                     helper_command=[sys.executable, str(SCRIPT), str(capture)],
                     config=GenerateConfig(reasoning_effort="medium", max_retries=0))


def test_public_completion_tool_roundtrip_and_usage(tmp_path):
    capture = tmp_path / "requests.jsonl"
    async def run():
        m = model(capture)
        try:
            first = await m.generate([ChatMessageSystem(content="system"), ChatMessageUser(content="hello")])
            pid = m.api.process.pid
            second = await m.generate([ChatMessageUser(content="lookup")], tools=[TOOL], tool_choice=ToolFunction(name="lookup"))
            third = await m.generate([ChatMessageUser(content="lookup"), second.message,
                                      ChatMessageTool(content="found", tool_call_id="pi-call", function="lookup")], tools=[TOOL])
            assert m.api.process.pid == pid  # one long-lived helper for all calls
            assert first.completion == third.completion == "pi-scripted-completed"
            assert first.message.content[0].signature == "synthetic-signature"
            assert second.message.tool_calls[0].arguments == {"key": "example"}
            assert second.choices[0].stop_reason == "tool_calls"
            assert first.usage.model_dump(exclude_none=True) == {
                "input_tokens": 12, "output_tokens": 3, "total_tokens": 24,
                "input_tokens_cache_read": 7, "input_tokens_cache_write": 2, "reasoning_tokens": 1}
        finally:
            await m.api.aclose()
        assert m.api.process is None
    asyncio.run(run())
    requests = [json.loads(line) for line in capture.read_text().splitlines()]
    assert len(requests) == 3
    assert all(r["options"]["reasoning"] == "medium" for r in requests)
    assert requests[1]["tool_choice"] == {"name": "lookup"}
    assert requests[1]["tools"][0]["parameters"] == TOOL.parameters.model_dump(exclude_none=True)
    assert requests[2]["messages"][1]["native"]["content"][0]["id"] == "pi-call"


@pytest.mark.parametrize("name", ["malformed", "error"])
def test_helper_errors_are_sanitized(tmp_path, name):
    async def run():
        m = model(tmp_path / "request", name)
        try:
            with pytest.raises(Exception) as error:
                await m.generate("hello")
            assert "synthetic-secret-canary" not in str(error.value)
            assert m.api.process is None
        finally:
            await m.api.aclose()
    asyncio.run(run())


def test_cancellation_during_retry_backoff_reaps_helper(tmp_path, monkeypatch):
    monkeypatch.setattr(pi, "RETRY_BASE_DELAY_SECONDS", 60)
    async def run():
        m = model(tmp_path / "requests", "retryable")
        task = asyncio.create_task(m.generate("hello"))
        while m.api.process is None or not (tmp_path / "requests").exists():
            await asyncio.sleep(.01)
        await asyncio.sleep(.05)  # scripted response has entered the long backoff
        process = m.api.process
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert process.returncode is not None and m.api.process is None
        await m.api.aclose()
    asyncio.run(run())


def test_cancellation_reaps_helper(tmp_path):
    async def run():
        m = model(tmp_path / "requests", "hang")
        task = asyncio.create_task(m.generate("hello"))
        while m.api.process is None:
            await asyncio.sleep(.01)
        process = m.api.process
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert process.returncode is not None and m.api.process is None
        await m.api.aclose()
    asyncio.run(run())


@pytest.mark.parametrize("role", ["models", "combination", "simulator", "judge"])
def test_pi_auth_is_protected(tmp_path, monkeypatch, role):
    monkeypatch.setenv("PI_CODING_AGENT_DIR", str(tmp_path))
    auth = tmp_path / "auth.json"
    auth.write_text('{"scripted":{"access":"synthetic-protected-pi-access"}}')
    data = dict(models=["fake/a"], simulator_model="fake/a", judge_model="fake/a", cases_path=".")
    if role == "models":
        data["models"] = ["pi/scripted/model"]
    elif role == "combination":
        data.pop("models")
        data["combinations"] = [dict(id="pi", driver="fake/a", implementer="pi/scripted/model", reviewer="fake/a")]
    else:
        data[role + "_model"] = "pi/scripted/model"
    settings = CampaignSettings(**data)
    assert settings.provider_config_paths == (auth,)
    for value in ("synthetic-protected-pi-access",):
        with pytest.raises(UnsafeReportError):
            scan_results([value], protected_values({}, settings.provider_config_paths))


@pytest.mark.parametrize("failure", [None, "transient", "non-retryable", "exhausted", "policy", "signature", "malformed"])
def test_actual_sdk_loads_provider_extension_and_makes_one_completion(tmp_path, monkeypatch, failure):
    """Production helper + real public SDK + discovered provider, entirely scripted."""
    agent = tmp_path / "agent"
    extensions = agent / "extensions"
    extensions.mkdir(parents=True)
    (agent / "auth.json").write_text("{}")
    (agent / "settings.json").write_text("{}")
    (extensions / "scripted.ts").write_text('''
import { createAssistantMessageEventStream, getCurrentTools } from "@earendil-works/pi-ai/compat";
export default function(pi) {
  let attempts = 0;
  pi.registerProvider("scripted-sdk", {
    api: "claude-compat-messages", apiKey: "synthetic-no-network", baseUrl: "https://invalid.example",
    models: [{ id: "model", name: "scripted", reasoning: true, input: ["text"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 10000, maxTokens: 1000 }],
    streamSimple(model, context, options) {
      const stream = createAssistantMessageEventStream();
      void (async () => {
        if (options.reasoning !== "medium" || options.toolChoice.name !== "lookup" ||
            getCurrentTools(context.messages)[0].parameters.required[0] !== "key" ||
            context.messages.at(-1).content[0].text !== "SDK probe") throw new Error("bad mapping");
        const message = { role: "assistant", api: model.api, provider: model.provider, model: model.id,
          timestamp: 1, stopReason: "toolUse", content: [{ type: "toolCall", id: "sdk-call", name: "lookup", arguments: { key: "sdk" } }],
          usage: { input: 1, output: 2, cacheRead: 3, cacheWrite: 4, totalTokens: 10,
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } } };
        stream.push({ type: "start", partial: message });
        stream.push({ type: "done", reason: "toolUse", message }); stream.end();
      })();
      return stream;
    }
  });
}
''')
    extension = extensions / "scripted.ts"
    if failure:
        condition = "++attempts === 1" if failure == "transient" else "++attempts > 0"
        message = {"non-retryable": "400 content refusal", "policy":
                   "400 This request was blocked as it seems to violate Anthropic's Terms of Service restrictions on reverse engineering or duplicating model outputs",
                   "signature": "400 messages.1.content.288: Invalid `signature` in `thinking` block",
                   "malformed": "400 malformed request"}.get(failure, "503 overloaded")
        message += " Bearer synthetic-sensitive-token sk-synthetic-sensitive-key eyJsynthetic.eyJpayloadxx.syntheticSignature"
        extension.write_text(extension.read_text().replace('        stream.push({ type: "start", partial: message });', f'''
        if ({condition}) {{
          message.stopReason = "error"; message.errorMessage = {json.dumps(message)};
          stream.push({{ type: "error", reason: "error", error: message }}); stream.end(); return;
        }}
        stream.push({{ type: "start", partial: message }});'''))
    log = Path(pi.__file__).resolve().parents[2] / "software-change/logs/pi-helper-errors.log"
    old_lines = len(log.read_text().splitlines()) if log.exists() else 0
    monkeypatch.setattr(pi, "RETRY_BASE_DELAY_SECONDS", 0)
    monkeypatch.setenv("PI_CODING_AGENT_DIR", str(agent))
    async def run():
        m = get_model("pi/scripted-sdk/model", memoize=False, config=GenerateConfig(reasoning_effort="medium", max_retries=0, timeout=30))
        try:
            if failure in ("non-retryable", "exhausted", "policy", "signature", "malformed"):
                with pytest.raises(Exception) as error:
                    await m.generate("SDK probe", tools=[TOOL], tool_choice=ToolFunction(name="lookup"))
                assert "sensitive" not in str(error.value)
                assert m.api.process is None
                if failure == "non-retryable":
                    from evalcore.target import ModelError
                    assert isinstance(error.value, ModelError)
                else:
                    from evalcore.target import InfrastructureError
                    assert isinstance(error.value, InfrastructureError)
                    if failure != "exhausted":
                        assert str(error.value) == ("provider policy block" if failure == "policy" else "provider rejected bridged request")
            else:
                output = await m.generate("SDK probe", tools=[TOOL], tool_choice=ToolFunction(name="lookup"))
                assert output.message.tool_calls[0].id == "sdk-call"
                assert output.message.tool_calls[0].arguments == {"key": "sdk"}
                assert output.usage.total_tokens == 10
        finally:
            await m.api.aclose()
    asyncio.run(run())
    if failure:
        entries = [json.loads(line) for line in log.read_text().splitlines()[old_lines:]]
        assert len(entries) == (6 if failure == "exhausted" else 1)
        assert all(e["timestamp"] and e["model"] == "scripted-sdk/model" for e in entries)
        assert all("sensitive" not in e["message"] and "eyJ" not in e["message"] for e in entries)
        assert all("[REDACTED]" in e["message"] for e in entries)


def test_node_mapping_tool_result_and_images(tmp_path):
    helper = Path(pi.__file__).with_name("pi_helper.mjs")
    script = tmp_path / "mapping.mjs"
    script.write_text(f'''
import {{ messagesForPi, toolChoiceForPi }} from {json.dumps(helper.as_uri())};
import {{ writeSync }} from "node:fs";
const model = {{ provider: "p", id: "m", api: "anthropic-messages" }};
const mapped = messagesForPi({{tools: [], messages: [
 {{role: "user", content: [{{type: "image", image: "data:image/png;base64,YQ=="}}]}},
 {{role: "assistant", content: "", tool_calls: [{{id: "c", function: "lookup", arguments: {{key: "a"}}}}]}},
 {{role: "tool", content: "bad", tool_call_id: "c", error: {{message: "failed"}}}}
]}}, model);
if (mapped.messages[3].toolName !== "lookup" || !mapped.messages[3].isError ||
    mapped.messages[1].content[0].mimeType !== "image/png" ||
    toolChoiceForPi("any", model).type !== "any") throw new Error("mapping failed");
writeSync(1, "mapping passed\\n");
''')
    result = subprocess.run(["node", str(script)], capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, "Node mapping failed"
    assert "mapping passed" in result.stdout


def test_signed_replay_and_drop_through_public_bridge_real_sdk(tmp_path, monkeypatch):
    from inspect_ai.agent import agent_bridge
    from openai import AsyncOpenAI
    agent = tmp_path / "agent"
    extensions = agent / "extensions"
    extensions.mkdir(parents=True)
    (agent / "auth.json").write_text("{}")
    (agent / "settings.json").write_text("{}")
    (extensions / "signed.ts").write_text(Path(__file__).with_name("scripted_signed_pi.ts").read_text())
    monkeypatch.setenv("PI_CODING_AGENT_DIR", str(agent))

    async def run():
        m = get_model("pi/signed-sdk/model", memoize=False,
                      config=GenerateConfig(reasoning_effort="medium", max_retries=0, timeout=30))
        try:
            with m:
                async with agent_bridge(web_search=False, code_execution=False, client_mcp_servers=False) as bridge:
                    bridge.model_aliases = {"inspect/probe": m}
                    async with AsyncOpenAI(api_key="synthetic-no-network", max_retries=0) as client:
                        messages = [{"role": "user", "content": "signed probe"}]
                        tools = [{"type": "function", "function": TOOL.model_dump(mode="json", exclude_none=True)}]
                        for n in range(3):
                            out = await client.chat.completions.create(model="inspect/probe", messages=messages, tools=tools)
                            msg = out.choices[0].message
                            if n < 2:
                                assert msg.tool_calls[0].id == f"signed-call-{n}"
                                messages += [msg.model_dump(exclude_none=True), {"role": "tool", "tool_call_id": f"signed-call-{n}", "content": "found"}]
                            else:
                                assert msg.content.strip() == "scripted-replay-accepted"
                        # Evict all originals: the very same lossy bridge history must
                        # now safely omit thinking, not attempt a guessed signature.
                        m.api.originals.clear()
                        messages[0]["content"] = "drop probe"
                        out = await client.chat.completions.create(model="inspect/probe", messages=messages, tools=tools)
                        assert out.choices[0].message.content.strip() == "scripted-replay-accepted"
        finally:
            await m.api.aclose()
    asyncio.run(run())


def test_replay_cache_is_byte_exact_immutable_and_collision_safe(tmp_path):
    from inspect_ai.model import ChatMessageAssistant, ContentReasoning, ContentText
    from inspect_ai.tool import ToolCall
    original = {"content": [{"type": "thinking", "thinking": " \nπ\t ", "thinkingSignature": "signature"}]}
    message = ChatMessageAssistant(content=[ContentReasoning(reasoning="π", signature="altered"), ContentText(text="text")],
                                   tool_calls=[ToolCall(id="c", function="lookup", arguments={"key": "x"})])
    cache = {pi.replay_key(message): original}
    request = pi.message_request(message, cache)
    assert json.dumps(request["native"], ensure_ascii=False).encode() == json.dumps(original, ensure_ascii=False).encode()
    request["native"]["content"][0]["thinking"] = "mutated"
    assert original["content"][0]["thinking"] == " \nπ\t "
    cache[pi.replay_key(message)] = None
    message.metadata = {"pi_message": original}
    request = pi.message_request(message, cache)
    assert request["native"] is None
    assert all(b["type"] != "reasoning" for b in request["content"])
