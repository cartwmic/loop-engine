"""JSONL backend fixture; no SDK, auth, network or tools executed here."""
import json
import sys
from pathlib import Path

attempt = 0
for line in sys.stdin:
    attempt += 1
    request = json.loads(line)
    if len(sys.argv) > 1:
        with Path(sys.argv[1]).open("a") as stream:
            stream.write(json.dumps(request) + "\n")
    if request["model"] == "retryable" or request["model"] == "transient" and attempt == 1:
        print(json.dumps({"error": "retryable"}), flush=True)
        continue
    if request["model"] == "hang":
        import time
        time.sleep(60)
    if request["model"] == "malformed":
        print("malformed synthetic-secret-canary", flush=True)
        continue
    if request["model"] == "error":
        print(json.dumps({"error": "synthetic-secret-canary"}), flush=True)
        continue
    results = [m for m in request["messages"] if m["role"] == "tool"]
    names = [t["name"] for t in request["tools"]]
    if request["model"] in ("signed-replay", "signed-drop"):
        # Nontrivial whitespace, multiple/redacted blocks and interleaved text:
        # Inspect's text bridge cannot preserve this signed native sequence.
        for assistant in (m for m in request["messages"] if m["role"] == "assistant"):
            original = assistant.get("native")
            if request["model"] == "signed-drop":
                assert original is None
                assert not any(b["type"] == "reasoning" for b in assistant["content"])
                continue
            assert original is not None, "signed history must be restored"
            assert original["content"][0] == {"type": "thinking", "thinking": " \nπ signed text\t \n",
                                               "thinkingSignature": "synthetic-signature-" + original["content"][-1]["id"]}
            assert original["content"][2] == {"type": "thinking", "thinking": "[Reasoning redacted]",
                                               "thinkingSignature": "synthetic-redacted-data", "redacted": True}
        if len(results) < 2:
            call_id = "pi-call-" + str(len(results) + 1)
            name = "bash" if "bash" in names else names[0]
            arguments = {"command": "printf pi-tool-proof"} if name == "bash" else {"key": "example"}
            content = [{"type": "thinking", "thinking": " \nπ signed text\t \n", "thinkingSignature": "synthetic-signature-" + call_id},
                       {"type": "text", "text": "Interleaved text "},
                       {"type": "thinking", "thinking": "[Reasoning redacted]", "thinkingSignature": "synthetic-redacted-data", "redacted": True},
                       {"type": "thinking", "thinking": "second signed block\n", "thinkingSignature": "synthetic-second-signature"},
                       {"type": "toolCall", "id": call_id, "name": name, "arguments": arguments}]
            stop = "toolUse"
        else:
            content = [{"type": "text", "text": "pi-scripted-completed"}]
            stop = "stop"
    elif names and not results and request["tool_choice"] != "none":
        name = request["tool_choice"].get("name") if isinstance(request["tool_choice"], dict) else ("bash" if "bash" in names else names[0])
        assert name in names
        arguments = {"command": "printf pi-tool-proof"} if name == "bash" else {"key": "example"}
        content = [{"type": "toolCall", "id": "pi-call", "name": name, "arguments": arguments}]
        stop = "toolUse"
    else:
        if results and "bash" in names:
            assert "pi-tool-proof" in str(results[-1]["content"])
        content = [{"type": "thinking", "thinking": "Scripted reasoning", "thinkingSignature": "synthetic-signature"},
                   {"type": "text", "text": "pi-scripted-completed", "textSignature": "synthetic-text-signature"}]
        stop = "stop"
    print(json.dumps({"message": {"role": "assistant", "provider": request["provider"], "model": request["model"],
                                  "api": "scripted", "timestamp": 1, "stopReason": stop, "content": content,
                                  "usage": {"input": 12, "output": 3, "cacheRead": 7, "cacheWrite": 2,
                                            "totalTokens": 24, "reasoning": 1,
                                            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}}}), flush=True)
