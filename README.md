# Loop Engine

## Overview

Loop Engine is a **pull-based gated state machine** for work performed outside the engine. A driver — human, agent, or script — reads current state, does the work, appends evidence, and requests an event. The engine accepts or rejects. It does not run the work, schedule the next job, or choose the next state.

The everyday analog is a **strict issue tracker**: `show` is the ticket (where you are, which transitions are legal, how a fresh actor resumes); `event` is a transition you request rather than a status you type. Unlike GitHub Issues, the caller cannot set state, a checked edge cannot fire without provider `allow`, and a deny survives a new session and a more fluent model. For quick human correlation, `show --compact` is a concise projection of that same ticket; the detailed JSON `show` and `invocation-progress` operations remain the machine-readable authorities.

The engine owns durable progression. The caller owns execution. Humans and agents use the same commands, evidence, and gates.

```text
show → perform work externally → append evidence → request an event → accept or reject → repeat
```

Current release is v0.17.0 (`MIT OR Apache-2.0`). The living product requirements are [docs/PRD.md](docs/PRD.md). Agent CLI semantics are [docs/agent-usage.md](docs/agent-usage.md). Checkout operating rules for agents are [AGENTS.md](AGENTS.md). When a repository enables Bookends, its configured living PRD is the sole requirement-ID authority; README.md and AGENTS.md remain outside Bookends coverage.

### Why not a workflow engine, Temporal, or an FSM library

DAG and job orchestrators (Airflow, Dagu, and kin) **push**: the runtime fires the next ready node, owns workers, retries, and queues. Loop Engine **waits to be asked**. Cycles and revise edges are normal topology, not a DAG to flatten. A job DAG still belongs **inside** a work slot (for example a plan of CLI workers). It does not replace the run.

**Temporal** is the closest durable-workflow cousin, and still the wrong shape. Temporal **runs your workflow function**: the runtime replays history, schedules activities, and resumes the orchestration after sleeps, signals, and worker polls. Activity workers pull jobs the workflow has already decided to run. Routing lives in code the runtime executes. Loop Engine never runs a workflow function. A driver pulls `show` and requests an event; the provider only `allow`s, `deny`s, or returns `unsupported` for that exact edge. Temporal is durable *execution* of orchestration. Loop Engine is durable *progression authority* for work it does not perform. Use Temporal (or Dagu) where a slot needs a push DAG of workers. Do not encode `approved` vs `revise` as Temporal control flow — that puts routing back in the thing that did the work.

Typical state-machine libraries **push events into** a machine (`send`) and assume something else is driving. Loop Engine is the driver protocol: one primary read (`show`) is enough for a new actor to resume without the last conversation.

Issue trackers share the pull shape and none of the kernel. Anyone with write can relabel a ticket. Loop Engine will not.

### Why the constraints

The gates are not there because a model is too dumb to follow a checklist. They are there because a capable model is a **bad witness of its own completeness**, and because a run is not one model.

A strong LLM will follow a process in a single sitting if you ask. It will also, when tired, steered, or incentivized to ship, declare the work done and produce fluent evidence that looks like review. Capability makes that easier, not harder: better models skip ceremony more fluently and justify the skip better.

Three properties the checklist-plus-model story usually erases:

1. **It is not one actor.** The next session, model, or compacted context does not have the last conversation. `show` and durable denies exist for the ensemble over days.
2. **Routing is the failure.** The dangerous move is picking `approved` instead of `revise`, or treating “the worker exited 0” as “the work is good.” A model that understands the graph will still choose the convenient edge. Callers cannot set state. Providers cannot route. They only `allow`, `deny`, or `unsupported` the exact event the engine selected.
3. **Evidence is not judgment.** The kernel does not decide whether a design is wise. It decides whether frozen obligations and review records permit the event. The same process that did the work must not be the thing that makes the edge true.

