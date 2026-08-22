# Research provider

## Overview

`research` is Loop Engine's reference provider for the research workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the fixed workflow topology and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence at verify and synthesize.

The provider does not invoke models, fetch the web, or judge semantic truth. Callers perform search, fetch, and writing outside the provider.

- Semantic review is external. The provider does not generate prompts, invoke a model, or decide whether review findings are true.
- Reviewer convergence contract lives in [data/reviewer-protocol.md](data/reviewer-protocol.md): binary evidence stays unchanged; candidate output is triaged before append or mutation; material in-scope findings require consequence proof, focused external reconsideration handles disputed candidates, and no waiver is granted by round count.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a standard run with [skills/using-research-provider/SKILL.md](skills/using-research-provider/SKILL.md), or extract a provisional PRD candidate with [skills/using-generate-prd/SKILL.md](skills/using-generate-prd/SKILL.md).

## Setup

Standalone releases are published for macOS arm64 and Linux x86_64. Each provider archive contains the `research` executable and both project license texts; verify its `.sha256` checksum before installation.

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/research-provider"
research data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/research-provider/...` with the profile, templates, reviewer protocol, review-worker preamble and output schema, skill, README, and AGENTS embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy the selected profile from that tree to a run-specific file. The engine allocates the durable directory and records that absolute path in object `initial_input`; `show` reveals it. Register the installed provider under an exact, case-sensitive alias:

```toml
[providers.research]
command = "/absolute/path/to/installed/research"
args = []
```

Keep machine-specific `providers.toml` outside committed repository files and pass its path with Loop Engine's `--config` option. No provider registration file is committed by this crate.

For repository development, build from the checkout instead:

```sh
cargo build -p research-provider
```

This produces `target/debug/research`; the checkout's `crates/research-provider/data/` tree supplies the development copies of shipped data.

## Usage

Build the engine binary too, or replace `target/debug/loop-engine` below with an installed `loop-engine` executable:

```sh
cargo build -p loop-cli -p research-provider -p bookends-check
ENGINE=target/debug/loop-engine
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
```

Copy the selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. Then:

```sh
DATA_ROOT="$HOME/.local/share/research-provider"

cp "$DATA_ROOT/crates/research-provider/data/configs/standard.json" /tmp/research-standard.json
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-standard.json" "research (standard)"
```

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. Once the run exists, `show` reveals the allocated (or caller) `artifact_root` inside object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Subject files use the fixed filenames expected by the selected schema: `brief.json`, `sources.json`, `verification.json`, and `report.json`.

## Validation

Crate tests cover describe, evaluate, shipped data, and data-dump:

```sh
cargo test -p research-provider
cargo clippy -p research-provider --all-targets -- -D warnings
```

Source and packaged journeys live in `scripts/research-journey.py` at the repository root. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. Crate tests remain the local proof surface for protocol, schema, evidence, and shipped data. `scripts/assert-generate-prd-profile.py` proves that the Generate-PRD profile uses the existing research binary and does not add a provider. The source Generate-PRD journey drives that profile with deterministic bound workers, writes `prd-candidate.md` plus exact repository evidence, reaches `end`, and runs `bookends-check candidate`; it does not edit `docs/PRD.md` or commit. The software-change journey `--self-test` executes this crate's verify and synthesize constructors against `data/configs/standard.json` and prints `worker-data skill/root policy assertions passed` only after worker count/order, axis/`example_prompt`/author/model metadata, required keys/data bytes/preview visibility, and fail-closed invalid cases pass.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. The config profile is a complete initial-input template; copy it for a run.

### Config profiles

- [data/configs/standard.json](data/configs/standard.json) (`config_version` `research-1`; verify axes `claim-grounded` and `adversarial`; synthesize axes `cited-conclusion` and `scope-faithful`)
- [data/configs/generate-prd.json](data/configs/generate-prd.json) (the same research topology and schemas, with Generate-PRD templates)

Generate-PRD candidate IDs are provisional proposals. A human must accept or reject `prd-candidate.md` before any commit to `docs/PRD.md`; the profile never auto-commits or claims semantic completeness.

### Templates

- [data/templates/brief.md](data/templates/brief.md)
- [data/templates/sources.md](data/templates/sources.md)
- [data/templates/verification.md](data/templates/verification.md)
- [data/templates/report.md](data/templates/report.md)
- [data/templates/generate-prd/brief.md](data/templates/generate-prd/brief.md)
- [data/templates/generate-prd/sources.md](data/templates/generate-prd/sources.md)
- [data/templates/generate-prd/verification.md](data/templates/generate-prd/verification.md)
- [data/templates/generate-prd/report.md](data/templates/generate-prd/report.md)

### Reviewer protocol and worker contract

- [data/reviewer-protocol.md](data/reviewer-protocol.md)
- [data/review-worker-preamble.txt](data/review-worker-preamble.txt)
- [data/review-worker-output-schema.json](data/review-worker-output-schema.json)

The shipped research skill constructs opt-in `verify` or `synthesize` bindings from the selected per-run profile and freezes one assigned worker per configured axis and required author. Review workers return judgments only; the driver owns research, artifact authoring, deterministic checks, evidence recording, and run progression. The Generate-PRD skill uses the same provider workflow for candidate extraction and requires human acceptance before PRD authority.

## Topology

`scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise`, `revise-brief`, and `revise-sources` edges. Checked `scoped` and `gathered` are schema and revision-link only. Checked `verified` and `completed` require independent review-evidence after schema and links. Check-free revise events do not evaluate artifacts.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use parent-directory segments in those links.
