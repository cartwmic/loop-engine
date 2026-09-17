# Loop Engine

## Overview

Loop Engine is a **pull-based gated state machine** for work performed outside the engine. A driver — human, agent, or script — reads current state, does the work, appends evidence, and requests an event. The progression kernel accepts or rejects that transition. It does not autonomously perform primary work or choose what to run. Caller-requested helpers such as `invoke`, `fan-out`, and `capture-command` can launch external processes without approving workflow transitions.

The everyday analog is a **strict issue tracker**: `show` is the ticket, and `event` requests a transition. Callers cannot set state directly. A normal checked edge requires provider `allow`; a deny survives a new session and a more fluent model. An explicit owner exception is separately and permanently labeled. Humans and agents use the same commands and gates.

```text
show → perform work externally → append evidence → request an event → accept or reject → repeat
```

The released baseline is v0.20.0 (`MIT OR Apache-2.0`). Current source adds contract-v3 review profiles, roster setup, process-incarnation checks and publication-history checking. Those changes are not part of the v0.20.0 release. Use a checkout build to try current source; source proof, publication and installation remain separate delivery facts.

### Why not a workflow engine, Temporal, or an FSM library

DAG and job orchestrators (Airflow, Dagu, and kin) **push**: the runtime fires the next ready node, owns workers, retries, and queues. Loop Engine **waits to be asked**. Cycles and revise edges are normal run topology. A job DAG belongs inside a work slot, such as a plan of CLI workers; it does not replace the run.

**Temporal** runs your workflow function: the runtime replays history, schedules activities, and resumes orchestration. Routing lives in code that runtime executes. Loop Engine gives progression authority to a kernel that accepts driver-requested events. The provider only `allow`s, `deny`s, or returns `unsupported` for the exact edge selected by the engine. Use Temporal or Dagu where a slot needs a push DAG of workers. Encoding `approved` versus `revise` inside that work would put routing back in the thing that did the work.

A state-machine library usually needs a separate driver protocol and persistence layer. Loop Engine supplies a durable `show` handoff so a fresh actor can resume without the previous conversation.

### Why the constraints

A capable model is a **bad witness of its own completeness**, and a run is not one model.

A strong LLM will follow a process in a single sitting if you ask. It will also, when tired, steered, or incentivized to ship, declare the work done and produce fluent evidence that looks like review. Better models can justify the skip more fluently.

Three properties the checklist-plus-model story usually erases:

1. **It is not one actor.** The next session, model, or compacted context does not have the last conversation. `show` and durable denies exist for the ensemble over days.
2. **Routing is the failure.** A driver may request `approved` when revision is needed, or treat a worker's exit 0 as proof that its work is good. The provider checks the exact requested edge against frozen obligations.
3. **Judgment remains external.** The kernel checks whether obligations and review records permit progression. It does not decide whether a design is wise. Authorship and independent review remain separate roles.

If a human is on every transition, a ticket plus discipline can be enough. The constraints pay off for multi-session or lightly attended agent driving, where handoff and rejection history must survive the conversation.

### Start here

