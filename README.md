# loop-engine

Local CLI for creating, advancing, inspecting, and terminating durable workflows supplied by executable workflow providers.

## Alpha surface

Current alpha exposes exactly nine application operations:

- `provider.add`, `provider.check`, `provider.list`
- `run.create`, `run.history`, `run.list`, `run.request`, `run.show`, `run.terminate`

Run `loop-engine --list-operations` for authoritative argv templates. Other operations in the frozen 21-operation MVP catalog remain unavailable until their post-alpha checkpoints close.

## Build and isolated quick start

Requires Rust 1.95.0 on macOS or glibc Linux.

```bash
env -u RUSTUP_TOOLCHAIN cargo build --locked -p loop-engine-cli
env -u RUSTUP_TOOLCHAIN CARGO_TARGET_DIR="$PWD/target/reference-provider" \
  cargo build --locked \
  --manifest-path test-support/providers/reference-provider/Cargo.toml

export LOOP_ENGINE_HOME="$(mktemp -d)"
export ARTIFACT_ROOT="$(mktemp -d)"
printf '{"artifact_root":"%s"}\n' "$ARTIFACT_ROOT" > "$ARTIFACT_ROOT/inputs.json"
printf '{"revision":"1"}\n' > "$ARTIFACT_ROOT/intent.json"

./target/debug/loop-engine provider add reference \
  --exec "$PWD/target/reference-provider/debug/reference-provider" \
  --working-directory "$PWD"
./target/debug/loop-engine provider check reference
./target/debug/loop-engine run create reference \
  --label "first change" --inputs "$ARTIFACT_ROOT/inputs.json"
./target/debug/loop-engine run list
```

Use run ID returned by `run create`:

```bash
./target/debug/loop-engine run show <RUN-ID>
./target/debug/loop-engine run request <RUN-ID> intent-ready
./target/debug/loop-engine run history <RUN-ID>
./target/debug/loop-engine run terminate <RUN-ID> --note "stopping this run"
```

Every invocation creates a private JSONL trace. Structured mode (`--format json`) emits exactly one JSON object; application exits are `0` completed, `2` rejected, `1` error, and `64` pre-dispatch failure.

## Operator paths

- [CLI and exit contract](docs/cli-contract.md)
- [Configuration and machine-local paths](docs/configuration.md)
- [Operational trace inspection](docs/operational-trace.md)
- [SQLite authority, durability, and recovery](docs/persistence.md)
- [Provider protocol](docs/provider-protocol-v1.md)
- [Reference software-change workflow](docs/reference-workflow.md)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
