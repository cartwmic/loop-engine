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


def test_actual_sdk_loads_provider_extension_and_makes_one_completion(tmp_path, monkeypatch):
    """Production helper + real public SDK + discovered provider, entirely scripted."""
    agent = tmp_path / "agent"
    extensions = agent / "extensions"
    extensions.mkdir(parents=True)
    (agent / "auth.json").write_text("{}")
    (agent / "settings.json").write_text("{}")
    (extensions / "scripted.ts").write_text('''
import { createAssistantMessageEventStream, getCurrentTools } from "@earendil-works/pi-ai/compat";
export default function(pi) {
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
    monkeypatch.setenv("PI_CODING_AGENT_DIR", str(agent))
    async def run():
        m = get_model("pi/scripted-sdk/model", memoize=False, config=GenerateConfig(reasoning_effort="medium", max_retries=0, timeout=30))
        try:
            output = await m.generate("SDK probe", tools=[TOOL], tool_choice=ToolFunction(name="lookup"))
            assert output.message.tool_calls[0].id == "sdk-call"
            assert output.message.tool_calls[0].arguments == {"key": "sdk"}
            assert output.usage.total_tokens == 10
        finally:
            await m.api.aclose()
    asyncio.run(run())


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
