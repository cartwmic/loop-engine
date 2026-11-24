#!/usr/bin/env python3
"""Tests for formal/fetch-statechart.sh, the statechart download in formal/check.sh.

A local git HTTP server (git http-backend) stands in for GitHub, so no test uses the
network. The server refuses fetch attempts with 429: the first N, or all of them.
Each attempt makes one info/refs request, so the server counts attempts exactly.
CI runs this test in the `checks` job. Standard library only; needs git.
"""

import http.server
import os
import pathlib
import re
import subprocess
import tempfile
import threading
import unittest

FORMAL = pathlib.Path(__file__).resolve().parent
FETCH = FORMAL / "fetch-statechart.sh"
STATECHART = "pstack/skills/statechart/scripts/statechart"


class StubHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.serve()

    def do_POST(self):
        self.serve()

    def serve(self):
        path, _, query = self.path.partition("?")
        server = self.server
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        if path.endswith("/info/refs"):
            server.attempts += 1
            if server.refuse is None or server.attempts <= server.refuse:
                self.send_error(429, "Too Many Requests")
                return
        env = dict(os.environ, GIT_PROJECT_ROOT=str(server.root), GIT_HTTP_EXPORT_ALL="1",
                   REQUEST_METHOD=self.command, PATH_INFO=path, QUERY_STRING=query,
                   CONTENT_TYPE=self.headers.get("Content-Type", ""),
                   CONTENT_LENGTH=str(len(body)), REMOTE_ADDR="127.0.0.1")
        for header, var in (("Git-Protocol", "HTTP_GIT_PROTOCOL"),
                            ("Content-Encoding", "HTTP_CONTENT_ENCODING")):
            if self.headers.get(header):
                env[var] = self.headers[header]
        cgi = subprocess.run(["git", "http-backend"], input=body, env=env,
                             capture_output=True).stdout
        parts = re.split(rb"\r?\n\r?\n", cgi, maxsplit=1)
        if len(parts) != 2:  # http-backend failed before it wrote headers (an unknown commit)
            self.send_error(500, "git http-backend failed")
            return
        head, out = parts
        status, headers = 200, []
        for line in head.decode().splitlines():
            name, _, value = line.partition(":")
            if name.lower() == "status":
                status = int(value.split()[0])
            else:
                headers.append((name, value.strip()))
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(out)


class GitStub(http.server.ThreadingHTTPServer):
    def __init__(self, root, refuse):
        super().__init__(("127.0.0.1", 0), StubHandler)
        self.root = root
        self.refuse = refuse  # attempts to refuse with 429; None refuses every attempt
        self.attempts = 0

    def url(self, repo):
        return f"http://127.0.0.1:{self.server_address[1]}/{repo}"


class FetchStatechartTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.cache = self.root / "cache"
        self.cache.mkdir()
        self.commit = self.make_source_repo()

    def make_source_repo(self):
        """A bare repo holding one commit with a statechart script; returns the commit."""
        work = self.root / "work"
        script = work / STATECHART
        script.parent.mkdir(parents=True)
        script.write_text("#!/bin/sh\necho fake statechart\n")
        script.chmod(0o755)
        git = ["git", "-c", "user.name=test", "-c", "user.email=test@example.invalid"]
        subprocess.run(["git", "init", "-q", str(work)], check=True)
        subprocess.run([*git, "-C", str(work), "add", "."], check=True)
        subprocess.run([*git, "-C", str(work), "commit", "-q", "-m", "statechart"], check=True)
        subprocess.run(["git", "clone", "-q", "--bare", str(work), str(self.root / "source.git")],
                       check=True)
        subprocess.run(["git", "-C", str(self.root / "source.git"), "config",
                        "uploadpack.allowAnySHA1InWant", "true"], check=True)
        return subprocess.run(["git", "-C", str(work), "rev-parse", "HEAD"], check=True,
                              capture_output=True, text=True).stdout.strip()

    def serve(self, refuse):
        server = GitStub(self.root, refuse)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server

    def fetch(self, repo_url, commit):
        dest = self.cache / f"statechart-{commit}"
        env = dict(os.environ, LOOP_FORMAL_FETCH_DELAY="0")
        result = subprocess.run([str(FETCH), repo_url, commit, str(dest)],
                                capture_output=True, text=True, env=env)
        return result, dest

    def test_refused_twice_then_fetched(self):
        server = self.serve(refuse=2)
        result, dest = self.fetch(server.url("source.git"), self.commit)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(os.access(dest / STATECHART, os.X_OK), result.stderr)
        self.assertEqual(server.attempts, 3)
        self.assertIn("fetch attempt 2 of 4 failed", result.stderr)
        self.assertNotIn("attempt 3 of 4 failed", result.stderr)

    def test_always_refused_fails_with_clear_message(self):
        server = self.serve(refuse=None)
        url = server.url("source.git")
        result, dest = self.fetch(url, self.commit)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn(f"could not fetch statechart {self.commit} from {url} after 4 attempts", result.stderr)
        self.assertIn("429", result.stderr)
        self.assertEqual(server.attempts, 4)
        self.assertFalse(dest.exists())
        self.assertEqual(list(self.cache.glob("statechart.*")), [])

    def test_unknown_commit_fails_with_clear_message(self):
        server = self.serve(refuse=0)
        result, dest = self.fetch(server.url("source.git"), "0" * 40)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("after 4 attempts", result.stderr)
        self.assertFalse(dest.exists())


if __name__ == "__main__":
    unittest.main()
