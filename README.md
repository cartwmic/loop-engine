# Loop Engine

Loop Engine is a durable workflow CLI. It stores run state in SQLite and coordinates external workflow-provider executables; primary work stays outside the engine.

Agent operation is documented in [`docs/agent-usage.md`](docs/agent-usage.md). Reference providers: [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md) and focused PRD section 11 [`crates/policy-document-provider/README.md`](crates/policy-document-provider/README.md).

## Install

### Prebuilt GitHub Releases

Releases publish separate cargo-dist archives for each binary and supported target:

- `loop-cli-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `loop-engine`.
- `loop-cli-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `loop-engine`.
- `software-change-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `software-change`.
- `software-change-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `software-change`.

Each archive has a matching `.sha256` file; release `sha256.sum` provides the unified checksum list. Archives include `LICENSE-MIT` and `LICENSE-APACHE`. Download the matching archives from [GitHub Releases](https://github.com/cartwmic/loop-engine/releases), verify their checksums, and place both binaries on `PATH`.

The generated cargo-dist installers are the simplest way to install both binaries and choose the platform automatically:

```sh
VERSION=v0.2.2
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/loop-cli-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/software-change-provider-installer.sh" | sh
```

With [mise](https://mise.jdx.dev/), manage `loop-engine` as one tool and use the provider's separate release installer. Do not add two executable selections for the same GitHub repository to one mise config: mise canonicalizes them to one tool entry, so one binary would be missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.2.2'
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/v0.2.2/software-change-provider-installer.sh" | sh
```

### Build from source

Install all three binaries from GitHub source (only the first two are release-distributed):

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
cargo install --git https://github.com/cartwmic/loop-engine policy-document-provider --bin policy-document --locked
```

Or build all three binaries from a checkout (the policy-document provider is source-only and excluded from the release matrix):

```sh
cargo build --release -p loop-cli -p software-change-provider -p policy-document-provider
# target/release/loop-engine
# target/release/software-change
# target/release/policy-document
```

## Release preflight

Supported publication matrix is exactly two applications (`loop-cli`, `software-change-provider`) by two native targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`). `dist plan` describes this matrix; it does not compile or run archives.

Run baseline checks, generated-workflow validation, plan assertion, and full source-tree production journey before release handoff:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
dist generate --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py
python3 scripts/production-journey.py --self-test
cargo build --locked -p loop-cli -p software-change-provider
python3 scripts/production-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-production-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full
```

Build local host-target archives and smoke extracted binaries before handoff:

```sh
dist build --tag=v0.2.2 --artifacts=local --target=aarch64-apple-darwin
```

Run packaged smoke with extracted `loop-engine` and `software-change` paths using the packaged adapter described in [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md). A macOS host build proves only macOS arm64; Linux x86_64 native build and archive smoke remain CI proof when no Linux host is available.

### Publication path

Release workflow is dispatch-only. Do not push a version tag to trigger publication. After preflight and review, dispatch the generated workflow with its tag input:

```sh
gh workflow run release.yml --ref main -f tag=v0.2.2
```

Dispatch runs cargo-dist's native local-build matrix first, then its generated global-artifact dependencies: preflight, global artifacts, and archive smoke. Host directly depends on all four proof gates and can create the version tag and GitHub Release only after each succeeds. cargo-dist 0.32's generated host expression tolerates skipped dependencies, so `scripts/assert-release-gates.py` proves supported hook topology makes skipped required gates unreachable on publishing paths and rejects failure/skipped regressions. Pull requests run the same preflight and upload-mode artifact path without publication.

Private/free GitHub repositories cannot fully prevent an owner from creating an out-of-band raw tag. Such a tag is outside supported release procedure and does not trigger this workflow; future repository rulesets or plan capability would be needed for prevention.

Historical `v0.2.0` and `v0.2.1` tags remain immutable. `v0.2.2` is fix-forward release for contract closure; historical release facts are not rewritten.
