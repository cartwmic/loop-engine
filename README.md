# loop-engine

Local CLI for creating, advancing, inspecting, and terminating durable workflows supplied by executable workflow providers.

## If you are familiar with workflow engines

A traditional workflow engine usually owns execution: it schedules tasks, dispatches workers or service calls, waits for their results, and applies retry or timeout policy. Loop Engine owns only coordination. A human, agent, script, or external system inspects the current state, performs the work elsewhere, and asks the engine to accept an event. The engine advances the run only when the stored graph permits the transition and any provider-defined gates pass.

That inversion is the point. Loop Engine is a durable control plane for externally performed work, not a worker runtime or agent orchestrator. An agent is simply one possible caller, using the same operations and workflow semantics as a human or script.

| Concern | Traditional workflow engine (common model) | Loop Engine |
|---|---|---|
| Primary abstraction | Tasks for the engine to execute | States of work performed outside the engine |
| Workflow definition | A declarative graph plus task or worker implementations | One executable provider that emits the graph and implements gates and guidance |
| Progress | Task completion, callbacks, or engine-triggered transitions | An explicit event request from the current actor |
| Enforcement | Worker outcome and orchestration policy | Engine-resolved transitions plus provider-defined validation gates |
| Working context | Workflow variables and task payloads | Immutable run inputs, append-only evidence references, notes, and current state |
| Durability | Long-running execution, queues, retries, and timers | Durable state and handoff across CLI processes, agent sessions, and actors |
| Runtime | Commonly a server, scheduler, queue, and worker fleet | A local CLI, executable providers, and SQLite |
| History | Execution and task history | An ordered journal explaining requests, gate verdicts, evidence, and state changes |

This model is useful when the hard part is keeping human or agent-driven work on-policy across interruptions and revision cycles. Prompts and agent sessions are temporary; the run's state, graph snapshot, evidence, and history are not. A new actor can inspect the run and continue without reconstructing the workflow from chat history or trusting a previous actor's claim that the work is done. Workflow-specific policy remains testable code in the provider instead of being split between a declarative graph, prompts, and ad hoc scripts.

Use Loop Engine when:

- primary work must remain in an existing human, agent, script, or tool environment;
- runs need to survive process, session, or actor boundaries;
- progress should require explicit, domain-specific validation rather than a completion claim;
- revision loops and evidence-backed handoffs are central to the workflow; or
- you want durable coordination without operating a workflow service.

Use a traditional workflow engine when the engine must schedule and execute jobs, provide timers or automatic retries, coordinate parallel workers or child workflows, or operate as a distributed multi-user service. Loop Engine deliberately does not provide those capabilities.

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

## Development validation

From existing committed checkout, install tracked Git hooks once:

```bash
cargo xtask hooks install
```

Pre-commit validates exact staged tree. Pre-push validates one aggregate publication and stores immutable evidence beneath Git common directory. See [development policy](docs/development-policy.md) for prerequisites, commands, reports, semantic approval, retry, and independent push-CI behavior.

## Operator paths

- [Harness-neutral agent skill seed](examples/skills/using-loop-engine/SKILL.md)
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
