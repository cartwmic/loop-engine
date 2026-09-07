# Agent instructions for software-change-provider

## Scope

This file covers work in this crate: the `software-change` binary, shipped configs/templates/protocol/calibration under `data/`, crate tests, `docs/prd.md`, and `skills/using-software-change-provider/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence` plus driver-authored `finding-ledger` and repository-checkpoint evidence. It does not generate prompts, invoke a model, edit artifacts, or decide whether findings are true.

Providers author worker-facing role and output content; the engine only transports and mechanically enforces it.
Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression.
Exit 0 does not establish a valid deliverable. Before accepting delivery, triage the captures and follow the [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) to append evidence and request the shown event.

## Authority

Frozen requirements this crate's acceptance suite traces to (R1–R29, A1–A16, including amendments) live in [docs/prd.md](docs/prd.md). A repository's Bookends IDs belong only to its configured living PRD; this crate must not mint them. Drive a run with [README.md](README.md) and [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md). Evidence shape and adjudication rules are [data/reviewer-protocol.md](data/reviewer-protocol.md). Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and CLI envelopes.

Per-run obligations are frozen in immutable `initial_input` (`review_policies`, `artifact_schemas`, `config_version`, `artifact_root`). `show` is the durable handoff; changing a source profile does not change an existing run. No policy, schema, prompt, or artifact shape is baked into provider code — those arrive in config data.

Operationally, before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative. This summary remains subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority. Apply LE-107's observed-ordinary-failure/smaller-mechanism burden; retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records. Preserve R16's independent-author aggregation and visible verdict history. R21's retained review, materiality, triage and source-visibility rules remain; normal disposition is exact-source driver judgment, and exceptional owner override is distinct engine history, never reviewer success; delivered stable references use context-record or invocation/assignment identities, with one explicit evidence-applicability declaration and no driver-copied mechanical coordinates.

Shipped profiles are `minimal-9`, `standard-9`, `high-rigor-9`, with `contract_version: 2` and independent `criterion_policy: {required_authors:1,goal_required_authors:1}`. Earlier semantic profiles require their fixed original providers. Evidence `config_version` must match the run's frozen value. Before choosing bindings/models, follow the [exact profile preflight](skills/using-software-change-provider/SKILL.md#exact-profile-and-external-fleet-preflight); shipped profiles omit `work_slot_bindings`, and opt-in review uses the [deterministic per-author-per-gate batch constructor](skills/using-software-change-provider/SKILL.md#deterministic-review-binding-constructor). Gate keys are `intent-review` and `validation-review`; `intent` and `validation` are invalid review-gate keys.

Before authoring AC-N intent or final criterion/goal proof, load the skill's [contract-v2 procedure](skills/using-software-change-provider/SKILL.md#contract-v2-simplicity-and-final-criterion-proof). When Bookends is enabled, use its [overlay procedure](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay): each current criterion has one `prd_traceability` disposition; a current candidate blocks final completion, and not-applicable never waives or fulfills the criterion.

The proposed recovery amendment in [docs/prd.md](docs/prd.md) explicitly updates R8/R13/R16/R20/R21 and related clauses; exact owner acceptance and commit remain pending. Simple-first/YAGNI/KISS apply to the engine and every provider. Added complexity needs a meaningful current reason and inadequate simpler alternative in the ordinary design, not a new justification framework. Existing protocols, schemas and dependencies have no presumption of preservation; no speculative hardening or unrelated provider features.

Disposition counts a current independent pass or an exact-source reasoned rejected/resolved fail; the latter is not a rewritten pass. Accepted-unresolved findings block across revisions. Resolved historical sources need no false applicability; retired-author needs recorded roster departure and replacement coverage. Workers run focused assigned checks; the driver/proof owner runs the stable-tree full matrix and repeats only invalidated checks. Reviewers consume retained command/criterion/goal evidence, never duplicate suites. See the shipped validation template for run-validation, fixed index/checkpoint, prechosen genuine record IDs and affected-versus-carried proof.

Common engine controls are documented in repository `docs/agent-usage.md`: future owner-attested amend-binding, ordinary invoke preview/controls, recorded-ownership cancellation and visibly overridden event. Overrun and cleanup-pending work block retry/departure until completion or verified cleanup. Cancellation is ten seconds per acquisition/resumption, at most three seconds graceful, not a bound across an interrupted controller's unbounded operator delay. Historical missing ownership is unsupported. New implement graphs expose revise-plan/design/intent without a fictitious report; old stored graphs gain no routes and no repository rollback occurs.

## Workflow

```sh
cargo test -p software-change-provider
cargo fmt --all -- --check
```

