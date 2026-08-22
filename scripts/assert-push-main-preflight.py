#!/usr/bin/env python3
"""Assert direct-main pushes dispatch the unchanged deterministic preflight."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parents[1]
DISPATCHER = ROOT / ".github/workflows/push-to-main.yml"
PREFLIGHT = ROOT / ".github/workflows/preflight.yml"


def fail(message: str) -> NoReturn:
    raise SystemExit(f"push-main preflight assertion failed: {message}")


def main() -> int:
    try:
        dispatcher = DISPATCHER.read_text(encoding="utf-8")
        preflight = PREFLIGHT.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"could not read workflow: {error}")

    if not re.search(r"^on:\n  push:\n    branches:\n      - main\n", dispatcher, re.MULTILINE):
        fail("dispatcher must listen to direct pushes on main only")
    if re.search(r"^  (pull_request|workflow_dispatch|workflow_run|schedule):", dispatcher, re.MULTILINE):
        fail("dispatcher has an unsupported trigger")
    if "permissions:\n  contents: read" not in dispatcher:
        fail("dispatcher must grant read-only contents permission")
    if "ref: ${{ github.sha }}" not in dispatcher:
        fail("dispatcher checkout must pin the pushed SHA")
    if "cargo-dist/releases/download/v0.32.0/" not in dispatcher:
        fail("dispatcher must install pinned cargo-dist 0.32.0")
    if "dist plan --output-format=json" not in dispatcher:
        fail("dispatcher must compute a compact dist plan")
    if 'jq -c .' not in dispatcher or '"$GITHUB_OUTPUT"' not in dispatcher:
        fail("dispatcher must pass compact plan output")
    if "uses: ./.github/workflows/preflight.yml" not in dispatcher:
        fail("dispatcher must call reusable preflight")
    if "plan: ${{ needs.plan.outputs.manifest }}" not in dispatcher:
        fail("dispatcher must pass generated plan to preflight")
    forbidden = ("dist host", "gh release", "contents: write", "release:")
    for token in forbidden:
        if token in dispatcher:
            fail(f"dispatcher contains publication token {token!r}")

    required_preflight = (
        "cargo test --workspace",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "cargo fmt --all -- --check",
        "dist generate --check",
        "python3 scripts/assert-dist-plan.py --self-test",
        "python3 scripts/assert-dist-plan.py \"$RUNNER_TEMP/dist-plan.json\"",
        "python3 scripts/assert-release-gates.py",
        "python3 scripts/software-change-journey.py --self-test",
        "python3 scripts/research-journey.py --self-test",
        "python3 scripts/generate-prd-journey.py --self-test",
        "scripts/bookends-check-gate.sh",
        "python3 scripts/assert-generate-prd-profile.py",
        "cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check",
        "--traversal-depth full",
        "python3 scripts/policy-document-journey.py",
        "--profile crates/policy-document-provider/data/readme.json",
        "for mode in draft audit",
        "python3 scripts/research-journey.py",
        "--provider target/debug/research",
        "--profile crates/research-provider/data/configs/standard.json",
        "--profile crates/research-provider/data/configs/generate-prd.json",
        "--checker target/debug/bookends-check",
    )
    missing = [token for token in required_preflight if token not in preflight]
    if missing:
        fail(f"reusable preflight lost required gate(s): {missing}")
    if 'run: scripts/bookends-check-gate.sh' not in preflight or 'BOOKENDS_BYPASS: ""' not in preflight:
        fail("required CI must run the Bookends gate without a bypass")
    if re.search(r"^    if:", preflight, re.MULTILINE):
        fail("reusable preflight must not be independently skippable")
    if "version=2.14.0" not in preflight or "github.com/dagu-org/dagu/releases/download/v${version}/" not in preflight:
        fail("reusable preflight must install operator-provided dagu 2.14.0")
    if "echo \"$bin\" >> \"$GITHUB_PATH\"" not in preflight:
        fail("reusable preflight must put dagu on PATH")
    dagu_at = preflight.find("Install operator-provided dagu")
    cargo_at = preflight.find("cargo test --workspace")
    if dagu_at < 0 or cargo_at < 0 or dagu_at > cargo_at:
        fail("reusable preflight must install dagu onto PATH before cargo test")
    if "continue-on-error" in preflight:
        fail("reusable preflight must not skip a missing dagu")
    if "$RUNNER_TEMP/dagu-bin" not in preflight:
        fail("reusable preflight must install dagu into RUNNER_TEMP, not dist artifacts")

    print("push-main preflight ok: pushed SHA plan -> unchanged read-only reusable preflight")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
