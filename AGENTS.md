# Agent instructions

## Scope

This file instructs agents working in this loop-engine checkout. In scope: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, the `software-change`, `policy-document`, and `research` reference providers, `tests/fixtures`, `scripts/`, skills, and release-proof workflows.

Out of scope: performing primary workflow work *inside* the engine or a provider; inventing engine policy, review orchestration, or core semantics; and treating this file as a human product overview. Humans start at [README.md](README.md).

Crate-level `AGENTS.md` files add crate-local procedure only. They must not contradict this file.

## Authority

When instructions conflict, use this order:

1. This file, for how to operate in this checkout.
2. [docs/PRD.md](docs/PRD.md), living engine product requirements.
3. `crates/software-change-provider/docs/prd.md`, frozen software-change provider requirements.
4. The relevant crate README and skill, for driving that provider.
5. [docs/agent-usage.md](docs/agent-usage.md), for CLI forms, JSON envelopes, and the `show` / `append` / `event` / `invoke` loop.

The engine owns durable run state and progression. Callers perform primary work externally. Providers `describe` topology and `evaluate` the exact transition the engine selected; they do not choose the next state, edit repositories, invoke reviewers, or judge semantic truth. Context `kind` and `data` are opaque to core; follow the active provider's record conventions.

Shipped skills: [skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md), `crates/software-change-provider/skills/using-software-change-provider/SKILL.md`, `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md`, and `crates/research-provider/skills/using-research-provider/SKILL.md`. Shipped software-change profiles omit `work_slot_bindings`; bound Pi workers are opt-in templates in those skills (`--no-extensions` plus `-e` placeholders, filled in the per-run profile JSON). `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`. Confirm work-slot policy with the user before `start`.

## Workflow

Build the engine and the reference providers, then run the repository baseline:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI preflight also runs `dist generate --check`, `scripts/assert-dist-plan.py`, `scripts/assert-release-gates.py`, `scripts/assert-push-main-preflight.py`, `scripts/software-change-journey.py --self-test`, `scripts/research-journey.py --self-test`, a locked build of the four release packages, the software-change source journey (`--mode source --traversal-depth full` against high-rigor), both policy-document source journey modes, and the research source journey.

Public-boundary Python journeys are required validation for every in-scope change. Workspace `cargo test` / clippy / fmt do not substitute for them. Build the four release packages, then run the journeys that cover the public boundary you touched; if that boundary is unclear, run all three source journeys.

- Engine, CLI, core, integrations, shared `scripts/`, or `invoke` / work-slot behavior: all three source journeys.
- `software-change` crate: `scripts/software-change-journey.py --mode source --traversal-depth full`.
- `policy-document` crate: both `scripts/policy-document-journey.py` modes.
- `research` crate: `scripts/research-journey.py --mode source`.
- Journey-harness edits: that script's `--self-test` when it has one, plus the journeys it runs.

```sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
python3 scripts/software-change-journey.py --self-test
python3 scripts/research-journey.py --self-test
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
python3 scripts/research-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/research \
  --profile crates/research-provider/data/configs/standard.json
```

Reproduce `dist generate --check` and the assert scripts when the change can affect release proof or generated workflow.

Drive production runs with `loop-engine`. Pass `--json` and parse the single JSON envelope. Pass `--config` on `start` with uncommitted machine-local provider TOML. Omit `--database` unless isolating (`--database /path/to/dir/loop.db`); when `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file. Omit `artifact_root` unless isolating files. Use exact aliases `software-change`, `policy-document`, and `research` and absolute `command` paths. Do not commit provider TOML, run databases, or secrets.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

Parse the single JSON envelope even on nonzero exit. Treat only `status: "completed"` as success. `rejected` (exit 10) is an understood denial — follow feedback and continue. `error` (exit 20) means nothing advanced; re-run `show`. Request events from the latest `show`, never states. Serialize `append`, `event`, `invoke`, and `terminate` per run.

`loop-engine`, `software-change`, and `research` accept `--help`/`-h` and `--version`/`-V` before stdin. `policy-document` does not: unsupported argv besides `data-dump DIR` is an error; describe/evaluate remain stdin JSON.

When drafting or auditing `README.md` or `AGENTS.md`, use the policy-document provider and a copy of the shipped profile (`readme-2` or `agents-2`). Keep `target.id` and `profile_version` unless intentionally authoring a custom profile. Local markdown links must resolve under the target file's directory; parent-directory segments (`..`) are rejected as escapes, so crate docs must not markdown-link outside the crate. Web, mail, `data:`, fragment-only, and protocol-relative links are ignored by that check.

Do not hand-edit `.github/workflows/release.yml`; it is cargo-dist generated. Change dist metadata and regenerate. Direct pushes to `main` run read-only preflight only; publication is dispatch-only via `gh workflow run release.yml --ref main -f tag="$TAG"` after versioning and review. Do not skip git hooks. Do not force-push `main`.

Synthetic journey evidence proves deterministic mechanics, routing, and persistence only. It is not semantic review quality.

## Completion and Handoff

A change is done when in-scope behavior matches the accepted intent, authoritative docs for that behavior are current, workspace baseline checks have been run, and the black-box Python journeys that cover the touched public boundary have passed.

Handoff must include:

- files changed and why
- commands run and outcomes
- any Loop Engine run IDs plus the database path used
- remaining risks, known residuals, and follow-up that was out of scope

Do not claim a provider "reviewed" work because a checked transition passed. Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there. A fresh actor resumes from `show` plus the same database and the external paths named in initial input, context, and instructions — not from chat history.
