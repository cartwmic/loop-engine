# Research provider

## Overview

`research` is Loop Engine's reference provider for the research workflow, distributed as a standalone binary with its shipped data embedded. A repository checkout remains the development path. It implements the provider subprocess contract:

- `describe` returns the fixed workflow topology and static authoring guidance.
- `evaluate` checks the exact transition, validates configured artifact schemas and revision links, then evaluates externally supplied review evidence at verify and synthesize.

The provider does not invoke models, fetch the web, or judge semantic truth. Callers perform search, fetch, and writing outside the provider.

- Semantic review is external. The provider does not generate prompts, invoke a model, or decide whether review findings are true.
- Reviewer convergence contract lives in [data/reviewer-protocol.md](data/reviewer-protocol.md): binary evidence stays unchanged; candidate output is triaged before append or mutation; material in-scope findings require consequence proof, focused external reconsideration handles disputed candidates, and no waiver is granted by round count.

Per-run obligations live in immutable initial input. The provider is called by Loop Engine; it does not discover or load a config profile by itself.

Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-research-provider/SKILL.md](skills/using-research-provider/SKILL.md).

## Setup

Standalone releases are published for macOS arm64 and Linux x86_64. Each provider archive contains the `research` executable and both project license texts; verify its `.sha256` checksum before installation.

An installed provider binary carries its shipped workflow data. Materialize that data under a caller-chosen root:

```sh
DATA_ROOT="$HOME/.local/share/research-provider"
research data-dump "$DATA_ROOT"
```

The command creates `DATA_ROOT/crates/research-provider/...` with the profile, templates, reviewer protocol, skill, README, and AGENTS embedded in the binary. It preserves those repository-relative paths so guidance citations resolve under `DATA_ROOT`; it refuses to overwrite an existing target file. Copy the selected profile from that tree to a run-specific file, replace its placeholder `artifact_root` with an absolute artifact directory, and register the installed provider under an exact, case-sensitive alias:

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
cargo build -p loop-cli -p research-provider
ENGINE=target/debug/loop-engine
DB="$PWD/.loop-engine/loop.db"
ARTIFACT_ROOT="$PWD/research-artifacts"
PROVIDER_CONFIG="/absolute/path/to/your/providers.toml"
mkdir -p "$ARTIFACT_ROOT"
```

Copy the selected profile to a run-specific file. For installed binaries after `data-dump`, source profiles from `$DATA_ROOT`; for checkout development, set `DATA_ROOT="$PWD"` first. Before starting, replace `/abs/path/to/research/artifacts` in that copy with `$ARTIFACT_ROOT` (or another absolute artifact directory), then:

```sh
DATA_ROOT="$HOME/.local/share/research-provider"

cp "$DATA_ROOT/crates/research-provider/data/configs/standard.json" /tmp/research-standard.json
"$ENGINE" --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start research "@/tmp/research-standard.json" "research (standard)"
```

`start` returns the run ID at `result.run.id`. The CLI accepts `@FILE` JSON input as shown above. The artifact directory contains the fixed subject filenames expected by the selected schema: `brief.json`, `sources.json`, `verification.json`, and `report.json`.

## Validation

Crate tests cover describe, evaluate, shipped data, and data-dump:

```sh
cargo test -p research-provider
cargo clippy -p research-provider --all-targets -- -D warnings
```

Source and packaged journeys live in `scripts/research-journey.py` at the repository root. Crate tests remain the local proof surface for protocol, schema, evidence, and shipped data.

## Shipped data

These are the shipped files consumed by provider tests, guidance, and review procedure. The config profile is a complete initial-input template; copy it for a run and replace its placeholder `artifact_root` with an absolute existing artifact directory.

### Config profile

- [data/configs/standard.json](data/configs/standard.json) (`config_version` `research-1`; verify axes `claim-grounded` and `adversarial`; synthesize axes `cited-conclusion` and `scope-faithful`)

### Templates

- [data/templates/brief.md](data/templates/brief.md)
- [data/templates/sources.md](data/templates/sources.md)
- [data/templates/verification.md](data/templates/verification.md)
- [data/templates/report.md](data/templates/report.md)

### Reviewer protocol

- [data/reviewer-protocol.md](data/reviewer-protocol.md)

## Topology

`scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise`, `revise-brief`, and `revise-sources` edges. Checked `scoped` and `gathered` are schema and revision-link only. Checked `verified` and `completed` require independent review-evidence after schema and links. Check-free revise events do not evaluate artifacts.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use parent-directory segments in those links.
