# Agent instructions

## Scope

This file instructs agents working in this loop-engine checkout. In scope: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, the `software-change`, `policy-document`, and `research` reference providers, `tests/fixtures`, `scripts/`, skills, and release-proof workflows.

Out of scope: performing primary workflow work *inside* the engine or a provider; inventing engine policy, review orchestration, or core semantics; and treating this file as a human product overview. Humans start at [README.md](README.md).

Nested `AGENTS.md` files govern crate-local procedure only and must never contradict this root file.

## Authority

Use this scoped order when sources conflict; each source is authoritative only for the scope named here:

1. Root `AGENTS.md` governs checkout operations.
2. A nested `AGENTS.md` governs crate-local procedure and must never contradict root `AGENTS.md`.
3. [docs/PRD.md](docs/PRD.md) governs living engine product requirements.
4. Frozen provider requirements and protocols govern provider-specific requirements and protocol contracts.
5. The relevant provider README governs that provider's public contract.
6. The relevant provider skill governs the procedure for driving that provider.
7. [docs/agent-usage.md](docs/agent-usage.md) governs generic CLI forms and loop operations.

The root [README.md](README.md) is the human product overview, not operational authority.

When a repository enables Bookends, its configured living markdown PRD is the sole requirement-ID authority. README.md and AGENTS.md remain outside Bookends coverage; policy-document owns their quality. Candidate IDs are proposals until a human accepts and commits them.

Operationally, before the commit introducing `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, that PRD is authoritative. This AGENTS summary is subordinate and referential, not a second product policy: for any new provenance burden, name the observed ordinary-use failure and why a smaller mechanism using existing durable state, history, capture, or driver judgment is insufficient. Keep driver-authored metadata small, trust explicit materiality and applicability declarations except for cheap mechanical identity mismatches, prefer the narrowest honest correction, and preserve rich engine-generated history. Apply these limits with reference to [`docs/PRD.md`](docs/PRD.md) LE-107.

The engine owns durable run state and progression. Callers perform primary work externally. Providers `describe` topology and `evaluate` the exact transition the engine selected; they do not choose the next state, edit repositories, invoke reviewers, or judge semantic truth. Context `kind` and `data` are opaque to core; follow the active provider's record conventions. Generate-PRD is a research-provider profile and skill, not a fourth provider or a software-change precondition; candidate IDs remain provisional until explicit human acceptance and commit.

Before driving any run, load [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md) and the active provider skill: [software-change](crates/software-change-provider/skills/using-software-change-provider/SKILL.md), [policy-document](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md), or [research](crates/research-provider/skills/using-research-provider/SKILL.md). Those skills own setup, binding and driving procedure; this file owns checkout obligations. Confirm work-slot policy with the user before `start`.

The selected per-run profile owns software-change policy, live review states and bound execution choices; follow the provider skill's exact-byte confirmation procedure and rehash that same file immediately before its hash-guarded start. For unbound launches, keep a separate owner-confirmed role→model manifest: verify exact models with `pi --list-models`, pass them explicitly and preserve launch evidence. Stop rather than substitute; an all-unbound profile does not supply that manifest.

Fan-out spawn/capture/conformance mechanics belong to the engine. Providers/callers own role framing and output content. Reviewers produce judgments only. Drivers run deterministic checks, `show`, capture triage, `append`, `event`, and progression. Exit 0 alone does not establish deliverable validity. Before fan-out or plan-graph invocation, load the engine and software-change skills above and [docs/agent-usage.md](docs/agent-usage.md) for working-directory, selection, stdin, capture, Dagu, overrun re-show and zero-axis review-binding rules. The driver maintains the existing checkout; graph execution grants no worktree lifecycle authority.

Plan-graph Do: before invocation, use unique task IDs matching `[A-Za-z0-9_-]+` except reserved `summarizer`, and ensure `dependency_graph` is acyclic and every endpoint names a declared task.

Before observation, mutation or evidence reuse, follow the engine skill's current-visit and full-inspection rules. Helpers require `show --view full` input; status observation is not mutation permission. Preserve fixed original runtime and skills for released v0.19.0 runs; candidate interfaces do not retrofit them. Core/provider identity checks do not judge semantic applicability.

For late findings, load the software-change skill's proportional late-finding guide and contract-v2 procedure before choosing a route or subset. Correct the actual owning phase and reconfirm affected downstream proof, preserving valid work. Human-facing challenge review requires current evidence, meaningful falsification, a violated frozen obligation and concrete consequence; `*-adversarial-review` remains a machine identifier.

Unrelated recovery amendments in `docs/PRD.md` and the software-change PRD retain their draft status. Source integration of accepted requirements is not committed PRD authority, semantic audit or acceptance of unrelated drafts. Exact owner acceptance, authorized commit, traceability reconciliation and final proof remain separate driver-owned duties. Their proposed LE-107 mandate makes simple-first, YAGNI and KISS apply to Loop Engine and every provider it generates, ships or uses. Added complexity needs a meaningful current requirement/failure and an inadequate simpler alternative explained briefly in the ordinary design. Existing architecture, protocols, schemas, dependencies and mechanisms have no presumption of preservation. No gold-plating, speculative hardening or separate justification framework.

Current source software-change profiles use contract v2 (`minimal-9`, `standard-9`, `high-rigor-9`) with independent criterion/goal author counts of one and remain unbound. Opt-in review construction defaults to one commission per used author per gate, distinct axis verdicts, separate ordinary/challenge slots and explicit unaffected carry. The driver dispositions exact failing sources; accepted-unresolved findings still block across revisions, and discharged fails are not passes. Final validation indexes real command evidence, every current AC-N and a separate goal judgment at one checkpoint; validation axes consume that collection rather than duplicate proof reviews.

Before execution correction, cancellation or exceptional override, load the engine skill and CLI reference. Overrun never authorizes overlapping retry/departure; historical missing ownership grants no arbitrary-PID cleanup permission. Overrides never fabricate reviewer success and permanently label completion `completed-with-overrides`. No active-run migration or frozen-policy reinterpretation is authorized.

Default compact bound delivery is distinct from independent full verification input. Do not substitute delivered stdin for required verification snapshots; load the engine/provider skills for capture and legacy compatibility rules.

Use [operational UX contracts](docs/operational-ux-contracts.md) for `capture`, deterministic `monitor`, optional budgeted advisory summaries, and content-checked post-commit delivery pointers. Observation/summary success never progresses a gate. Use [coordinator guidance](skills/coordinating-loop-engine/SKILL.md) for separate driver ownership and escalation, not implied start authority. After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof; a repository checkpoint is not a commit, and workers never commit independently. Any authorized Git identity change requires current proof rather than receipt relabeling.

Do for provider authors: closed object initial-input parsing and schemas must allow the reserved engine-injected `artifact_root` property. Read [allocation semantics](docs/agent-usage.md) before changing that boundary; caller isolation is not a workaround.

## Workflow

Build the engine and the reference providers, then run the repository baseline. The dependency audit requires the exact `cargo-machete` version checked by its repository command:

```sh
cargo install cargo-machete --version 0.9.2 --locked
python3 scripts/dependency-audit.py
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Use focused commands while iterating; they do not replace the workspace completion gate. Package integration roots are now imported as modules by the one central target, so `--test NAME` names `workspace`, not a former source module:

