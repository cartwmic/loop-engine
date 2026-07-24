# loop-engine

Local CLI for creating, advancing, inspecting, and terminating durable workflows supplied by executable workflow providers.

## Operation surface

Current release exposes all 21 application operations:

- Provider catalog: `provider.add`, `provider.check`, `provider.disable`, `provider.list`, `provider.rename`, `provider.restore`, `provider.update`
- Run reads and export: `run.compatibility`, `run.evidence.list`, `run.export`, `run.graph`, `run.guidance`, `run.history`, `run.list`, `run.show`
- Run mutations: `run.annotate`, `run.create`, `run.evidence.add`, `run.label`, `run.request`, `run.terminate`

Run `loop-engine --list-operations` for authoritative argv templates.

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

Additional inspection, metadata, and audit operations:

```bash
./target/debug/loop-engine run graph <RUN-ID>
./target/debug/loop-engine run evidence add <RUN-ID> \
  --kind artifact --ref 'opaque:artifact-location' \
  --digest 'sha256:<HEX>' --media-type application/json \
  --metadata ./metadata.json
./target/debug/loop-engine run evidence list <RUN-ID>
./target/debug/loop-engine run annotate <RUN-ID> \
  --note "operator note" --actor ./actor.json
./target/debug/loop-engine run label <RUN-ID> --set "new label"
./target/debug/loop-engine run guidance <RUN-ID> --evidence-id <EVIDENCE-ID>
./target/debug/loop-engine run compatibility <RUN-ID>
./target/debug/loop-engine run export <RUN-ID> --output ./run-export
```

Evidence metadata and annotation actor files must contain one strict JSON object; duplicate keys, trailing values, and non-object roots are rejected. Evidence locators stay opaque and are never opened by engine. Export publishes `manifest.json`, `state.json`, and `journal.jsonl` atomically into a new target directory.

### Complete workflow walkthrough

See the [Go reference-provider walkthrough](examples/providers/reference-go/README.md) for a start-to-finish tutorial: author and build a provider, register it, create a run, drive every workflow transition, inspect guidance and compatibility, and check the authoritative journal and evidence before reaching `end/final`.

Provider lifecycle operations preserve stable registration identity:

```bash
./target/debug/loop-engine provider update <TARGET> --exec <PATH> [--arg <VALUE> ...]
./target/debug/loop-engine provider rename <TARGET> <NEW-HANDLE>
./target/debug/loop-engine provider disable <TARGET>
# If active runs exist, repeat with returned acknowledgement token:
./target/debug/loop-engine provider disable <TARGET> --allow-active-runs <ACK-TOKEN>
./target/debug/loop-engine provider restore <REGISTRATION-ID> \
  --handle <HANDLE> --exec <PATH> --working-directory <PATH>
```

Every invocation creates a private JSONL trace. Structured mode (`--format json`) emits exactly one JSON object; application exits are `0` completed, `2` rejected, `1` error, and `64` pre-dispatch failure.

## Operator paths

- [Install, operation, backup, migration, and troubleshooting guide](docs/operator-guide.md)
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
