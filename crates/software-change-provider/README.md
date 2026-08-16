# Software-change provider

## Overview

`software-change` is Loop Engine's reference provider for the software-change workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the fixed workflow topology and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence.

The frozen requirement record this crate's acceptance suite traces to (R1–R27, A1–A15, including amendments) lives at [`docs/prd.md`](docs/prd.md).
- Semantic review is external. The provider does not generate prompts, invoke a model, or decide whether review findings are true.
- Reviewer convergence contract lives in [`data/reviewer-protocol.md`](data/reviewer-protocol.md): binary evidence stays unchanged; candidate output is triaged before append or mutation; material in-scope findings require consequence proof, focused external reconsideration handles disputed candidates, and no waiver is granted by round count.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md).

## Setup

Standalone releases are published for macOS arm64 and Linux x86_64. Each provider archive contains the `software-change` executable and both project license texts; verify its `.sha256` checksum before installation.

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"
software-change data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/software-change-provider/data/...` with the configs, templates, reviewer protocol, calibration manifest, and fixtures embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy a selected profile from that tree to a run-specific file. Omit `artifact_root` in the usual case so the engine allocates the durable directory; pass `artifact_root` only to isolate files to a caller-chosen absolute existing directory. Register the installed provider under an exact, case-sensitive alias:

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

## Usage

Build the engine binary too, or replace `target/debug/loop-engine` below with an installed `loop-engine` executable:

```sh
cargo build -p loop-cli -p software-change-provider
ENGINE=target/debug/loop-engine
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
```

Copy each selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. Omit `artifact_root` from that copy in the usual case so the engine allocates the durable directory, then run the matching command:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/minimal.json" /tmp/software-change-minimal.json
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@/tmp/software-change-minimal.json" "software change (minimal)"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/standard.json" /tmp/software-change-standard.json
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@/tmp/software-change-standard.json" "software change (standard)"

cp "$DATA_ROOT/crates/software-change-provider/data/configs/high-rigor.json" /tmp/software-change-high-rigor.json
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@/tmp/software-change-high-rigor.json" "software change (high-rigor)"
```

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. Once the run exists, `show` reveals the allocated (or caller) `artifact_root` inside object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Subject files use the fixed filenames expected by the selected schema: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, and `validation-report.json`. Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`. Pass a nonempty `artifact_root` only to isolate files to a caller-chosen absolute existing directory.

## Validation

The repository-owned journey runner has one contract with two adapters:

```sh
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full
```

Source full mode uses separate engine processes for every operation, explicit provider TOML, a fresh SQLite file, and the checkout's shipped profile/fixtures. It proves schema and evidence mechanics, revision-link denial, aggregation, transitions, persisted context, and terminal state. Its synthetic conforming evidence is shape/routing test data only; it does not establish semantic review quality.

Packaged smoke accepts extracted `loop-engine` and `software-change` paths, calls `software-change data-dump` into an empty root, and uses only the dumped high-rigor profile and fixtures:

```sh
python3 scripts/software-change-journey.py \
  --mode packaged \
  --engine /path/to/extracted/loop-engine \
  --provider /path/to/extracted/software-change \
  --data-root "${TMPDIR:-/tmp}/software-change-dump" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-packaged-journey" \
  --profile high-rigor.json \
  --traversal-depth checked-prefix
```

The packaged adapter does not read checkout provider data after `data-dump`. Native archive smoke runs on macOS arm64 and Linux x86_64 in the dispatch-only cargo-dist workflow before host publication.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. Config profiles are complete initial-input templates; copy one for a run and omit `artifact_root` unless isolating files to a caller-chosen absolute existing directory.

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

## Convergence and owning-phase routes

At each review state, first perform a comprehensive review of all visible material findings. Triage candidate reviewer output before append or mutation against mandatory failure burden, independent scope/materiality, consequence, and current evidence. Append accepted in-scope material failures; use focused external reconsideration for disputed candidates. After a fix, confirmation review checks accepted fixes, affected scope, downstream consistency, and regressions. A late finding must supply current evidence, violated in-scope obligation, concrete consequence, validation gap, and provenance classifying it as newly exposed, fix-introduced, or previously overlooked; previous visibility or reviewer overlook does not waive a known material defect. Comprehensive-first review still bars drip-feeding, and unrelated reopening must meet independent scope/materiality burden. A default three-round circuit breaker changes review method only and never waives a known defect.

Validation-report-local defects stay in validation: edit and recheck `validation-report.json`, then retry checked `passed`; do not use `revise` for report-local corrections. From validation, nearest check-free `revise` is only for implementation-owned defects. Select phase-named owning routes for earlier defects: `revise-intent` (`design-review → explore`), `revise-design` (`plan-review → design` or later review states), and `revise-plan` (`implementation-review`/`validation → plan`). Zero advisory comments is not required for completion.

## Read obligations and continue

Use the engine's provider-free `show` operation after `start` and before each event:

```sh
"$ENGINE" --json show RUN_ID
```

Inspect `initial_input.review_policies` and `initial_input.artifact_schemas` there. `show` is the durable handoff for run-frozen obligations; changing a source profile does not change an existing run. Follow state guidance, append external review records with `append` using [`crates/software-change-provider/data/reviewer-protocol.md`](data/reviewer-protocol.md), and request events with `event` rather than targeting states directly.

For checked transitions, evaluation performs deterministic schema and link checks before consulting review evidence. Missing or unparseable expected artifacts produce a schema denial; invalid or inaccessible artifact roots produce an evaluation error. Evidence denials identify unsatisfied policy axes. Check-free `revise` transitions do not require provider evaluation.

See the repository file `docs/agent-usage.md` for the complete Loop Engine command surface and JSON outcome handling.

## Candidate identity and supplied calibration data

Binaries built from this source revision accept `software-change --help` (or `-h`) and `software-change --version` (or `-V`) without stdin. Help names `describe`, `evaluate`, and `data-dump`; version comes from the packaged Cargo version. The public v0.2.2 release predates these flags. No-argument stdin protocol and `data-dump DIR` behavior remain unchanged.

`data-dump` includes `data/calibration/reviewer-instruction.txt` and stable fictional companions under `data/calibration/companions/fictional-repo/`. Calibration fixtures use `fictional-repo/` labels only; reviewers receive mapped companion bytes and never resolve labels against a live checkout. `data/calibration/PROCEDURE.md` defines one fresh exact-byte external review per row: fixed instruction, prompt, protocol, template, recursively sorted schema, subject, required good predecessors, sorted companions, and canonical request JSON. A11 hashes those ordered source records with big-endian length framing; framing identifies exact supplied records and is not model-call transport. Digest is mechanical test identity, not semantic review proof; changing supplied bytes requires fresh owner review before changing attestation metadata. No shipped harness invokes reviewers or rewrites manifest attestations.

Evidence denial details separate current blockers (`details.diagnostics`) from stale or stale-config recovery context (`details.informational`). Prior denials and inert records remain separate fields. Stale evidence never satisfies current obligations.
