# Go reference-workflow provider

This standalone Go example implements loop-engine provider protocol v1 without importing product crates or third-party packages. It recreates the software-change workflow documented in [`docs/reference-workflow.md`](../../../docs/reference-workflow.md):

```text
explore -> design -> design-review -> plan -> plan-review
        -> implement -> implementation-review -> validation -> end
```

Review states can return to their authoring state with `changes-requested`; failed validation returns to `implement`.

## What owns what

Loop Engine owns run identity, current state, lifecycle, journal, stored graph, and committed evidence. Provider owns:

- graph declaration and static guidance;
- run-input validation;
- gate policy and provider-produced evidence;
- optional live guidance;
- compatibility judgment for stored graphs.

Provider receives snapshots, evaluates them, and returns data. It never opens or writes engine SQLite state.

## Source tour

| File | Responsibility |
|---|---|
| [`main.go`](main.go) | One bounded JSON object from stdin, duplicate/trailing-value rejection, one JSON result on stdout |
| [`protocol.go`](protocol.go) | Language-local protocol v1 DTOs |
| [`graph.go`](graph.go) | Nine states, twelve transitions, input declarations, guidance, and gate IDs |
| [`provider.go`](provider.go) | Five-role dispatcher, input validation, guidance, and compatibility |
| [`artifacts.go`](artifacts.go) | Artifact loading, revision linkage, verdict gates, SHA-256 evidence |
| [`main_test.go`](main_test.go) | Transport, graph, input, and gate checks |
| [`fixtures/artifacts/happy-path/`](fixtures/artifacts/happy-path/) | Self-contained artifacts for one successful run |

Normative wire rules and schemas remain in [`docs/provider-protocol-v1.md`](../../../docs/provider-protocol-v1.md) and [`schemas/provider/v1/`](../../../schemas/provider/v1/).

## 1. Build and test

Repository walkthrough uses Go 1.26.5 through `mise` without changing root tool configuration:

```bash
mise install go@1.26.5
mise x go@1.26.5 -- go -C examples/providers/reference-go test ./...
mkdir -p target/reference-go-provider
mise x go@1.26.5 -- go -C examples/providers/reference-go build \
  -o ../../../target/reference-go-provider/reference-go .

env -u RUSTUP_TOOLCHAIN cargo build -p loop-engine-cli
```

Provider has no network or third-party dependency.

## 2. Understand transport

Engine starts one fresh provider process per role invocation. Process must:

1. consume exactly one UTF-8 JSON request object from stdin;
2. dispatch `describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, or `check_compatibility`;
3. preserve `protocol_major`, `role`, and `invocation_id` in result envelope;
4. write exactly one bounded JSON object to stdout;
5. reserve stderr for process diagnostics.

`main.go` enforces one-object framing before `handleRequest` applies role policy.

## 3. Declare graph

`referenceGraph()` in `graph.go` is executable workflow authority. Important fields:

- `initial_state: "explore"`;
- exactly one final state, `end`;
- each `(source_state, event)` selects one transition;
- every transition names provider-owned gate IDs;
- `artifact_root` is required string/path input;
- `live_guidance_supported` advertises optional guidance role.

Changing returned graph changes new-run graph revision. Existing runs retain creation-time canonical graph snapshot.

## 4. Implement provider policy

### `describe`

Returns `{"kind":"description","graph":...}`. `provider check` validates and canonicalizes graph.

### `validate_inputs`

Checks graph declarations against candidate values. Accepted values become authoritative run inputs. This example requires non-empty string `artifact_root`.

### `evaluate_gates`

Reads provider-owned artifact files beneath accepted `artifact_root`. Rules include:

- `design.json.intent_revision == intent.json.revision`;
- `plan.json.subject_revision == design.json.revision`;
- `implementation.json.plan_revision == plan.json.revision`;
- review `subject_revision` equals current artifact revision;
- review or validation verdict matches requested event.

Successful gates return exact verdict set plus opaque evidence records. Evidence includes file URI, media type, artifact revision metadata, and SHA-256 content digest. Engine decides whether transition commits atomically.

### `live_guidance`

Returns state-specific advisory text. Guidance does not authorize state transition.

### `check_compatibility`

Checks current provider implementation against run's stored graph. Unknown stored gate IDs make `evaluate_gates` incompatible; unknown requested capabilities return `unknown`.

## 5. Register and inspect

Use isolated state so tutorial cannot affect normal provider catalog or runs:

```bash
ROOT="$PWD"
export LOOP_ENGINE_HOME="$(mktemp -d /tmp/loop-engine-reference-go.XXXXXX)"
cat > "$LOOP_ENGINE_HOME/config.toml" <<'TOML'
schema_version = 1
[defaults]
format = "json"
TOML

