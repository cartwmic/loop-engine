// Real SDK provider fixture: no network or credentials.
import { createAssistantMessageEventStream } from "@earendil-works/pi-ai/compat";
export default function(pi) {
  let originals = [];
  pi.registerProvider("signed-sdk", {
    api: "claude-compat-messages", apiKey: "synthetic-no-network", baseUrl: "https://invalid.example",
    models: [{ id: "model", name: "scripted", reasoning: true, input: ["text"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 10000, maxTokens: 1000 }],
    streamSimple(model, context, options) {
      const stream = createAssistantMessageEventStream();
      const history = context.messages.filter(m => m.role === "assistant");
      const drop = context.messages.some(m => m.role === "user" && m.content[0]?.text === "drop probe");
      const valid = options.reasoning === "medium" && (drop ?
        history.every(m => m.content.every(b => b.type !== "thinking")) :
        history.every((m, i) => JSON.stringify(m) === JSON.stringify(originals[i])));
      const n = originals.length;
      const content = n < 2 && !drop ? [
        { type: "thinking", thinking: " \nπ signed text\t \n", thinkingSignature: "synthetic-signature-" + n },
        { type: "text", text: "Interleaved text " },
        { type: "thinking", thinking: "[Reasoning redacted]", thinkingSignature: "synthetic-redacted-data", redacted: true },
        { type: "thinking", thinking: "second signed block\n", thinkingSignature: "synthetic-second-signature" },
        { type: "toolCall", id: "signed-call-" + n, name: "lookup", arguments: { key: String(n) } }
      ] : [{ type: "text", text: "scripted-replay-accepted" }];
      const message = { role: "assistant", api: model.api, provider: model.provider, model: model.id,
        timestamp: n + 1, stopReason: content.at(-1).type === "toolCall" ? "toolUse" : "stop", content,
        usage: { input: 1, output: 2, cacheRead: 0, cacheWrite: 0, totalTokens: 3,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } } };
      if (!valid) {
        message.stopReason = "error"; message.errorMessage = "400 Invalid `signature` in `thinking` block";
        stream.push({ type: "error", reason: "error", error: message });
      } else {
        originals.push(JSON.parse(JSON.stringify(message)));
        stream.push({ type: "done", reason: message.stopReason, message });
      }
      stream.end();
      return stream;
    }
  });
}
