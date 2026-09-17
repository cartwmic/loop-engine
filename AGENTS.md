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

### Required referential reminders

The journey self-test checks the following LE-107 and execution-ownership reminders. Their linked PRD and skills govern policy and procedure; these reminders do not replace those sources.

Operationally, before the commit introducing `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, that PRD is authoritative. This AGENTS summary is subordinate and referential, not a second product policy: for any new provenance burden, name the observed ordinary-use failure and why a smaller mechanism using existing durable state, history, capture, or driver judgment is insufficient. Keep driver-authored metadata small, trust explicit materiality and applicability declarations except for cheap mechanical identity mismatches, prefer the narrowest honest correction, and preserve rich engine-generated history. Apply these limits with reference to [`docs/PRD.md`](docs/PRD.md) LE-107.

Fan-out spawn/capture/conformance mechanics belong to the engine. Providers/callers own role framing and output content. Reviewers produce judgments only. Drivers run deterministic checks, `show`, capture triage, `append`, `event`, and progression. Exit 0 alone does not establish deliverable validity. Before fan-out or plan-graph execution, load the engine and software-change skills below plus [docs/agent-usage.md](docs/agent-usage.md) for working-directory, selection, stdin, capture, Dagu, overrun re-show and zero-axis review-binding rules.

### Load before acting

When using Loop Engine, select software-change for code delivery and research for research; generate-PRD is a research profile. README/AGENTS authoring or assessment requires policy-document, including those document portions of a software-change run. Its checks are additional to the software-change workflow. These choices do not require a new software-change run for every repository edit. Revision or audit of an existing PRD is owner/driver requirements work; research can investigate unresolved claims, while generate-PRD extracts candidates. Policy-document does not accept requirements. Code-related PRD amendments belong to that software-change run's intent/requirements work, with Bookends and code proof remaining additive.

| Work | Owning procedure |
|---|---|
| Start, observe, invoke, record, progress, recover or reuse evidence | [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md) plus the active provider skill: [software-change](crates/software-change-provider/skills/using-software-change-provider/SKILL.md), [policy-document](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md), or [research](crates/research-provider/skills/using-research-provider/SKILL.md) |
| Confirm a software-change launch | [Exact profile confirmation](crates/software-change-provider/skills/using-software-change-provider/SKILL.md#exact-profile-confirmation): live review states, when to rehash that same file immediately before start, and how to preserve launch evidence |
| Route a software-change finding or choose AC-N/Bookends obligations | [Software-change skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md), including its proportional late-finding guide |
| Capture, monitor, summarize or publish a delivery pointer | [Operational UX contracts](docs/operational-ux-contracts.md) |
| Coordinate separate drivers | [Coordinator guidance](skills/coordinating-loop-engine/SKILL.md) |
| Extract a PRD candidate | [Generate-PRD skill](crates/research-provider/skills/using-generate-prd/SKILL.md) |
| Change provider initial-input parsing | [Allocation contract](docs/agent-usage.md#deterministic-setup); preserve reserved `artifact_root` acceptance |

Existing runs keep their original runtime, guidance and frozen obligations. No active-run migration is authorized. A graph runner grants no worktree lifecycle authority.

## Workflow

Run commands from the repository root. During iteration, use focused checks:

```sh
cargo test -p loop-cli --lib
cargo test -p software-change-provider --lib
python3 scripts/run-nextest.py --filter TEST_SUBSTRING
python3 scripts/run-central-tests.py --filter TEST_SUBSTRING
```

For final proof, first load the [workspace test/cache procedure](docs/agent-usage.md#workspace-rust-test-and-preflight-path) and [.github/workflows/preflight.yml](.github/workflows/preflight.yml). Run nextest/doctests and the required stock-Cargo compatibility gate through that procedure's ordered fresh-handoff commands. A bare `cargo test --workspace` is not the final compatibility gate.

The designated final-proof owner supplies and executes local tool/cache startup and verification through the current matrix, before workspace compilation. Hosted startup belongs to preflight. The CI workflow is not a local startup script. Missing local setup coordinates block proof. These additional baseline checks remain required:

```sh
cargo install cargo-machete --version 0.9.2 --locked
python3 scripts/dependency-audit.py
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Maintenance traps