If a human is on every transition, a ticket plus discipline can be enough, and this kernel is a tax. The constraints pay off for **multi-session or lightly attended agent driving**: frozen topology and bindings, checked `allow`/`deny` that stick, and a handoff that does not depend on chat.

### Start here

- Operators installing or invoking binaries: [Getting Started](#getting-started) and [Usage](#usage).
- Agents running a workflow: [docs/agent-usage.md](docs/agent-usage.md) plus the provider README and skill.
- Maintainers cutting a release: [Validation](#validation).

Reference providers:

- [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md) — software-change workflow (PRD section 10).
- [`crates/policy-document-provider/README.md`](crates/policy-document-provider/README.md) — policy-document workflow (PRD section 11).
- [`crates/research-provider/README.md`](crates/research-provider/README.md) — research workflow (PRD section 12).

## Getting Started

### Prebuilt GitHub Releases

Current releases publish separate cargo-dist archives for all four binaries and supported targets:

- `loop-cli-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `loop-engine`.
- `loop-cli-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `loop-engine`.
- `software-change-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `software-change`.
- `software-change-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `software-change`.
- `policy-document-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `policy-document`.
- `policy-document-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `policy-document`.
- `research-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `research`.
- `research-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `research`.

Each archive has a matching `.sha256` file; release `sha256.sum` provides the unified checksum list. Archives include `LICENSE-MIT` and `LICENSE-APACHE`. They do not contain, vendor, bundle, or install the `dagu` binary or Dagu source; `dagu` stays an operator-provided PATH dependency (minimum 2.14.0) resolved at run time. Dagu is GPLv3: facades invoke that binary as a subprocess only and do not embed its Go API. Download matching archives from [GitHub Releases](https://github.com/cartwmic/loop-engine/releases), verify checksums, and place all four binaries on `PATH`.

Generated cargo-dist installers choose platform automatically:

```sh
VERSION=v0.17.0
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/loop-cli-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/software-change-provider-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/policy-document-provider-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/research-provider-installer.sh" | sh
```

With [mise](https://mise.jdx.dev/), manage `loop-engine` as one tool and use separate provider installers. Do not add multiple executable selections for the same GitHub repository to one mise config: mise canonicalizes them to one tool entry, so binaries would be missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.17.0'
for app in software-change-provider policy-document-provider research-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/v0.17.0/$app-installer.sh" | sh
done
```

Historical v0.2.2 releases include only `loop-engine` and `software-change`.

### Build from source

Install all four binaries from GitHub source:

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
cargo install --git https://github.com/cartwmic/loop-engine policy-document-provider --bin policy-document --locked
cargo install --git https://github.com/cartwmic/loop-engine research-provider --bin research --locked
```

Or build all four binaries from a checkout:

```sh
cargo build --release -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
# target/release/loop-engine
# target/release/software-change
# target/release/policy-document
# target/release/research
```

## Usage

`loop-engine` stores run state in a SQLite catalog and snapshots provider association, workflow topology, and state instructions at `start`. For normal production use, omit `--database` and `artifact_root` unless the human explicitly asked to isolate that session; the engine then uses the user-level catalog and an engine-owned per-run artifact directory. When those options and database environment variables are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db`, `$LOOP_HOME/loop.db`, `$XDG_DATA_HOME/loop-engine/loop.db`, or `$HOME/.local/share/loop-engine/loop.db` in that order. Perform the work named by `show` externally, and call `show` before every mutating `append`, `event`, `invoke`, or `terminate`; a state transition requires a new observation. The same view exposes selected-output identity and the durable change report. Fresh bound evidence names only its invocation and assignment; core resolves the selected output metadata. Review reuse uses one `evidence-applicability` record referring to the original evidence context record, current target, attesting driver, and short reason. Request one event from `requestable_events`; semantic applicability remains the driver's judgment.

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show [--compact] RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MS] invoke RUN_ID SLOT_ID [--input DATA_JSON]
loop-engine [--database DB] [--json] [--timeout-ms MS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--task ID ... | --tasks ID,ID,...] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

The first eight forms are run-state operations. `show --compact RUN_ID` is a human-only mode of the existing `show` operation, not a ninth operation: it prints fixed-order lifecycle/state, requestable events, the latest checked result, the active-or-latest invocation overlay, and available Dagu helper counts. It never derives overlay success from inner progress. If inner progress is unavailable, it says so while preserving a successful durable show; `reaped` means a Dagu helper finished, not that the work succeeded. Compact omits observation-time elapsed/remaining counters, so unchanged state produces the same text; use detailed `--json show` for those fields and `--json invocation-progress` for complete machine-readable inner-progress data. `--json show --compact` is rejected. `invocation-progress`, `fan-out`, and `preview-bindings` are other commands, not a ninth primary. `fan-out` and `preview-bindings` do not open the run database or advance a run. `invocation-progress` reads the catalog for one invocation and does not append, invoke, request events, or write overlay. `software-change run-plan-graph` is a provider command, not an engine operation. Its required `--working-directory ABS` must name one existing absolute directory selected and maintained by the driver; it is applied to every ordinary plan task and the summarizer, and the provider does not create or manage worktrees. For a bound `implement` slot, omitted invocation input runs the full plan; `--input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'` focuses a later invocation on validated roots plus dependants while requiring same-revision standing prerequisites. Direct provider callers may still select roots on argv. The summarizer and repository checkpoint describe the resulting tree. Omitted `--max-active` allows four ordinary plan tasks; the mandatory summarizer runs only after all ordinary tasks succeed. If an ordinary task fails, the graph leaves mechanical `summary.json` and captures and does not write `implementation-report.json`. Dagu is an operator-provided PATH dependency (minimum 2.14.0), not part of release packages. Exact binding, stdin, capture, polling, and review procedures belong in the linked agent documentation and provider skills.

Pass `--json` and parse the single envelope. For software-change, read the frozen `artifact_root/intent.json` operating context before every phase. Reviewer output is candidate evidence: the driver preserves raw captures, treats advisory proposals as inert, and appends the authoritative finding ledger. A full-schema reviewer receives at most one same-worker correction; inspect `attempts.json` before triage.

Implementation and validation reports do not establish completion on their own. Run `software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS` against one existing absolute repository. The provider checks the report, document revisions, HEAD, index, status, tracked/non-ignored-untracked paths, and bytes. It never stages, commits, branches, pushes, creates, selects, merges, cleans, manages, or suggests worktrees. If validation exposes stale proof, use `revise-implementation`, regenerate both checkpoints and fresh review evidence, and retry final approval against the current tree.

For a minimal installed-binary start, materialize the provider's embedded data into an empty temporary root, copy one shipped profile, and create an uncommitted machine-local provider TOML with resolved absolute executable paths:

```sh
ENGINE="$(command -v loop-engine)"
PROVIDER="$(command -v software-change)"
case "$ENGINE:$PROVIDER" in
  /*:/*) ;;
  *) echo "PATH must resolve installed binaries to absolute paths" >&2; exit 1 ;;
esac

data_root="$(mktemp -d)"
"$PROVIDER" data-dump "$data_root"
profile=/tmp/loop-engine-software-change-minimal.json
cp "$data_root/crates/software-change-provider/data/configs/minimal.json" "$profile"
cat >/tmp/loop-engine-providers.toml <<EOF
[providers.software-change]
command = "$PROVIDER"
args = []
EOF
"$ENGINE" --json --config /tmp/loop-engine-providers.toml \
  start software-change "@$profile" "my run"
```

`start` initial input and `append` data accept JSON inline, `@FILE`, or `-` (stdin). Use `origin: {kind: "selected-assignment-output", id: INVOCATION_ID, assignment_id: ASSIGNMENT_ID}` for fresh bound evidence; use `evidence-applicability` with a context-record origin for explicit review reuse. Core and the provider resolve current mechanical identities; the driver supplies semantic applicability. `start` returns the run ID at `result.run.id`; reuse the same catalog and run ID for later operations. With `--json`, exit `0` is `completed`, `10` is `rejected` (follow feedback), `20` is `error` (re-read `show`), and `2` is `invalid-invocation`. Full handoff, binding, capture, and review procedures are in [AGENTS.md](AGENTS.md), [docs/agent-usage.md](docs/agent-usage.md), [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), and the provider skills. `loop-engine --help` and `--version` work before operations; `software-change` and `research` also support `--help`/`--version` and `data-dump`, while `policy-document` accepts `data-dump DIR` on argv and otherwise reads JSON on stdin.

## Adoption limits

The v0.1 scope is deliberately local and narrow ([docs/PRD.md](docs/PRD.md), especially its non-goals and authority invariants):

- no distributed execution, multi-user authentication, workflow migration, or special sensitive-data handling;
- one logical mutating actor per run;
- CLI and provider validation may evolve, but a stored run's workflow topology and provider association remain frozen;
- shipped binaries are the supported interface; workspace crates are not public API.

## Bookends

An enabled repository configures one living markdown PRD and explicit proof surfaces in `bookends.toml`. The `bookends-check` library/CLI parses the PRD, resolves `bookends:LE-<n>` citations, checks live and optional contract coverage, and verifies tracked, non-skipped files are collected by named required-CI commands. It compares only with the immediately preceding committed PRD: exact ID plus title is identity, retirement keeps an exact-title tombstone, and tombstones cannot disappear or revive. `bookends-check candidate PRD.md` validates only candidate grammar.

Repository gates use `scripts/bookends-check-gate.sh`, which prints `GREEN`, `RED`, or `BYPASS`. Only an explicit `BOOKENDS_BYPASS=<class>:<reason>` bypasses a red gate, and its printed output is the invocation evidence. README.md and AGENTS.md are not coverage classes. The software-change overlay is off by default; enable it only on a per-run copy of a shipped profile with `extra.bookends.enabled: true`. Its artifact IDs, live-ID checks, validation gate, and worker citation instructions are documented in the provider skill.

A repository without a schema-valid git PRD can use the research provider's [Generate-PRD skill](crates/research-provider/skills/using-generate-prd/SKILL.md) and `crates/research-provider/data/configs/generate-prd.json` to produce a provisional `prd-candidate.md` with per-requirement tracked evidence. Validate it with `bookends-check candidate prd-candidate.md`; the parser-only command does not check coverage, CI, or continuity. A human must accept or reject the candidate before any commit to `docs/PRD.md`; the path never auto-edits that file or commits.

## Validation

Supported publication matrix is exactly four applications (`loop-cli`, `software-change-provider`, `policy-document-provider`, `research-provider`) by two native targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`). `dist plan` describes this matrix; it does not compile or run archives.

Run baseline checks, generated-workflow validation, plan assertion, and full source-tree public-boundary journeys before release handoff:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
dist generate --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py --self-test
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py
python3 scripts/assert-push-main-preflight.py
python3 scripts/software-change-journey.py --self-test
python3 scripts/research-journey.py --self-test
python3 scripts/generate-prd-journey.py --self-test
python3 scripts/assert-generate-prd-profile.py
scripts/bookends-check-gate.sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check
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

Build local host-target archives and smoke extracted binaries before handoff. Use only a newly approved, unpublished release tag; never reuse an existing public tag:

```sh
TAG=vX.Y.Z
dist build --tag="$TAG" --artifacts=local --target=aarch64-apple-darwin
```

Run packaged smoke with extracted `loop-engine`, `software-change`, `policy-document`, and `research` paths. Each provider must materialize embedded data, and all provider journeys must run outside checkout; policy-document covers both draft and audit modes, and the research packaged journey materializes embedded data via `data-dump` / `--mode packaged`. A macOS host build proves only macOS arm64; Linux x86_64 native build and archive smoke remain CI proof when no Linux host is available.

Journey evidence records are synthetic and schema-conforming. They prove deterministic policy mechanics, routing, aggregation, persistence, sparse work-slot `invoke` via `scripts/dummy-work-slot-worker.py`, and contracted fan-out stdin/conformance via `scripts/dummy-stdin-worker.py`; they do not prove semantic review quality. Source full mode additionally uses separate CLI processes and real temporary Git repositories to prove context forwarding, driver ledger/proposal routing, preserved retry attempts, report-only denial, every named implementation/validation state invalidation, validation recovery, and current-tree final proof. Dummy inner workers prove Dagu-backed `fan-out` and `run-plan-graph` facade contracts (compact `artifact_root` stdin, bound `capture_dir/summary.json`, per-index or per-task stdout/stderr). The bound plan-graph journey freezes a driver-owned symlink alias, requires the checkout's `.git` marker, and checks every task and summarizer cwd with filesystem-equivalence semantics. The plan-graph dummy writes `implementation-report.json` only when stdin is the summarizer assignment. `python3 scripts/software-change-journey.py --self-test` must print `worker-data skill/root policy assertions passed` after the three provider skill constructors and root AGENTS rules pass. Source full mode must print `contracted fan-out failure` after the bound conforming/refusal overlay proof.

A software-change aggregate `implementation-report.json` for this checkout is checked by `scripts/assert-implementation-report.py` (`--report`, `--revision`, `--plan-revision`; prove with `--self-test`). That checker is not a publication gate. It requires `coverage.commit` to be current `git rev-parse HEAD` plus `+uncommitted-worktree` and `changed_surface` to match `git status --porcelain=v1 --untracked-files=all` pathnames.

### Publication path

Release workflow is dispatch-only. Do not push a version tag to trigger publication. After versioning, preflight, and review for a new unpublished tag, dispatch the generated workflow with that tag input:

```sh
gh workflow run release.yml --ref main -f tag="$TAG"
```

Dispatch runs cargo-dist's native local-build matrix first, then its generated global-artifact dependencies: preflight, global artifacts, and archive smoke. Host directly depends on all four proof gates and can create the version tag and GitHub Release only after each succeeds. cargo-dist 0.32's generated host expression tolerates skipped dependencies, so `scripts/assert-release-gates.py` proves supported hook topology makes skipped required gates unreachable on publishing paths and rejects failure/skipped regressions. Pull requests run the same preflight and upload-mode artifact path without publication.

Private/free GitHub repositories cannot fully prevent an owner from creating an out-of-band raw tag. Such a tag is outside supported release procedure and does not trigger this workflow; future repository rulesets or plan capability would be needed for prevention.

Historical `v0.2.0`, `v0.2.1`, and `v0.2.2` tags remain immutable. `v0.2.2` was the fix-forward release for contract closure; historical release facts are not rewritten. v0.3.0 added policy-document to the same native archive, installer, source-journey, and packaged-smoke gates as the engine and software-change provider. Research joins the same native archive, installer, source-journey, and packaged-smoke gates.

### Direct pushes to main

Direct pushes to `main` run `.github/workflows/push-to-main.yml`. That read-only dispatcher checks out the pushed SHA, computes the pinned cargo-dist 0.32.0 plan, and calls reusable `preflight.yml`; preflight installs operator-provided `dagu` 2.14.0 onto `PATH` (into `$RUNNER_TEMP`, not dist artifacts or crate packages), then owns workspace tests, warning-denying clippy, formatting, generated-release checks, release assertions, and source software-change-journey proof. A missing `dagu` fails those tests with the resolver error rather than a skip. The dispatcher has no publication job. `.github/workflows/release.yml` remains cargo-dist-generated and dispatch-only.