```sh
cargo test -p loop-cli --lib
cargo test -p software-change-provider --lib
python3 scripts/run-nextest.py --filter TEST_SUBSTRING
python3 scripts/run-central-tests.py --filter TEST_SUBSTRING
```

`run-nextest.py` is the supported fresh-binary command. It builds the current
five production binaries and three `loop-reference-fixtures` binaries,
exports their direct active-target paths through
`LOOP_ENGINE_TEST_BINARY_HANDOFF`, runs workspace unit tests and
`workspace-integration/workspace` with pinned cargo-nextest, then runs Cargo
doctests. It rejects stale, hashed, outside-target, missing, and
non-executable handoffs. Inspect the one-target contract with:

```sh
python3 scripts/run-central-tests.py \
  --no-run \
  --compiler-artifacts /tmp/central-test-artifacts.jsonl \
  --handoff-output /tmp/stock-cargo-handoff.json
python3 scripts/assert-test-topology.py --compiler-artifacts /tmp/central-test-artifacts.jsonl
python3 scripts/assert-test-inventory.py \
  --compiler-artifacts /tmp/central-test-artifacts.jsonl --current-only
```

The topology check uses Cargo metadata and compiler artifacts and requires
exactly one integration-test executable across the workspace. The inventory
check maps the emitted names to every retained source declaration. The
central runner's `--handoff-output` is only for the following independent
stock compatibility gate; it is not a second test runner:

```sh
LOOP_ENGINE_TEST_BINARY_HANDOFF=/tmp/stock-cargo-handoff.json \
  cargo test --workspace
```

