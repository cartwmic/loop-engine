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

Standalone releases are published for macOS arm64 and Linux x86_64. At [Loop Engine releases](https://github.com/cartwmic/loop-engine/releases), choose one release tag and download its matching platform archives for both `research-provider` and `loop-cli` (the latter contains `loop-engine`). Verify the published `.sha256` checksums and place both executables on PATH. No development build is required for binary users.

From that same release page, download and extract GitHub's **Source code** archive for the same tag. It contains the required engine companion `skills/using-loop-engine/SKILL.md` and its related `docs/` guidance. Read those matching files together with the provider skill before start; do not substitute latest-main guidance for an installed release. Keep this guidance checkout separate from the chosen `DATA_ROOT` below: `research data-dump` supplies only the provider tree, not the engine companion.

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
# Run from the repository root.
cargo build -p research-provider
DATA_ROOT="$PWD"
```

This produces `target/debug/research`; the checkout's `crates/research-provider/data/` tree supplies the development copies of shipped data.

## Usage

Build the engine binary too, or replace `target/debug/loop-engine` below with an installed `loop-engine` executable:

```sh
cargo build -p loop-cli -p research-provider -p bookends-check
ENGINE=target/debug/loop-engine
DATA_ROOT="$PWD"
# Register command = "/absolute/checkout/target/debug/research" for this build.
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
```

Copy the selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. Before start, load [Setup](skills/using-research-provider/SKILL.md#setup), including canonical engine Deterministic setup. Use the normal user catalog: no database or artifact override unless the human explicitly requests isolation in this session; other runs and old preferences confer no permission. Then:

```sh
cp "$DATA_ROOT/crates/research-provider/data/configs/standard.json" /tmp/research-standard.json
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-standard.json" "research (standard)"
```

`start` returns the run ID at `result.run.id`; `@FILE` supplies JSON input. The engine allocates `artifact_root` in object `initial_input`, visible through `show`. Closed initial-input schemas must allow that reserved field. Subject filenames are `brief.json`, `sources.json`, `verification.json`, and `report.json`.

For observation, invocation evidence and progression, follow [AGENTS.md](AGENTS.md) and the skill's [Required companion and engine driving minimum](skills/using-research-provider/SKILL.md#required-companion-and-engine-driving-minimum). Current-source focused views and default compact delivery with independent full verification are candidate functionality, not upgrades to released v0.19.0 runs. Monitors and advisory summaries never supply research judgments or progression.

## Compatibility limits

Artifact schemas use a bounded language, not general JSON Schema: object supports `type`, `properties`, `required`, `additionalProperties`; array supports `type`, `items`, `minItems`; string supports `type`, `enum`, `minLength`. Numeric/boolean schemas, `$ref`, `oneOf`, and `pattern` are unsupported. See [the schema implementation](src/schema.rs).

## Validation

From the repository root, the fresh-binary central runner covers the research integration suites, including describe, evaluate, shipped data and data-dump. The package command covers units only (`autotests = false`):

```sh
python3 scripts/run-nextest.py --filter research
cargo test -p research-provider --lib
cargo clippy -p research-provider --all-targets -- -D warnings
```

Source and packaged journeys live in `scripts/research-journey.py` at the repository root. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. The central integration suites are the local proof surface for protocol, schema, evidence and shipped data; focused checks supplement the workspace completion gate in repository-root AGENTS.md. `scripts/assert-generate-prd-profile.py` proves that the Generate-PRD profile uses the existing research binary and does not add a provider. The source Generate-PRD journey drives that profile with deterministic bound workers, writes `prd-candidate.md` plus exact repository evidence, reaches `end`, and runs `bookends-check candidate`; it does not edit `docs/PRD.md` or commit. The software-change journey `--self-test` executes this crate's verify and synthesize constructors against `data/configs/standard.json` and prints `worker-data skill/root policy assertions passed` only after worker count/order, axis/`example_prompt`/author/model metadata, required keys/data bytes/preview visibility, and fail-closed invalid cases pass.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. The config profile is a complete initial-input template; copy it for a run.

### Config profiles

- [data/configs/standard.json](data/configs/standard.json) (`config_version` `research-1`; verify axes `claim-grounded` and `adversarial`; synthesize axes `cited-conclusion` and `scope-faithful`)
- [data/configs/generate-prd.json](data/configs/generate-prd.json) (the same research topology and schemas, with Generate-PRD templates)

The synthetic Generate-PRD journey fixture performs three predetermined repository lookups; that bounded test is not an exhaustive semantic audit. Real extraction follows the [Generate-PRD skill](skills/using-generate-prd/SKILL.md), which requires repository discovery without a predetermined requirement list. Generate-PRD candidate IDs are provisional proposals. A human must accept or reject `prd-candidate.md` before any commit to `docs/PRD.md`; the profile never auto-commits or claims semantic completeness.

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