CLI="$ROOT/target/debug/loop-engine"
PROVIDER="$ROOT/target/reference-go-provider/reference-go"
WORKDIR="$ROOT/examples/providers/reference-go"

"$CLI" provider add reference-go \
  --exec "$PROVIDER" \
  --working-directory "$WORKDIR"
"$CLI" provider check reference-go
```

`provider check` executes real `describe` process and validates returned protocol envelope and graph.

## 6. Create run

Input file uses absolute artifact root because provider owns interpretation of that accepted string:

```bash
cat > "$LOOP_ENGINE_HOME/inputs.json" <<JSON
{
  "artifact_root": "$ROOT/examples/providers/reference-go/fixtures/artifacts/happy-path",
  "change_id": "reference-go-demo"
}
JSON

"$CLI" run create reference-go \
  --label "Go reference walkthrough" \
  --inputs "$LOOP_ENGINE_HOME/inputs.json" \
  > "$LOOP_ENGINE_HOME/run-create.json"

RUN_ID="$(python3 -c '
import json, os
p = os.path.join(os.environ["LOOP_ENGINE_HOME"], "run-create.json")
print(json.load(open(p))["data"]["run"]["id"])
')"

"$CLI" run graph "$RUN_ID"
"$CLI" run guidance "$RUN_ID"
"$CLI" run compatibility "$RUN_ID"
```

Creation invokes `describe` and `validate_inputs`. `run graph` is provider-free because engine serves stored snapshot.

## 7. Advance happy path

All fixtures already exist, so each provider gate can pass:

```bash
"$CLI" run request "$RUN_ID" intent-ready
"$CLI" run request "$RUN_ID" design-ready
"$CLI" run request "$RUN_ID" approved
"$CLI" run request "$RUN_ID" plan-ready
"$CLI" run request "$RUN_ID" approved
"$CLI" run request "$RUN_ID" implementation-ready
"$CLI" run request "$RUN_ID" approved
"$CLI" run request "$RUN_ID" passed
```

Repeated `approved` event is unambiguous because current state selects design, plan, or implementation review transition.

Inspect authoritative results:

```bash
"$CLI" run show "$RUN_ID"
"$CLI" run history "$RUN_ID" --limit 100
"$CLI" run evidence list "$RUN_ID" --limit 100
```

Expected final run state is `end`, lifecycle is `final`, and provider-generated evidence contains eight artifact records.

## 8. Exercise rejection loop

To test provider policy rather than only happy path, copy fixtures, alter verdict, and create another isolated run. For example, at `design-review`, set:

```json
{
  "revision": "1",
  "subject_revision": "1",
  "verdict": "changes_requested"
}
```

Then request `changes-requested`; run returns from `design-review` to `design`. Requesting `approved` against same artifact is rejected because event and artifact verdict disagree.

## Validation layers

- `go test ./...`: provider-local mechanics and strict transport checks;
- `provider check`: published `describe` result schema plus graph semantics;
- `run create`: `describe` and `validate_inputs` request/result adapters;
- `run request`: `evaluate_gates` request/result adapter and atomic persistence;
- `run guidance`: `live_guidance` adapter;
- `run compatibility`: `check_compatibility` adapter;
- published Draft 2020-12 schemas: offline validator input for independent release pipelines.
