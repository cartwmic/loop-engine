"""Owner context: every launcher gives its Pi the owner's design principles as the global AGENTS.md, byte for byte.

Two layers. The first runs the real launcher scripts with a stub pi that records the AGENTS.md in its agent directory
(no Docker, no model). The second, in a real arms image, runs real Pi against a stub OpenAI-compatible server and reads the
system prompt Pi sent. It runs only when SC_OWNER_CONTEXT_IMAGE names an image; SC_OWNER_CONTEXT_EXPECT is "present"
(default) or "absent" (the negative control: an image built before the change).

    SC_OWNER_CONTEXT_IMAGE=loop-engine-sc-evals:arms-s2-XXXX uv run --project evals pytest evals/software-change/tests/test_owner_context.py
"""
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading

import pytest

IMAGE_DIR = Path(__file__).resolve().parents[1] / "image"
CONTEXT = IMAGE_DIR / "owner-context" / "design-principles.md"
# design-principles.md at chezmoi commit 3517fd6, unedited. A changed file is a decision, not a drive-by edit.
PINNED_SHA256 = "82062f0e68be91969f2b6557e20557fc6407ec5d0ba9d0a314624d31e5b770bf"
LAUNCHERS = {"sc-driver": "pi-role.py", "sc-implement": "pi-role.py", "sc-review": "pi-role.py",
             "arm-native": "pi-arm.py", "arm-superpowers": "pi-arm.py", "arm-openspec": "pi-arm.py"}


def test_the_context_file_is_the_pinned_bytes_and_the_image_pins_the_same_hash():
    assert hashlib.sha256(CONTEXT.read_bytes()).hexdigest() == PINNED_SHA256
    assert PINNED_SHA256 in (IMAGE_DIR / "Dockerfile.arms").read_text()


def run_launcher(tmp_path, name, scripts_dir, extra_env=None):
    """Run the launcher under its installed name with stub pi and openspec; return the AGENTS.md pi saw (or None)."""
    bin_dir, stub_dir = tmp_path / "bin", tmp_path / "stub"
    bin_dir.mkdir()
    stub_dir.mkdir()
    (bin_dir / name).symlink_to(scripts_dir / LAUNCHERS[name])
    record = tmp_path / "agents.md"
    (stub_dir / "pi").write_text(
        f"#!{sys.executable}\nimport os, shutil\nsource = os.path.join(os.environ['PI_CODING_AGENT_DIR'], 'AGENTS.md')\n"
        "if os.path.exists(source):\n    shutil.copyfile(source, os.environ['SC_TEST_RECORD'])\n")
    (stub_dir / "openspec").write_text("#!/bin/sh\nexit 0\n")
    for stub in stub_dir.iterdir():
        stub.chmod(0o755)
    env = {"PATH": f"{stub_dir}{os.pathsep}{os.environ['PATH']}", "SC_TEST_RECORD": str(record), **(extra_env or {})}
    result = subprocess.run([sys.executable, str(bin_dir / name), "do it"], env=env, capture_output=True, text=True,
                            timeout=60, cwd=tmp_path)
    assert result.returncode == 0, result.stderr
    return record.read_bytes() if record.exists() else None


@pytest.mark.parametrize("name", sorted(LAUNCHERS))
def test_every_launcher_gives_pi_the_owner_context_byte_for_byte(tmp_path, name):
    assert run_launcher(tmp_path, name, IMAGE_DIR) == CONTEXT.read_bytes()


@pytest.mark.parametrize("name", sorted(LAUNCHERS))
def test_owner_context_off_is_the_only_switch_and_leaves_no_file(tmp_path, name):
    (tmp_path / "off").mkdir()
    (tmp_path / "on").mkdir()
    assert run_launcher(tmp_path / "off", name, IMAGE_DIR, {"SC_OWNER_CONTEXT": "off"}) is None
    assert run_launcher(tmp_path / "on", name, IMAGE_DIR, {"SC_OWNER_CONTEXT": "on"}) == CONTEXT.read_bytes()


@pytest.mark.parametrize("name", sorted(LAUNCHERS))
def test_an_image_without_the_file_gets_no_owner_context(tmp_path, name):
    scripts = tmp_path / "scripts"
    scripts.mkdir()
    for script in ("pi-role.py", "pi-arm.py"):
        shutil.copyfile(IMAGE_DIR / script, scripts / script)
    assert run_launcher(tmp_path, name, scripts) is None


# ---- real Pi in a real image -------------------------------------------------------------------------------------

IMAGE = os.environ.get("SC_OWNER_CONTEXT_IMAGE")
EXPECT = os.environ.get("SC_OWNER_CONTEXT_EXPECT", "present")


class StubModel(http.server.BaseHTTPRequestHandler):
    """Minimal OpenAI chat-completions server: log each request body, answer with one short text reply."""
    bodies = []

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        StubModel.bodies.append(body)
        chunk = {"id": "stub", "object": "chat.completion.chunk", "created": 0, "model": body.get("model", "stub")}
        text = {**chunk, "choices": [{"index": 0, "delta": {"role": "assistant", "content": "done"}, "finish_reason": None}]}
        end = {**chunk, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
        if body.get("stream"):
            payload = "".join(f"data: {json.dumps(c)}\n\n" for c in (text, end)) + "data: [DONE]\n\n"
            kind = "text/event-stream"
        else:
            payload = json.dumps({**end, "object": "chat.completion",
                                  "choices": [{"index": 0, "message": {"role": "assistant", "content": "done"},
                                               "finish_reason": "stop"}]})
            kind = "application/json"
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(payload.encode())))
        self.end_headers()
        self.wfile.write(payload.encode())

    def log_message(self, *args):
        pass


def system_prompts():
    return ["\n".join(m["content"] if isinstance(m["content"], str) else json.dumps(m["content"])
                      for m in body["messages"] if m["role"] in ("system", "developer"))
            for body in StubModel.bodies]


@pytest.mark.skipif(not IMAGE, reason="SC_OWNER_CONTEXT_IMAGE not set")
@pytest.mark.parametrize("name", sorted(LAUNCHERS))
def test_real_pi_in_the_image_sends_the_owner_context(name):
    server = http.server.ThreadingHTTPServer(("0.0.0.0", 0), StubModel)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    StubModel.bodies.clear()
    try:
        script = ("mkdir -p /workspace/public/repo && cd /workspace/public/repo && git init -q && "
                  f"sha256sum /opt/sc-evals/owner-context/design-principles.md 2>/dev/null; {name} 'say done'")
        result = subprocess.run(
            ["docker", "run", "--rm", "--platform", "linux/arm64",
             "-e", f"SC_BRIDGE_URL=http://host.docker.internal:{server.server_port}/v1",
             IMAGE, "sh", "-c", script], capture_output=True, text=True, timeout=600)
    finally:
        server.shutdown()
    assert result.returncode == 0, result.stderr[-2000:]
    prompts = system_prompts()
    assert prompts, "Pi never reached the stub model"
    text = CONTEXT.read_text()
    if EXPECT == "present":
        assert PINNED_SHA256 in result.stdout
        assert all(prompt.count(text.strip()) == 1 for prompt in prompts), "owner context missing or duplicated"
    else:
        assert PINNED_SHA256 not in result.stdout
        assert all("# Design principles" not in prompt for prompt in prompts), "owner context unexpectedly present"
