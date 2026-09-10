# Loop Engine

## Overview

Loop Engine is a **pull-based gated state machine** for work performed outside the engine. A driver — human, agent, or script — reads current state, does the work, appends evidence, and requests an event. The progression kernel accepts or rejects the requested transition; it does not autonomously perform primary work or choose work to run. Caller-requested execution helpers such as `invoke`, `fan-out`, and `capture-command` can launch external processes without choosing or approving workflow transitions.

The everyday analog is a **strict issue tracker**: `show` is the ticket (where you are, which transitions are legal, how a fresh actor resumes); `event` is a transition you request rather than a status you type. Unlike GitHub Issues, the caller cannot set state, a normal checked edge requires provider `allow` (an explicit owner exception is separately and permanently labeled), and a deny survives a new session and a more fluent model. In current source, ordinary `show` is the focused action view; `show --view status` (or human-readable `show --compact`) observes without arming mutation. Use `show --view full` for complete frozen input, context, evaluation history and invocation/change-report evidence, and `invocation-progress` for inner progress.

The engine owns durable progression. The caller directs execution, including any use of engine execution helpers. Humans and agents use the same commands, evidence, and gates.

```text
show → perform work externally → append evidence → request an event → accept or reject → repeat
```

The preserved released baseline for this source work is v0.19.0 (`MIT OR Apache-2.0`). The focused views, monitoring/capture helpers and default compact bound delivery described here are candidate source functionality, not a claim of published availability or completed final proof. The living product requirements are [docs/PRD.md](docs/PRD.md). Agent CLI semantics are [docs/agent-usage.md](docs/agent-usage.md). Checkout operating rules for agents are [AGENTS.md](AGENTS.md). When a repository enables Bookends, its configured living PRD is the sole requirement-ID authority; README.md and AGENTS.md remain outside Bookends coverage.

### Why not a workflow engine, Temporal, or an FSM library

DAG and job orchestrators (Airflow, Dagu, and kin) **push**: the runtime fires the next ready node, owns workers, retries, and queues. Loop Engine **waits to be asked**. Cycles and revise edges are normal topology, not a DAG to flatten. A job DAG still belongs **inside** a work slot (for example a plan of CLI workers). It does not replace the run.

**Temporal** is the closest durable-workflow cousin, and still the wrong shape. Temporal **runs your workflow function**: the runtime replays history, schedules activities, and resumes the orchestration after sleeps, signals, and worker polls. Activity workers pull jobs the workflow has already decided to run. Routing lives in code the runtime executes. Loop Engine's progression kernel does not run a workflow function; caller-requested execution helpers are a separate boundary. A driver pulls `show` and requests an event; the provider only `allow`s, `deny`s, or returns `unsupported` for that exact edge. Temporal is durable *execution* of orchestration. Loop Engine is durable *progression authority* for externally performed work directed by the caller. Use Temporal (or Dagu) where a slot needs a push DAG of workers. Do not encode `approved` vs `revise` as Temporal control flow — that puts routing back in the thing that did the work.

Typical state-machine libraries **push events into** a machine (`send`) and assume something else is driving. Loop Engine is the driver protocol: one primary read (`show`) is enough for a new actor to resume without the last conversation.

Issue trackers share the pull shape and none of the kernel. Anyone with write can relabel a ticket. Loop Engine will not.

### Why the constraints

The gates are not there because a model is too dumb to follow a checklist. They are there because a capable model is a **bad witness of its own completeness**, and because a run is not one model.

A strong LLM will follow a process in a single sitting if you ask. It will also, when tired, steered, or incentivized to ship, declare the work done and produce fluent evidence that looks like review. Capability makes that easier, not harder: better models skip ceremony more fluently and justify the skip better.

Three properties the checklist-plus-model story usually erases:

1. **It is not one actor.** The next session, model, or compacted context does not have the last conversation. `show` and durable denies exist for the ensemble over days.
2. **Routing is the failure.** The dangerous move is picking `approved` instead of `revise`, or treating “the worker exited 0” as “the work is good.” A model that understands the graph will still choose the convenient edge. Callers cannot set state. Providers cannot route. They only `allow`, `deny`, or `unsupported` the exact event the engine selected.
3. **Evidence is not judgment.** The kernel does not decide whether a design is wise. It decides whether frozen obligations and review records permit the event. The same process that did the work must not be the thing that makes the edge true.

