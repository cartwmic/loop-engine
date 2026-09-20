# Software-change provider

## Overview

`software-change` is Loop Engine's reference provider for software-change workflows. Its standalone binary includes versioned profiles, artifact templates and review guidance. The engine owns durable state and progression; callers author work and review it externally.

- `describe` returns the workflow selected by `initial_input.review_policies`. Bare discovery returns a sixteen-state union; generation-11 contract-v3 profiles add reconciliation, yielding seventeen states when ordinary and challenge reviews are enabled for all five phases. Discovery is not configuration validation.
- `evaluate` checks the exact engine-selected transition against frozen schemas, revision links, external review evidence and applicable checkpoint/ledger rules.
- The deterministic `setup` helper prepares a per-run configuration from shipped review data and explicit caller commands. It does not start or progress a run.
- New contract-v3 profiles add a provider-owned `reconciliation` state after implementation editing. It records document and behavior observations before downstream report, checkpoint, review, validation, and final-proof work.

The `describe` and `evaluate` protocol operations do not launch models, edit authored subject artifacts or judge semantic truth. Separate execution helpers can launch workers and replace artifacts, as detailed below. An allowed implementation-to-validation `evaluate` records checkpoint history under `implementation-proof-history/`. Requirements R1–R29 and A1–A16, including amendments, are in [docs/prd.md](docs/prd.md); [data/reviewer-protocol.md](data/reviewer-protocol.md) defines the review contract.

Checkpoints bind report bytes to Git HEAD, index/status and tracked or non-ignored untracked repository entries. Accepted implementation checkpoints are retained under `implementation-proof-history/`; later validation must match that accepted identity. Checkpointing does not stage, commit, push or manage worktrees.

Submodule entries identify their HEAD. Dirty submodule contents require separate content proof or committed, clean submodules because different edits can leave the same parent status.

### Start here

