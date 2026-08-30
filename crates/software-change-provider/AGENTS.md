# Agent instructions for software-change-provider

## Scope

This file covers work in this crate: the `software-change` binary, shipped configs/templates/protocol/calibration under `data/`, crate tests, `docs/prd.md`, and `skills/using-software-change-provider/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence` plus driver-authored `finding-ledger` and repository-checkpoint evidence. It does not generate prompts, invoke a model, edit artifacts, or decide whether findings are true.

Providers author worker-facing role and output content; the engine only transports and mechanically enforces it.
Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression.
Exit 0 does not establish a valid deliverable.

## Authority

Frozen requirements this crate's acceptance suite traces to (R1–R29, A1–A16, including amendments) live in [docs/prd.md](docs/prd.md). A repository's Bookends IDs belong only to its configured living PRD; this crate must not mint them. Drive a run with [README.md](README.md) and [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md). Evidence shape and adjudication rules are [data/reviewer-protocol.md](data/reviewer-protocol.md). Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and CLI envelopes.

Per-run obligations are frozen in immutable `initial_input` (`review_policies`, `artifact_schemas`, `config_version`, `artifact_root`). `show` is the durable handoff; changing a source profile does not change an existing run. No policy, schema, prompt, or artifact shape is baked into provider code — those arrive in config data.

Operationally, before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative. This summary remains subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority. Apply LE-107's observed-ordinary-failure/smaller-mechanism burden; retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records. Preserve R16's independent-author aggregation and visible verdict history. R21's retained review, materiality, triage, source-visibility, and no-waiver rules remain; delivered stable references use context-record or invocation/assignment identities, with one explicit evidence-applicability declaration and no driver-copied mechanical coordinates.

Shipped profile versions currently in-tree: `minimal-6`, `standard-7`, `high-rigor-7`. Evidence `config_version` must match the run's frozen value, not whatever file is currently shipped. Shipped profiles omit `work_slot_bindings`; bound workers are opt-in, and review bindings use the skill's deterministic per-axis constructor. Gate keys are `intent-review` and `validation-review` (not `intent` or `validation`). Standard and high-rigor ship 1:1 challenge counterparts (`required_authors` 1) on every parent review list under unchanged `*-adversarial-review` IDs; minimal ships no challenge lists.

## Workflow

```sh
cargo test -p software-change-provider
cargo fmt --all -- --check
```

Crate tests are not a substitute for the public-boundary journey. After any crate change, also run the repository source journey from the repo root (build `loop-engine` and `software-change` first). `python3 scripts/software-change-journey.py --self-test` must print `worker-data skill/root policy assertions passed`. Source full mode must print `full software-change journey passed` after walking parent and challenge reviews, `stitched software-change journey passed` after a second run on shipped `minimal.json`, `contracted fan-out failure` after the bound nonconforming-worker overlay proof, and `Package 7b review-candidates scenario passed: selected retry, exhausted assignment, raw capture preservation, deterministic repeated inspection, inert-before-records, and driver-action-afterward progression` after the read-only candidate pipe proof:

```sh
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

Register `target/debug/software-change` under exact alias `software-change` with an absolute command path in uncommitted provider TOML. Copy a profile. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. The engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it; then `start`. Artifact filenames are fixed: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, `validation-report.json`, `implementation-checkpoint.json`, and `validation-checkpoint.json`; accepted implementation checkpoints are preserved under content-addressed `implementation-proof-history/`.

Topology: `describe` returns the live graph implied by frozen `review_policies`. Omitted `review_policies` is the sixteen-state union: explore, intent-review, intent-adversarial-review, design, design-review, design-adversarial-review, plan, plan-review, plan-adversarial-review, implement, implementation-review, implementation-adversarial-review, validation, validation-review, validation-adversarial-review, and end. A present object keeps only nonempty review lists and rewires ready/approved/`passed` onto the next live successor; `passed` is only the live last hop into end. Catalog draft ids are `intent-draft` and `validation-draft`. Checked `*-ready` and approval/`passed` transitions schema-check the current subject before evidence aggregation. Check-free `revise` does not evaluate.

At review states, follow `data/reviewer-protocol.md`: comprehensive first review, triage candidates before append or mutation, append only accepted in-scope material failures or conforming passes, confirmation review after fixes. Quiet, progress, and thrash count per review state. No waivers. Validation-report-local defects stay in the validation draft (edit the report, retry the next checked hop `validation-ready` or `passed`). The validation draft also exposes check-free `revise-implementation` for regenerating implementation proof after a repository-state mismatch. From validation-review and validation-adversarial-review, nearest `revise` returns to the validation draft; those states also expose `revise-implementation`. Use `revise-plan`, `revise-design`, or `revise-intent` for earlier owners.

The subject's declared author never counts toward its own review. High-rigor design-review and validation axes require two distinct reviewers. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use `..` in those links. Refer to repository-root files such as `docs/agent-usage.md` in prose.

`software-change --help`/`-h` names `describe`, `evaluate`, `data-dump`, `checkpoint`, `review-candidates`, and `run-plan-graph`. `--version`/`-V` prints the Cargo package version. Hidden `stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help` and `--version`. `review-candidates` reads one completed ordinary `show` envelope from stdin and emits an inert deterministic view of selected bound review output; it does not retry, deduplicate, rewrite captures, append, route, or satisfy a gate. Plan-graph uses `--exit-mode propagate` only. `run-plan-graph` fail-closes if PATH `dagu` is missing, not runnable, or older than 2.14.0, naming the path or PATH miss and required version, before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`plan-graph-<capture-dir-name>`). Full/selected mode emits a Dagu `type:graph` (`max_active_steps` 4, fail-fast, no `continue_on`); a mandatory `summarizer` is the sole writer of `artifact_root/implementation-report.json`. Bound exact `{repair_finding_ids}` mode is only for current accepted unresolved implementation findings with empty `task_ids`: after current ledger/checkpoint preflight it runs one `ad-hoc-repair` worker, no plan task or summarizer, leaves `plan-task-results.json` unchanged, requires a globally fresh schema-valid report revision, and then creates the checkpoint. Task-owned correction uses `{plan_revision,task_roots}`; materially wrong decomposition revises the plan. Per-step progress is `dagu status` / `dagu history` against the isolated home. Provider packages do not ship `dagu`. Review slots carry the current finding-ledger context plus their assigned axis; the implementation slot additionally forwards review-evidence and evidence-applicability context so provider-side stable source references resolve for exact-task `finding_context` or no-task repair selection, while preserving the closed worker packet for each mode. Advisory-finding-proposal records are inert until the driver accepts or edits them into a finding-ledger snapshot. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files.

Calibration: `data/calibration/PROCEDURE.md` and `manifest.json`. Fixtures use `fictional-repo/` labels; reviewers receive mapped companion bytes and must not resolve those labels against a live checkout. Digest identity is mechanical, not semantic review proof. No shipped harness invokes reviewers or rewrites attestations.

## Completion and Handoff

Crate work is complete when crate tests and the source software-change journey pass (and calibration procedure, when that procedure applies), shipped configs/templates/protocol still match runtime behavior, and this crate's README/AGENTS.md remain accurate. Doc integration for a software-change run belongs in the repository's authoritative documents, not only in change-scoped artifacts.

Handoff the files changed, commands run, run ID and database path if a software-change run was used, coverage/revision identities, and residuals: unread locked artifacts, synthetic journey evidence is not semantic quality, and round state lives outside the provider.
