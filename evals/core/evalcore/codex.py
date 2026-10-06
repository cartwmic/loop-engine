"""Host-only Codex subscription provider for the pinned Inspect Responses API."""
import json
import math
import time
from pathlib import Path

from inspect_ai.model import GenerateConfig, modelapi
from inspect_ai.model._providers.openai import OpenAIAPI
import httpx

CODEX_BASE_URL = "https://chatgpt.com/backend-api/codex"


def codex_auth_path() -> Path:
    return Path.home() / ".pi/agent/auth.json"


def read_auth() -> tuple[str, str]:
    # Never surface parser errors (they can contain credential bytes).
    try:
        auth = json.loads(codex_auth_path().read_text())["openai-codex"]
        access, account = auth["access"], auth["accountId"]
        expires = auth["expires"]
        valid = (isinstance(access, str) and bool(access.strip())
                 and isinstance(account, str) and bool(account.strip())
                 and isinstance(expires, (int, float)) and not isinstance(expires, bool)
                 and math.isfinite(expires))
        if not valid:
            raise ValueError()
    except (OSError, ValueError, KeyError, TypeError):
        raise ValueError("Codex auth unavailable or invalid in ~/.pi/agent/auth.json; run pi once to sign in/refresh.") from None
    if expires <= time.time() * 1000:
        raise ValueError("Codex subscription credentials expired; run pi once to refresh, then retry.")
    return access, account


class OutputItemStream:
    """Keep item.done payloads when Codex's terminal response omits output.

    Each request owns its accumulator; the SDK stream still owns connection
    cleanup and Inspect still handles deltas, errors, parsing and token usage.
    """
    def __init__(self, stream):
        self.stream = stream

    async def __aenter__(self):
        await self.stream.__aenter__()
        return self

    async def __aexit__(self, *args):
        return await self.stream.__aexit__(*args)

    async def __aiter__(self):
        items = {}
        async for event in self.stream:
            if event.type == "response.output_item.done":
                items[event.output_index] = event.item
            elif event.type in ("response.completed", "response.incomplete", "response.failed"):
                if not event.response.output and items:
                    response = event.response.model_copy(update={"output": [items[i] for i in sorted(items)]})
                    event = event.model_copy(update={"response": response})
            yield event


class CodexResponses:
    def __init__(self, responses):
        self.responses = responses

    def __getattr__(self, name):
        return getattr(self.responses, name)

    async def create(self, **request):
        request["stream"] = True
        request["store"] = False
        # Codex expects initial system instructions in this top-level field.
        messages = list(request.get("input", []))
        instructions = []
        while messages and isinstance(messages[0], dict) and messages[0].get("role") in ("system", "developer"):
            message = messages.pop(0)
            content = message.get("content", [])
            if isinstance(content, str):
                instructions.append(content)
            else:
                instructions.extend(part["text"] for part in content if part.get("type") == "input_text")
        request["input"] = messages
        request["instructions"] = request.get("instructions") or "\n\n".join(instructions) or "You are a helpful assistant."
        include = list(request.get("include", []))
        if "reasoning.encrypted_content" not in include:
            include.append("reasoning.encrypted_content")
        request["include"] = include
        # The subscription backend does not accept the API token-limit field.
        request.pop("max_output_tokens", None)
        request.pop("background", None)  # the subscription backend rejects the parameter
        return OutputItemStream(await self.responses.create(**request))


def is_transport_error(ex):
    """A dropped or failed connection (httpx or the httpcore error under it)."""
    seen = set()
    while ex is not None and id(ex) not in seen:
        seen.add(id(ex))
        if (type(ex).__module__.split(".")[0] in ("httpx", "httpcore")
                and type(ex).__name__ in TRANSPORT_ERRORS):
            return True
        ex = ex.__cause__ or ex.__context__
    return False


TRANSPORT_ERRORS = {"RemoteProtocolError", "ReadError", "WriteError", "ConnectError", "ReadTimeout"}


@modelapi(name="codex")
class CodexAPI(OpenAIAPI):
    def __init__(self, model_name, base_url=None, api_key=None, config=GenerateConfig(), **model_args):
        if base_url is not None or api_key is not None or any(key in model_args for key in
                ("responses_api", "responses_store", "streaming", "background", "default_headers")):
            raise ValueError("codex uses host Pi auth and fixed subscription endpoint/stream settings")
        access, account = read_auth()
        super().__init__(model_name, base_url=CODEX_BASE_URL, api_key=access,
                         config=config, responses_api=True, responses_store=False,
                         streaming=True, background=False,
                         default_headers={"chatgpt-account-id": account, "originator": "pi",
                                          "OpenAI-Beta": "responses=experimental"}, **model_args)

    def _create_client(self):
        client = super()._create_client()
        client.responses = CodexResponses(client.responses)
        return client

    async def reasoning_summaries(self):
        # Inspect's OpenAI organization probe would make an unrelated extra call.
        return False

    def should_retry(self, ex):
        # The subscription backend sometimes drops a stream mid-body; that is a
        # transient transport failure, not model behaviour. Retry it.
        if is_transport_error(ex):
            return True
        return super().should_retry(ex)

    async def generate(self, input, tools, tool_choice, config):
        if config.batch or config.extra_body and config.extra_body.get("background"):
            raise ValueError("Codex subscription requires foreground streaming; batch/background unsupported")
        return await super().generate(input, tools, tool_choice, config)
