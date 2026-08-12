# Software-change provider

`software-change` is Loop Engine's reference provider for the software-change workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the fixed workflow topology and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence.

The frozen requirement record this crate's acceptance suite traces to (R1–R27, A1–A15, including amendments) lives at [`docs/prd.md`](docs/prd.md).
- Semantic review is external. The provider does not generate prompts, invoke a model, or decide whether review findings are true.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

## Distribution and registration

Standalone releases are published for macOS arm64 and Linux x86_64. Each provider archive contains the `software-change` executable and both project license texts; verify its `.sha256` checksum before installation.

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"
software-change data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/software-change-provider/data/...` with the configs, templates, reviewer protocol, calibration manifest, and fixtures embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy a selected profile from that tree to a run-specific file, replace its placeholder `artifact_root` with an absolute artifact directory, and register the installed provider under an exact, case-sensitive alias:

```toml
[providers.software-change]
command = "/absolute/path/to/installed/software-change"
args = []
```

Keep machine-specific `providers.toml` outside committed repository files and pass its path with Loop Engine's `--config` option. No provider registration file is committed by this crate.

For repository development, build from the checkout instead:

```sh
cargo build -p software-change-provider
```

This produces `target/debug/software-change`; the checkout's `crates/software-change-provider/data/` tree supplies the development copies of shipped data.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. Config profiles are complete initial-input templates; copy one for a run and replace its placeholder `artifact_root` with an absolute existing artifact directory.

### Config profiles

- [`crates/software-change-provider/data/configs/minimal.json`](data/configs/minimal.json)
- [`crates/software-change-provider/data/configs/standard.json`](data/configs/standard.json)
- [`crates/software-change-provider/data/configs/high-rigor.json`](data/configs/high-rigor.json)

The profiles configure the same five policy gates: `intent` (`explore → design`), `design-review`, `plan-review`, `implementation-review`, and `validation`. `minimal` keeps only `validation`/`intent-delivered`; `standard` supplies the standard intent, design-review, and validation axes; `high-rigor` supplies all shipped axes and requires two distinct reviewers for design-review and validation axes.

### Artifact templates

- [`crates/software-change-provider/data/templates/intent.md`](data/templates/intent.md)
- [`crates/software-change-provider/data/templates/design.md`](data/templates/design.md)
- [`crates/software-change-provider/data/templates/task-packet.md`](data/templates/task-packet.md)
- [`crates/software-change-provider/data/templates/implementation-report.md`](data/templates/implementation-report.md)
- [`crates/software-change-provider/data/templates/validation-report.md`](data/templates/validation-report.md)

### Review and calibration

- [`crates/software-change-provider/data/reviewer-protocol.md`](data/reviewer-protocol.md) defines the `review-evidence` record and external adjudication rules.
- [`crates/software-change-provider/data/calibration/PROCEDURE.md`](data/calibration/PROCEDURE.md) defines owner-attested calibration.
- [`crates/software-change-provider/data/calibration/manifest.json`](data/calibration/manifest.json) records calibration rows.
Calibration fixtures:

- [`crates/software-change-provider/data/calibration/fixtures/intent-good.json`](data/calibration/fixtures/intent-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/intent-defective.json`](data/calibration/fixtures/intent-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/design-good.json`](data/calibration/fixtures/design-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/design-defective.json`](data/calibration/fixtures/design-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/plan-good.json`](data/calibration/fixtures/plan-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/plan-defective.json`](data/calibration/fixtures/plan-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/implementation-report-good.json`](data/calibration/fixtures/implementation-report-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/implementation-report-defective.json`](data/calibration/fixtures/implementation-report-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/validation-report-good.json`](data/calibration/fixtures/validation-report-good.json)
- [`crates/software-change-provider/data/calibration/fixtures/validation-report-defective.json`](data/calibration/fixtures/validation-report-defective.json)
- [`crates/software-change-provider/data/calibration/fixtures/example-evidence.json`](data/calibration/fixtures/example-evidence.json)

## Start a run

Build the engine binary too, or replace `target/debug/loop-engine` below with an installed `loop-engine` executable:

```sh
cargo build -p loop-cli -p software-change-provider
ENGINE=target/debug/loop-engine
DB="$PWD/.loop-engine/loop.db"
ARTIFACT_ROOT="$PWD/change-artifacts"
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
mkdir -p "$ARTIFACT_ROOT"
```

Copy each selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. Before starting, replace `/abs/path/to/change/artifacts` in that copy with `$ARTIFACT_ROOT` (or another absolute artifact directory), then run the matching command:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/minimal.json" /tmp/software-change-minimal.json
"$ENGINE" --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start software-change "@/tmp/software-change-minimal.json" "software change (minimal)"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/standard.json" /tmp/software-change-standard.json
"$ENGINE" --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start software-change "@/tmp/software-change-standard.json" "software change (standard)"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/high-rigor.json" /tmp/software-change-high-rigor.json
"$ENGINE" --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start software-change "@/tmp/software-change-high-rigor.json" "software change (high-rigor)"
```

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. The artifact directory contains the fixed subject filenames expected by the selected schema: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, and `validation-report.json`.

## Read obligations and continue

Use the engine's provider-free `show` operation after `start` and before each event:

```sh
"$ENGINE" --database "$DB" --json show RUN_ID
```

Inspect `initial_input.review_policies` and `initial_input.artifact_schemas` there. `show` is the durable handoff for run-frozen obligations; changing a source profile does not change an existing run. Follow state guidance, append external review records with `append` using [`crates/software-change-provider/data/reviewer-protocol.md`](data/reviewer-protocol.md), and request events with `event` rather than targeting states directly.

For checked transitions, evaluation performs deterministic schema and link checks before consulting review evidence. Missing or unparseable expected artifacts produce a schema denial; invalid or inaccessible artifact roots produce an evaluation error. Evidence denials identify unsatisfied policy axes. Check-free `revise` transitions do not require provider evaluation.

See [`docs/agent-usage.md`](../../docs/agent-usage.md) for the complete Loop Engine command surface and JSON outcome handling.
