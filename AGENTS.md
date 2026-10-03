# Agent instructions

## Scope

This file governs the entire loop-engine checkout: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, `bookends-check`, the software-change/policy-document/research providers, `tests/workspace-integration`, `tests/fixtures`, `tests/bounded_process.rs`, scripts, skills and release workflows. `crates/bookends-check` has no nested guide and follows this file.

Primary workflow work belongs outside the engine/provider. Do not invent engine policy, review orchestration or core semantics. Humans start at [README.md](README.md). Nested `AGENTS.md` files add crate-local procedure and must not contradict this file.

## Authority

Each source governs its named scope:

1. Root `AGENTS.md`: checkout operations.
2. Nested `AGENTS.md`: additional crate-local procedure.
3. [docs/PRD.md](docs/PRD.md): living engine product requirements.
4. Frozen provider requirements and protocols: provider requirements and protocol contracts.
5. Provider README: that provider's public contract.
6. Provider skill: driving procedure.
7. [docs/agent-usage.md](docs/agent-usage.md): generic CLI forms and loop operations.

README.md is the human overview. Requirement changes need exact owner acceptance and an authorized commit. Unrelated recovery drafts remain drafts. Source integration alone establishes no requirement authority or semantic approval.

When proposing provenance obligations, load [docs/PRD.md](docs/PRD.md) LE-107; this guide is referential, not a second product policy. Owner-accepted wording remains a proposal until committed in that PRD.

## External work

Before driving the CLI, load [the engine skill](skills/using-loop-engine/SKILL.md) and [agent usage](docs/agent-usage.md). Before commissioning provider work, load its crate-local guide to select supported commands and its skill. Each run has one logical mutating driver: serialize all mutations through that driver, rather than starting independent concurrent mutators. Test catalogs and captures belong in driver-owned directories outside the checkout: explicitly set database and provider config, and unset unintended LOOP/XDG redirects rather than using user state.

Before rebuilding or replacing a provider executable, inspect active runs in each driver-owned catalog with explicit database/config and full show. Runs freeze command/args paths, not executable bytes. If an active run points to a build output that would be overwritten, stop the build: retain that executable and its required runtime dependencies at the exact frozen path, and build new binaries into a separate `CARGO_TARGET_DIR`. Do not move the old binary and assume the run follows it; there is no migration. Resume only with the compatible runtime preserved or after an authorized termination of the affected run.

### Load before acting

