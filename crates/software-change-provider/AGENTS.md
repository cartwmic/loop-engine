# Agent instructions for software-change-provider

## Scope

This file covers work in this crate: the `software-change` binary, shipped configs/templates/protocol/calibration under `data/`, crate tests, `docs/prd.md`, and `skills/using-software-change-provider/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence`. It does not generate prompts, invoke a model, edit artifacts, or decide whether findings are true.

## Authority

Frozen requirements this crate's acceptance suite traces to (R1–R27, A1–A15, including amendments) live in [docs/prd.md](docs/prd.md). Drive a run with [README.md](README.md) and [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md). Evidence shape and adjudication rules are [data/reviewer-protocol.md](data/reviewer-protocol.md). Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and CLI envelopes.

Per-run obligations are frozen in immutable `initial_input` (`review_policies`, `artifact_schemas`, `config_version`, `artifact_root`). `show` is the durable handoff; changing a source profile does not change an existing run. No policy, schema, prompt, or artifact shape is baked into provider code — those arrive in config data.

Shipped profile versions currently in-tree: `minimal-3`, `standard-4`, `high-rigor-4`. Evidence `config_version` must match the run's frozen value, not whatever file is currently shipped.

## Workflow

```sh
cargo test -p software-change-provider
cargo fmt --all -- --check
```

Crate tests are not a substitute for the public-boundary journey. After any crate change, also run the repository source journey from the repo root (build `loop-engine` and `software-change` first):

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

Register `target/debug/software-change` under exact alias `software-change` with an absolute command path in uncommitted provider TOML. Copy a profile; omit `artifact_root` in the usual case; the engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it; pass a nonempty `artifact_root` only to isolate files to a caller-chosen absolute existing directory; then `start`. Artifact filenames are fixed: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, `validation-report.json`.

Topology: `explore → design → design-review → plan → plan-review → implement → implementation-review → validation → end`, plus check-free owning-phase `revise*` edges. Checked `*-ready` and approval/`passed` transitions schema-check the current subject before evidence aggregation. Check-free `revise` does not evaluate.

At review states, follow `data/reviewer-protocol.md`: comprehensive first review, triage candidates before append or mutation, append only accepted in-scope material failures or conforming passes, confirmation review after fixes. No waivers. Validation-report-local defects stay in validation (edit the report, retry `passed`). From validation, nearest `revise` is implementation-owned only; use `revise-plan`, `revise-design`, or `revise-intent` for earlier owners.

The subject's declared author never counts toward its own review. High-rigor design-review and validation axes require two distinct reviewers. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use `..` in those links. Refer to repository-root files such as `docs/agent-usage.md` in prose.

`software-change --help`/`-h` names `describe`, `evaluate`, and `data-dump`. `--version`/`-V` prints the Cargo package version. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files.

Calibration: `data/calibration/PROCEDURE.md` and `manifest.json`. Fixtures use `fictional-repo/` labels; reviewers receive mapped companion bytes and must not resolve those labels against a live checkout. Digest identity is mechanical, not semantic review proof. No shipped harness invokes reviewers or rewrites attestations.

## Completion and Handoff

Crate work is complete when crate tests and the source software-change journey pass (and calibration procedure, when that procedure applies), shipped configs/templates/protocol still match runtime behavior, and this crate's README/AGENTS.md remain accurate. Doc integration for a software-change run belongs in the repository's authoritative documents, not only in change-scoped artifacts.

Handoff the files changed, commands run, run ID and database path if a software-change run was used, coverage/revision identities, and residuals: unread locked artifacts, synthetic journey evidence is not semantic quality, and round state lives outside the provider.