The local cache proof is `python3 scripts/sccache-proof.py proof --artifact
/tmp/testing-sccache.json`; it compiles equivalent real workspace inputs into
two temporary target directories, requires a positive warm hit, and never uses
`cargo clean`. The required preflight pins sccache 0.17.0, cargo-nextest
0.9.143, and cargo-machete 0.9.2, starts the credential-free GitHub Actions
cache before any Cargo compilation, and emits final cache statistics. Run the
whole dependency audit with:

```sh
cargo install cargo-machete --version 0.9.2 --locked
python3 scripts/dependency-audit.py
```

For an enabled repository, run `scripts/bookends-check-gate.sh` from the repository root in pre-push and required CI. It emits `GREEN`, `RED`, or `BYPASS`; only an explicit `BOOKENDS_BYPASS=<class>:<reason>` may bypass a red repository gate. Current source requires durable local invocation evidence as well as visible output; recording failure refuses bypass permission. Missing required shallow parent history fails closed; continuity still compares the immediate parent only. The parser-only candidate command is `bookends-check candidate PRD.md`. Bookends does not load README.md or AGENTS.md as coverage classes. The optional software-change overlay is off unless a per-run profile sets `extra.bookends.enabled` to JSON `true`; its driver and bound-worker citation procedure is in the provider skill.

The reusable CI preflight installs and checks pinned cargo-nextest, sccache, and cargo-machete, starts sccache before Cargo compilation, then runs `python3 scripts/dependency-audit.py`, `python3 scripts/run-nextest.py`, `python3 scripts/prove-nextest-timeout.py`, the central one-target and current-inventory assertions, the direct `cargo test --workspace` compatibility gate with a fresh binary handoff, workspace clippy/fmt, locked package builds, `scripts/bookends-check-gate.sh` without a bypass, `dist generate --check`, `scripts/assert-dist-plan.py`, `scripts/assert-release-gates.py`, `scripts/assert-push-main-preflight.py`, `scripts/assert-generate-prd-profile.py`, the journey interface self-tests, and all four public source journeys. Each is a separate serialized step; `.github/workflows/release.yml` remains cargo-dist-generated.

