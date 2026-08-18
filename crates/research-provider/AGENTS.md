# Agent instructions for research-provider

## Scope

This file covers work in this crate: the `research` binary, shipped configs/templates/protocol under `data/`, crate tests, and `skills/using-research-provider/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence` at verify and synthesize. It does not generate prompts, invoke a model, fetch the web, edit artifacts, or decide whether findings are true.

Providers author worker-facing role and output content; the engine only transports and mechanically enforces it.
Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression.
Exit 0 does not establish a valid deliverable.

## Authority

Drive a run with [README.md](README.md) and [skills/using-research-provider/SKILL.md](skills/using-research-provider/SKILL.md). Evidence shape and adjudication rules are [data/reviewer-protocol.md](data/reviewer-protocol.md). Repository-root AGENTS.md and docs/agent-usage.md govern checkout-wide operation and CLI envelopes.

Per-run obligations are frozen in immutable `initial_input` (`review_policies`, `artifact_schemas`, `config_version`, `artifact_root`). `show` is the durable handoff; changing a source profile does not change an existing run. No policy, schema, prompt, or artifact shape is baked into provider code — those arrive in config data.

Shipped profile version currently in-tree: `research-1`. Evidence `config_version` must match the run's frozen value, not whatever file is currently shipped. Verify axes are `claim-grounded` and `adversarial`; synthesize axes are `cited-conclusion` and `scope-faithful`.

## Workflow

```sh
cargo test -p research-provider
cargo clippy -p research-provider --all-targets -- -D warnings
cargo fmt --all -- --check
```

Crate tests are not a substitute for the public-boundary journey. After any crate change, also run the repository source journey from the repo root (build `loop-engine` and `research` first):

```sh
python3 scripts/research-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/research \
  --profile crates/research-provider/data/configs/standard.json
```

That journey command is a harness example, distinct from the production start; do not copy isolation flags from it into production start.

Register `target/debug/research` under exact alias `research` with an absolute command path in uncommitted provider TOML. Copy the standard profile. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. The engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it; then `start`. Artifact filenames are fixed: `brief.json`, `sources.json`, `verification.json`, `report.json`.

Topology: `scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise*` edges. Checked `scoped` and `gathered` schema-check the current subject and revision links. Checked `verified` and `completed` then aggregate evidence. Check-free `revise` does not evaluate.

At review states, follow `data/reviewer-protocol.md`: comprehensive first review, triage candidates before append or mutation, append only accepted in-scope material failures or conforming passes, confirmation review after fixes. No waivers. Verification-local `verification.json` corrections stay in verify (edit the artifact, retry `verified`). Report-local `report.json` corrections stay in synthesize (edit the artifact, retry `completed`). From synthesize, nearest `revise` is verification-owned only; use `revise-sources` or `revise-brief` for earlier owners.

The subject's declared author never counts toward its own review. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use parent-directory segments in those links. Refer to repository-root files such as docs/agent-usage.md in prose.

`research --help`/`-h` names `describe`, `evaluate`, and `data-dump`. `--version`/`-V` prints the Cargo package version. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files.

## Completion and Handoff

Crate work is complete when `cargo test -p research-provider`, `cargo clippy -p research-provider --all-targets -- -D warnings`, and the source research journey pass, shipped configs/templates/protocol still match runtime behavior, and this crate's README/AGENTS.md remain accurate.

Handoff the files changed, commands run, and residuals: unread locked artifacts, synthetic test evidence is not semantic quality, and round state lives outside the provider.