- **Operators:** [Getting Started](#getting-started), [Usage](#usage) and [Troubleshooting](#troubleshooting).
- **Agents:** [checkout rules](AGENTS.md), the [engine skill](skills/using-loop-engine/SKILL.md), then the active provider skill. The [CLI reference](docs/agent-usage.md) owns exact command forms.
- **Maintainers:** [checkout validation and release procedure](AGENTS.md#workflow) and the [testing/cache procedure](docs/agent-usage.md#workspace-rust-test-and-preflight-path).
- **Product requirements:** [docs/PRD.md](docs/PRD.md). In a Bookends-enabled repository, the configured living PRD is the sole requirement-ID authority.

Reference providers have their own contracts and driving procedures:

- [Software-change](crates/software-change-provider/README.md) — implementation and review workflows.
- [Policy-document](crates/policy-document-provider/README.md) — document drafting and audits.
- [Research](crates/research-provider/README.md) — research, including [Generate-PRD](crates/research-provider/skills/using-generate-prd/SKILL.md) for provisional requirements.

## Getting Started

### Execution prerequisite

For fan-out or plan-graph workers, install **Dagu 2.14.0 or newer** from [Dagu releases](https://github.com/dagu-org/dagu/releases) and put `dagu` on `PATH`. Check the installed version:

```sh
dagu version
```

Engine/provider archives do not include Dagu. It is an operator-provided GPLv3 subprocess dependency; Loop Engine does not embed its Go API.

### Prebuilt GitHub Releases

[GitHub Releases](https://github.com/cartwmic/loop-engine/releases) publishes separate cargo-dist archives for `loop-cli` (the `loop-engine` binary), `software-change-provider`, `policy-document-provider`, and `research-provider`. Supported targets are macOS arm64 (`aarch64-apple-darwin`) and Linux x86_64 (`x86_64-unknown-linux-gnu`). Archive names follow `APPLICATION-TARGET.tar.xz`; each has a `.sha256` file, and `sha256.sum` lists all checksums. Archives include both license files.

These generated installers select the platform and install the released v0.20.0 binaries:

```sh
VERSION=v0.20.0
for app in loop-cli software-change-provider policy-document-provider research-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/$app-installer.sh" | sh
done
```

With [mise](https://mise.jdx.dev/), manage the engine separately and use the provider installers above. Multiple executable selections for the same GitHub repository collapse to one mise tool entry and can leave binaries missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.20.0'
```

### Build from source

Install [rustup](https://rustup.rs/) and native build tools: macOS Command Line Tools or a Linux C compiler/linker, also needed for bundled SQLite. `rust-toolchain.toml` pins the checkout's Rust 1.98.0 toolchain; there is no public MSRV promise. From the checkout root:

```sh
cargo build --release --locked \
  -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
export PATH="$PWD/target/release:$PATH"
```

Alternatively, install committed GitHub source directly:

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
cargo install --git https://github.com/cartwmic/loop-engine policy-document-provider --bin policy-document --locked
cargo install --git https://github.com/cartwmic/loop-engine research-provider --bin research --locked
```

## Usage

A normal run uses a user-level SQLite catalog and an engine-owned artifact directory. The [engine setup procedure](skills/using-loop-engine/SKILL.md#deterministic-setup) covers overrides and provider registration; agents must follow its confirmation rules before starting a run.

For a human-operated unbound software-change run, materialize the installed provider's matching data and select a profile:

```sh
ENGINE="$(command -v loop-engine)"
PROVIDER="$(command -v software-change)"
case "$ENGINE:$PROVIDER" in
  /*:/*) ;;
  *) echo "PATH must resolve installed binaries to absolute paths" >&2; exit 1 ;;
esac

data_root="$(mktemp -d)"
"$PROVIDER" data-dump "$data_root"
profile="$data_root/minimal.json"
cp "$data_root/crates/software-change-provider/data/configs/minimal.json" "$profile"
cat >"$data_root/providers.toml" <<EOF
[providers.software-change]
command = "$PROVIDER"
args = []
EOF
"$ENGINE" --json --config "$data_root/providers.toml" \
  start software-change "@$profile" "my run"
```

The response supplies the run ID at `result.run.id`. Inspect it with the same catalog:

```sh
loop-engine --json list
loop-engine --json show RUN_ID
loop-engine --json show RUN_ID --view full
```

`show` provides current work and available events. Full view includes frozen input and retained evidence. Follow the [active provider's procedure](#start-here) to perform work, record evidence and request the shown event. Binding, selection, capture, monitoring and recovery command forms belong in the [CLI reference](docs/agent-usage.md) and [engine skill](skills/using-loop-engine/SKILL.md).

### Review profiles in current source

Contract v3 uses `minimal-10`, `standard-10` and `high-rigor-10`. Each covers ordinary and challenge review at intent, design, plan, implementation and validation:

| Profile | Review pattern | Final criterion/goal authors |
|---|---|---|
| Minimal | One reviewer covers all axes together | 1 |
| Standard | Two reviewers each cover all axes together | 2 |
| High | The same two identities review individual axes, then each reviews all axes in a fresh session before fixes | 2 |

The source `software-change setup` helper prepares bindings from a small explicit command/argument roster. Models and effort are chosen per run. See the [provider skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md) for setup and confirmation. Released v0.20.0 carries the earlier contract-v2 profiles.

## Adoption limits

This is a local workflow tool for a trusted sole owner and well-intentioned drivers. Author identities are caller claims; the provider checks their required relationships without signatures or identity allowlists. Do not treat workflow enforcement as an adversarial security boundary.

- Interfaces are still evolving during 0.x, including exact CLI spelling. Pin matching binaries, profiles/data and documentation for automation; expect to check compatibility when changing releases.
- Custom workflows require provider executables. Profiles configure the workflow their provider supports. There is no general workflow expression language, provider package-management API or stable SDK.
- One logical actor mutates each run. Parallel workers inside a slot do not create parallel workflow states.
- Arbitrary context records are caller assertions. Provider conventions determine whether they count as evidence or supersede earlier judgments. Every checked evaluation receives the accumulated append-only context; the engine provides no context filtering or deletion service for evaluation.
- Run history records durable workflow events. It is not an exhaustive execution or compliance audit trace; worker output lives in separate captures.
- The product assumes reasonably sized inputs/context and provides no special handling for sensitive data. Callers own confidentiality and retention decisions.
- Transaction atomicity covers engine-owned state/history. Provider reads of external files are not atomic with transition commits.
- The catalog freezes provider command/args, topology and instructions. It does not pin executable bytes. Operators must preserve version-matched binaries for active runs; replacing a binary at a stored path can make an existing run unsupported.
- No active-run migration, distributed execution, multi-user authentication, child workflows or compensation is provided.
- Shipped binaries are the supported interface; workspace crates are not public APIs.

## Troubleshooting

- **A long bound worker becomes `overrun`:** `invoke` shares the 30-second global timeout default. Choose an adequate `--timeout-ms` before launch. After overrun, wait or use supported `cancel-invocation` only for verified owned work; confirm cleanup and read `show` before retrying. Live work or pending cleanup blocks retry and departure. See the [execution recovery procedure](skills/using-loop-engine/SKILL.md#execution-recovery-minimum).
- **`missing standing prerequisites` after successful work:** released v0.20.0 can treat an optional `repository_effect` absent from both records as changed. Current source corrects that comparison, and facade preparation rejects invalid selections before creating an invocation. The [public retry regressions](crates/loop-cli/tests/engine/backlog_t02.rs) also check that rejected force-fresh selection preserves standing prerequisites. Changed or genuinely failed prerequisites still require a supported rerun; inspect full `show` before choosing its smallest necessary dependency chain. See [investigation dispositions](investigations.md) and the [provider recovery guide](crates/software-change-provider/skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide).
- **Completed work still shows `live_owned_work: true`:** v0.20.0's numeric PID/PGID checks can match unrelated processes after reuse. Current source adds process-incarnation checks for new runs. Do not cancel or signal a process whose ownership is uncertain, or edit ownership records to bypass the guard. Preserve the captures and stop for an owner recovery decision if the block persists; old runs require their matching runtime and guidance.
- **`succeeded` or exit 0, but the event is rejected:** facade success is process-level. Inner workers may have failed, returned empty/malformed output, or lack semantic approval. Inspect the named capture directory's `summary.json`, attempt records and raw output, then append valid provider evidence before requesting the event. See [work-slot delegation](docs/agent-usage.md#work-slot-delegation) and the [bound fan-out procedure](skills/using-loop-engine/SKILL.md#non-run-state-command-fan-out).
- **Output looks finished, but status or cleanup disagrees:** a sidecar or worker claim cannot replace catalog status and verified cleanup. Inspect full `show` and `invocation-progress`/`monitor`; resume supported cancellation only for verified owned work. Preserve captures and stop for owner recovery if cleanup cannot be established. See [execution correction and cancellation](docs/agent-usage.md#execution-correction-and-cancellation).
- **`unsupported`, unknown commands or runtime/catalog incompatibility after an upgrade:** use version-matched tagged documentation/data and preserved binaries for an existing run. A complete same-checkout build of engine and providers is the route for new source-profile runs. Mixing released v0.20.0 executables with current-checkout profiles does not migrate them. See the [v0.20.0 CLI reference](https://github.com/cartwmic/loop-engine/blob/v0.20.0/docs/agent-usage.md) or the [current source reference](docs/agent-usage.md).

## Bookends

Bookends checks a repository's living PRD against declared proof locations and required CI collection. Current source also checks every newly published reachable commit, including merged branches, and acquires missing shallow history. Incomplete coverage/history cannot produce complete GREEN. Explicit bypass remains visibly distinct as BYPASS.

README.md and AGENTS.md are outside Bookends coverage. Public API/CLI contract tests can qualify when they prove the promised behavior; citation tokens alone cannot. The software-change overlay is optional and off in shipped profiles. Use the [Bookends configuration contract](crates/bookends-check/schema/repo-config.md) and [provider skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md) for adoption.

[Generate-PRD](crates/research-provider/skills/using-generate-prd/SKILL.md) can prepare a provisional candidate when a repository lacks a living PRD. Human acceptance and an authorized commit are required before that candidate becomes requirement authority.

## Validation

After the pinned tool/cache setup in [checkout validation](AGENTS.md#workflow), the local Rust test entry is:

```sh
python3 scripts/run-nextest.py
```

The [checkout workflow](AGENTS.md#workflow) owns the complete validation matrix and release procedure. [Testing/cache guidance](docs/agent-usage.md#workspace-rust-test-and-preflight-path) covers fresh binary handoffs and cache proof. Synthetic journeys establish deterministic mechanics; semantic review remains external. Local checks do not establish hosted success or another platform's behavior.

Direct pushes to `main` run read-only preflight for the pushed commit. Publication is separately dispatched after the required review and proof; the [release workflow](.github/workflows/release.yml) is cargo-dist-generated. Follow [maintainer procedure](AGENTS.md#workflow) for dispatch and [receipt requirements](docs/implementation-report-proof.md) for implementation reports.

Historical `v0.2.0`, `v0.2.1` and `v0.2.2` tags remain immutable. `v0.2.2` was the fix-forward release for contract closure; `v0.3.0` added policy-document. Current native archives cover all four applications on both supported targets.