Workers run their assigned focused validation; reviewers consume retained command/outcome evidence and do not rerun suites. One designated driver/proof owner runs the complete current plan-owned final stable-tree matrix once and repeats only checks invalidated by later changes. Its serialized phases include tools/cache setup, dependency/release/Bookends checks, nextest/doctests, timeout/topology/inventory, stock Cargo, clippy/fmt/builds, discovery/profile/interface assertions, public source journeys, required benchmark collect/compare and local final cache statistics/wall time. The software-change journey's independent isolated jobs use one explicit `--jobs` budget (default 2, serial 1); shared-run steps and outer matrix remain ordered. Final benchmarks require the current matrix's exact benchmark argv, inputs, repeated comparable datasets and pass criteria, not a pilot speedup claim or invented public benchmark CLI. Missing matrix/harness coordinates are a driver proof dependency; follow [receipt procedure](docs/implementation-report-proof.md). For cache and hosted proof, load [testing/cache procedure](docs/agent-usage.md#workspace-rust-test-and-preflight-path) and `.github/workflows/preflight.yml`: identify the owner-authorized commit and its actual push-to-main preflight run, with successful startup, final statistics and wall time. Static assertions are not hosted execution. Provisional reports name pending proof honestly; local statistics are not hosted execution. After one owner-approved commit, run the push-to-main preflight for that exact commit and require its hosted sccache startup, final statistics, and total wall time to pass before Package 7b.

Public-boundary Python journeys are required validation for every in-scope change. Workspace `cargo test` / clippy / fmt do not substitute for them. Build the four release packages, then run the journeys that cover the public boundary you touched; if that boundary is unclear, run all four source journeys: software-change full source traversal, policy-document draft AND audit, research source, and generate-prd source.

- Engine, CLI, core, integrations, shared `scripts/`, or `invoke` / work-slot behavior: all four source journeys: software-change full source traversal, policy-document draft AND audit, research source, and generate-prd source.
- `software-change` crate: `scripts/software-change-journey.py --mode source --traversal-depth full`.
- `policy-document` crate: both `scripts/policy-document-journey.py` modes.
- `research` crate: `scripts/research-journey.py --mode source` and `scripts/generate-prd-journey.py --mode source`.
- Bookends checker or repository gate: `cargo test -p bookends-check --offline` plus the enabled repository gate.
- Journey-harness edits: that script's `--self-test` when it has one, plus the journeys it runs.

`python3 scripts/software-change-journey.py --self-test` must print `worker-data skill/root policy assertions passed` after executing the three provider skill constructors (software-change high-rigor design-review, policy-document shipped semantic policies/target/mode, research verify and synthesize) and root AGENTS rules. Source full mode must print `contracted fan-out failure` only after bound deterministic workers prove compact plan-worker location stdin, separately forwarded review context where declared, exit-0 nonconformance, persisted summary/captures, and failed overlay.

```sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check
python3 scripts/software-change-journey.py --self-test
python3 scripts/research-journey.py --self-test
python3 scripts/generate-prd-journey.py --self-test
python3 scripts/assert-generate-prd-profile.py
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full \
  --jobs 2
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
python3 scripts/research-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/research \
  --profile crates/research-provider/data/configs/standard.json
python3 scripts/generate-prd-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/research \
  --checker target/debug/bookends-check \
  --profile crates/research-provider/data/configs/generate-prd.json
```

When a change can affect release proof or generated workflow, run the complete proof set:

```sh
dist generate --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py --self-test
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py
python3 scripts/assert-push-main-preflight.py
```

Drive production runs with `loop-engine`. Pass `--json` and parse the single JSON envelope. Pass `--config` on `start` with uncommitted machine-local provider TOML. Use the normal user catalog. No database or artifact override unless the human explicitly requests isolation in this session; other runs and old preferences do not authorize isolation. Before start, load canonical [Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup), inspect effective overrides and resolve the actual catalog there. Use exact aliases `software-change`, `policy-document`, and `research` and absolute `command` paths. Do not commit provider TOML, run databases, or secrets.

Use the engine skill and [CLI reference](docs/agent-usage.md) for start argv, JSON outcomes and serialized progression; use the active provider skill for its stdin/argv contract. These common mechanics are not a second checkout runbook.

When drafting or auditing `README.md` or `AGENTS.md`, use the policy-document provider and a copy of the shipped profile (`readme-2` or `agents-2`). Keep `target.id` and `profile_version` unless intentionally authoring a custom profile. Local markdown links must resolve under the target file's directory; parent-directory segments (`..`) are rejected as escapes, so crate docs must not markdown-link outside the crate. Web, mail, `data:`, fragment-only, and protocol-relative links are ignored by that check.

Do not hand-edit `.github/workflows/release.yml`; it is cargo-dist generated. Change dist metadata and regenerate. Direct pushes to `main` run read-only preflight only; publication is dispatch-only via `gh workflow run release.yml --ref main -f tag="$TAG"` after versioning and review. Do not force-push `main`.

Synthetic journey evidence proves deterministic mechanics, routing, and persistence only. It is not semantic review quality.

## Safety tiers

- **Advisory documentation:** Markdown in `README.md` and this file is advisory and non-enforcing; resolve conflicts using the scoped Authority order above.
- **Repository safety:** this checkout has no tracked hook for secret or runtime-artifact detection. Before committing, inspect staged paths and the staged diff with `git diff --cached --name-status` and `git diff --cached`; never commit secrets, machine-local provider TOML, run databases, or runtime artifacts.
- **Operator confirmation:** ask before committing, pushing, destructive actions, or any worktree lifecycle action.
- **Release safety:** keep `.github/workflows/release.yml` generated, keep direct pushes to `main` read-only, and use the dispatch-only publication path above.

## Completion and Handoff

A change is done when in-scope behavior matches the accepted intent, authoritative docs for that behavior are current, workspace baseline checks have been run, and the black-box Python journeys that cover the touched public boundary have passed.

A software-change `implementation-report.json` for this checkout must identify repository state as the current `git rev-parse HEAD` value plus `+uncommitted-worktree`, and `changed_surface` must equal the pathname list from `git status --porcelain=v1 --untracked-files=all` in that order (the path after each two-letter status). `scripts/assert-implementation-report.py` checks that identity/path contract against the supplied current plan matrix and actual `proof/receipts.json` execution evidence, not a hard-coded success-string inventory. Drive it with `--report PATH --revision REVISION --plan-revision PLAN_REVISION --matrix PATH`. Missing/failed/pending local proof fails the final check; retain this check externally as post-report evidence, not its own preexisting pass. After-authorization Git/hosted/dogfood work remains separately pending until observed. Receipt and status forms are in [docs/implementation-report-proof.md](docs/implementation-report-proof.md). Prove the checker with `python3 scripts/assert-implementation-report.py --self-test`.

Handoff must include:

- files changed and why
- commands run and outcomes
- any Loop Engine run IDs plus the database path used
- remaining risks, known residuals, and follow-up that was out of scope

Do not claim a provider "reviewed" work because a checked transition passed. Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there. A fresh actor resumes from `show` plus the same database and the external paths named in initial input, context, and instructions — not from chat history.
