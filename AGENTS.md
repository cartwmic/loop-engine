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

Shipped skills: [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), `crates/software-change-provider/skills/using-software-change-provider/SKILL.md`, `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md`, and `crates/research-provider/skills/using-research-provider/SKILL.md`. Shipped software-change profiles omit `work_slot_bindings`; bound Pi workers are opt-in templates in those skills (`--no-extensions` plus `-e` placeholders, filled in the per-run profile JSON). `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`, and reports a `dagu` PATH check (minimum 2.14.0) as ok with path and version or as a warning; warnings still exit 0. Confirm work-slot policy with the user before `start`.

For a software-change start, the exact selected per-run profile is the authority for `config_version`, live review states/axis IDs, normalized author counts, Bookends state, sparse bindings, and bound model arguments. Display its exact bytes and SHA-256, show the `preview-bindings` result, obtain owner confirmation, and rehash that same file immediately before `start`. Keep a generic owner-confirmed role→model manifest separate for unbound analyst, writer, ordinary-reviewer, challenge-reviewer, or other launches: verify each exact model with `pi --list-models`, pass it explicitly, preserve launch evidence, and stop rather than fall back or substitute. An all-unbound profile does not make that manifest profile-derived.

Fan-out spawn/capture/conformance mechanics belong to the engine. `software-change run-plan-graph --working-directory ABS` requires one existing absolute directory selected and maintained by the driver and does not create or manage worktrees. Full and selected-plan modes apply it to ordinary tasks and the summarizer; the summarizer runs only after all selected tasks succeed, while task failure leaves mechanical `summary.json` and captures and writes no `implementation-report.json`. Bound no-task repair applies it to exactly one `ad-hoc-repair` worker, runs no task or summarizer, and requires that worker to write the fresh report before provider checkpointing. Providers/callers own role framing and output content; the engine transports and checks those opaque values but does not author or interpret them. Reviewers produce judgments only. Drivers run deterministic checks, `show`, capture triage, `append`, `event`, and progression. Exit 0 alone does not establish deliverable validity. Load the exact fan-out and plan-graph binding, stdin, capture, polling, Dagu, overrun, and review-binding rules from [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), [docs/agent-usage.md](docs/agent-usage.md), and [crates/software-change-provider/skills/using-software-change-provider/SKILL.md](crates/software-change-provider/skills/using-software-change-provider/SKILL.md); the authoritative overrun re-show and zero-axis review-binding rules live there; do not duplicate those mechanics here.

Plan-graph Do: before invocation, use unique task IDs matching `[A-Za-z0-9_-]+` except reserved `summarizer`, and ensure `dependency_graph` is acyclic and every endpoint names a declared task.

Before any mutating run operation, the driver must call `show` for the current state and instructions. `show` arms the current state visit; `append`, `event`, `invoke`, and `terminate` refuse otherwise, while `list`, `history`, and `invocation-progress` do not arm it. After a state transition, observe again before the next mutation. Completed invocation records expose selected assignment/attempt identity and a durable fail-closed change report; consult that report before reuse. Fresh bound evidence names only its invocation and assignment; core resolves the selected attempt, capture, command, binding, path, and digest. Review reuse is one `evidence-applicability` record naming the original evidence context record, current target, attesting driver, and short reason. Core and provider do not infer semantic applicability. Bound invoke and `software-change run-plan-graph` support validated subsets without rewriting frozen bindings; plan-graph still summarizes and checkpoints the resulting tree.

For late findings, follow the provider skill's proportional guide: keep validation-report-only corrections in validation; use `revise-implementation` plus `{plan_revision,task_roots}` selection when an existing task owns an implementation defect; use bound `{repair_finding_ids}` only for a current accepted unresolved implementation finding whose empty `task_ids` honestly means no frozen task owns it; use `revise-plan`, `revise-design`, or `revise-intent` only when that owning phase's obligation is materially wrong. Observe with `show` before invoking the same frozen slot, then resolve the ledger finding and reconfirm affected implementation and validation proof. Reconfirm only the affected downstream proof and preserve valid work; deeper backtracking is exceptional. Semantic owner override is not shipped. Human-facing challenge review must still require current evidence, meaningful falsification, a violated frozen obligation, and a concrete consequence; unchanged `*-adversarial-review` IDs remain machine identifiers.

