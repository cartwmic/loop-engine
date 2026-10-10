// JSONL single-completion server. stdout is exclusively protocol frames;
// trusted owner extensions can log during loading, but those logs must never
// contaminate the protocol or disclose host credentials via Python exceptions.
import { pathToFileURL } from "node:url";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { appendFileSync, mkdirSync, chmodSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function redactProviderError(message) {
  return String(message)
    .replace(/\bBearer\s+[^\s"'<>]+/gi, "Bearer [REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]+/g, "[REDACTED]")
    .replace(/\b[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g, "[REDACTED]");
}

export function logProviderError(model, message) {
  const path = fileURLToPath(new URL("../../software-change/logs/pi-helper-errors.log", import.meta.url));
  mkdirSync(fileURLToPath(new URL("../../software-change/logs/", import.meta.url)), { recursive: true, mode: 0o700 });
  // JSONL escapes embedded newlines, preserving one timestamped entry per failure.
  appendFileSync(path, JSON.stringify({ timestamp: new Date().toISOString(), model: redactProviderError(model),
    message: redactProviderError(message) }) + "\n", { mode: 0o600 });
  chmodSync(path, 0o600);
}

export function providerInfrastructureReason(message) {
  // Only called on provider error payloads, never model completion/refusal text.
  const text = String(message);
  if (/request was blocked.*Terms of Service restrictions on reverse engineering or duplicating model outputs/is.test(text)) {
    return "provider policy block";
  }
  if (/(?:\b4\d\d\b|invalid_request_error|bad_request)/i.test(text) &&
      /invalid\s+[`'"]?signature|malformed request|invalid request|invalid.*thinking.*block|invalid request field in bridged request/i.test(text)) {
    return "provider rejected bridged request";
  }
  return null;
}

const write = process.stdout.write.bind(process.stdout);
process.stdout.write = () => true;
process.stderr.write = () => true;
for (const name of ["log", "info", "warn", "error", "debug"]) console[name] = () => {};
const zeroUsage = () => ({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
                          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } });

export function messagesForPi(request, model) {
  const names = new Map();
  for (const message of request.messages) {
    for (const call of message.tool_calls ?? []) names.set(call.id, call.function);
  }
  const blocks = (content) => typeof content === "string" ? [{ type: "text", text: content }] :
    content.map((block) => {
      if (block.type === "text") return { type: "text", text: block.text };
      if (block.type === "image") {
        const match = /^data:(image\/[^;]+);base64,(.+)$/s.exec(block.image);
        if (!match) throw new Error("Only inline images supported");
        return { type: "image", mimeType: match[1], data: match[2] };
      }
      if (block.type === "reasoning") return { type: "thinking", thinking: block.reasoning,
        thinkingSignature: block.signature, redacted: block.redacted };
      throw new Error("Unsupported message content");
    });
  const messages = request.messages.map((message) => {
    const timestamp = Date.now();
    if (message.role === "system") return { role: "system", content: message.content, timestamp };
    if (message.role === "user") return { role: "user", content: blocks(message.content), timestamp };
    if (message.role === "tool") return { role: "toolResult", toolCallId: message.tool_call_id,
      toolName: message.function ?? names.get(message.tool_call_id), content: blocks(message.content),
      isError: !!message.error, timestamp };
    if (message.role === "assistant") {
      if (message.native?.provider === model.provider && message.native?.model === model.id) return message.native;
      return { role: "assistant", api: model.api, provider: model.provider, model: model.id, timestamp,
        // Without a matching original, the bridge's signed reasoning is untrusted:
        // its whitespace, block boundaries and order may already have changed.
        content: [...blocks(message.content).filter((block) => block.type !== "thinking"), ...(message.tool_calls ?? []).map((call) => ({
          type: "toolCall", id: call.id, name: call.function, arguments: call.arguments }))],
        usage: zeroUsage(), stopReason: message.tool_calls?.length ? "toolUse" : "stop" };
    }
    throw new Error("Unsupported message role");
  });
  const tools = request.tools.map(({ name, description, parameters }) => ({ name, description, parameters }));
  // A leading declaration is needed even when Inspect supplies no system prompt.
  if (messages[0]?.role !== "system") messages.unshift({ role: "system", content: "", timestamp: Date.now() });
  messages[0] = { ...messages[0], toolsAdded: tools };
  return { messages };
}

export function toolChoiceForPi(choice, model) {
  if (choice === "auto" || choice === "none" || choice == null) return choice ?? "auto";
  // Pi's Anthropic implementation accepts its native forced-tool choice through
  // streamSimple. The compatibility extension itself encodes the wire tool name.
  if (model.api === "anthropic-messages" || model.api === "claude-compat-messages") {
    return choice === "any" ? { type: "any" } : { type: "tool", name: choice.name };
  }
  if (model.api === "openai-completions") {
    return choice === "any" ? "required" : { type: "function", function: { name: choice.name } };
  }
  if (model.api === "openai-responses") {
    return choice === "any" ? "required" : { type: "function", name: choice.name };
  }
  throw new Error("Forced tool selection unsupported for this Pi API");
}

async function main() {
  const sdkUrl = pathToFileURL(process.argv[2]);
  const { createAgentSession, SessionManager, ModelRuntime, getAgentDir } = await import(sdkUrl.href);
  // npm may hoist pi-ai next to pi-coding-agent (managed Pi) rather than
  // nest it in the SDK package (global npm). Resolve the actual installed copy.
  let directory = new URL("../", sdkUrl);
  let retryUrl;
  for (;;) {
    const candidate = new URL("node_modules/@earendil-works/pi-ai/dist/utils/retry.js", directory);
    if (existsSync(fileURLToPath(candidate))) { retryUrl = candidate; break; }
    const parent = new URL("../", directory);
    if (parent.href === directory.href) throw new Error("Installed Pi retry module missing");
    directory = parent;
  }
  const { isRetryableAssistantError } = await import(retryUrl.href);
  // The public session factory loads normal extensions and applies their provider
  // registrations to ModelRuntime. Never prompt this session or bind lifecycle
  // hooks: this is NOT an agent run and must not start MCP/tools/background work.
  const runtime = await ModelRuntime.create({ authPath: `${getAgentDir()}/auth.json`,
    modelsPath: `${getAgentDir()}/models.json`, allowModelNetwork: false });
  const { session, extensionsResult } = await createAgentSession({ agentDir: getAgentDir(), modelRuntime: runtime,
    noTools: "all", sessionManager: SessionManager.inMemory() });
  if (extensionsResult.errors.length) throw new Error("Host Pi extension loading failed");
  const sessionId = randomUUID();
  try {
    for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
      let modelName = "unknown";
      try {
        const request = JSON.parse(line);
        modelName = `${request.provider}/${request.model}`;
        const model = runtime.getModel(request.provider, request.model);
        if (!model) throw new Error("Host Pi model unavailable");
        const options = { ...request.options, sessionId, toolChoice: toolChoiceForPi(request.tool_choice, model) };
        if (options.reasoning === "off") delete options.reasoning;
        const result = await runtime.completeSimple(model, messagesForPi(request, model), options);
        if (result.stopReason === "error") {
          logProviderError(modelName, result.errorMessage ?? "Unknown provider error");
          const reason = providerInfrastructureReason(result.errorMessage);
          write(JSON.stringify(reason ? { error: "infrastructure", reason } :
            { error: isRetryableAssistantError(result) ? "retryable" : "non_retryable" }) + "\n");
        } else if (["aborted", "pending", "deferred"].includes(result.stopReason)) {
          throw new Error(`Pi completion ${result.stopReason}`);
        } else {
          write(JSON.stringify({ message: result }) + "\n");
        }
      } catch (error) {
        logProviderError(modelName, error?.message ?? String(error));
        const reason = providerInfrastructureReason(error?.message);
        write(JSON.stringify(reason ? { error: "infrastructure", reason } : { error: "non_retryable" }) + "\n");
      }
    }
  } finally {
    session.dispose();
  }
}

// An import is useful for offline mapping tests; execution starts only as CLI.
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  main().catch((error) => {
    try { logProviderError("initialization", error?.message ?? String(error)); } catch { /* disk failure */ }
    write(JSON.stringify({ error: "non_retryable" }) + "\n"); process.exitCode = 1;
  });
}
