"""Public Inspect generation through the real OpenAI SDK, with scripted SSE."""
import asyncio
import json
import time
import tomllib
from pathlib import Path

import httpx
import pytest
from inspect_ai.model import ChatMessageSystem, ChatMessageUser, GenerateConfig, get_model
from inspect_ai.tool import ToolFunction, ToolInfo
from tenacity import RetryError

from evalcore import codex
from evalcore.report import UnsafeReportError, protected_values, scan_results, write_report
from evalcore.settings import CampaignSettings, Combination


@pytest.fixture
def auth_file(tmp_path, monkeypatch):
    path = tmp_path / "auth.json"
    path.write_text(json.dumps({"openai-codex": {
        "access": "synthetic-access-credential", "refresh": "synthetic-refresh-credential",
        "accountId": "synthetic-account-identifier", "expires": time.time() * 1000 + 60000}}, indent=2))
    monkeypatch.setattr(codex, "codex_auth_path", lambda: path)
    return path


def response(output=None, status="completed"):
    return {"id": "resp_fixture", "object": "response", "created_at": 1,
            "model": "gpt-6.1-sol", "status": status, "output": output or [],
            "parallel_tool_calls": True, "tool_choice": "auto", "tools": [],
            "usage": {"input_tokens": 12, "output_tokens": 3, "total_tokens": 15,
                      "input_tokens_details": {"cached_tokens": 7},
                      "output_tokens_details": {"reasoning_tokens": 1}}}


def text_item(text="Scripted nonempty reply."):
    return {"type": "message", "id": "msg_fixture", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}]}


def tool_item():
    return {"type": "function_call", "id": "fc_fixture", "call_id": "call_fixture",
            "name": "lookup", "arguments": '{"key":"example"}', "status": "completed"}


def sse(items, *, terminal_output=None, terminal="response.completed", duplicate=False):
    events = []
    # Deliberately reversed arrival: assembly must follow output_index.
    for index, item in reversed(list(enumerate(items))):
        events.append({"type": "response.output_item.done", "output_index": index, "item": item})
    if duplicate and items:
        events.append({"type": "response.output_item.done", "output_index": 0, "item": items[0]})
    if terminal:
        events.append({"type": terminal, "response": response(terminal_output, terminal.split(".")[-1])})
    return "".join("data: " + json.dumps(dict(event, sequence_number=i)) + "\n\n"
                   for i, event in enumerate(events)).encode()


def scripted_model(items, *, requests, **stream_args):
    def handle(request):
        requests.append(json.loads(request.content))
        assert str(request.url) == codex.CODEX_BASE_URL + "/responses"
        assert request.headers["originator"] == "pi"
        assert request.headers["OpenAI-Beta"] == "responses=experimental"
        assert request.headers["chatgpt-account-id"] == "synthetic-account-identifier"
        return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=sse(items, **stream_args))
    client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    return get_model("codex/gpt-6.1-sol", memoize=False, http_client=client)


