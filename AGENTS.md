# Agent instructions

## Scope

This file instructs agents working in this loop-engine checkout. In scope: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, the `software-change` and `policy-document` reference providers, `tests/fixtures`, `scripts/`, skills, and release-proof workflows.

Out of scope: performing primary workflow work *inside* the engine or a provider; inventing engine policy, review orchestration, or core semantics; and treating this file as a human product overview. Humans start at [README.md](README.md).

Crate-level `AGENTS.md` files add crate-local procedure only. They must not contradict this file.

## Authority

When instructions conflict, use this order:

1. This file, for how to operate in this checkout.
2. [docs/PRD.md](docs/PRD.md), living engine product requirements.
3. `crates/software-change-provider/docs/prd.md`, frozen software-change provider requirements.
4. The relevant crate README and skill, for driving that provider.
5. [docs/agent-usage.md](docs/agent-usage.md), for CLI forms, JSON envelopes, and the `show` / `append` / `event` loop.

The engine owns durable run state and progression. Callers perform primary work externally. Providers `describe` topology and `evaluate` the exact transition the engine selected; they do not choose the next state, edit repositories, invoke reviewers, or judge semantic truth. Context `kind` and `data` are opaque to core; follow the active provider's record conventions.

Shipped skills: [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), `crates/software-change-provider/skills/using-software-change-provider/SKILL.md`, and `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md`.

## Workflow

Build the engine and both reference providers, then run the repository baseline:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI preflight also runs `dist generate --check`, `scripts/assert-dist-plan.py`, `scripts/assert-release-gates.py`, `scripts/assert-push-main-preflight.py`, `scripts/production-journey.py --self-test`, a locked build of the three release packages, the software-change source journey (`--mode source --traversal-depth full` against high-rigor), and both policy-document source journey modes. Reproduce those when the change can affect release proof, journeys, or generated workflow.

Drive production runs with `loop-engine`. Pass `--json` and an explicit `--database` on every invocation; pass `--config` on `start` with uncommitted machine-local provider TOML. Use exact aliases `software-change` and `policy-document` and absolute `command` paths. Do not commit provider TOML, run databases, or secrets.

Parse the single JSON envelope even on nonzero exit. Treat only `status: "completed"` as success. `rejected` (exit 10) is an understood denial — follow feedback and continue. `error` (exit 20) means nothing advanced; re-run `show`. Request events from the latest `show`, never states. Serialize `append`, `event`, and `terminate` per run.

`loop-engine` and `software-change` accept `--help`/`-h` and `--version`/`-V` before stdin. `policy-document` does not: unsupported argv besides `data-dump DIR` is an error; describe/evaluate remain stdin JSON.

When drafting or auditing `README.md` or `AGENTS.md`, use the policy-document provider and a copy of the shipped profile (`readme-1` or `agents-1`). Keep `target.id` and `profile_version` unless intentionally authoring a custom profile. Local markdown links must resolve under the target file's directory; parent-directory segments (`..`) are rejected as escapes, so crate docs must not markdown-link outside the crate. Web, mail, `data:`, fragment-only, and protocol-relative links are ignored by that check.

Do not hand-edit `.github/workflows/release.yml`; it is cargo-dist generated. Change dist metadata and regenerate. Direct pushes to `main` run read-only preflight only; publication is dispatch-only via `gh workflow run release.yml --ref main -f tag="$TAG"` after versioning and review. Do not skip git hooks. Do not force-push `main`.

Synthetic journey evidence proves deterministic mechanics, routing, and persistence only. It is not semantic review quality.

## Completion and Handoff

A change is done when in-scope behavior matches the accepted intent, authoritative docs for that behavior are current, and the checks required by the change have been run.

Handoff must include:

- files changed and why
- commands run and outcomes
- any Loop Engine run IDs plus the database path used
- remaining risks, known residuals, and follow-up that was out of scope

Do not claim a provider "reviewed" work because a checked transition passed. Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there. A fresh actor resumes from `show` plus the same database and the external paths named in initial input, context, and instructions — not from chat history.
