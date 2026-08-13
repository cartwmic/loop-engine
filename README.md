# Loop Engine

## Overview

Loop Engine is a durable workflow CLI. It stores run state in SQLite and coordinates external workflow-provider executables; primary work stays outside the engine.

Current release is v0.4.0 (`MIT OR Apache-2.0`). The living product requirements are [docs/PRD.md](docs/PRD.md). Agent CLI semantics are [docs/agent-usage.md](docs/agent-usage.md). Checkout operating rules for agents are [AGENTS.md](AGENTS.md).

- Operators installing or invoking binaries: [Getting Started](#getting-started) and [Usage](#usage).
- Agents running a workflow: [docs/agent-usage.md](docs/agent-usage.md) plus the provider README and skill.
- Maintainers cutting a release: [Validation](#validation).

Reference providers:

- [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md) — software-change workflow (PRD section 10).
- [`crates/policy-document-provider/README.md`](crates/policy-document-provider/README.md) — policy-document workflow (PRD section 11).

## Getting Started

### Prebuilt GitHub Releases

Starting with v0.3.0, releases publish separate cargo-dist archives for all three binaries and supported targets:

- `loop-cli-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `loop-engine`.
- `loop-cli-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `loop-engine`.
- `software-change-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `software-change`.
- `software-change-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `software-change`.
- `policy-document-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `policy-document`.
- `policy-document-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `policy-document`.

Each archive has a matching `.sha256` file; release `sha256.sum` provides the unified checksum list. Archives include `LICENSE-MIT` and `LICENSE-APACHE`. Download matching archives from [GitHub Releases](https://github.com/cartwmic/loop-engine/releases), verify checksums, and place all three binaries on `PATH`.

Generated cargo-dist installers choose platform automatically:

```sh
VERSION=v0.4.0
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/loop-cli-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/software-change-provider-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/policy-document-provider-installer.sh" | sh
```

With [mise](https://mise.jdx.dev/), manage `loop-engine` as one tool and use separate provider installers. Do not add multiple executable selections for the same GitHub repository to one mise config: mise canonicalizes them to one tool entry, so binaries would be missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.4.0'
for app in software-change-provider policy-document-provider; do
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/cartwmic/loop-engine/releases/download/v0.4.0/$app-installer.sh" | sh
done
```

Historical v0.2.2 releases include only `loop-engine` and `software-change`.

### Build from source

Install all three binaries from GitHub source:

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
cargo install --git https://github.com/cartwmic/loop-engine policy-document-provider --bin policy-document --locked
```

Or build all three binaries from a checkout:

```sh
cargo build --release -p loop-cli -p software-change-provider -p policy-document-provider
# target/release/loop-engine
# target/release/software-change
# target/release/policy-document
```

## Usage

`loop-engine` stores run state in the `--database` SQLite file and snapshots provider association, workflow topology, and state instructions at `start`. Perform the work named by `show` externally, append durable context when the provider requires it, then request one event from `requestable_events`.

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
```

Pass `--json` on every invocation and parse the single envelope. Pass `--config` to `start` with uncommitted machine-local provider TOML using an exact alias and an absolute command path:

```toml
[providers.software-change]
command = "/absolute/path/to/software-change"
args = []

[providers.policy-document]
command = "/absolute/path/to/policy-document"
args = []
```

`start` initial input and `append` data accept JSON inline, `@FILE`, or `-` (stdin). `start` returns the run ID at `result.run.id`. Reuse the same database and run ID for every later operation. `show` is provider-free.

With `--json`, exit `0` is `completed`, `10` is `rejected` (follow feedback; nothing is inferred as advancement), `20` is `error` (re-read `show`), and `2` is `invalid-invocation`. Full envelope and handoff rules: [docs/agent-usage.md](docs/agent-usage.md).

`loop-engine --help` and `--version` work before any operation. `software-change --help`/`-h` describes `describe`, `evaluate`, and `data-dump`; `--version`/`-V` prints the Cargo package version. `policy-document` accepts `data-dump DIR` on argv and otherwise reads one JSON request on stdin; it does not implement `--help` or `--version`. Historical v0.2.2 release installers predate the software-change CLI flags.

## Validation

Supported publication matrix is exactly three applications (`loop-cli`, `software-change-provider`, `policy-document-provider`) by two native targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`). `dist plan` describes this matrix; it does not compile or run archives.

Run baseline checks, generated-workflow validation, plan assertion, and full source-tree production journeys before release handoff:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
dist generate --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py --self-test
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py
python3 scripts/production-journey.py --self-test
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider
python3 scripts/production-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-production-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

Build local host-target archives and smoke extracted binaries before handoff. Use only a newly approved, unpublished release tag; never reuse an existing public tag:

```sh
TAG=vX.Y.Z
dist build --tag="$TAG" --artifacts=local --target=aarch64-apple-darwin
```

Run packaged smoke with extracted `loop-engine`, `software-change`, and `policy-document` paths. Each provider must materialize embedded data, and both provider journeys must run outside checkout; policy-document covers both draft and audit modes. A macOS host build proves only macOS arm64; Linux x86_64 native build and archive smoke remain CI proof when no Linux host is available.

Journey evidence records are synthetic and schema-conforming. They prove deterministic policy mechanics, routing, aggregation, and persistence; they do not prove semantic review quality.

### Publication path

Release workflow is dispatch-only. Do not push a version tag to trigger publication. After versioning, preflight, and review for a new unpublished tag, dispatch the generated workflow with that tag input:

```sh
gh workflow run release.yml --ref main -f tag="$TAG"
```

Dispatch runs cargo-dist's native local-build matrix first, then its generated global-artifact dependencies: preflight, global artifacts, and archive smoke. Host directly depends on all four proof gates and can create the version tag and GitHub Release only after each succeeds. cargo-dist 0.32's generated host expression tolerates skipped dependencies, so `scripts/assert-release-gates.py` proves supported hook topology makes skipped required gates unreachable on publishing paths and rejects failure/skipped regressions. Pull requests run the same preflight and upload-mode artifact path without publication.

Private/free GitHub repositories cannot fully prevent an owner from creating an out-of-band raw tag. Such a tag is outside supported release procedure and does not trigger this workflow; future repository rulesets or plan capability would be needed for prevention.

Historical `v0.2.0`, `v0.2.1`, and `v0.2.2` tags remain immutable. `v0.2.2` was the fix-forward release for contract closure; historical release facts are not rewritten. v0.3.0 added policy-document to the same native archive, installer, source-journey, and packaged-smoke gates as the engine and software-change provider.

### Direct pushes to main

Direct pushes to `main` run `.github/workflows/push-to-main.yml`. That read-only dispatcher checks out the pushed SHA, computes the pinned cargo-dist 0.32.0 plan, and calls reusable `preflight.yml`; preflight owns workspace tests, warning-denying clippy, formatting, generated-release checks, release assertions, and source production-journey proof. The dispatcher has no publication job. `.github/workflows/release.yml` remains cargo-dist-generated and dispatch-only.
