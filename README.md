# Loop Engine

## Overview

Loop Engine is a durable workflow CLI. It stores run state in SQLite and coordinates external workflow-provider executables; primary work stays outside the engine.

Current release is v0.11.0 (`MIT OR Apache-2.0`). The living product requirements are [docs/PRD.md](docs/PRD.md). Agent CLI semantics are [docs/agent-usage.md](docs/agent-usage.md). Checkout operating rules for agents are [AGENTS.md](AGENTS.md).

- Operators installing or invoking binaries: [Getting Started](#getting-started) and [Usage](#usage).
- Agents running a workflow: [docs/agent-usage.md](docs/agent-usage.md) plus the provider README and skill.
- Maintainers cutting a release: [Validation](#validation).

Reference providers:

- [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md) — software-change workflow (PRD section 10).
- [`crates/policy-document-provider/README.md`](crates/policy-document-provider/README.md) — policy-document workflow (PRD section 11).
- [`crates/research-provider/README.md`](crates/research-provider/README.md) — research workflow (PRD section 12).

## Getting Started

### Prebuilt GitHub Releases

Starting with v0.3.0, releases publish separate cargo-dist archives for all four binaries and supported targets:

- `loop-cli-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `loop-engine`.
- `loop-cli-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `loop-engine`.
- `software-change-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `software-change`.
- `software-change-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `software-change`.
- `policy-document-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `policy-document`.
- `policy-document-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `policy-document`.
- `research-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `research`.
- `research-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `research`.

Each archive has a matching `.sha256` file; release `sha256.sum` provides the unified checksum list. Archives include `LICENSE-MIT` and `LICENSE-APACHE`. Download matching archives from [GitHub Releases](https://github.com/cartwmic/loop-engine/releases), verify checksums, and place all four binaries on `PATH`.

Generated cargo-dist installers choose platform automatically:

```sh
VERSION=v0.11.0
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/loop-cli-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/software-change-provider-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/policy-document-provider-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/research-provider-installer.sh" | sh
```

With [mise](https://mise.jdx.dev/), manage `loop-engine` as one tool and use separate provider installers. Do not add multiple executable selections for the same GitHub repository to one mise config: mise canonicalizes them to one tool entry, so binaries would be missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.11.0'
for app in software-change-provider policy-document-provider research-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/v0.11.0/$app-installer.sh" | sh
done
```

Historical v0.2.2 releases include only `loop-engine` and `software-change`.

### Build from source

Install all four binaries from GitHub source:

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
cargo install --git https://github.com/cartwmic/loop-engine policy-document-provider --bin policy-document --locked
cargo install --git https://github.com/cartwmic/loop-engine research-provider --bin research --locked
```

Or build all four binaries from a checkout:

```sh
cargo build --release -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
# target/release/loop-engine
# target/release/software-change
# target/release/policy-document
# target/release/research
```

## Usage

`loop-engine` stores run state in a SQLite catalog and snapshots provider association, workflow topology, and state instructions at `start`. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file. Perform the work named by `show` externally, append durable context when the provider requires it, then request one event from `requestable_events`.

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
```

Pass `--json` and parse the single envelope. Pass `--config` to `start` with uncommitted machine-local provider TOML using an exact alias and an absolute command path:

```toml
[providers.software-change]
command = "/absolute/path/to/software-change"
args = []

[providers.policy-document]
command = "/absolute/path/to/policy-document"
args = []

[providers.research]
command = "/absolute/path/to/research"
args = []
```

The engine allocates a durable per-run directory and records that absolute path in object `initial_input`. `list` JSON includes optional `provider` (the start alias) and `artifact_root`; `show` and `history` JSON keys are unchanged.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

`start` initial input and `append` data accept JSON inline, `@FILE`, or `-` (stdin). `start` returns the run ID at `result.run.id`. Reuse the same catalog and run ID for every later operation. `show` is provider-free.

Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`), so implement and review slots stay driver-performed. Bound workers are opt-in skill templates: keep `--no-skills --no-extensions`, add `-e CURSOR_EXTENSION_PATH -e CLAUDE_BRIDGE_EXTENSION_PATH`, name `--model MODEL`, and fill those placeholders in the per-run profile JSON. `loop-engine preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`. Details: [docs/agent-usage.md](docs/agent-usage.md) and the shipped skills.

With `--json`, exit `0` is `completed`, `10` is `rejected` (follow feedback; nothing is inferred as advancement), `20` is `error` (re-read `show`), and `2` is `invalid-invocation`. Full envelope and handoff rules: [docs/agent-usage.md](docs/agent-usage.md).

`loop-engine --help` and `--version` work before any operation. `software-change --help`/`-h` and `research --help`/`-h` describe `describe`, `evaluate`, and `data-dump`; `--version`/`-V` prints the Cargo package version. `policy-document` accepts `data-dump DIR` on argv and otherwise reads one JSON request on stdin; it does not implement `--help` or `--version`. Historical v0.2.2 release installers predate the software-change CLI flags.

## Validation

Supported publication matrix is exactly four applications (`loop-cli`, `software-change-provider`, `policy-document-provider`, `research-provider`) by two native targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`). `dist plan` describes this matrix; it does not compile or run archives.

Run baseline checks, generated-workflow validation, plan assertion, and full source-tree public-boundary journeys before release handoff:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
dist generate --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py --self-test
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py
python3 scripts/software-change-journey.py --self-test
python3 scripts/research-journey.py --self-test
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider
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

Build local host-target archives and smoke extracted binaries before handoff. Use only a newly approved, unpublished release tag; never reuse an existing public tag:

```sh
TAG=vX.Y.Z
dist build --tag="$TAG" --artifacts=local --target=aarch64-apple-darwin
```

Run packaged smoke with extracted `loop-engine`, `software-change`, `policy-document`, and `research` paths. Each provider must materialize embedded data, and all provider journeys must run outside checkout; policy-document covers both draft and audit modes, and the research packaged journey materializes embedded data via `data-dump` / `--mode packaged`. A macOS host build proves only macOS arm64; Linux x86_64 native build and archive smoke remain CI proof when no Linux host is available.

Journey evidence records are synthetic and schema-conforming. They prove deterministic policy mechanics, routing, aggregation, persistence, and sparse work-slot `invoke` via `scripts/dummy-work-slot-worker.py`; they do not prove semantic review quality.

### Publication path

Release workflow is dispatch-only. Do not push a version tag to trigger publication. After versioning, preflight, and review for a new unpublished tag, dispatch the generated workflow with that tag input:

```sh
gh workflow run release.yml --ref main -f tag="$TAG"
```

Dispatch runs cargo-dist's native local-build matrix first, then its generated global-artifact dependencies: preflight, global artifacts, and archive smoke. Host directly depends on all four proof gates and can create the version tag and GitHub Release only after each succeeds. cargo-dist 0.32's generated host expression tolerates skipped dependencies, so `scripts/assert-release-gates.py` proves supported hook topology makes skipped required gates unreachable on publishing paths and rejects failure/skipped regressions. Pull requests run the same preflight and upload-mode artifact path without publication.

Private/free GitHub repositories cannot fully prevent an owner from creating an out-of-band raw tag. Such a tag is outside supported release procedure and does not trigger this workflow; future repository rulesets or plan capability would be needed for prevention.

Historical `v0.2.0`, `v0.2.1`, and `v0.2.2` tags remain immutable. `v0.2.2` was the fix-forward release for contract closure; historical release facts are not rewritten. v0.3.0 added policy-document to the same native archive, installer, source-journey, and packaged-smoke gates as the engine and software-change provider. Research joins the same native archive, installer, source-journey, and packaged-smoke gates.

### Direct pushes to main

Direct pushes to `main` run `.github/workflows/push-to-main.yml`. That read-only dispatcher checks out the pushed SHA, computes the pinned cargo-dist 0.32.0 plan, and calls reusable `preflight.yml`; preflight owns workspace tests, warning-denying clippy, formatting, generated-release checks, release assertions, and source software-change-journey proof. The dispatcher has no publication job. `.github/workflows/release.yml` remains cargo-dist-generated and dispatch-only.
