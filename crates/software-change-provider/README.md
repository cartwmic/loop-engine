# Software-change provider

## Overview

`software-change` is Loop Engine's reference provider for the software-change workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the live workflow implied by optional `initial_input.review_policies` (the sixteen-state union when that key is omitted) and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence.

The frozen requirement record this crate's acceptance suite traces to (R1–R27, A1–A15, including amendments) lives at [`docs/prd.md`](docs/prd.md).
- Semantic review is external. The provider does not generate prompts, invoke a model, or decide whether review findings are true.
- Reviewer convergence contract lives in [`data/reviewer-protocol.md`](data/reviewer-protocol.md): binary evidence stays unchanged; candidate output is triaged before append or mutation; material in-scope findings require consequence proof, focused external reconsideration handles disputed candidates, and no waiver is granted by round count.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-software-change-provider/SKILL.md](skills/using-software-change-provider/SKILL.md).

## Setup

Standalone releases are published for macOS arm64 and Linux x86_64. Each provider archive contains the `software-change` executable and both project license texts; verify its `.sha256` checksum before installation. Those archives, like loop-engine's, do not contain, vendor, or install the `dagu` binary or Dagu source. Dagu is GPLv3: `run-plan-graph` invokes the operator-provided binary as a subprocess only and does not embed its Go API. `software-change run-plan-graph` resolves operator-provided `dagu` from PATH (runnable file, version `>= 2.14.0`) and fail-closes before any worker spawn, naming the resolved path or that PATH lookup found nothing and the required version. Isolated home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`plan-graph-<capture-dir-name>`). `loop-engine preview-bindings` reports that same `dagu` PATH check before `start` (warning still exits 0; execute fail-closes). Drivers poll `invoke`/`show` for overlay; per-step progress is `dagu status` / `dagu history` against that locator. True inner waitpid lives in the sidecar and `summary.json`; fan-out `dagu status` is helper liveness (helper exit 0 after the worker terminates). Bound review `fan-out` joins mechanically (`fan-out-join` writes `summary.json`, invokes no model).

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/software-change-provider"
software-change data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/software-change-provider/data/...` with the configs, templates, reviewer protocol, review-worker preamble/output contract, calibration manifest, and fixtures embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy a selected profile from that tree to a run-specific file. The engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it. Register the installed provider under an exact, case-sensitive alias:

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

Copy each selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. Shipped profiles omit `work_slot_bindings`. Bound workers are opt-in: use the skill's deterministic constructor for review bindings (or its implement-worker guidance), then confirm the resulting profile, binding preview, models, and SHA-256 before the hash-guarded `start`. Then run the matching command:

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

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. Once the run exists, `show` reveals the allocated (or caller) `artifact_root` inside object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Subject files use the fixed filenames expected by the selected schema: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, and `validation-report.json`.

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

Source full mode uses separate engine processes for every operation, explicit provider TOML, a fresh SQLite file, and the checkout's shipped profile/fixtures. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. It proves schema and evidence mechanics, revision-link denial, aggregation, transitions, persisted context, terminal state, and bound contracted fan-out: deterministic stdin-capturing workers emit conforming JSON or exit-0 refusal text; after the compact one-key `artifact_root` context, failed overlay, and persisted summary/captures, it prints `contracted fan-out failure`. After the high-rigor run reaches `end`, it starts a second run from shipped `minimal.json` and walks the stitched hops (empty review lists omitted, last-hop `passed`). Constructor fixtures are the shipped high-rigor, policy-document readme/agents, and research standard profiles copied into a temporary directory. `python3 scripts/software-change-journey.py --self-test` must print `worker-data skill/root policy assertions passed`. Its synthetic conforming evidence is shape/routing test data only; it does not establish semantic review quality.

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

These are the shipped files consumed by provider tests, guidance, and review procedure. Config profiles are complete initial-input templates; copy one for a run.

### Config profiles

- [`crates/software-change-provider/data/configs/minimal.json`](data/configs/minimal.json)
- [`crates/software-change-provider/data/configs/standard.json`](data/configs/standard.json)
- [`crates/software-change-provider/data/configs/high-rigor.json`](data/configs/high-rigor.json)

The profiles configure parent review gates `intent-review`, `design-review`, `plan-review`, `implementation-review`, and `validation-review`. Standard and high-rigor also ship one counterpart axis per parent axis on the matching `*-adversarial-review` gate. `minimal` keeps only `validation-review`/`intent-delivered` and ships no adversarial lists; `standard` supplies the standard intent-review, design-review, and validation-review axes plus 1:1 counterparts; `high-rigor` supplies all shipped parent axes (two distinct reviewers for design-review and validation-review) plus 1:1 counterparts (`required_authors` 1).

Shipped profiles omit `work_slot_bindings` (or `{}`), so draft slots (`intent-draft`, `design-draft`, `plan-draft`, `implement`, `validation-draft`) stay driver-performed by convention. Review slots are bindable. Review bindings are opt-in and must use the deterministic constructor in [`skills/using-software-change-provider/SKILL.md`](skills/using-software-change-provider/SKILL.md): it reads the shipped [`review-worker-preamble.txt`](data/review-worker-preamble.txt) and [`review-worker-output-schema.json`](data/review-worker-output-schema.json), expands the selected profile's policies in policy/roster order, atomically rewrites that same per-run profile, and requires hash-guarded confirmation before `start`. It keeps `--no-skills --no-extensions`, adds caller-filled extension paths, freezes every model in argv, and does not add `--no-context-files`. When `--task-worker` is omitted, `run-plan-graph` still defaults to `pi --print --no-skills --no-extensions` with no `-e` and no `--model`; that fallback must not pass `--no-context-files` and does not pass `--tools`. Copying a profile is not model lock-in.

### Artifact templates

- [`crates/software-change-provider/data/templates/intent.md`](data/templates/intent.md)
- [`crates/software-change-provider/data/templates/design.md`](data/templates/design.md)
- [`crates/software-change-provider/data/templates/task-packet.md`](data/templates/task-packet.md)
- [`crates/software-change-provider/data/templates/implementation-report.md`](data/templates/implementation-report.md)
- [`crates/software-change-provider/data/templates/validation-report.md`](data/templates/validation-report.md)
- [`crates/software-change-provider/data/templates/accepted-findings.json`](data/templates/accepted-findings.json)

### Review and calibration

- [`crates/software-change-provider/data/review-worker-preamble.txt`](data/review-worker-preamble.txt) defines the read-only review-worker role and driver boundary.
- [`crates/software-change-provider/data/review-worker-output-schema.json`](data/review-worker-output-schema.json) declares the complete required top-level judgment keys.
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

At each review state, first perform a comprehensive review of all visible material findings. Triage candidate reviewer output before append or mutation against mandatory failure burden, independent scope/materiality, consequence, and current evidence. Append accepted in-scope material failures; use focused external reconsideration for disputed candidates. After a fix, confirmation review checks accepted fixes, affected scope, downstream consistency, and regressions. A late finding must supply current evidence, violated in-scope obligation, concrete consequence, validation gap, and provenance classifying it as newly exposed, fix-introduced, or previously overlooked; previous visibility or reviewer overlook does not waive a known material defect. Comprehensive-first review still bars drip-feeding, and unrelated reopening must meet independent scope/materiality burden. Quiet, progress, and thrash count per review state on the post-triage accepted-finding set; they replace a numeric breaker and never waive a known defect.

Validation-report-local defects stay in the validation draft: edit and recheck `validation-report.json`, then retry the next checked hop (`validation-ready` or `passed`); do not use nearest `revise` for report-local corrections. From validation-review and validation-adversarial-review, nearest check-free `revise` returns to the validation draft; those two states also expose `revise-implementation` to implement. Select phase-named owning routes for earlier defects: `revise-intent` (`design-review`/`design-adversarial-review` → explore), `revise-design` (`plan-review`/`plan-adversarial-review` → design or later review states), and `revise-plan` (`implementation-review`/`implementation-adversarial-review` → plan). Zero advisory comments is not required for completion.

## Read obligations and continue

Use the engine's provider-free `show` operation after `start` and before each event:

```sh
"$ENGINE" --json show RUN_ID
```

Inspect `initial_input.review_policies` and `initial_input.artifact_schemas` there. `show` is the durable handoff for run-frozen obligations; changing a source profile does not change an existing run. Follow state guidance, append external review records with `append` using [`crates/software-change-provider/data/reviewer-protocol.md`](data/reviewer-protocol.md), and request events with `event` rather than targeting states directly.

For checked transitions, evaluation performs deterministic schema and link checks before consulting review evidence. Missing or unparseable expected artifacts produce a schema denial; invalid or inaccessible artifact roots produce an evaluation error. Evidence denials identify unsatisfied policy axes. Check-free `revise` transitions do not require provider evaluation.

See the repository file `docs/agent-usage.md` for the complete Loop Engine command surface and JSON outcome handling.

## Candidate identity and supplied calibration data

Binaries built from this source revision accept `software-change --help` (or `-h`) and `software-change --version` (or `-V`) without stdin. Help names `describe`, `evaluate`, `data-dump`, and `run-plan-graph`; it omits hidden `stdin-exec`. Version comes from the packaged Cargo version. Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec`: `stdin-exec --stdin-file ABS --exit-mode sidecar|propagate [--sidecar-file ABS] -- COMMAND [ARG]...`. Duty bytes live only in that file. Plan-graph uses `--exit-mode propagate` only so the helper exit is the inner waitpid; `--sidecar-file` is rejected in that mode. `run-plan-graph` fail-closes if PATH `dagu` is missing, not runnable, or older than 2.14.0, naming the path or PATH miss and required version, before writing `capture_dir/<task_id>/stdout`. It then emits a local Dagu `type:graph` under `capture_dir/dagu-home/` (`max_active_steps` 4, fail-fast, no `continue_on`) and waitpids `dagu start --quiet --dagu-home`. Drivers poll `invoke`/`show` for overlay; per-step progress is `dagu status` / `dagu history` against that isolated home; overlay remains the facade process exit. True inner waitpid for plan-graph steps is the helper exit (`--exit-mode propagate`) and is recorded in `summary.json`. A mandatory `summarizer` step is the sole writer of `artifact_root/implementation-report.json`; ordinary task stdin is compact `artifact_root` JSON plus that task's plan object only. Isolated home locator keys are `dagu_home`, `dag_name`, and `run_name` (`plan-graph-<capture-dir-name>`). Provider packages do not ship `dagu`. Dagu is GPLv3 subprocess-only. The public v0.2.2 release predates these flags. No-argument stdin protocol and `data-dump DIR` behavior remain unchanged.

`data-dump` includes `data/calibration/reviewer-instruction.txt` and stable fictional companions under `data/calibration/companions/fictional-repo/`. Calibration fixtures use `fictional-repo/` labels only; reviewers receive mapped companion bytes and never resolve labels against a live checkout. `data/calibration/PROCEDURE.md` defines one fresh exact-byte external review per row: fixed instruction, prompt, protocol, template, recursively sorted schema, subject, required good predecessors, sorted companions, and canonical request JSON. A11 hashes those ordered source records with big-endian length framing; framing identifies exact supplied records and is not model-call transport. Digest is mechanical test identity, not semantic review proof; changing supplied bytes requires fresh owner review before changing attestation metadata. No shipped harness invokes reviewers or rewrites manifest attestations.

Evidence denial details separate current blockers (`details.diagnostics`) from stale or stale-config recovery context (`details.informational`). Prior denials and inert records remain separate fields. Stale evidence never satisfies current obligations.
