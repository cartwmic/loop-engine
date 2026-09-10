# Software-change provider

## Overview

`software-change` is Loop Engine's reference provider for the software-change workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the live workflow implied by optional `initial_input.review_policies` (the sixteen-state union when that key is omitted) and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence, stable selected-output origins, evidence-applicability declarations, and driver-authored finding-ledger snapshots.

The frozen requirement record this crate's acceptance suite traces to (R1–R29, A1–A16, including amendments) lives at [`docs/prd.md`](docs/prd.md).
- Semantic review and finding disposition are external and driver-owned respectively. The provider does not generate prompts, invoke a model, or decide whether review findings are true.
- Intent acceptance is a closed criterion spine: each current record is `{id, statement}` with a stable run-local `AC-N` ID. Design, plan and implementation references remain optional; contract v2 final validation requires a complete independent criterion/goal index and real named command evidence at one current checkpoint.
- Reviewer convergence contract lives in [`data/reviewer-protocol.md`](data/reviewer-protocol.md): binary evidence stays unchanged; candidate output is triaged before append or mutation; material in-scope findings require consequence proof, focused external reconsideration handles disputed candidates, and no waiver is granted by round count.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

`software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS` writes the matching closed `implementation-checkpoint.json` or `validation-checkpoint.json`. It hashes exact report bytes plus Git HEAD, index/status command bytes, and every tracked or non-ignored untracked repository entry; it never stages, commits, branches, pushes, or creates worktrees. Checked implementation and validation hops recompute this evidence. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, whether or not implementation review is configured. Validation requires the sole history entry for the current report revision to match the current report, document revisions, and repository state. Later context, regenerated mutable checkpoints, or overwritten history bytes cannot move that accepted-state anchor.

Checkpoint submodule entries hash their HEAD, not exact dirty submodule working-tree contents. Different submodule edits can retain the same parent dirty status and therefore need not invalidate checkpoint identity. Users depending on submodule content need separate content proof or committed, clean submodules.

Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md).

## Setup

This README describes the checkout containing it. For an installed release, use its version-matched documentation and keep engine/provider binaries paired with their embedded profiles. A GitHub-source installation includes committed changes only.

