"""Host-only Inspect provider using Pi's installed SDK and normal agent resources.

The JSONL helper owns OAuth and provider extensions. No credentials cross this
protocol, and no agent loop or host tools are ever run by the helper.
"""
import asyncio
import json
import shutil
from pathlib import Path

from inspect_ai.model import (ChatCompletionChoice, ChatMessageAssistant, ContentReasoning,
                              ContentText, GenerateConfig, ModelAPI, ModelOutput, ModelUsage, modelapi)
from inspect_ai.tool import ToolCall, ToolFunction

from .codex import codex_auth_path


def pi_config_paths() -> tuple[Path, ...]:
    # Pi respects PI_CODING_AGENT_DIR; match that public SDK setting on the host.
    import os
    directory = Path(os.environ.get("PI_CODING_AGENT_DIR", str(codex_auth_path().parent))).expanduser()
    return (directory / "auth.json",)


def helper_command() -> list[str]:
    node = shutil.which("node")
    if not node:
        raise ValueError("pi provider requires host Node and the installed Pi SDK")
    sdk = Path(node).resolve().parents[1] / "lib/node_modules/@earendil-works/pi-coding-agent/dist/index.js"
    if not sdk.is_file():
        raise ValueError("pi provider requires the globally installed Pi SDK beside host Node")
    return [node, str(Path(__file__).with_name("pi_helper.mjs")), str(sdk)]


def message_request(message):
    data = message.model_dump(mode="json", exclude_none=True)
    # Preserve native signed thinking/text/tool blocks for multi-turn replay.
    if message.role == "assistant":
        data["native"] = (message.metadata or {}).get("pi_message")
    return data


@modelapi(name="pi")
class PiAPI(ModelAPI):
    def __init__(self, model_name, base_url=None, api_key=None, config=GenerateConfig(),
                 helper_command=None, **model_args):
        if base_url is not None or api_key is not None or model_args:
            raise ValueError("pi uses the host Pi SDK, agent resources and credentials; endpoint/auth overrides unsupported")
        provider, separator, model = model_name.partition("/")
        if not separator or not provider or not model:
            raise ValueError("pi model must be pi/<pi-provider>/<model>")
        super().__init__(model_name, config=config)
        self.provider, self.pi_model = provider, model
        self.command = helper_command or globals()["helper_command"]()
        self.process = None
        self.lock = asyncio.Lock()

    async def _stop(self):
        process, self.process = self.process, None
        if process:
            if process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
            await process.wait()

    async def _exchange(self, request, timeout):
        async with self.lock:
            try:
                async with asyncio.timeout(timeout):
                    if self.process is None:
                        self.process = await asyncio.create_subprocess_exec(
                            *self.command, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                            stderr=asyncio.subprocess.DEVNULL, limit=16 * 1024 * 1024)
                    self.process.stdin.write((json.dumps(request) + "\n").encode())
                    await self.process.stdin.drain()
                    response = json.loads(await self.process.stdout.readline())
                    if not isinstance(response, dict) or "message" not in response:
                        raise ValueError()
                    return response["message"]
            except asyncio.CancelledError:
                await self._stop()
                raise
            except Exception:
                await self._stop()
                # Never propagate helper stderr, request content or provider error payloads.
                raise RuntimeError("Pi host completion failed; check host Pi setup/auth and retry") from None

    async def generate(self, input, tools, tool_choice, config):
        if config.batch or config.num_choices not in (None, 1) or config.response_schema or config.extra_body:
            raise ValueError("pi supports one foreground completion; batch/response_schema/extra_body unsupported")
        effort = config.reasoning_effort
        levels = {"none": "off", "minimal": "minimal", "low": "low", "medium": "medium", "high": "high",
                  "xhigh": "xhigh", "max": "max"}
        if effort is not None and effort not in levels:
            raise ValueError("pi reasoning_effort must be none/minimal/low/medium/high/xhigh/max")
        choice = {"name": tool_choice.name} if isinstance(tool_choice, ToolFunction) else tool_choice
        options = {key: value for key, value in {
            "reasoning": levels.get(effort), "maxTokens": config.max_tokens,
            "temperature": config.temperature, "topP": config.top_p,
            "maxRetries": 0,  # Inspect owns retries; no hidden helper retry loop.
        }.items() if value is not None}
        native = await self._exchange({"provider": self.provider, "model": self.pi_model,
                                      "messages": [message_request(m) for m in input],
                                      "tools": [t.model_dump(mode="json", exclude_none=True) for t in tools],
                                      "tool_choice": choice, "options": options}, config.timeout or 300)
        try:
            if native["stopReason"] in ("error", "aborted", "pending", "deferred"):
                raise ValueError()
            content, calls = [], []
            for block in native["content"]:
                if block["type"] == "text":
                    content.append(ContentText(text=block["text"]))
                elif block["type"] == "thinking":
                    content.append(ContentReasoning(reasoning=block["thinking"],
                                                    signature=block.get("thinkingSignature"),
                                                    redacted=block.get("redacted", False)))
                elif block["type"] == "toolCall":
                    calls.append(ToolCall(id=block["id"], function=block["name"], arguments=block["arguments"]))
                else:
                    raise ValueError()
            usage = native["usage"]
            return ModelOutput(model=self.model_name, choices=[ChatCompletionChoice(
                message=ChatMessageAssistant(content=content or "", tool_calls=calls or None,
                                             metadata={"pi_message": native}),
                stop_reason={"stop": "stop", "length": "max_tokens", "toolUse": "tool_calls"}[native["stopReason"]])],
                usage=ModelUsage(input_tokens=usage["input"], output_tokens=usage["output"],
                                 input_tokens_cache_read=usage["cacheRead"], input_tokens_cache_write=usage["cacheWrite"],
                                 total_tokens=usage["totalTokens"], reasoning_tokens=usage.get("reasoning")))
        except (ValueError, KeyError, TypeError):
            raise RuntimeError("Pi host returned an invalid completion; no provider payload exported") from None

    async def aclose(self):
        async with self.lock:
            await self._stop()

    def close(self):
        if self.process and self.process.returncode is None:
            try:
                self.process.kill()
            except ProcessLookupError:
                pass