- **Human operators:** [Setup](#setup), [Usage](#usage), [Limits](#limits) and [Troubleshooting](#troubleshooting).
- **Agents and drivers:** [AGENTS.md](AGENTS.md) governs this checkout. Drive source builds with the [current provider skill](skills/using-software-change-provider/SKILL.md); drive released v0.20.0 with its [versioned skill](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md). Each names its matching engine companion.
- **Maintainers:** [crate validation](AGENTS.md#workflow) and the repository-root checkout instructions.

## Setup

This README describes its checkout. Current source uses contract v3. Release numbers (such as `0.21.1`), semantic contract versions (`2`/`3`), and profile generations are separate: “v10” and “v11” below mean profile names ending in `-10` and `-11`, respectively, both using contract v3. Generation 11 adds reconciliation and the two intent questions; it is not product version 11 or contract version 11. Released v0.20.0 carries earlier contract-v2 profiles; preserve its matching binaries and [versioned driving guide](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md) for those runs. Source builds and published releases have separate delivery status.

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

**Known v0.20.0 limits:** an absent optional `repository_effect` can falsely cause `missing standing prerequisites`; inspect full `show` before choosing a supported dependency rerun. Numeric PID/PGID reuse can falsely mark completed work `live_owned_work: true`. Never cancel or signal uncertain ownership or edit ownership records to bypass the guard; preserve captures and seek an owner recovery decision. Current-source builds correct the comparison and add process-incarnation checks for new runs. Choose a matching current-source engine/provider pair for new runs needing those fixes; existing runs still require their preserved runtime and guidance.

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

Use a complete dumped or setup-generated profile. `start` stores the provider's described workflow but does not ask it to validate all initial-input fields. For example, omitting `review_policies` can create a durable run whose first checked transition fails evaluation. Successful creation or bare `describe` therefore does not establish usable configuration.

The data dump preserves `crates/software-change-provider/data/...` paths. It refuses existing target files. Subject filenames are `intent.json`, `design.json`, `plan.json`, `implementation-report.json` and `validation-report.json`; v11 runs also use `reconciliation.json`.

For a released v0.20.0 run, continue with its [versioned per-gate loop](https://github.com/cartwmic/loop-engine/blob/v0.20.0/crates/software-change-provider/skills/using-software-change-provider/SKILL.md#per-gate-loop). The remaining interface sections below describe current-source contract v3 and link to the [current per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop). Do not apply those v3 procedures to a v2 run.

### Review profiles

Current source profiles are `minimal-11`, `standard-11` and `high-rigor-11`, with `contract_version: 3`. Every level includes ordinary and challenge review at intent, design, plan, implementation and validation. Intent review additionally asks `acceptance-granularity` and `owner-comprehensible`; high-rigor keeps those new questions at its aggregate stage while its existing axes retain individual and aggregate stages. Bookends is off until explicitly enabled.

| Profile | Review pattern | Independent criterion/goal authors |
|---|---|---|
| Minimal | One author reviews all axes together | 1 |
| Standard | Two authors each review all axes together | 2 |
| High | The same two authors review individual axes, then each reviews all axes in a fresh session on unchanged work before fixes | 2 |

High's first aggregate inputs exclude individual-stage findings. Both stages remain required; aggregate ordinary-validation authors alone produce the criterion/goal rows from retained command evidence. The [reviewer protocol](data/reviewer-protocol.md) defines the full stage and evidence rules.

Source `software-change setup` accepts a small ordered `{author,command,args}` roster. Each command must satisfy the worker contract; model/effort arguments are explicit caller choices. The optional `--draft-worker PATH` input is a closed `{command,args}` object for the caller-supplied `intent-draft` worker; it does not replace the roster's review bindings. The helper exposes the complete profile for approval, previews bindings, and writes no run state. Use the skill's [exact profile confirmation](skills/using-software-change-provider/SKILL.md#exact-profile-confirmation) before hash-guarded start. Compact model-facing delivery is the supported default for bound drafting and review, while the full routed context and verification records remain available to deterministic consumers.

### Review output

For contract v3, the complete worker batch is `{review_stage,author,judgments:[...]}`. Contract-v3 aggregate `validation-review` workers also supply `validation_verdicts` under their generated schema. See the [base worker schema](data/review-worker-output-schema.json), [worker preamble](data/review-worker-preamble.txt) and [reviewer protocol](data/reviewer-protocol.md).

A completed bound review can be inspected through this read-only pipe:

```sh
"$ENGINE" --json show --view full "$RUN_ID" | "$PROVIDER" review-candidates
```

`review-candidates` extracts per-axis candidates in durable invocation/assignment order and reports missing, malformed or exhausted output. `ready` means the selected bytes were readable, digest-matching and mechanically conforming. The command does not perform a new semantic review, retry workers or record evidence. The original worker judgments become gate evidence through driver inspection and recording under the [per-gate procedure](skills/using-software-change-provider/SKILL.md#per-gate-loop). A process exit 0 does not establish approval.

## Reconciliation and document integration

New `minimal-11`, `standard-11`, and `high-rigor-11` runs insert `reconciliation` after implementation editing and before the final implementation proof boundary. The unbound `reconciliation-draft` slot authors `reconciliation.json`; the checked `reconciliation-ready` event validates it against [data/reconciliation-schema.json](data/reconciliation-schema.json). The result records its mode, branch, document and behavior observations, action, authorization/application/commit status, traceability, proof references, blockers, and completion decision.

The decision is conditional, not a requirement rewrite. Sufficient existing wording needs only implementation and public proof. Change-specific proof creates no permanent requirement. An implementation defect requires a code correction. Only missing or changed enduring meaning requires exact owner acceptance, separately authorized application and commit, mode-appropriate traceability, and independent inspection of the resulting documents and proof. Bookends-enabled changes update requirement links; the disabled document-edit path keeps traceability `not-applicable` with empty references. A justified no-document-change result is valid; unresolved or unauthorized gaps remain blocked.

Bookends-enabled runs inspect the accepted text of each cited requirement and every authoritative cross-reference. Bookends-disabled runs inspect relevant repository documents against the approved intent and delivered behavior without PRD IDs, candidate machinery, or overlay obligations. The provider does not accept a related ID, matching token, parser result, or command exit as semantic coverage, and reconciliation evaluation itself does not edit repository documents, perform Git actions or owner approval, or author reports/primary checkpoints. Separate provider helpers do write reports/checkpoints, and accepted implementation evaluation retains checkpoint history as described above. Final reports, repository checkpoints, implementation review, validation, and proof must use the post-reconciliation tree. Older profiles and stored runs keep their original graph and are not migrated.

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

The optional Bookends overlay adds one `prd_traceability` disposition per criterion: `linked-live`, `candidate` or `not-applicable`. A current candidate blocks final completion. `not-applicable` concerns traceability and does not waive or fulfill the criterion. Enabled final validation requires GREEN; BYPASS cannot satisfy it. Configuration and recording belong to the [overlay procedure](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay). New semantic-coverage guidance reads the cited requirement text and explicit authoritative cross-references; it does not treat a related live ID as sufficient coverage. Bookends-off runs keep the same semantic distinction without adding PRD metadata.

## Limits

- This is a local workflow for a trusted sole owner and well-intentioned drivers, with one logical mutator per run. It provides no multi-user authentication, remote scheduling/lifecycle management or special handling for sensitive data. Callers own confidentiality and retention.
- The 0.x CLI and profile/protocol interfaces can change; workspace crates are not supported public APIs. Current source reports `0.21.1`; the v0.20.0 release reports `0.20.0` and carries the earlier contract-v2 profiles. `--version` alone cannot establish compatibility: preserve or verify the exact binary/profile identity and matching guidance.
- Direct `run-plan-graph` without `--task-worker` launches `pi --print --no-skills --no-extensions`: it needs an installed/configured Pi and uses its unpinned default model. Supply an explicit worker command with a pinned model, or obtain explicit owner acceptance of that changing default. Direct graph execution defaults to four concurrent ordinary tasks in one shared working directory. It supplies no worktree isolation, file locking or conflict detection. Use `--max-active 1` unless task write ownership is demonstrably disjoint; setup-generated implementation bindings already use that serial limit. The summarizer uses the same checkout after tasks finish.
- Profiles configure this provider's defined workflow. Custom topology requires another provider; the engine has no general workflow-expression language.
- Run history is not an exhaustive execution or compliance-audit trace. Worker output lives in separate captures; retain those alongside durable state. Inputs and context are assumed to be reasonably sized; high-volume operation is not guaranteed.
- Author identities, declared revisions/config versions and semantic applicability are trusted claims. False author claims or material edits without a revision bump can evade those checks. The provider does not authenticate reviewers.
- Artifact reads are not locked or atomic with engine transition commits. The workflow provides no adversarial or transactional assurance over external files.
- Checkpointing requires a valid Git HEAD and no unmerged index entries. Repository paths must be UTF-8; supported entries are regular files, symlinks and submodules. On supported Unix platforms, symlink targets are hashed as raw bytes. Read failures or unsupported entries prevent a valid checkpoint.
- Failed workers can leave partial edits or missing/replaced proof. Graph and ad-hoc repair delete the implementation report/checkpoint before workers start; `run-validation` replaces the validation report before execution. Use non-mutating `invoke --preview` or `prepare-validation` before intentional regeneration; see the recovery entries below. Neither successful execution nor a retry establishes semantic approval.
- Reconciliation is provider-owned state, not a commit service: owner acceptance, document application, Git commit, and current-run traceability remain separate driver actions. A justified no-change result is valid, but an unresolved or unauthorized durable gap blocks completion.
- New v11 graphs are not applied to v10 profiles or stored runs. Replacing a binary or profile does not migrate their topology, runtime, evidence, or obligations.
- Stable digests and synthetic journeys establish identity/mechanics. They do not establish semantic truth. Calibration fixtures use supplied fictional companions and must not be resolved against a live checkout.

## Troubleshooting

- **`data-dump` refuses to overwrite:** choose a fresh empty dump root. Retain matching dumped data needed by active runs; do not overwrite it with a different release.
- **`setup --output PATH` replaces a file:** this helper uses atomic replacement. Use a new run-specific path, or inspect/back up the existing destination before deliberately replacing it.
- **`checkpoint mismatch` after checkpoint creation succeeded:** checked evaluation uses the provider process's CWD. The checkpoint command's `--working-directory` does not configure later evaluation. Request checked events from the intended checkout and keep HEAD, index/status and tracked/non-ignored untracked bytes stable. Follow the skill's [owning-phase recovery](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide) to regenerate affected reports/checkpoints and review evidence.
- **Implementation report/checkpoint missing after failure:** graph and ad-hoc repair remove stale `implementation-report.json` and `implementation-checkpoint.json` before launching workers. Preserve captures and partial source edits; deliberately regenerate proof through the skill's [implementation correction route](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide). Use `invoke --preview` when only preparing, not executing, a retry.
- **Validation report incomplete or checkpoint stale after failure:** `run-validation` replaces `validation-report.json` before execution. Inspect retained failures and follow the [validation correction/checkpoint procedure](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide); do not reuse the old checkpoint as proof of the replacement report. [prepare-validation](skills/using-software-change-provider/SKILL.md#prepare-validation-from-retained-commands) prepares from retained captures without executing or checkpointing.
- **Retry or departure blocked by overrun/cleanup-pending work:** allowed time elapsed while owned work remains live, or cancellation has not finished verified cleanup. Follow [execution recovery](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide): wait or cancel only recorded, verified owned work, verify cleanup, then re-read full `show` before retrying. An observer deadline or vanished waiter is not cleanup proof.
- **Worker output is rejected despite exit 0:** inspect selected/raw captures and the generated output schema, including `review_stage` and validation-specific rows. Keep failed attempts and follow the [review procedure](skills/using-software-change-provider/SKILL.md#per-gate-loop).
- **`reconciliation-ready` is denied:** inspect `reconciliation.json` against [data/reconciliation-schema.json](data/reconciliation-schema.json). Keep branch, authorization, application, commit, traceability, proof, and blocker fields consistent; do not use the state to silently rewrite requirements or certify the pre-reconciliation tree.
- **A target-specific document workflow is still in `prepare`:** a successful `show` or fixture journey is not completion. The driver must perform deterministic review, digest-bound semantic evidence for every configured axis, checked `passed` transitions, and a full-envelope assertion before treating the document as integrated.
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

Templates: [intent](data/templates/intent.md), [design](data/templates/design.md), [task packet](data/templates/task-packet.md), [implementation report](data/templates/implementation-report.md), [validation report](data/templates/validation-report.md), [finding ledger](data/templates/finding-ledger.json) and [advisory finding proposal](data/templates/advisory-finding-proposal.json). The reconciliation result uses [data/reconciliation-schema.json](data/reconciliation-schema.json). Advisory proposals require driver disposition before authoritative recording.

### Review and calibration

The [worker schema](data/review-worker-output-schema.json) declares `{review_stage,author,judgments}`; aggregate ordinary validation uses the generated `validation_verdicts` extension. The [reviewer protocol](data/reviewer-protocol.md) defines evidence and adjudication rules. The [calibration procedure](data/calibration/PROCEDURE.md) defines supplied-material framing, fresh review and attestation; the [manifest](data/calibration/manifest.json) records actual row status. Changed supplied bytes require fresh review. No shipped harness invokes reviewers or rewrites attestations. The coordinating driver owns the complete final stable-tree proof matrix; workers run assigned focused checks and reviewers consume retained outcomes rather than rerunning the matrix.
