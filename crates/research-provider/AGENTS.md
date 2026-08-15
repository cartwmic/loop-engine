# Agent instructions for research-provider

## Scope

This file covers work in this crate: the `research` binary, shipped configs/templates/protocol under `data/`, crate tests, and `skills/using-research-provider/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence` at verify and synthesize. It does not generate prompts, invoke a model, fetch the web, edit artifacts, or decide whether findings are true.

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

Register `target/debug/research` under exact alias `research` with an absolute command path in uncommitted provider TOML. Copy the standard profile; omit `artifact_root` in the usual case; the engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it; pass a nonempty `artifact_root` only to isolate files to a caller-chosen absolute existing directory; then `start`. Artifact filenames are fixed: `brief.json`, `sources.json`, `verification.json`, `report.json`.

Topology: `scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise*` edges. Checked `scoped` and `gathered` schema-check the current subject and revision links. Checked `verified` and `completed` then aggregate evidence. Check-free `revise` does not evaluate.

At review states, follow `data/reviewer-protocol.md`: comprehensive first review, triage candidates before append or mutation, append only accepted in-scope material failures or conforming passes, confirmation review after fixes. No waivers. Verification-local `verification.json` corrections stay in verify (edit the artifact, retry `verified`). Report-local `report.json` corrections stay in synthesize (edit the artifact, retry `completed`). From synthesize, nearest `revise` is verification-owned only; use `revise-sources` or `revise-brief` for earlier owners.

The subject's declared author never counts toward its own review. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use parent-directory segments in those links. Refer to repository-root files such as docs/agent-usage.md in prose.

`research --help`/`-h` names `describe`, `evaluate`, and `data-dump`. `--version`/`-V` prints the Cargo package version. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files.

## Completion and Handoff

Crate work is complete when `cargo test -p research-provider` and `cargo clippy -p research-provider --all-targets -- -D warnings` pass, shipped configs/templates/protocol still match runtime behavior, and this crate's README/AGENTS.md remain accurate.

Handoff the files changed, commands run, and residuals: unread locked artifacts, synthetic test evidence is not semantic quality, and round state lives outside the provider.
