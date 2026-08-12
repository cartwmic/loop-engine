# Loop Engine

Loop Engine is a durable workflow CLI. It stores run state in SQLite and coordinates external workflow-provider executables; primary work stays outside the engine.

Agent operation is documented in [`docs/agent-usage.md`](docs/agent-usage.md). The reference software-change provider is documented in [`crates/software-change-provider/README.md`](crates/software-change-provider/README.md).

## Install

### Prebuilt GitHub Releases

Releases publish separate cargo-dist archives for each binary and target:

- `loop-cli-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `loop-engine`.
- `loop-cli-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `loop-engine`.
- `software-change-provider-aarch64-apple-darwin.tar.xz` — macOS arm64, contains `software-change`.
- `software-change-provider-x86_64-unknown-linux-gnu.tar.xz` — Linux x86_64, contains `software-change`.

Each archive has a matching `.sha256` file; release `sha256.sum` provides the unified checksum list. Archives include `LICENSE-MIT` and `LICENSE-APACHE`. Download the matching archives from [GitHub Releases](https://github.com/cartwmic/loop-engine/releases), verify its checksum, and place both binaries on `PATH`.

The generated cargo-dist installers are the simplest way to install both binaries and choose the platform automatically:

```sh
VERSION=v0.2.1
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/loop-cli-installer.sh" | sh
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/$VERSION/software-change-provider-installer.sh" | sh
```

With [mise](https://mise.jdx.dev/), manage `loop-engine` as one tool and use the provider's separate release installer. Do not add two executable selections for the same GitHub repository to one mise config: mise canonicalizes them to one tool entry, so one binary would be missing.

```sh
mise use --global 'github:cartwmic/loop-engine[exe=loop-engine]@v0.2.1'
curl --proto '=https' --tlsv1.2 -LsSf \
  "https://github.com/cartwmic/loop-engine/releases/download/v0.2.1/software-change-provider-installer.sh" | sh
```

Verified with `mise use --dry-run --path /tmp/mise.toml 'github:cartwmic/loop-engine[exe=loop-engine]@v0.2.1'`: one `github:cartwmic/loop-engine@v0.2.1` entry is created, which is why provider installation remains a separate command.

### Build from source

Install either binary directly from GitHub:

```sh
cargo install --git https://github.com/cartwmic/loop-engine loop-cli --bin loop-engine --locked
cargo install --git https://github.com/cartwmic/loop-engine software-change-provider --bin software-change --locked
```

Or build both from a checkout:

```sh
cargo build --release -p loop-cli -p software-change-provider
# target/release/loop-engine
# target/release/software-change
```

Release preflight: before pushing a version tag, run a real cargo-dist build on each supported release target; `dist plan` alone does not compile. For this release, the macOS arm64 proof is:

```sh
dist build --tag=v0.2.1 --artifacts=local --target=aarch64-apple-darwin
```

The build must succeed and produce both binary archives before tagging. Repeat for `x86_64-unknown-linux-gnu` in CI or a matching Linux environment.
