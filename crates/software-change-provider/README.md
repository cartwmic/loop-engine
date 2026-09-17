# Software-change provider

## Overview

`software-change` is Loop Engine's reference provider for software-change workflows. Its standalone binary includes versioned profiles, artifact templates and review guidance. The engine owns durable state and progression; callers author work and review it externally.

- `describe` returns the workflow selected by optional `initial_input.review_policies`, with the sixteen-state union when that key is omitted.
- `evaluate` checks the exact engine-selected transition against frozen schemas, revision links, external review evidence and applicable checkpoint/ledger rules.
- The deterministic `setup` helper prepares a per-run configuration from shipped review data and explicit caller commands. It does not start or progress a run.

The protocol operations do not launch models, edit authored subject artifacts or judge semantic truth. An allowed implementation-to-validation `evaluate` records checkpoint history under `implementation-proof-history/`. Requirements R1–R29 and A1–A16, including amendments, are in [docs/prd.md](docs/prd.md); [data/reviewer-protocol.md](data/reviewer-protocol.md) defines the review contract.

Checkpoints bind report bytes to Git HEAD, index/status and tracked or non-ignored untracked repository entries. Accepted implementation checkpoints are retained under `implementation-proof-history/`; later validation must match that accepted identity. Checkpointing does not stage, commit, push or manage worktrees.

Submodule entries identify their HEAD. Dirty submodule contents require separate content proof or committed, clean submodules because different edits can leave the same parent status.

### Start here