## Workflow

Build the engine and the reference providers, then run the repository baseline:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Use focused stock Cargo commands while iterating; they do not replace the workspace completion gate. `--test NAME` names a retained suite root, not a former source module:

```sh
cargo test -p loop-cli --lib
cargo test -p loop-cli --test engine       # engine, workers, or dagu
cargo test -p software-change-provider --lib
cargo test -p software-change-provider --test contracts  # contracts, provider, cli, or plan_graph
```

For an enabled repository, run `scripts/bookends-check-gate.sh` from the repository root in pre-push and required CI. It emits `GREEN`, `RED`, or `BYPASS`; only an explicit `BOOKENDS_BYPASS=<class>:<reason>` may bypass a red repository gate, and that output is the invocation evidence. The parser-only candidate command is `bookends-check candidate PRD.md`. Bookends does not load README.md or AGENTS.md as coverage classes. The optional software-change overlay is off unless a per-run profile sets `extra.bookends.enabled` to JSON `true`; its driver and bound-worker citation procedure is in the provider skill.

CI preflight also runs `scripts/bookends-check-gate.sh` without a bypass, `dist generate --check`, `scripts/assert-dist-plan.py`, `scripts/assert-release-gates.py`, `scripts/assert-push-main-preflight.py`, `scripts/assert-generate-prd-profile.py`, `scripts/generate-prd-journey.py --self-test`, the software-change, policy-document, and research journey interface self-tests, a locked build of the four release packages plus `bookends-check`, the software-change source journey (`--mode source --traversal-depth full` against high-rigor), both policy-document source journey modes, the research source journey, and the Generate-PRD source journey.

Public-boundary Python journeys are required validation for every in-scope change. Workspace `cargo test` / clippy / fmt do not substitute for them. Build the four release packages, then run the journeys that cover the public boundary you touched; if that boundary is unclear, run all three source journeys.

- Engine, CLI, core, integrations, shared `scripts/`, or `invoke` / work-slot behavior: all three source journeys.
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
  --traversal-depth full
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

Drive production runs with `loop-engine`. Pass `--json` and parse the single JSON envelope. Pass `--config` on `start` with uncommitted machine-local provider TOML. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file. Use exact aliases `software-change`, `policy-document`, and `research` and absolute `command` paths. Do not commit provider TOML, run databases, or secrets.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

Parse the single JSON envelope even on nonzero exit. Treat only `status: "completed"` as success. `rejected` (exit 10) is an understood denial — follow feedback and continue. `error` (exit 20) means nothing advanced; re-run `show`. Request events from the latest `show`, never states. Serialize `append`, `event`, `invoke`, and `terminate` per run.

`loop-engine`, `software-change`, and `research` accept `--help`/`-h` and `--version`/`-V` before stdin. `policy-document` does not: unsupported argv besides `data-dump DIR` is an error; describe/evaluate remain stdin JSON.

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

A software-change `implementation-report.json` for this checkout must identify repository state as the current `git rev-parse HEAD` value plus `+uncommitted-worktree`, and `changed_surface` must equal the pathname list from `git status --porcelain=v1 --untracked-files=all` in that order (the path after each two-letter status). `scripts/assert-implementation-report.py` is the deterministic checker for that contract and the plan-owned final-matrix passed-string list, including the quoted pipefail commands stored as Python data. Drive it with `--report PATH --revision REVISION --plan-revision PLAN_REVISION`. Prove the checker with `python3 scripts/assert-implementation-report.py --self-test`.

Handoff must include:

- files changed and why
- commands run and outcomes
- any Loop Engine run IDs plus the database path used
- remaining risks, known residuals, and follow-up that was out of scope

Do not claim a provider "reviewed" work because a checked transition passed. Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there. A fresh actor resumes from `show` plus the same database and the external paths named in initial input, context, and instructions — not from chat history.