Crate tests are not a substitute for the public-boundary journey. After any crate change, run the locked build and source journey below from the repository root. `python3 scripts/software-change-journey.py --self-test` must print `worker-data skill/root policy assertions passed`. Source full mode must print `full software-change journey passed` after walking parent and challenge reviews, `stitched software-change journey passed` after a second run on shipped `minimal.json`, `contracted fan-out failure` after the bound nonconforming-worker overlay proof, and `Package 7b review-candidates scenario passed: selected retry, exhausted assignment, raw capture preservation, deterministic repeated inspection, inert-before-records, and driver-action-afterward progression` after the read-only candidate pipe proof:

```sh
cargo build --locked -p loop-cli -p software-change-provider
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full
```

That journey command is a harness example, distinct from the production start; do not copy isolation flags from it into production start.

Register `target/debug/software-change` under exact alias `software-change` with an absolute command path in uncommitted provider TOML. Copy a profile. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. `start` allocates the durable directory and records its absolute path in object `initial_input`; `show` then reveals it. Artifact filenames are fixed: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, `validation-report.json`, `implementation-checkpoint.json`, and `validation-checkpoint.json`; accepted implementation checkpoints are preserved under content-addressed `implementation-proof-history/`.

Before addressing a slot, inspect the `describe` catalog and the skill's [work-slot policy](skills/using-software-change-provider/SKILL.md#work-slot-policy-confirm-before-start). Draft/work-slot IDs are `intent-draft`, `design-draft`, `plan-draft`, `implement`, and `validation-draft`. The live graph follows frozen `review_policies`: omission gives the sixteen-state union, while a present object keeps nonempty review lists. Follow the [gate map](skills/using-software-change-provider/SKILL.md#gate-map) for checked and revise edges; `passed` is only the live last hop into end.

Before commissioning review, triaging candidates or appending verdicts, load [data/reviewer-protocol.md](data/reviewer-protocol.md) and the skill's [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop). For a late correction, use the [proportional late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide) to choose the owning phase and reconfirm affected proof.

The subject's declared author never counts toward its own review. High-rigor design-review and validation axes require two distinct reviewers. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use `..` in those links. Refer to repository-root files such as `docs/agent-usage.md` in prose.

`software-change --help`/`-h` names `describe`, `evaluate`, `data-dump`, `checkpoint`, `review-candidates`, `commission`, `run-validation`, and `run-plan-graph`. `--version`/`-V` prints the Cargo package version. Hidden `stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from help. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files; choose a fresh dump root.

Load the named procedure when crossing these boundaries:

- Before freezing Pi argv, follow [work-slot policy](skills/using-software-change-provider/SKILL.md#work-slot-policy-confirm-before-start) and the [engine driving minimum](skills/using-software-change-provider/SKILL.md#required-companion-and-engine-driving-minimum). Omitted `--task-worker` defaults to Pi with `--no-skills --no-extensions`, without `-e` or `--model`; freeze the confirmed model and required extensions. Hidden `stdin-exec` uses `PI_CODING_AGENT_SESSION_DIR` for session placement; follow that procedure's `--session-dir` and `--mode json` restrictions.
- Before plan-graph or bound repair, follow the [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) and [late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide). `software-change run-plan-graph --working-directory ABS` requires one existing absolute directory and runnable PATH `dagu` >=2.14.0 before spawning workers; use a Git working tree for checkpointing. The skill owns selection, repair, capture and polling mechanics.
- Before `commission` or `run-validation`, use the README's [helper command forms](README.md#recovery-and-contract-v2) and the skill's [contract-v2 procedure](skills/using-software-change-provider/SKILL.md#contract-v2-simplicity-and-final-criterion-proof). Both consume a completed `show` envelope on stdin; neither appends evidence or progresses a gate. `run-validation` executes the named proof commands. After authoring a report/index, follow that procedure to create its checkpoint against existing absolute artifact and working directories before requesting approval.
- `review-candidates` reads one completed ordinary `show` envelope from stdin. Its output is inert and does not satisfy a gate; use the [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) to triage captures and append accepted evidence. Advisory proposals likewise require driver triage into the finding ledger.

Calibration: `data/calibration/PROCEDURE.md` and `manifest.json`. Fixtures use `fictional-repo/` labels; reviewers receive mapped companion bytes and must not resolve those labels against a live checkout. Digest identity is mechanical, not semantic review proof. No shipped harness invokes reviewers or rewrites attestations.

## Completion and Handoff

Crate work is complete when crate tests and the source software-change journey pass (and calibration procedure, when that procedure applies), shipped configs/templates/protocol still match runtime behavior, and this crate's README/AGENTS.md remain accurate. Doc integration for a software-change run belongs in the repository's authoritative documents, not only in change-scoped artifacts.

Handoff the files changed, commands run, run ID and database path if a software-change run was used, coverage/revision identities, and residuals: unread locked artifacts, synthetic journey evidence is not semantic quality, and round state lives outside the provider.