- Before operating a run, use [deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup) to inspect the operation's `LOOP_*`, XDG and home overrides, record the resolved catalog, pass explicit `--config`, and unset only unintended redirects.
- Before mutation and after every transition, read action/full `show`. Status/compact, monitor, list, history and invocation-progress do not arm mutation; follow [the observation interlock](skills/using-loop-engine/SKILL.md#choose-an-observation).
- The global `--timeout-ms` default is 30000 for provider calls and invocation allowance. Choose an adequate explicit value before slow calls or bound work; use the engine skill's recovery procedure on overrun.
- Integration roots are modules in the one central executable. Use `--test workspace`. Old source-module names are not Cargo test targets. Register new roots with `#[path] mod` in `tests/workspace-integration/tests/workspace.rs`. With `autotests = false`, an unregistered root can be invisible to both Cargo and inventory checks. Require topology and inventory assertions from the linked test procedure.
- When adding or renaming public tests, check eligibility in [bookends.toml](bookends.toml) and the [runner grammar](crates/bookends-check/schema/runner-grammar.md). Update pathspecs when the new location is not covered, retain a recognized CI collection command, and run the repository gate. A citation alone cannot add an ineligible file to coverage.
- When adding or renaming binaries, update `scripts/run-central-tests.py` (`REQUIRED`), `scripts/test_contract.py` (`REQUIRED_BINARIES`), and `tests/workspace-integration/src/lib.rs` (`REQUIRED_BINARIES`, `FRESH_BUILD_ARGS`). `tests/fixtures` executables are protocol fixtures; do not substitute them for production providers. Use the supported fresh handoff and independent stock-Cargo compatibility gate from the test procedure.
- Provider data shipping uses hand-maintained `FILES` inventories: [software-change](crates/software-change-provider/src/embedded_data.rs), [policy-document](crates/policy-document-provider/src/embedded_data.rs), and [research](crates/research-provider/src/embedded_data.rs). Update the matching inventory for every data-file addition/rename; tests of listed entries can miss an omitted new file. For policy-document, run `cargo test -p policy-document-provider --bin policy-document manifest_exactly_matches_focused_data` and both applicable journey modes.
- One designated proof owner runs the current plan-owned final stable-tree matrix once. Workers run assigned focused checks; reviewers consume retained results. Repeat only checks invalidated by later changes. Missing matrix, benchmark argv/inputs or receipts block final proof; use the [receipt procedure](docs/implementation-report-proof.md). Do not invent a benchmark CLI or substitute an earlier pilot result.

### Required public boundaries

These checks are additive to the workspace baseline. The table selects coverage; the local templates below supply the argv. Run the applicable commands after required tool/cache setup, building once. The [preflight workflow](.github/workflows/preflight.yml) and [proof-boundary reference](docs/agent-usage.md#executable-proof-boundaries) describe the corresponding CI coverage.

| Changed boundary | Required source proof |
|---|---|
| Engine, CLI, core, integrations, shared scripts, work slots, or unclear boundary | All four journeys below |
| Software-change | `software-change-journey.py`: source, full traversal; default `--jobs 2`. The driver may declare `--jobs 1` for serial proof before execution. |
| Policy-document | `policy-document-journey.py`: both draft and audit |
| Research | `research-journey.py` and `generate-prd-journey.py`: source |
| Bookends checker or repository gate | `cargo test -p bookends-check --offline` and the enabled repository gate |

```sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check
ENGINE="$PWD/target/debug/loop-engine"
python3 scripts/software-change-journey.py --mode source --engine "$ENGINE" --provider "$PWD/target/debug/software-change" --data-root "$PWD" --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" --profile crates/software-change-provider/data/configs/high-rigor.json --traversal-depth full --jobs 2
for mode in draft audit; do
  python3 scripts/policy-document-journey.py --engine "$ENGINE" --provider "$PWD/target/debug/policy-document" --profile crates/policy-document-provider/data/readme.json --mode "$mode"
done
python3 scripts/research-journey.py --mode source --engine "$ENGINE" --provider "$PWD/target/debug/research" --profile crates/research-provider/data/configs/standard.json
python3 scripts/generate-prd-journey.py --mode source --engine "$ENGINE" --provider "$PWD/target/debug/research" --checker "$PWD/target/debug/bookends-check" --profile crates/research-provider/data/configs/generate-prd.json
```

For journey-harness changes, also run its `--self-test` when available and every affected journey variant it runs. For package/archive adapters or release-boundary changes, add packaged validation; source proof does not replace it. Use installed mode for installer/installed-path changes, archive mode for archive layout/extraction, and both for shared wrapper logic, embedded/package identity, or release metadata/workflow changes. [archive-smoke.yml](.github/workflows/archive-smoke.yml) owns archive preparation, identity inputs and the complete `scripts/packaged-smoke.py` invocation. That script has no self-test; exercise its packaged path. For release metadata/workflow changes, also run the dist plan/generation and release-gate assertions listed in preflight.

Local packaged templates follow. Replace `X.Y.Z`, `NATIVE_TARGET`, paths and checksum placeholders with actual package evidence. Use a distinct external output root per invocation. [Shared packaged smoke](docs/operational-ux-contracts.md#shared-packaged-smoke) defines the identity JSON; archive hashes and installed-executable hashes are different inputs.

```sh
python3 scripts/packaged-smoke.py --mode installed --expected-version X.Y.Z --platform NATIVE_TARGET --output-root /absolute/installed-proof --package-identity /absolute/installed-identity.json \
  --binary "loop-cli=/absolute/bin/loop-engine" --binary "software-change-provider=/absolute/bin/software-change" \
  --binary "policy-document-provider=/absolute/bin/policy-document" --binary "research-provider=/absolute/bin/research"
python3 scripts/packaged-smoke.py --mode archive --expected-version X.Y.Z --platform NATIVE_TARGET --output-root /absolute/archive-proof --package-identity /absolute/archive-identity.json \
  --archive "loop-cli=/absolute/loop-cli.tar.xz" --checksum "loop-cli=ENGINE_ARCHIVE_SHA256" \
  --archive "software-change-provider=/absolute/software-change-provider.tar.xz" --checksum "software-change-provider=SOFTWARE_ARCHIVE_SHA256" \
  --archive "policy-document-provider=/absolute/policy-document-provider.tar.xz" --checksum "policy-document-provider=POLICY_ARCHIVE_SHA256" \
  --archive "research-provider=/absolute/research-provider.tar.xz" --checksum "research-provider=RESEARCH_ARCHIVE_SHA256"
```

For skill/constructor or root policy changes, `python3 scripts/software-change-journey.py --self-test` must reach `worker-data skill/root policy assertions passed` after testing setup/constructors and root rules. Full source must reach `contracted fan-out failure` after proving the actual contracted-worker failure path. Synthetic evidence establishes mechanics and persistence; it supplies no semantic review.

Before every push, run `scripts/bookends-check-gate.sh` from the root; do not assume the checkout's `.githooks/pre-push` has been activated. Required CI also runs the gate. Only explicit `BOOKENDS_BYPASS=<class>:<reason>` may bypass RED, with durable evidence; recording failure refuses bypass. Follow the [Bookends command contract](crates/bookends-check/schema/runner-grammar.md) and [operational UX contract](docs/operational-ux-contracts.md) for publication history, shallow acquisition and proof eligibility. README.md and AGENTS.md are outside Bookends coverage.

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-2`/`agents-2` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Preserve `target.id` and `profile_version` unless intentionally authoring a custom profile. Local links resolve under the target's directory; `..` is rejected, so crate docs cannot markdown-link outside the crate.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and regenerate. Direct pushes to main run read-only preflight. Publication uses the separately authorized sequence below. Do not force-push main.

### Authorized release sequence

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). [prepare-release.sh](scripts/prepare-release.sh) requires a clean tree and owns version/lockfile preparation, locked builds and release assertions:

```sh
VERSION="X.Y.Z"
TAG="v$VERSION"
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

Completion requires the accepted behavior, current authoritative documentation, passing workspace baseline and all applicable public-boundary proof. Missing or failed required checks block completion. Record an explicit owner exception with its scope and risk; never report it as a pass.

For software-change report finalization, load [docs/implementation-report-proof.md](docs/implementation-report-proof.md). It owns the exact current Git/path identity, matrix/receipt and post-report checker contract. Keep the graph summarizer as sole report writer. Keep separately authorized Git/hosted work pending until observed. Hosted delivery requires successful preflight for the exact owner-authorized commit, including cache startup, final statistics and wall time; require that proof before Package 7b.

Handoff includes changed files and why; command outcomes including failures/skips; current Git revision and staged/committed/pushed/uncommitted state; run IDs and actual database path; remaining risks and out-of-scope follow-up. A checked transition alone establishes no semantic review. Use the engine skill for durable resumption and terminal-state limits.
