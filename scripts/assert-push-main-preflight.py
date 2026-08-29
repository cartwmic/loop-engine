#!/usr/bin/env python3
"""Assert the direct-main dispatcher and reusable preflight remain fail-closed.

The push workflow is a read-only dispatcher.  The reusable preflight owns the
actual Rust/tooling and public-journey gates.  This checker intentionally uses
small textual contracts instead of a YAML dependency so it can run before the
workspace has been built.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parents[1]
DISPATCHER = ROOT / ".github/workflows/push-to-main.yml"
PREFLIGHT = ROOT / ".github/workflows/preflight.yml"
RUN_NEXTEST = ROOT / "scripts/run-nextest.py"
RUN_CENTRAL = ROOT / "scripts/run-central-tests.py"


class PreflightError(RuntimeError):
    """A required dispatcher or preflight property is missing."""


def fail(message: str) -> NoReturn:
    raise SystemExit(f"push-main preflight assertion failed: {message}")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PreflightError(message)


def position(text: str, token: str) -> int:
    found = text.find(token)
    require(found >= 0, f"missing required token {token!r}")
    return found


def validate_dispatcher(dispatcher: str) -> None:
    require(
        "on:\n  push:\n    branches:\n      - main\n" in dispatcher,
        "dispatcher must listen to direct pushes on main only",
    )
    require(
        not any(f"  {trigger}:" in dispatcher for trigger in ("pull_request", "workflow_dispatch", "workflow_run", "schedule")),
        "dispatcher has an unsupported trigger",
    )
    require("permissions:\n  contents: read" in dispatcher, "dispatcher must grant read-only contents permission")
    require("ref: ${{ github.sha }}" in dispatcher, "dispatcher checkout must pin the pushed SHA")
    require("cargo-dist/releases/download/v0.32.0/" in dispatcher, "dispatcher must install pinned cargo-dist 0.32.0")
    require("dist plan --output-format=json" in dispatcher, "dispatcher must compute a compact dist plan")
    require('jq -c .' in dispatcher and '"$GITHUB_OUTPUT"' in dispatcher, "dispatcher must pass compact plan output")
    require("uses: ./.github/workflows/preflight.yml" in dispatcher, "dispatcher must call reusable preflight")
    require("plan: ${{ needs.plan.outputs.manifest }}" in dispatcher, "dispatcher must pass generated plan to preflight")
    for token in ("dist host", "gh release", "contents: write", "release:"):
        require(token not in dispatcher, f"dispatcher contains publication token {token!r}")


def validate_runner_scripts() -> None:
    try:
        runner = RUN_NEXTEST.read_text(encoding="utf-8")
        central = RUN_CENTRAL.read_text(encoding="utf-8")
    except OSError as error:
        raise PreflightError(f"could not read repository runner: {error}") from error

    for token in (
        "run-central-tests.py",
        'cargo", "test", "--locked", "--workspace", "--doc',
        "--no-doctests",
    ):
        require(token in runner, f"ordinary nextest runner lost required phase token {token!r}")
    for token in ('["cargo", "nextest", "run"', '"--nextest"'):
        require(token in central, f"central nextest runner lost required phase token {token!r}")
    require(
        '"--handoff-output"' in central,
        "central runner lost the persistent handoff needed by stock Cargo compatibility",
    )
    require(
        'handoff_output.unlink(missing_ok=True)' in central,
        "central runner must remove a stale stock-Cargo handoff before building",
    )


def validate_preflight(preflight: str) -> None:
    required = (
        # Tool setup and exact versions.
        "Install operator-provided dagu",
        "version=2.14.0",
        "github.com/dagu-org/dagu/releases/download/v${version}/",
        'echo "$bin" >> "$GITHUB_PATH"',
        "$RUNNER_TEMP/dagu-bin",
        "mozilla-actions/sccache-action@v0.0.11",
        'version: "v0.17.0"',
        "name: Export GitHub Actions cache runtime variables",
        "uses: actions/github-script@v7",
        "core.exportVariable('ACTIONS_RESULTS_URL', process.env.ACTIONS_RESULTS_URL || '');",
        "core.exportVariable('ACTIONS_RUNTIME_TOKEN', process.env.ACTIONS_RUNTIME_TOKEN || '');",
        "name: Start credential-free sccache GHA backend",
        'SCCACHE_GHA_ENABLED: "true"',
        'SCCACHE_GHA_VERSION: "loop-engine-preflight-v1"',
        'SCCACHE_GHA_RW_MODE: "READ_WRITE"',
        'SCCACHE_IDLE_TIMEOUT: "0"',
        "printf 'SCCACHE_IDLE_TIMEOUT=%s\\n' \"$SCCACHE_IDLE_TIMEOUT\" >> \"$GITHUB_ENV\"",
        'RUSTC_WRAPPER: "sccache"',
        'test "$("$SCCACHE_PATH" --version)" = "sccache 0.17.0"',
        '"$SCCACHE_PATH" --start-server',
        '"$SCCACHE_PATH" --zero-stats',
        '"$SCCACHE_PATH" --show-stats --stats-format=json',
        "cargo install cargo-nextest --version 0.9.143 --locked",
        "cargo install cargo-machete --version 0.9.2 --locked",
        'test "$(cargo machete --version)" = "0.9.2"',
        # Tool, timeout, structure, inventory, and dependency assertions.
        "python3 scripts/assert-nextest.py --self-test",
        "python3 scripts/assert-nextest.py",
        # Require the real version/config gate, not only its self-test.
        "python3 scripts/assert-nextest.py\n",
        "python3 scripts/assert-test-topology.py --self-test",
        "python3 scripts/assert-test-inventory.py --self-test",
        "python3 scripts/sccache-proof.py --self-test",
        "python3 scripts/assert-push-main-preflight.py --self-test",
        "python3 scripts/dependency-audit.py",
        "python3 scripts/run-nextest.py",
        "python3 scripts/prove-nextest-timeout.py",
        'python3 scripts/prove-nextest-timeout.py \\\n            --report "$RUNNER_TEMP/nextest-timeout-proof.json"',
        "python3 scripts/run-central-tests.py",
        '--compiler-artifacts "$RUNNER_TEMP/central-test-artifacts.jsonl"',
        '--handoff-output "$RUNNER_TEMP/stock-cargo-handoff.json"',
        'python3 scripts/assert-test-topology.py \\\n            --repo "$GITHUB_WORKSPACE"',
        'python3 scripts/assert-test-inventory.py \\\n            --repo "$GITHUB_WORKSPACE"',
        "--current-only",
        'test -s "$RUNNER_TEMP/stock-cargo-handoff.json"',
        'LOOP_ENGINE_TEST_BINARY_HANDOFF="$RUNNER_TEMP/stock-cargo-handoff.json" \\\n            cargo test --workspace',
        # Existing required gates.
        "cargo clippy --workspace --all-targets -- -D warnings",
        "cargo fmt --all -- --check",
        "cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check",
        "run: scripts/bookends-check-gate.sh",
        "scripts/bookends-check-gate.sh",
        "BOOKENDS_BYPASS: \"\"",
        "dist generate --check",
        "python3 scripts/assert-dist-plan.py --self-test",
        'python3 scripts/assert-dist-plan.py "$RUNNER_TEMP/dist-plan.json"',
        "python3 scripts/assert-release-gates.py",
        "python3 scripts/assert-push-main-preflight.py",
        # The exact newline distinguishes the required gate from the tooling
        # self-test, which has a --self-test suffix.
        "python3 scripts/assert-push-main-preflight.py\n",
        "python3 scripts/assert-generate-prd-profile.py",
        "python3 scripts/software-change-journey.py --self-test",
        "python3 scripts/research-journey.py --self-test",
        "python3 scripts/generate-prd-journey.py --self-test",
        "python3 scripts/software-change-journey.py",
        "python3 scripts/policy-document-journey.py",
        "python3 scripts/research-journey.py",
        "python3 scripts/generate-prd-journey.py",
        # Exact no-argument discovery invocations must remain, not merely a
        # parameterized source journey with the same script name.
        "run: python3 scripts/software-change-journey.py\n",
        "run: python3 scripts/policy-document-journey.py\n",
        "run: python3 scripts/research-journey.py\n",
        "run: python3 scripts/generate-prd-journey.py\n",
        "--traversal-depth full",
        "--mode source",
        "--profile crates/policy-document-provider/data/readme.json",
        "for mode in draft audit",
        "--provider target/debug/research",
        "--profile crates/research-provider/data/configs/standard.json",
        "--profile crates/research-provider/data/configs/generate-prd.json",
        "--checker target/debug/bookends-check",
        "python3 scripts/software-change-journey.py \\\n            --mode source",
        "python3 scripts/policy-document-journey.py \\\n              --engine",
        "python3 scripts/research-journey.py \\\n            --mode source",
        "python3 scripts/generate-prd-journey.py \\\n            --mode source",
        # Final hosted proof.
        "PREFLIGHT_STARTED_AT",
        "printf 'PREFLIGHT_STARTED_AT=%s\\n' \"$(date +%s)\" >> \"$GITHUB_ENV\"",
        "preflight_total_wall_seconds=",
        '"$RUNNER_TEMP/sccache-final-stats.json"',
        '"$SCCACHE_PATH" --show-stats --stats-format=json | tee \\\n            "$RUNNER_TEMP/sccache-final-stats.json"',
    )
    missing = [token for token in required if token not in preflight]
    require(not missing, f"reusable preflight lost required gate(s): {missing}")

    require(
        not any(line.lstrip().startswith("if:") for line in preflight.splitlines()),
        "reusable preflight must not be independently skippable",
    )
    require("continue-on-error" not in preflight, "reusable preflight must not skip a missing tool or gate")
    require("cargo clean" not in preflight, "preflight must not use cargo clean")
    require(
        "sed -n '1s/^cargo-nextest //p'" not in preflight,
        "cargo-nextest version check must not compare metadata-bearing output as a bare version",
    )
    required_files = (
        ".config/nextest.toml",
        "scripts/assert-nextest.py",
        "scripts/assert-test-topology.py",
        "scripts/assert-test-inventory.py",
        "scripts/sccache-proof.py",
        "scripts/dependency-audit.py",
        "scripts/prove-nextest-timeout.py",
        "scripts/assert-dist-plan.py",
        "scripts/assert-release-gates.py",
        "scripts/assert-generate-prd-profile.py",
        "scripts/bookends-check-gate.sh",
        "scripts/software-change-journey.py",
        "scripts/policy-document-journey.py",
        "scripts/research-journey.py",
        "scripts/generate-prd-journey.py",
    )
    for relative in required_files:
        require((ROOT / relative).is_file(), f"repository must retain required preflight file {relative}")

    # The cache must be running before *any* Cargo compilation, including the
    # source-built tool installs.  It must also remain observable at the end.
    dagu_at = position(preflight, "Install operator-provided dagu")
    runtime_export = position(preflight, "name: Export GitHub Actions cache runtime variables")
    cache_start = position(preflight, '"$SCCACHE_PATH" --start-server')
    nextest_install = position(preflight, "Install pinned cargo-nextest")
    nextest_install_command = position(preflight, "cargo install cargo-nextest --version 0.9.143 --locked")
    nextest_version_check = preflight.find("python3 scripts/assert-nextest.py", nextest_install_command)
    machete_install = position(preflight, "Install pinned cargo-machete")
    tooling_validation = position(preflight, "Validate repository test tooling")
    require(runtime_export < cache_start, "GHA cache runtime variables must be exported before sccache startup")
    require(nextest_version_check >= 0, "pinned cargo-nextest install must run the canonical version check")
    require(
        cache_start
        < nextest_install
        < nextest_install_command
        < nextest_version_check
        < machete_install
        < tooling_validation,
        "sccache startup, pinned nextest install/check, and machete install must precede tooling validation",
    )
    # There must be no unrecognized Cargo invocation before the cache is
    # running.  The literal space avoids matching the cargo-dist installer URL.
    first_cargo = position(preflight, "cargo ")
    require(dagu_at < first_cargo, "preflight must install dagu before its first Cargo invocation")
    require(cache_start < first_cargo, "preflight must start sccache before its first Cargo invocation")
    final_stats = preflight.rfind('"$SCCACHE_PATH" --show-stats')
    require(final_stats > first_cargo, "preflight must emit final sccache statistics after compilation")

    # Dependency hygiene precedes any ordinary workspace compile.  The
    # ordinary runner and timeout probe precede the direct stock compatibility
    # gate; structural assertions are run against a fresh central build.
    audit = position(preflight, "python3 scripts/dependency-audit.py")
    ordinary = position(preflight, "python3 scripts/run-nextest.py")
    timeout = position(preflight, "python3 scripts/prove-nextest-timeout.py")
    central_build = position(preflight, "--no-run")
    central_topology = position(preflight, 'python3 scripts/assert-test-topology.py \\\n            --repo "$GITHUB_WORKSPACE"')
    central_inventory = position(preflight, 'python3 scripts/assert-test-inventory.py \\\n            --repo "$GITHUB_WORKSPACE"')
    stock = position(preflight, 'LOOP_ENGINE_TEST_BINARY_HANDOFF="$RUNNER_TEMP/stock-cargo-handoff.json"')
    clippy = position(preflight, "cargo clippy --workspace")
    fmt = position(preflight, "cargo fmt --all -- --check")
    build = position(preflight, "cargo build --locked -p")
    generated_workflow = position(preflight, "Check generated release workflow")
    release_plan = position(preflight, "Assert release plan matrix")
    bookends = position(preflight, "Run Bookends check")
    require(
        audit < generated_workflow < release_plan < bookends < ordinary,
        "tooling, audit, generated-release assertions, and Bookends must precede ordinary nextest",
    )
    require(
        ordinary < timeout < central_build < central_topology < central_inventory < stock,
        "nextest, timeout, and central structural checks must precede stock Cargo compatibility",
    )
    require(
        stock < clippy < fmt < build,
        "stock Cargo compatibility must precede the final lint/build gates",
    )

    # The public journey commands are separate YAML steps and therefore run
    # serially.  Keep their relative order explicit so a future refactor does
    # not accidentally launch competing Cargo consumers.
    journey_order = (
        "Collect software-change public journey surface for Bookends",
        "Collect policy-document public journey surface for Bookends",
        "Collect research public journey surface for Bookends",
        "Collect Generate-PRD public journey surface for Bookends",
        "Assert Generate-PRD profile",
        "Run journey interface negative self-test",
        "Run research journey interface negative self-test",
        "Run Generate-PRD journey self-test",
        "Run full software-change source journey",
        "Run policy-document source journeys",
        "Run research source journey",
        "Run Generate-PRD source journey",
        "Emit hosted sccache statistics and total preflight wall time",
    )
    positions = [position(preflight, name) for name in journey_order]
    require(positions == sorted(positions), "public journeys and final hosted statistics must remain serialized")
    require(
        build < positions[0],
        "the locked journey-binary build must precede every public journey step",
    )


def validate(dispatcher: str, preflight: str) -> None:
    validate_dispatcher(dispatcher)
    validate_preflight(preflight)
    validate_runner_scripts()


def self_test(dispatcher: str, preflight: str) -> int:
    # Validate the real files first: this command is also a fail-closed CI
    # assertion, not merely a unit test of a parser.
    validate(dispatcher, preflight)

    checks = (
        # Tool setup and version checks.
        ("sccache action", "mozilla-actions/sccache-action@v0.0.11", "pinned sccache setup"),
        ("sccache version", 'test "$("$SCCACHE_PATH" --version)" = "sccache 0.17.0"', "sccache version check"),
        ("cache runtime export", "core.exportVariable('ACTIONS_RUNTIME_TOKEN', process.env.ACTIONS_RUNTIME_TOKEN || '');", "GHA cache runtime export"),
        ("cache startup", '"$SCCACHE_PATH" --start-server', "sccache startup"),
        ("nextest install", "cargo install cargo-nextest --version 0.9.143 --locked", "pinned cargo-nextest install"),
        ("nextest version", "python3 scripts/assert-nextest.py", "canonical cargo-nextest version check"),
        ("machete install", "cargo install cargo-machete --version 0.9.2 --locked", "pinned cargo-machete install"),
        ("machete version", 'test "$(cargo machete --version)" = "0.9.2"', "cargo-machete version check"),
        # Tooling and structural gates.
        ("nextest self-test", "python3 scripts/assert-nextest.py --self-test", "nextest policy self-test"),
        ("nextest config gate", "python3 scripts/assert-nextest.py\n", "nextest configuration gate"),
        ("topology self-test", "python3 scripts/assert-test-topology.py --self-test", "topology checker self-test"),
        ("inventory self-test", "python3 scripts/assert-test-inventory.py --self-test", "inventory checker self-test"),
        ("sccache self-test", "python3 scripts/sccache-proof.py --self-test", "sccache proof self-test"),
        ("push assertion self-test", "python3 scripts/assert-push-main-preflight.py --self-test", "preflight assertion self-test"),
        ("dependency audit", "python3 scripts/dependency-audit.py", "dependency audit"),
        ("timeout proof", 'python3 scripts/prove-nextest-timeout.py \\\n            --report "$RUNNER_TEMP/nextest-timeout-proof.json"', "bounded timeout proof"),
        ("central build", "python3 scripts/run-central-tests.py \\\n            --no-run", "fresh central integration build"),
        ("topology gate", 'python3 scripts/assert-test-topology.py \\\n            --repo "$GITHUB_WORKSPACE"', "one-target topology gate"),
        ("inventory gate", 'python3 scripts/assert-test-inventory.py \\\n            --repo "$GITHUB_WORKSPACE"', "current source inventory gate"),
        ("stock compatibility", 'LOOP_ENGINE_TEST_BINARY_HANDOFF="$RUNNER_TEMP/stock-cargo-handoff.json" \\\n            cargo test --workspace', "stock Cargo gate"),
        # Existing release, Bookends, lint, build, and journey gates.
        ("generated workflow", "dist generate --check", "generated release workflow check"),
        ("dist plan assertion", 'python3 scripts/assert-dist-plan.py "$RUNNER_TEMP/dist-plan.json"', "release plan assertion"),
        ("release gates", "python3 scripts/assert-release-gates.py", "release gate assertion"),
        ("push assertion gate", "python3 scripts/assert-push-main-preflight.py\n", "required preflight assertion gate"),
        ("Bookends gate", "run: scripts/bookends-check-gate.sh", "Bookends gate"),
        ("ordinary nextest", "python3 scripts/run-nextest.py", "ordinary nextest runner"),
        ("clippy", "cargo clippy --workspace --all-targets -- -D warnings", "workspace clippy gate"),
        ("format", "cargo fmt --all -- --check", "workspace format gate"),
        ("locked journey build", "cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check", "locked journey binary build"),
        ("Generate-PRD profile", "python3 scripts/assert-generate-prd-profile.py", "Generate-PRD profile gate"),
        ("software-change discovery", "run: python3 scripts/software-change-journey.py\n", "software-change Bookends discovery surface"),
        ("policy-document discovery", "run: python3 scripts/policy-document-journey.py\n", "policy-document Bookends discovery surface"),
        ("research discovery", "run: python3 scripts/research-journey.py\n", "research Bookends discovery surface"),
        ("Generate-PRD discovery", "run: python3 scripts/generate-prd-journey.py\n", "Generate-PRD Bookends discovery surface"),
        ("software-change source journey", "python3 scripts/software-change-journey.py \\\n            --mode source", "software-change source journey"),
        ("policy-document source journey", "python3 scripts/policy-document-journey.py \\\n              --engine", "policy-document source journey"),
        ("research source journey", "python3 scripts/research-journey.py \\\n            --mode source", "research source journey"),
        ("Generate-PRD source journey", "python3 scripts/generate-prd-journey.py \\\n            --mode source", "Generate-PRD source journey"),
        ("preflight start record", "printf 'PREFLIGHT_STARTED_AT=%s\\n' \"$(date +%s)\" >> \"$GITHUB_ENV\"", "preflight start timestamp"),
        ("final wall time", "preflight_total_wall_seconds=", "hosted total wall time"),
        ("final cache JSON", '"$SCCACHE_PATH" --show-stats --stats-format=json | tee \\\n            "$RUNNER_TEMP/sccache-final-stats.json"', "hosted final sccache statistics"),
    )
    for label, token, description in checks:
        broken = preflight.replace(token, f"REMOVED_{label.replace(' ', '_')}", 1)
        try:
            validate(dispatcher, broken)
        except PreflightError:
            continue
        raise PreflightError(f"self-test accepted removal of {description}")

    # The structural order itself is part of the contract.  Swap the textual
    # locations of the ordinary and stock commands in a fixture and ensure the
    # order assertion rejects it.
    ordinary_token = "python3 scripts/run-nextest.py"
    stock_token = 'LOOP_ENGINE_TEST_BINARY_HANDOFF="$RUNNER_TEMP/stock-cargo-handoff.json"'
    ordinary_at = preflight.find(ordinary_token)
    stock_at = preflight.find(stock_token)
    require(ordinary_at >= 0 and stock_at >= 0, "self-test fixture tokens are missing")
    swapped = (
        preflight[:ordinary_at]
        + stock_token
        + preflight[ordinary_at + len(ordinary_token) : stock_at]
        + ordinary_token
        + preflight[stock_at + len(stock_token) :]
    )
    try:
        validate(dispatcher, swapped)
    except PreflightError:
        pass
    else:
        raise PreflightError("self-test accepted ordinary-runner/stock-gate reordering")

    # Also reject an otherwise unrecognized Cargo command inserted before the
    # cache startup.  This guards the ordering check against future new steps,
    # not only the currently enumerated compile phases.
    cache_at = preflight.find('"$SCCACHE_PATH" --start-server')
    require(cache_at >= 0, "self-test cache token is missing")
    before_cache = preflight[:cache_at] + "\ncargo test --workspace\n" + preflight[cache_at:]
    try:
        validate(dispatcher, before_cache)
    except PreflightError:
        pass
    else:
        raise PreflightError("self-test accepted a Cargo invocation before cache startup")

    print(
        "push-main preflight assertion self-test passed: real workflow valid; "
        "tool, ordering, inventory, stock, cache, and audit removals rejected"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    try:
        dispatcher = DISPATCHER.read_text(encoding="utf-8")
        preflight = PREFLIGHT.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"could not read workflow: {error}")

    try:
        if arguments == ["--self-test"]:
            return self_test(dispatcher, preflight)
        if arguments:
            fail(f"unsupported arguments: {' '.join(arguments)}")
        validate(dispatcher, preflight)
    except PreflightError as error:
        fail(str(error))
    print("push-main preflight ok: pushed SHA plan -> unchanged read-only reusable preflight")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