If a human is on every transition, a ticket plus discipline can be enough, and this kernel is a tax. The constraints pay off for **multi-session or lightly attended agent driving**: frozen topology and policy, inspectable effective execution bindings, checked `allow`/`deny` that stick, and a handoff that does not depend on chat.

### Start here

- Operators installing or invoking binaries: [Getting Started](#getting-started) and [Usage](#usage).
- Agents running a workflow: start with [AGENTS.md](AGENTS.md) and the [engine skill](skills/using-loop-engine/SKILL.md), then load the [software-change](crates/software-change-provider/skills/using-software-change-provider/SKILL.md), [policy-document](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md), or [research](crates/research-provider/skills/using-research-provider/SKILL.md) skill. [CLI reference](docs/agent-usage.md) owns exact command forms.
- Maintainers cutting a release: [Validation](#validation).

Reference providers:

- [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md) — software-change workflow (PRD section 10).
- [`crates/policy-document-provider/README.md`](crates/policy-document-provider/README.md) — policy-document workflow (PRD section 11).
- [`crates/research-provider/README.md`](crates/research-provider/README.md) — research workflow (PRD section 12).

## Getting Started

### Prebuilt GitHub Releases

The v0.18.0 installation examples below are historical pinned examples, not a way to install the current candidate interfaces. Build the checkout containing the changes to try them; GitHub-source installs contain only committed source. Released binaries carry their matching profiles through `data-dump`.

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
VERSION=v0.18.0
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
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.18.0'
for app in software-change-provider policy-document-provider research-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/v0.18.0/$app-installer.sh" | sh
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

For a checkout build, install [rustup](https://rustup.rs/) and native build tools (macOS Command Line Tools or a Linux C compiler/linker toolchain, also needed for bundled SQLite). `rust-toolchain.toml` pins Rust 1.98.0; this is the checkout toolchain, not a public MSRV promise. Run from the repository root:

```sh
cargo build --release -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
# target/release/loop-engine
# target/release/software-change
# target/release/policy-document
# target/release/research
export PATH="$PWD/target/release:$PATH"
```

## Usage

`loop-engine` stores run state in a SQLite catalog and snapshots provider association, workflow topology, and state instructions at `start`. For normal production use, omit `--database` and `artifact_root` unless the human explicitly asked to isolate that session; the engine then uses the user-level catalog and an engine-owned per-run artifact directory. Before start, load canonical [Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup) to inspect overrides and resolve the effective catalog; other runs or old preferences do not authorize isolation. `show` gives the current work and available events. Follow the [CLI reference](docs/agent-usage.md) for observation before mutations and the [engine skill](skills/using-loop-engine/SKILL.md) for driving procedure.

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show [--view action|status|full | --compact] RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID [--override JSON]
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MS] invoke RUN_ID SLOT_ID [--preview] [--controls JSON] [--input DATA_JSON]
loop-engine [--database DB] [--json] amend-binding RUN_ID SLOT_ID JSON
loop-engine [--database DB] [--json] cancel-invocation RUN_ID INVOCATION_ID
loop-engine [--database DB] [--json] [--timeout-ms MS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--task ID ... | --tasks ID,ID,...] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

The first ten forms are the primary run-state operations in this source checkout. `show --compact RUN_ID` is a human-readable mode of `show`; it adds no operation. Use `--json show` for current action, `--json show --view full` for complete evidence and helper input, and `--json invocation-progress` for inner progress. `--compact` aliases non-arming status in JSON or text; combining it with full view is rejected. The remaining forms cover invocation inspection, ad hoc workers, binding previews, and the software-change provider's plan graph.

Current source also provides `loop-engine capture-command`, `capture-matrix`, and `capture-abort` for serial command/matrix execution and retained failure evidence, and `loop-engine monitor` for deterministic observation with optional separately budgeted advisory summaries. Neither approves, advances, retries or cancels workflow work. See [operational UX contracts](docs/operational-ux-contracts.md) for exact forms, cleanup and resume rules. The [coordinator skill](skills/coordinating-loop-engine/SKILL.md) helps separate run drivers, ownership and escalation; it does not replace provider procedures or authorize starts.

Bound fan-out delivery is compact by default for every worker, not model-selected: only engine-owned top-level `data.loop_engine_origin` is omitted from cloned context records. Meaningful context and concise origins remain. Full routed-input snapshots are captured independently for verification; captured stdin stays truthful. Ad-hoc instruction bytes and plan-graph task/summarizer boundaries are unchanged. A successful process or mechanically valid capture is not semantic acceptance.

For binding, capture, evidence and checkpoint procedures, load the [engine skill](skills/using-loop-engine/SKILL.md) and [software-change skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md).

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

`start` initial input and `append` data accept JSON inline, `@FILE`, or `-` (stdin). `start` returns the run ID at `result.run.id`; reuse the same catalog and run ID for later operations. With `--json`, exit `0` is `completed`, `10` is `rejected` (follow feedback), `20` is `error` (re-read `show`), and `2` is `invalid-invocation`. Full handoff, binding, capture, and review procedures are in [AGENTS.md](AGENTS.md), [docs/agent-usage.md](docs/agent-usage.md), [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), and the provider skills. `loop-engine --help` and `--version` work before operations; `software-change` and `research` also support `--help`/`--version` and `data-dump`, while current-source `policy-document` accepts `data-dump DIR` and `commission FROZEN_PROFILE_JSON` on argv and otherwise reads protocol JSON on stdin.

### Recovery in the current source contract

Recovery keeps execution corrections, cancellation, rescoping and review history inside the existing run. The [CLI reference](docs/agent-usage.md) owns command forms and cleanup timing. The [software-change recovery contract](crates/software-change-provider/README.md#recovery-and-contract-v2) covers finding disposition, steering, review batching and criterion/goal proof.

Owner-attested `event --override` preserves failures and skipped checks and permanently labels final completion `completed-with-overrides`. It does not create a review pass, provider allow or Bookends GREEN. See the [engine driving procedure](skills/using-loop-engine/SKILL.md) before using it.

Current-source software-change profiles declare contract v2 (`minimal-9`, `standard-9`, `high-rigor-9`). Older semantic profiles require their fixed original provider; stored graphs retain their original routes. No active-run migration is provided. Policy-document and research keep their own evidence contracts.

Unrelated recovery requirement amendments retain their pending status. Source integration alone does not establish committed PRD authority, semantic acceptance or final identity-bound proof: exact human acceptance, authorized commit and required evidence are separate duties. The calibration manifest records actual row status; calibration approval does not replace document review, exhaustive clause review or stable-tree measurements. Simple-first, YAGNI and KISS apply product-wide: complexity needs a meaningful current reason and inadequate simpler alternative in the ordinary design; current architecture/protocol/dependency choices need not be preserved for their own sake. Focused deterministic proof does not establish semantic review, final benchmark acceptance, hosted exact-commit proof or later live dogfood.

## Adoption limits

The v0.1 scope is deliberately local and narrow ([docs/PRD.md](docs/PRD.md), especially its non-goals and authority invariants):

- no distributed execution, multi-user authentication, workflow migration, or special sensitive-data handling;
- no parallel or hierarchical workflow states, child workflows, or compensation; parallel workers inside a slot are not parallel workflow progression;
- one logical mutating actor per run;
- transition atomicity covers engine-owned state and history, not provider observations of external files together with transition commits. The engine does not lock or version external work; callers needing approval of an exact artifact revision must arrange external identity/revision controls;
- CLI and provider validation may evolve, but a stored run's workflow topology and provider association remain frozen;
- shipped binaries are the supported interface; workspace crates are not public API.

## Bookends

An enabled repository configures one living markdown PRD and explicit proof surfaces in `bookends.toml`. The `bookends-check` library/CLI parses the PRD, resolves `bookends:LE-<n>` citations, checks live and optional contract coverage, and verifies tracked, non-skipped files are collected by named required-CI commands. It compares only with the immediately preceding committed PRD: exact ID plus title is identity, retirement keeps an exact-title tombstone, and tombstones cannot disappear or revive. `bookends-check candidate PRD.md` validates only candidate grammar.

Repository gates use `scripts/bookends-check-gate.sh`, which prints `GREEN`, `RED`, or `BYPASS`. Only an explicit `BOOKENDS_BYPASS=<class>:<reason>` can bypass a red gate. Current source also requires durable local invocation evidence; a recording failure refuses bypass permission. BYPASS is never GREEN. Required parent history must be available in shallow clones or continuity fails closed; the scope remains the immediate parent, not every pushed transition. README.md and AGENTS.md are not coverage classes. The software-change overlay is off by default; enable it only on a per-run copy of a shipped profile with `extra.bookends.enabled: true`. Its artifact IDs, live-ID checks, validation gate, and worker citation instructions are documented in the provider skill.

A repository without a schema-valid git PRD can use the research provider's [Generate-PRD skill](crates/research-provider/skills/using-generate-prd/SKILL.md) and `crates/research-provider/data/configs/generate-prd.json` to produce a provisional `prd-candidate.md` with per-requirement tracked evidence. Validate it with `bookends-check candidate prd-candidate.md`; the parser-only command does not check coverage, CI, or continuity. A human must accept or reject the candidate before any commit to `docs/PRD.md`; the path never auto-edits that file or commits.

## Validation

Supported publication matrix is exactly four applications (`loop-cli`, `software-change-provider`, `policy-document-provider`, `research-provider`) by two native targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`). `dist plan` describes this matrix; it does not compile or run archives.

After the pinned tool setup in [checkout validation](AGENTS.md#workflow), the supported local Rust test entry is:

```sh
python3 scripts/run-nextest.py
```

Completion also requires workspace checks, public source journeys and applicable release/archive proof. Load [AGENTS Workflow](AGENTS.md#workflow) for the required commands and [testing/cache procedure](docs/agent-usage.md#workspace-rust-test-and-preflight-path) for test handoffs, topology, tool pins and cache proof. Local proof does not establish hosted exact-commit success, and a macOS archive test does not prove Linux behavior.

Synthetic journeys prove deterministic mechanics, not semantic review quality. Implementation reports index actual execution evidence; missing, failed or pending proof cannot establish completion. The [receipt contract](docs/implementation-report-proof.md) and [handoff procedure](AGENTS.md#completion-and-handoff) own identity and report checks. The report checker is not a publication gate.

### Publication path

Release workflow is dispatch-only. Do not push a version tag to trigger publication. After versioning, preflight, and review for a new unpublished tag, dispatch the generated workflow with that tag input:

```sh
gh workflow run release.yml --ref main -f tag="$TAG"
```

Dispatch runs cargo-dist's native local-build matrix first, then its generated global-artifact dependencies: preflight, global artifacts, and archive smoke. Host directly depends on all four proof gates and can create the version tag and GitHub Release only after each succeeds. cargo-dist 0.32's generated host expression tolerates skipped dependencies, so `scripts/assert-release-gates.py` proves supported hook topology makes skipped required gates unreachable on publishing paths and rejects failure/skipped regressions. Pull requests run the same preflight and upload-mode artifact path without publication.

Private/free GitHub repositories cannot fully prevent an owner from creating an out-of-band raw tag. Such a tag is outside supported release procedure and does not trigger this workflow; future repository rulesets or plan capability would be needed for prevention.

Historical `v0.2.0`, `v0.2.1`, and `v0.2.2` tags remain immutable. `v0.2.2` was the fix-forward release for contract closure; historical release facts are not rewritten. v0.3.0 added policy-document to the same native archive, installer, source-journey, and packaged-smoke gates as the engine and software-change provider. Research joins the same native archive, installer, source-journey, and packaged-smoke gates.

### Direct pushes to main

Direct pushes to `main` run the read-only [push preflight](.github/workflows/push-to-main.yml) for the pushed SHA; they do not publish. Required proof and tool setup live in [AGENTS Workflow](AGENTS.md#workflow) and [reusable preflight](.github/workflows/preflight.yml). `.github/workflows/release.yml` remains cargo-dist-generated and dispatch-only.