Standalone releases support macOS arm64 and Linux x86_64. Provider archives contain the `software-change` executable and project license texts; verify the archive checksum before installation. They do not ship Dagu. Plan-graph execution requires operator-installed `dagu` >= 2.14.0 on PATH and one existing absolute Git working directory supplied with `--working-directory`. Dagu is used as a GPLv3 subprocess, not an embedded library; the provider does not manage worktrees. The [work-slot procedure](skills/using-software-change-provider/SKILL.md#work-slot-policy-confirm-before-start) owns prerequisites, capture locations and progress inspection.

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"
software-change data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/software-change-provider/data/...` with the configs, templates, reviewer protocol, review-worker preamble/output contract, calibration manifest, and fixtures embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy a selected profile from that tree to a run-specific file. The engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it. Register the installed provider under an exact, case-sensitive alias:

```toml
[providers.software-change]
command = "/absolute/path/to/installed/software-change"
args = []
```

Keep machine-specific `providers.toml` outside committed repository files and pass its path with Loop Engine's `--config` option. No provider registration file is committed by this crate.

For repository development, build from the checkout instead:

```sh
# Run from the repository root.
cargo build -p software-change-provider
DATA_ROOT="$PWD"
```

This produces `target/debug/software-change`; the checkout's `crates/software-change-provider/data/` tree supplies the development copies of shipped data.

## Usage

Build the engine binary too, or replace `target/debug/loop-engine` below with an installed `loop-engine` executable:

```sh
cargo build -p loop-cli -p software-change-provider
ENGINE=target/debug/loop-engine
PROVIDER=target/debug/software-change
DATA_ROOT="$PWD"
# Register command = "/absolute/checkout/target/debug/software-change" for this build.
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
```

Choose `minimal`, `standard`, or `high-rigor` from the matching `$DATA_ROOT` tree. Before start, follow [Setup](skills/using-software-change-provider/SKILL.md#setup) for profile copying, optional binding construction, confirmation and hash-guarded start, including canonical engine Deterministic setup. Do not replace the confirmed profile with a pristine copy afterward. Shipped profiles omit `work_slot_bindings`; bound workers are opt-in. Use the normal user catalog: no database or artifact override unless the human explicitly requests isolation in this session; other runs and old preferences confer no permission.

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. Once the run exists, `show --view full` reveals the allocated (or caller) `artifact_root` inside object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Subject files use the fixed filenames expected by the selected schema: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, and `validation-report.json`.

`intent.json.operating_context` freezes the sole-operator boundary, accepted residual risks and outside obligations. Reviewer output is candidate evidence; only driver-authored finding dispositions control gate agreement. Fresh bound evidence uses invocation/assignment origins; reuse is a separate applicability declaration, not a rewritten judgment. Reports alone do not establish completion, and validation proof must match accepted implementation identity.

For phase work, evidence recording, checkpoint recovery and selected/no-task repair, use the [provider driving skill](skills/using-software-change-provider/SKILL.md). A failed worker may leave partial checkout edits; process success or a retry is not semantic approval. Bound schema correction permits one same-worker correction attempt with raw attempts preserved.

For a completed bound review, use the exact read-only pipe:

```sh
"$ENGINE" --json show --view full "$RUN_ID" | "$PROVIDER" review-candidates
```

The provider consumes that completed full `show` envelope on stdin and writes one closed JSON document in durable invocation/assignment order. Worker output is a batch `{author,judgments:[...]}`; projection expands it to per-axis candidate rows `{axis,author,result,findings,...}`. These projected rows are current, not legacy worker output. `ready` means only that the engine-selected bytes were readable, digest-matching, and mechanically conformed to the frozen review contract; it exposes the stable `selected-assignment-output` origin plus normalized `axis`, `author`, `result`, and `findings`. `malformed`, `unavailable`, `missing-selection`, and `exhausted` are mechanical diagnostics and never fabricate judgment fields. The command does not open the catalog, retry a worker, deduplicate distinct durable invocations, rewrite raw attempts, append context, or satisfy a gate. Candidate output itself is not evidence or semantic review; the [skill](skills/using-software-change-provider/SKILL.md) owns triage and recording. Distinct completed invocations remain distinct candidates; the command does not retry or collapse them.

## Recovery and contract v2

These additions describe candidate source, not upgrades to frozen released v0.19.0 runs. Default compact bound delivery preserves meaningful context and truthful stdin; independently retained full routed inputs serve verification. It does not trim core/provider history or change ad-hoc bytes. The [skill](skills/using-software-change-provider/SKILL.md) owns view, capture and compatibility procedure.

`prepare-validation` is inert preparation of command-evidence candidates, an index draft and independent criterion/goal commissions from applicable retained execution. It does not execute proof, append, checkpoint, judge or progress. Explicit `validation-command` context records add distinct command IDs without replacing frozen required IDs; `proof_updates` only corrects execution of existing IDs. Follow the [validation procedure](skills/using-software-change-provider/SKILL.md#contract-v2-simplicity-and-final-criterion-proof) for packet forms, receipt/checkpoint identity distinctions and finalization before review. The worker entry point `validation-command WORKING_DIRECTORY SPEC_JSON TIMEOUT_MS` executes the exact command spec through shared capture; its receipt is execution evidence, not a verdict.

Repository checkpoint creation is not Git authorization; workers do not commit. Post-commit delivery does not rewrite terminal evidence. The calibration manifest records actual row status; calibration approval does not replace final audit, document reviews or stable-tree proof. Source integration alone is not committed requirement authority or semantic completion; exact human acceptance, authorized commit and required evidence remain separate duties.

Current-source profiles are `minimal-9`, `standard-9`, `high-rigor-9` with `contract_version: 2` and `criterion_policy: {required_authors:1,goal_required_authors:1}` independent of unchanged axis counts. Shipped bindings remain absent. Older semantic profiles explicitly refuse under the new provider; retain their fixed original provider for execution. Bare describe is discovery. Engine show/history preserves original input, evidence and stored graph without promising new ownership/control capabilities or injecting routes. No active bootstrap migration is included.

Normal disposition is driver-owned and exact-source: rejection/resolution is not a rewritten pass, and accepted-unresolved findings still block across revisions. Review batches use `{author,judgments}` with each assigned axis exactly once; ordinary and challenge gates remain separate. Mixed verdicts are conforming output, not approval. Commission allocation, correction, unaffected carry and disposition procedure belong to the [contract-v2 skill](skills/using-software-change-provider/SKILL.md#contract-v2-simplicity-and-final-criterion-proof) and [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop).

`software-change commission --slot SLOT [--task TASK]` consumes `show --view full` on stdin. User-steering targets all, slots or current-plan tasks; an already-launched attempt retains its selection and task-only instructions do not spread to dependants/summarizer. Unbound incorporation is testimony, not proof of obedience. The [skill](skills/using-software-change-provider/SKILL.md) owns steering selection and binding procedure.

Future execution corrections never rewrite frozen policy or old attempts. Live or cleanup-pending work blocks overlapping retry/departure; historical missing ownership is unsupported. Earlier-phase revise routes do not roll back repositories. Exceptional owner override preserves failures and permanently labels completion `completed-with-overrides`, never reviewer pass, provider allow or Bookends GREEN. Load the required engine companion through the provider skill for control and recovery procedures.

`software-change run-validation --engine ABS --working-directory ABS --revision REV [--commands ID,...] [--timeout-ms N]` consumes `show --view full` on stdin. It executes named plan commands and emits inert command candidates plus [the index](data/templates/validation-report.md), not an append or progression. Final validation requires every current AC-N and separate `goal_verdict_ids`, independent judgments, real command evidence and unchanged accepted checkpoint identity. Selection does not waive other final obligations; stale, duplicate, unknown, self-authored, unsupported or unresolved failed evidence blocks. The [contract-v2 validation procedure](skills/using-software-change-provider/SKILL.md#contract-v2-simplicity-and-final-criterion-proof) owns finalization, repair revalidation and unaffected carry.

Workers run focused assigned validation; one driver/proof owner runs the full stable-tree matrix, repeating only invalidated checks. Reviewers consume retained outputs. `--jobs 2` is the public journey's default independent isolated proof budget, with serial `--jobs 1`. Final comparable benchmark acceptance, hosted exact-commit proof and later real dogfood remain separate from synthetic mechanics. Simple-first/YAGNI/KISS apply product-wide; complexity needs meaningful current need and an inadequate simpler alternative, not speculative hardening or preservation of incidental design. The PRD recovery amendment is pending exact owner acceptance/commit and document audits.

## Criterion spine and Bookends overlay

The three shipped profiles use the same run-local AC-N criterion spine. `intent.json.acceptance` contains closed `{id, statement}` records with IDs matching `^AC-[1-9][0-9]*$`. Optional design/plan/implementation references name current intent IDs. Final validation requires a complete criterion/goal index; reviewers judge semantic coverage, not token presence.

Bookends is off in shipped profiles. The optional overlay adds one `prd_traceability` disposition per criterion: `linked-live`, `candidate`, or `not-applicable`. A current candidate blocks final completion; `not-applicable` concerns traceability only and never waives or fulfills a criterion. Overlay-off keeps AC-N without optional PRD metadata. Overlay-on adds `ids-grounded` and `bypass-not-green` review axes and requires final `GREEN`; repository `BYPASS` is never validation green. Profile configuration, traceability authoring, test-boundary citations and evidence recording belong to the [criterion spine and Bookends overlay procedure](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay).

## Validation

The repository-owned journey runner has one contract with two adapters. For focused recovery, append `--scenario NAME` to the complete source command below; all normal required options, including `--traversal-depth full`, still apply. Named scenarios are dispositions, steering, execution-controls, cancellation, backtracking, override, batched-review, criteria and composed-recovery; these synthetic scenarios establish mechanics, not semantic review quality:

Full-source validation requires all five sibling binaries in `target/debug`, not just the two binaries used for normal provider setup. From the repository root:

```sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full \
  --jobs 2
```

Full source mode drives real engine/provider processes through workflow progression, execution, evidence, recovery and terminal outcomes. Its isolated fixture catalogs are test machinery, not a production-start recommendation. The constructor/interface self-test and complete repository gate are documented in [AGENTS.md](AGENTS.md#workflow). Synthetic journey evidence proves deterministic mechanics, not semantic review quality.

Packaged smoke accepts extracted `loop-engine` and `software-change` paths, calls `software-change data-dump` into an empty root, and uses only the dumped high-rigor profile and fixtures:

```sh
python3 scripts/software-change-journey.py \
  --mode packaged \
  --engine /path/to/extracted/loop-engine \
  --provider /path/to/extracted/software-change \
  --data-root "${TMPDIR:-/tmp}/software-change-dump" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-packaged-journey" \
  --profile high-rigor.json \
  --traversal-depth checked-prefix
```

The packaged adapter does not read checkout provider data after `data-dump`. Native archive smoke covers macOS arm64 and Linux x86_64 in the cargo-dist workflow. Pull requests may plan/build; publication is controlled by explicit workflow dispatch and its publishing conditions.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. Config profiles are complete initial-input templates; copy one for a run.

### Config profiles

- [`crates/software-change-provider/data/configs/minimal.json`](data/configs/minimal.json)
- [`crates/software-change-provider/data/configs/standard.json`](data/configs/standard.json)
- [`crates/software-change-provider/data/configs/high-rigor.json`](data/configs/high-rigor.json)

The profiles configure parent review gates `intent-review`, `design-review`, `plan-review`, `implementation-review`, and `validation-review`. Standard and high-rigor also ship one counterpart axis per parent axis on the matching `*-adversarial-review` gate. `minimal` keeps only `validation-review`/`intent-delivered` and ships no adversarial lists; `standard` supplies the standard intent-review, design-review, and validation-review axes plus 1:1 counterparts; `high-rigor` supplies all shipped parent axes (two distinct reviewers for design-review and validation-review) plus 1:1 counterparts (`required_authors` 1).

Shipped profiles omit `work_slot_bindings`; work remains driver-performed unless the caller opts into a binding. The [binding constructor](skills/using-software-change-provider/SKILL.md#deterministic-review-binding-constructor) owns roster allocation, Pi arguments, model confirmation and profile freezing. Copying a profile alone does not authorize execution or choose a model.

### Artifact templates

Configurable `artifact_schemas` use a bounded schema language, not general JSON Schema: object, array and string declarations with restricted keywords, and only designated built-in ID patterns at supported schema locations. General numeric/boolean type schemas, `$ref`, composition and arbitrary regex are unsupported. See [the schema implementation](src/schema.rs) and [shipped profiles](#config-profiles) for exact supported forms. This is separate from the worker `full_output_schema` contract.

- [`crates/software-change-provider/data/templates/intent.md`](data/templates/intent.md)
- [`crates/software-change-provider/data/templates/design.md`](data/templates/design.md)
- [`crates/software-change-provider/data/templates/task-packet.md`](data/templates/task-packet.md)
- [`crates/software-change-provider/data/templates/implementation-report.md`](data/templates/implementation-report.md)
- [`crates/software-change-provider/data/templates/validation-report.md`](data/templates/validation-report.md)
- [`crates/software-change-provider/data/templates/finding-ledger.json`](data/templates/finding-ledger.json)
- [`crates/software-change-provider/data/templates/advisory-finding-proposal.json`](data/templates/advisory-finding-proposal.json)

`advisory-finding-proposal` is a separate, inert context kind. Its candidate source IDs and proposed disposition, reason, owner phase, task IDs, review axes, and rationale are suggestions; the driver accepts, edits, or rejects them before appending any authoritative `finding-ledger` snapshot.

### Review and calibration

- [`crates/software-change-provider/data/review-worker-preamble.txt`](data/review-worker-preamble.txt) defines the read-only review-worker role and driver boundary.
- [`crates/software-change-provider/data/review-worker-output-schema.json`](data/review-worker-output-schema.json) declares the complete worker batch contract `{author,judgments}`; projected per-axis candidates are separate from this worker output.
- [`crates/software-change-provider/data/reviewer-protocol.md`](data/reviewer-protocol.md) defines the `review-evidence` record and external adjudication rules.
- [`crates/software-change-provider/data/calibration/PROCEDURE.md`](data/calibration/PROCEDURE.md) defines owner-attested calibration.
- [`crates/software-change-provider/data/calibration/manifest.json`](data/calibration/manifest.json) records calibration rows.
Calibration fixtures:

- [`crates/software-change-provider/data/calibration/fixtures/intent-good.json`](data/calibration/fixtures/intent-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/intent-defective.json`](data/calibration/fixtures/intent-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/design-good.json`](data/calibration/fixtures/design-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/design-defective.json`](data/calibration/fixtures/design-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/plan-good.json`](data/calibration/fixtures/plan-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/plan-defective.json`](data/calibration/fixtures/plan-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/implementation-report-good.json`](data/calibration/fixtures/implementation-report-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/implementation-report-defective.json`](data/calibration/fixtures/implementation-report-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/validation-report-good.json`](data/calibration/fixtures/validation-report-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/validation-report-defective.json`](data/calibration/fixtures/validation-report-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/example-evidence.json`](data/calibration/fixtures/example-evidence.json)

## Convergence and owning-phase routes

Convergence does not waive known material defects by round count or prior reviewer overlook. Corrections belong to the actual owning phase, not an automatic deep restart; zero advisory comments is not required. The skill owns the [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) and [proportional late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide), with the [reviewer protocol](data/reviewer-protocol.md) defining materiality and reconsideration.

## Read obligations and continue

`show` is the durable handoff for frozen obligations; editing a source profile does not change an existing run. Checked transitions apply schemas and revision links before review/ledger aggregation; check-free revise edges do not evaluate artifacts. Missing/unparseable artifacts deny, while invalid or inaccessible artifact roots produce evaluation errors. Ledger and evidence denials name mechanical blockers, not semantic judgments.

The [skill](skills/using-software-change-provider/SKILL.md) owns observation and continuation. Public record contracts remain in [reviewer protocol](data/reviewer-protocol.md) and the shipped templates. Ledger sources are immutable `context-record` references; proposal records remain inert. Commission filters select eligible context, and graph task stdin stays `{artifact_root, task}`. Repository `docs/agent-usage.md` owns common CLI forms and JSON outcomes.

## Candidate identity and supplied calibration data

The driver owns semantic applicability; core resolves same-run opaque context identity and the provider checks current software-change identities. Follow the [skill](skills/using-software-change-provider/SKILL.md) for full change-report inspection and evidence reuse.

Public help/version flags work without stdin. Help lists provider protocol operations and public data, checkpoint, candidate, commission, validation-preparation and plan-graph helpers; internal worker plumbing is not a user interface. `checkpoint` accepts optional `--json`. The public plan-graph form is:

```text
software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--task ID ... | --tasks ID,ID,...] [--max-active N]
```

Selection, bound invocation input, repair, task/summarizer duties and captured evidence are specified in the [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop). The [Setup](#setup) section states release pairing and Dagu/working-directory prerequisites. No-argument stdin protocol and `data-dump DIR` remain separate interfaces.

`data-dump` includes calibration instructions and fictional companions. Fixture labels refer to supplied companion bytes, never a live checkout. Exact input digests identify supplied material mechanically, not semantic truth; changing that material requires fresh owner review before updating attestations. The [calibration procedure](data/calibration/PROCEDURE.md) owns record framing and review steps. No shipped harness invokes reviewers or rewrites attestations.

Evidence denial details separate current blockers (`details.diagnostics`) from stale or stale-config recovery context (`details.informational`). A claim about a bound worker includes only the concise `origin` with its invocation and assignment. Core resolves `loop_engine_origin` from the durable invocation record; provider evaluation then refuses the linked claim as `unverified` when selected stdout is unavailable, digest-mismatched, or its mechanical `axis`/`author`/`result`/`findings` fields disagree. Genuinely external evidence omits that worker origin. Finding-ledger sources are immutable context-record references, and applicability preserves the original judgment while recording the separate attestation. Prior denials and inert records remain separate fields. Stale evidence never satisfies current obligations. Historical completed-run records remain inspectable but are not a new provider path.
