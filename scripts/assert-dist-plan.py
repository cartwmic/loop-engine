#!/usr/bin/env python3
"""Assert cargo-dist release plan contains exactly supported applications/targets."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, NoReturn, Set

TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
}
APPS = {
    "loop-cli",
    "policy-document-provider",
    "research-provider",
    "software-change-provider",
}
ARCHIVE_RE = re.compile(
    r"^(?P<app>loop-cli|policy-document-provider|research-provider|software-change-provider)-(?P<target>"
    r"aarch64-apple-darwin|x86_64-unknown-linux-gnu)\.tar\.xz$"
)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"cargo-dist plan assertion failed: {message}")


def archive_pairs(release: Dict[str, Any]) -> Set[tuple[str, str]]:
    artifacts = release.get("artifacts")
    if not isinstance(artifacts, list) or not all(isinstance(item, str) for item in artifacts):
        fail("each release must expose string artifact names")
    pairs: Set[tuple[str, str]] = set()
    for artifact in artifacts:
        if not artifact.endswith(".tar.xz"):
            continue
        match = ARCHIVE_RE.fullmatch(artifact)
        if not match:
            fail(f"unexpected archive artifact {artifact!r}")
        pair = (match.group("app"), match.group("target"))
        if pair in pairs:
            fail(f"duplicate archive artifact {artifact!r}")
        pairs.add(pair)
    return pairs


def self_test() -> int:
    duplicate = {
        "artifacts": [
            "loop-cli-aarch64-apple-darwin.tar.xz",
            "loop-cli-aarch64-apple-darwin.tar.xz",
        ]
    }
    try:
        archive_pairs(duplicate)
    except SystemExit as error:
        if "duplicate archive artifact" not in str(error):
            raise
    else:
        fail("self-test accepted duplicate archive artifact")
    print("cargo-dist plan assertion self-test passed: duplicate archive rejected")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) != 2:
        print(
            f"usage: {Path(sys.argv[0]).name} DIST-PLAN.json | --self-test",
            file=sys.stderr,
        )
        return 2
    path = Path(sys.argv[1])
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"could not read {path}: {error}")
    if not isinstance(manifest, dict):
        fail("manifest must be an object")

    releases = manifest.get("releases")
    if not isinstance(releases, list) or len(releases) != len(APPS):
        fail(f"expected exactly {len(APPS)} release applications")
    release_names: list[str] = []
    pairs: Set[tuple[str, str]] = set()
    for release in releases:
        if not isinstance(release, dict):
            fail("release entries must be objects")
        app = release.get("app_name")
        if not isinstance(app, str) or app not in APPS:
            fail(f"unexpected release application {app!r}")
        release_names.append(app)
        release_pairs = archive_pairs(release)
        expected_app_pairs = {(app, target) for target in TARGETS}
        if release_pairs != expected_app_pairs:
            fail(
                f"expected {app} archive targets {sorted(expected_app_pairs)}, "
                f"got {sorted(release_pairs)}"
            )
        pairs.update(release_pairs)
    if set(release_names) != APPS or len(release_names) != len(set(release_names)):
        fail(f"expected release applications {sorted(APPS)}, got {release_names}")
    expected_pairs = {(app, target) for app in APPS for target in TARGETS}
    if pairs != expected_pairs:
        fail(f"expected archive matrix {sorted(expected_pairs)}, got {sorted(pairs)}")

    ci = manifest.get("ci")
    github = ci.get("github") if isinstance(ci, dict) else None
    matrix = github.get("artifacts_matrix") if isinstance(github, dict) else None
    includes = matrix.get("include") if isinstance(matrix, dict) else None
    if not isinstance(includes, list) or len(includes) != len(TARGETS):
        fail(f"expected exactly {len(TARGETS)} native matrix entries")
    matrix_targets: list[str] = []
    for entry in includes:
        if not isinstance(entry, dict):
            fail("native matrix entries must be objects")
        targets = entry.get("targets")
        if not isinstance(targets, list) or len(targets) != 1 or not isinstance(targets[0], str):
            fail("each native matrix entry must select exactly one target")
        matrix_targets.append(targets[0])
    if set(matrix_targets) != TARGETS or len(matrix_targets) != len(set(matrix_targets)):
        fail(f"expected native targets {sorted(TARGETS)}, got {sorted(matrix_targets)}")

    print(
        "cargo-dist plan matrix ok: "
        f"apps={len(APPS)} targets={','.join(sorted(TARGETS))} archives={len(pairs)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