- **Human operators:** [Setup](#setup), [Usage](#usage), [Limits](#limits) and [Troubleshooting](#troubleshooting).
- **Agents and drivers:** [AGENTS.md](AGENTS.md) governs this checkout. Drive source builds with the [current provider skill](skills/using-software-change-provider/SKILL.md); drive released v0.20.0 with its [versioned skill](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md). Each names its matching engine companion.
- **Maintainers:** [crate validation](AGENTS.md#workflow) and the repository-root checkout instructions.

## Setup

This README describes its checkout. Current source uses contract v3. Released v0.20.0 carries earlier contract-v2 profiles; preserve its matching binaries and [versioned driving guide](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md) for those runs. Source builds and published releases have separate delivery status.

### Install a release

[GitHub Releases](https://github.com/cartwmic/loop-engine/releases) supplies separate engine and provider archives for macOS arm64 and Linux x86_64. Archives contain the executable and license texts. GitHub Releases provides separate SHA-256 checksum files. The generated installers below select the platform and install v0.20.0:

```sh
VERSION=v0.20.0
for app in loop-cli software-change-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/$app-installer.sh" | sh
done
ENGINE="$(command -v loop-engine)"
PROVIDER="$(command -v software-change)"
```

If the installed commands are missing from PATH, follow the installer's printed destination/PATH instructions before continuing.

### Build current source

Install Rust through rustup and native compiler/linker tools for bundled SQLite. The repository's `rust-toolchain.toml` selects the toolchain. Run from the repository root:

```sh
cargo build --locked -p loop-cli -p software-change-provider
ENGINE="$PWD/target/debug/loop-engine"
PROVIDER="$PWD/target/debug/software-change"
```

Fan-out and plan-graph execution also require operator-installed [Dagu](https://github.com/dagu-org/dagu/releases) >=2.14.0 on PATH. Archives do not include it. Dagu runs as a GPLv3 subprocess. Check it before binding such workers:

```sh
dagu version
```

## Usage

### First unbound run

This first-run example works with either matching binary pair above and selects the minimal profile embedded in that provider. It launches no model workers. Before an agent executes `start`, confirm the profile and work-slot policy using the matching guide: [released v0.20.0 setup](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md#setup) or [current-source setup](skills/using-software-change-provider/SKILL.md#setup). Shipped profiles are unbound; copying one does not select or authorize a model.

```sh
set -eu
case "$ENGINE:$PROVIDER" in
  /*:/*) ;;
  *) echo "ENGINE and PROVIDER must be absolute executable paths" >&2; exit 1 ;;
esac
DATA_ROOT="$(mktemp -d)"
"$PROVIDER" data-dump "$DATA_ROOT"
PROFILE="$DATA_ROOT/run-profile.json"
cp "$DATA_ROOT/crates/software-change-provider/data/configs/minimal.json" "$PROFILE"
PROVIDER_CONFIG="$DATA_ROOT/providers.toml"
cat >"$PROVIDER_CONFIG" <<EOF
[providers.software-change]
command = "$PROVIDER"
args = []
EOF
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@$PROFILE" "my software change"
```

Set `RUN_ID` to `result.run.id` from the completed start response, replacing the placeholder below:

```sh
RUN_ID="REPLACE_WITH_RETURNED_RUN_ID"
"$ENGINE" --json show "$RUN_ID" --view full
```

`start` allocates the run's durable artifact directory and records its absolute path as `initial_input.artifact_root`; full `show` reveals it with the frozen obligations. Keep version-matched binaries and dumped guidance for the run, and retain machine-local provider TOML outside committed files. Use the normal user catalog unless the owner explicitly requests isolation.

The data dump preserves `crates/software-change-provider/data/...` paths. It refuses existing target files. Subject filenames are `intent.json`, `design.json`, `plan.json`, `implementation-report.json` and `validation-report.json`.

For a released v0.20.0 run, continue with its [versioned per-gate loop](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md#per-gate-loop). The remaining interface sections below describe current-source contract v3 and link to the [current per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop). Do not apply those v3 procedures to a v2 run.

### Review profiles

Current source profiles are `minimal-10`, `standard-10` and `high-rigor-10`, with `contract_version: 3`. Every level includes ordinary and challenge review at intent, design, plan, implementation and validation. Bookends is off until explicitly enabled.

| Profile | Review pattern | Independent criterion/goal authors |
|---|---|---|
| Minimal | One author reviews all axes together | 1 |
| Standard | Two authors each review all axes together | 2 |
| High | The same two authors review individual axes, then each reviews all axes in a fresh session on unchanged work before fixes | 2 |

High's first aggregate inputs exclude individual-stage findings. Both stages remain required; aggregate ordinary-validation authors alone produce the criterion/goal rows from retained command evidence. The [reviewer protocol](data/reviewer-protocol.md) defines the full stage and evidence rules.

Source `software-change setup` accepts a small ordered `{author,command,args}` roster. Each command must satisfy the worker contract; model/effort arguments are explicit caller choices. The helper exposes the complete profile for approval. Use the skill's [exact profile confirmation](skills/using-software-change-provider/SKILL.md#exact-profile-confirmation) before hash-guarded start.

### Review output

For contract v3, the complete worker batch is `{review_stage,author,judgments:[...]}`. Contract-v3 aggregate `validation-review` workers also supply `validation_verdicts` under their generated schema. See the [base worker schema](data/review-worker-output-schema.json), [worker preamble](data/review-worker-preamble.txt) and [reviewer protocol](data/reviewer-protocol.md).

A completed bound review can be inspected through this read-only pipe:

```sh
"$ENGINE" --json show --view full "$RUN_ID" | "$PROVIDER" review-candidates
```

`review-candidates` extracts per-axis candidates in durable invocation/assignment order and reports missing, malformed or exhausted output. `ready` means the selected bytes were readable, digest-matching and mechanically conforming. The command does not perform a new semantic review, retry workers or record evidence. The original worker judgments become gate evidence through driver inspection and recording under the [per-gate procedure](skills/using-software-change-provider/SKILL.md#per-gate-loop). A process exit 0 does not establish approval.

## Recovery and profile contracts

Public helpers keep execution and judgment separate:

| Helper | Public responsibility |
|---|---|
| `commission` | Select provider-scoped context for a slot/task from a completed full-show envelope |
| `review-candidates` | Extract inspectable candidate judgments from completed full-show data |
| `run-validation` | Execute named plan commands and emit command candidates plus an index draft |
| `prepare-validation` | Prepare report/candidate drafts and pending judgment-batch metadata from retained execution without executing, appending or checkpointing |
| `checkpoint` | Bind the matching implementation or validation report to repository identity |
| `run-plan-graph` | Execute the supplied task graph in one existing absolute Git working directory and retain task/summarizer results |

The [provider skill](skills/using-software-change-provider/SKILL.md) owns exact argv/stdin forms, checkpoint finalization, source disposition, evidence reuse and selected repair. The [late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide) identifies the owning phase for corrections.

Earlier-phase routes do not roll back repository edits. Live or cleanup-pending work blocks overlapping retry and departure. An exceptional owner override preserves failures and labels completion `completed-with-overrides`; it supplies no reviewer pass or Bookends GREEN. Older profiles keep their original runtime and obligations; current helpers provide no active-run migration.

## Criterion spine and Bookends overlay

Intent acceptance uses run-local AC-N IDs. Every new plan task needs meaningful current `criterion_ids`; other intermediate links remain optional. Final validation requires every current criterion and a separate goal judgment backed by real named command evidence at one accepted checkpoint.

The optional Bookends overlay adds one `prd_traceability` disposition per criterion: `linked-live`, `candidate` or `not-applicable`. A current candidate blocks final completion. `not-applicable` concerns traceability and does not waive or fulfill the criterion. Enabled final validation requires GREEN; BYPASS cannot satisfy it. Configuration and recording belong to the [overlay procedure](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay).

## Limits

- This is a local workflow for a trusted sole owner and well-intentioned drivers, with one logical mutator per run. It provides no multi-user authentication, remote scheduling/lifecycle management or special handling for sensitive data. Callers own confidentiality and retention.
- The 0.x CLI and profile/protocol interfaces can change; workspace crates are not supported public APIs. At this checkout, current source and the published v0.20.0 binaries both report `0.20.0` despite their contract-v3/v2 difference. `--version` alone cannot establish compatibility: preserve or verify the exact binary/profile identity and matching guidance.
- Profiles configure this provider's defined workflow. Custom topology requires another provider; the engine has no general workflow-expression language.
- Run history is not an exhaustive execution or compliance-audit trace. Worker output lives in separate captures; retain those alongside durable state. Inputs and context are assumed to be reasonably sized; high-volume operation is not guaranteed.
- Author identities, declared revisions/config versions and semantic applicability are trusted claims. False author claims or material edits without a revision bump can evade those checks. The provider does not authenticate reviewers.
- Artifact reads are not locked or atomic with engine transition commits. The workflow provides no adversarial or transactional assurance over external files.
- Checkpointing requires a valid Git HEAD and no unmerged index entries. Paths and symlink targets must be UTF-8; supported entries are regular files, symlinks and submodules. Read failures or unsupported entries prevent a valid checkpoint.
- Failed workers can leave partial edits. Neither successful execution nor a retry establishes semantic approval.
- Stable digests and synthetic journeys establish identity/mechanics. They do not establish semantic truth. Calibration fixtures use supplied fictional companions and must not be resolved against a live checkout.

## Troubleshooting

- **`data-dump` refuses to overwrite:** choose a fresh empty dump root. Retain matching dumped data needed by active runs; do not overwrite it with a different release.
- **`setup --output PATH` replaces a file:** this helper uses atomic replacement. Use a new run-specific path, or inspect/back up the existing destination before deliberately replacing it.
- **`checkpoint mismatch` after checkpoint creation succeeded:** checked evaluation uses the provider process's CWD. The checkpoint command's `--working-directory` does not configure later evaluation. Request checked events from the intended checkout and keep HEAD, index/status and tracked/non-ignored untracked bytes stable. Follow the skill's [owning-phase recovery](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide) to regenerate affected reports/checkpoints and review evidence.
- **Worker output is rejected despite exit 0:** inspect selected/raw captures and the generated output schema, including `review_stage` and validation-specific rows. Keep failed attempts and follow the [review procedure](skills/using-software-change-provider/SKILL.md#per-gate-loop).
- **Plan-graph rejects a task ID:** IDs must match `[A-Za-z0-9_-]+` and cannot be `summarizer`. Correct IDs and dependency references through the owning plan phase and normal review before retrying. See the [plan repair route](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide).
- **An older profile is unsupported:** restore its preserved matching runtime and guidance. A new binary at the same path does not migrate the run.

## Validation

For an operator installation smoke check, use the `PROVIDER` path established in Setup:

```sh
printf '%s\n' '{"operation":"describe"}' | "$PROVIDER"
```

A successful workflow description establishes basic provider execution only. It does not prove profile compatibility, completed workflow behavior or semantic review quality.

Maintainers must follow the complete [crate/workspace validation procedure](AGENTS.md#workflow), including its focused tests, self-tests, formatting and public journeys. Full source journeys use real CLI processes and completed outcomes; packaged journeys consume extracted binaries and dumped data. Their isolated fixture catalogs grant no production-isolation authority. Calibration, document review and final stable-tree proof remain separate obligations; hosted success requires observed execution there.

## Shipped data

### Config profiles

- [Minimal](data/configs/minimal.json)
- [Standard](data/configs/standard.json)
- [High-rigor](data/configs/high-rigor.json)

### Artifact templates

`artifact_schemas` use a bounded language: object, array and string declarations, restricted keywords, and designated built-in ID patterns at supported locations. General numeric/boolean schemas, `$ref`, composition and arbitrary regex are unsupported. [src/schema.rs](src/schema.rs) and the shipped profiles define the exact subset. Worker `full_output_schema` is a separate contract.

Templates: [intent](data/templates/intent.md), [design](data/templates/design.md), [task packet](data/templates/task-packet.md), [implementation report](data/templates/implementation-report.md), [validation report](data/templates/validation-report.md), [finding ledger](data/templates/finding-ledger.json) and [advisory finding proposal](data/templates/advisory-finding-proposal.json). Advisory proposals require driver disposition before authoritative recording.

### Review and calibration

The [worker schema](data/review-worker-output-schema.json) declares `{review_stage,author,judgments}`; aggregate ordinary validation uses the generated `validation_verdicts` extension. The [reviewer protocol](data/reviewer-protocol.md) defines evidence and adjudication rules. The [calibration procedure](data/calibration/PROCEDURE.md) defines supplied-material framing, fresh review and attestation; the [manifest](data/calibration/manifest.json) records actual row status. Changed supplied bytes require fresh review. No shipped harness invokes reviewers or rewrites attestations.