| Work | Owning procedure |
|---|---|
| Start or resume a run | [Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup) and the active provider skill |
| Confirm a software-change launch | [Exact profile confirmation](crates/software-change-provider/skills/using-software-change-provider/SKILL.md#exact-profile-confirmation) |
| Observe before mutation | [Choose an observation](skills/using-loop-engine/SKILL.md#choose-an-observation) |
| Deliver code | [Software-change skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md) |
| Draft or audit README/AGENTS | [Policy-document skill](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md) |
| Research a question | [Research skill](crates/research-provider/skills/using-research-provider/SKILL.md) |
| Coordinate independent drivers | [Coordinator skill](skills/coordinating-loop-engine/SKILL.md) |
| Extract a PRD candidate | [Generate-PRD skill](crates/research-provider/skills/using-generate-prd/SKILL.md) |

## Workflow

Run from the repository root. Load the engine and active provider skills. Use policy-document for README/AGENTS authoring or assessment. Requirement changes require owner acceptance. Choose proof by source identity: for uncommitted source or without an exact authorized main-push preflight, the designated final-proof owner uses the local route below. For an authorized committed main push, require successful hosted [production preflight](.github/workflows/preflight.yml) for that exact commit; its collected journeys and Bookends gate satisfy the matrix without redundant local reruns. CI startup is not a local shell script. Local proof requires Rust/Cargo and Python 3.14.6 provisioned through an approved environment, plus an unused driver-owned sccache server port. Install pinned tools only into the new proof-owned root below; obtain approval before changing persistent user tooling. Keep this shell's variables for subsequent proof commands:

```sh
set -eu
PROOF_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/loop-proof.XXXXXX")"
export PATH="$PROOF_ROOT/tools/bin:$PATH"
BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
cargo install cargo-dist --version 0.32.0 --locked --root "$PROOF_ROOT/tools"
cargo install cargo-nextest --version 0.9.143 --locked --root "$PROOF_ROOT/tools"
cargo install cargo-machete --version 0.9.2 --locked --root "$PROOF_ROOT/tools"
cargo install sccache --version 0.17.0 --locked --root "$PROOF_ROOT/tools"
: "${SCCACHE_SERVER_PORT:?set an unused port owned by this proof driver}"
export SCCACHE_SERVER_PORT SCCACHE_DIR="$PROOF_ROOT/cache" SCCACHE_IDLE_TIMEOUT=0
unset SCCACHE_GHA_ENABLED
export RUSTC_WRAPPER="$PROOF_ROOT/tools/bin/sccache"
sccache --start-server
sccache --zero-stats
python3 scripts/assert-nextest.py
python3 scripts/dependency-audit.py
dist generate --check
dist plan --output-format=json > "$PROOF_ROOT/dist-plan.json"
python3 scripts/assert-dist-plan.py "$PROOF_ROOT/dist-plan.json"
python3 scripts/assert-release-gates.py
python3 scripts/assert-push-main-preflight.py
python3 scripts/run-nextest.py
python3 scripts/prove-nextest-timeout.py --report "$PROOF_ROOT/nextest-timeout.json"
python3 scripts/run-central-tests.py --no-run --compiler-artifacts "$PROOF_ROOT/central-artifacts.jsonl" --handoff-output "$PROOF_ROOT/handoff.json"
python3 scripts/assert-test-topology.py --compiler-artifacts "$PROOF_ROOT/central-artifacts.jsonl"
python3 scripts/assert-test-inventory.py --compiler-artifacts "$PROOF_ROOT/central-artifacts.jsonl" --current-only
LOOP_ENGINE_TEST_BINARY_HANDOFF="$PROOF_ROOT/handoff.json" cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
sccache --show-stats --stats-format=json > "$PROOF_ROOT/sccache-final.json"
```

Do not stop another driver's cache server or clean unrelated state. Retain the proof root and its receipts; a missing prerequisite blocks proof. For local proof, run the source journeys below and the Bookends gate in addition to this matrix. Cargo may select a non-default target directory via environment/config: use the metadata-resolved BIN_DIR above, never stale hard-coded target/debug paths. Workers run focused checks only. Missing proof blocks completion.

```sh
cargo build --workspace --all-targets
RESEARCH_PROVIDER="$BIN_DIR/research" python3 scripts/assert-generate-prd-profile.py
python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --software "$BIN_DIR"/software-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"
python3 scripts/software-change-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/software-change --profile crates/software-change-provider/data/configs/high-rigor.json --traversal-depth full --jobs 2
for mode in draft audit; do
  python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode "$mode"
done
python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json
python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json
```

Bare journey invocations and `--self-test` prove collection/interface mechanics only; parameterized commands above exercise end-to-end progression. Before research source journeys, use preflight's “Install dist” setup (cargo-dist 0.32.0); do not report a journey complete if `dist` is unavailable. Obtain approval for persistent tool installation, or use a disposable driver-owned tool root without changing unrelated user state.

For tests, use `python3 scripts/run-central-tests.py --filter NAME` for focused central integration tests and `python3 scripts/run-nextest.py` for ordinary nextest plus doctests; both own fresh production/fixture binary handoffs. Direct test invocation is not a substitute for this verified handoff. For final topology/inventory and stock-Cargo compatibility, run preflight's “Build central target for structure and inventory assertions”, “Assert one integration target and current test inventory”, and “Run stock Cargo compatibility gate” steps in order with the same retained handoff and compiler-artifact files.

For packaging boundaries, choose `scripts/packaged-smoke.py --mode archive` when validating release archives, supplying `--archive` and `--checksum`; choose `--mode installed` only for installed executables, supplying `--binary`. Both require actual package identity, expected version, platform and an external output root; use `--help` for their required per-application inputs. Never describe installed identity as archive identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs before using the central runner. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands.

Before push, build the current checker and require GREEN from the repository gate; BYPASS is not proof, so clear inherited BOOKENDS_BYPASS:

```sh
CHECKER_BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
cargo build --locked -p bookends-check --bin bookends-check
unset BOOKENDS_BYPASS
PATH="$CHECKER_BIN_DIR:$PATH" scripts/bookends-check-gate.sh </dev/null | tee "$PROOF_ROOT/bookends.log"
grep -qx GREEN "$PROOF_ROOT/bookends.log"
```

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-4`/`agents-4` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Load the policy-document skill for profile preparation and evidence rules. Local references must remain within the target directory; normalized paths such as `docs/../exists.md` are allowed, but escapes outside the directory are rejected. For root guidance in crate documents, use an explicit checkout-root-relative prose path instead of an escaping markdown link.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures. These markdown boundaries are advisory, not an enforcement mechanism; inspect staged contents and retain sensitive/runtime artifacts outside the checkout.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and regenerate. Direct pushes to main run read-only preflight. Publication uses the separately authorized sequence below. Do not force-push main.

### Authorized release sequence

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). [prepare-release.sh](scripts/prepare-release.sh) requires a clean tree and owns version/lockfile preparation, locked builds and release assertions:

```sh
VERSION="X.Y.Z"
TAG="v$VERSION"
test ! -e /tmp/loop-engine-dist-plan.json || { echo "Release plan destination already exists; obtain ownership/cleanup approval before proceeding" >&2; exit 1; }
scripts/prepare-release.sh "$VERSION"
```

Review the result and obtain separate authorization for the Git commit, tag and push. After those actions, locate preflight for the exact committed HEAD:

```sh
set -eu
COMMIT="$(git rev-parse HEAD)"
PREFLIGHT_RUN="$(gh run list --workflow push-to-main.yml --branch main --event push --commit "$COMMIT" --limit 1 --json databaseId,status,conclusion --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId')"
test -n "$PREFLIGHT_RUN" || { echo "Exact-commit preflight is missing, pending or failed" >&2; exit 1; }
gh run view "$PREFLIGHT_RUN" --json headSha,status,conclusion,url
PREFLIGHT_LOG="$(mktemp)"
FINAL_CACHE_LOG="$(mktemp)"
gh run view "$PREFLIGHT_RUN" --log >"$PREFLIGHT_LOG"
grep -F 'Start credential-free sccache GHA backend' "$PREFLIGHT_LOG" >/dev/null
grep -F 'Emit hosted sccache statistics and total preflight wall time' "$PREFLIGHT_LOG" >"$FINAL_CACHE_LOG"
grep -E 'preflight_total_wall_seconds=[0-9]+$' "$FINAL_CACHE_LOG"
grep -F '"cache_hits"' "$FINAL_CACHE_LOG" >/dev/null
printf 'Retain release evidence: %s %s\n' "$PREFLIGHT_LOG" "$FINAL_CACHE_LOG"
```

The commit filter selects the approved HEAD; require the displayed `headSha` to match it. These commands fail on missing successful preflight or required current log markers. Inspect and retain the logs as release evidence. Only after this check and publication authorization:

```sh
: "${TAG:?set TAG from the owner-approved VERSION}"
gh workflow run release.yml --ref main -f "tag=$TAG"
```

## Completion and handoff

Report changed files, exact command outcomes and missing checks, Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. No checked transition establishes semantic review.