@pytest.mark.parametrize("mode", ["text", "tool", "mixed", "terminal-output", "reasoning", "incomplete"])
def test_public_generation_assembles_stream_and_usage(auth_file, mode):
    requests = []
    items = [text_item()] if mode in ("text", "terminal-output") else [tool_item()]
    if mode == "mixed":
        items = [text_item(), tool_item()]
    if mode == "reasoning":
        items = [{"type": "reasoning", "id": "rs_fixture", "summary": [],
                  "encrypted_content": "synthetic-reasoning-state"}, text_item()]
    if mode == "incomplete":
        items = [text_item()]
    terminal_output = [text_item("Authoritative terminal reply.")] if mode == "terminal-output" else None

    async def run():
        model = scripted_model(items, requests=requests, terminal_output=terminal_output, duplicate=True,
                               terminal="response.incomplete" if mode == "incomplete" else "response.completed")
        try:
            return await model.generate(
                [ChatMessageSystem(content="System fixture."), ChatMessageUser(content="Tiny request.")],
                tools=[ToolInfo(name="lookup", description="Look up a key", parameters={
                    "type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]})],
                tool_choice=ToolFunction(name="lookup") if mode == "tool" else "auto", config=GenerateConfig(max_tokens=20))
        finally:
            await model.api.aclose()
    output = asyncio.run(run())
    assert len(requests) == 1  # no OpenAI organization probe
    request = requests[0]
    assert request["stream"] is True and request["store"] is False
    assert request["instructions"] == "System fixture."
    assert request["include"] == ["reasoning.encrypted_content"]
    assert "max_output_tokens" not in request
    assert all(item.get("role") != "developer" for item in request["input"])
    assert request["tools"][0]["strict"] is False
    if mode == "tool":
        assert request["tool_choice"] == {"type": "function", "name": "lookup"}
    if mode in ("text", "mixed", "reasoning", "incomplete"):
        assert output.completion == "Scripted nonempty reply."
    if mode == "reasoning":
        assert any(getattr(content, "reasoning", None) == "synthetic-reasoning-state"
                   and content.signature == "rs_fixture" and content.redacted
                   for content in output.message.content)
    if mode == "terminal-output":
        assert output.completion == "Authoritative terminal reply."
    if mode in ("tool", "mixed"):
        calls = output.message.tool_calls
        assert len(calls) == 1
        assert calls[0].function == "lookup" and calls[0].arguments == {"key": "example"}
        assert calls[0].id == "call_fixture"
    # Inspect reports uncached input separately from cache-read tokens.
    assert output.usage.input_tokens == 5 and output.usage.output_tokens == 3
    assert output.usage.input_tokens_cache_read == 7 and output.usage.total_tokens == 15


def test_instructions_default_and_concurrent_requests(auth_file):
    requests = []
    async def run():
        model = scripted_model([text_item()], requests=requests)
        try:
            outputs = await asyncio.gather(model.generate("First"), model.generate("Second"))
            return outputs
        finally:
            await model.api.aclose()
    assert all(output.completion for output in asyncio.run(run()))
    assert len(requests) == 2
    assert all(request["instructions"] == "You are a helpful assistant." for request in requests)


def test_no_terminal_is_error(auth_file):
    async def run():
        model = scripted_model([text_item()], requests=[], terminal=None)
        try:
            with pytest.raises(RetryError) as error:
                await model.generate("Tiny", config=GenerateConfig(max_retries=0))
            assert "without a terminal" in str(error.value.last_attempt.exception())
        finally:
            await model.api.aclose()
    asyncio.run(run())


@pytest.mark.parametrize("failure", ["missing", "malformed", "provider-missing", "expired", "bad-expiry", "bad-account"])
def test_auth_fails_before_client_creation(auth_file, failure, monkeypatch):
    data = json.loads(auth_file.read_text())
    if failure == "missing":
        auth_file.unlink()
    elif failure == "malformed":
        auth_file.write_text("{synthetic-access-credential")
    elif failure == "provider-missing":
        auth_file.write_text("{}")
    else:
        key, value = {"expired": ("expires", 0), "bad-expiry": ("expires", "bad"),
                      "bad-account": ("accountId", None)}[failure]
        data["openai-codex"][key] = value
        auth_file.write_text(json.dumps(data))
    monkeypatch.setattr(codex.OpenAIAPI, "_create_client", lambda self: pytest.fail("client must not be created"))
    with pytest.raises(ValueError, match="run pi once") as error:
        get_model("codex/gpt-6.1-sol", memoize=False)
    assert "synthetic-access-credential" not in str(error.value)


@pytest.mark.parametrize("role", ["models", "combination", "simulator", "judge"])
def test_settings_add_host_auth_to_protection(auth_file, role):
    data = dict(models=["fake/a"], simulator_model="fake/a", judge_model="fake/a", cases_path=".")
    if role == "models":
        data["models"] = ["codex/gpt-6.1-sol"]
    elif role == "combination":
        data.pop("models")
        data["combinations"] = [Combination(id="combo", driver="fake/a", implementer="codex/gpt-6.1-sol", reviewer="fake/a")]
    else:
        data[role + "_model"] = "codex/gpt-6.1-sol"
    settings = CampaignSettings(**data)
    assert settings.provider_config_paths == (auth_file,)
    assert CampaignSettings(**dict(data, provider_config_paths=[auth_file])).provider_config_paths == (auth_file,)
    protected = protected_values({}, settings.provider_config_paths)
    scan_results(['{"safe": true}'], protected)
    for key in ("access", "refresh", "accountId"):
        with pytest.raises(UnsafeReportError):
            scan_results([json.loads(auth_file.read_text())["openai-codex"][key]], protected)


def test_report_refuses_codex_auth_before_writing(auth_file, tmp_path):
    settings = CampaignSettings(models=["codex/gpt-6.1-sol"], simulator_model="fake/a", judge_model="fake/a", cases_path=".")
    summary = dict(campaign="fixture", target="fake", classification_rules="fixture", settings={},
                   pinned_models={}, versions={}, matched_assignment=[], comparisons=[],
                   trials=[{"reason": "synthetic-refresh-credential"}])
    destination = tmp_path / "results"
    with pytest.raises(UnsafeReportError):
        write_report(destination, summary, provider_config_paths=settings.provider_config_paths, environment={})
    assert not destination.exists()


def test_entry_point_declared():
    project = Path(__file__).resolve().parents[2] / "pyproject.toml"
    data = tomllib.loads(project.read_text())
    assert data["project"]["entry-points"]["inspect_ai"]["codex"] == "evalcore.codex"
