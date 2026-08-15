# Policy document provider

## Overview

`policy-document` is the release- and source-distributed external provider for PRD section 11. It never edits the target, invokes a reviewer, or judges semantic quality. It reads exact UTF-8 target bytes, applies run-frozen deterministic policies, and aggregates externally supplied semantic verdicts bound to the current digest.

Fixed topology is `prepare` → `deterministic-review` → `semantic-review` → `end`; both revision edges are check-free. Initial input is closed JSON containing `schema_version`, `profile_version`, `mode` (`draft` or `audit`), absolute target `{id,path}`, non-empty deterministic policies, and non-empty semantic policies. Reserved `artifact_root` is accepted and ignored; omit it in the usual case. The provider is not required to write artifact files. Other unknown `initial_input` keys still fail. Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-policy-document-provider/SKILL.md](skills/using-policy-document-provider/SKILL.md).

README profile `readme-2` supplies title, purpose, onboarding, usage, validation, command, and local-reference deterministic floors; those floors are unchanged. Semantic axes add honest fitness, verifiable claims, and troubleshooting sharp edges, and tighten audience navigation so README does not impersonate AGENTS.md. AGENTS profile `agents-2` supplies scope/authority, workflow/validation, completion/handoff, command, and local-reference floors; those floors are unchanged, and no title or exact heading spelling is required. Semantic axes add non-discoverable sharp edges, ambiguity resolution, signal density, and living config, and tighten operational precision, authority resolution, and risk-boundary sufficiency.

## Setup

```sh
cargo build --release -p loop-cli -p policy-document-provider
./target/release/policy-document data-dump /tmp/policy-data
```

Dump refuses to overwrite any directory entry, including dangling symlinks. On write failure, rollback removes only files created by that invocation; caller-owned files and destination entries remain untouched. Shipped files appear at:

- `/tmp/policy-data/crates/policy-document-provider/data/readme.json`
- `/tmp/policy-data/crates/policy-document-provider/data/agents.json`
- `/tmp/policy-data/crates/policy-document-provider/data/reviewer-protocol.md`
- `/tmp/policy-data/crates/policy-document-provider/data/target-guidance.md`

Copy chosen JSON profile, set `mode`, and replace target path with absolute path. Keep target ID `README.md` or `AGENTS.md` for shipped profiles.

## Usage

Create `/tmp/policy-document-providers.toml`:

```toml
[providers.policy-document]
command = "/absolute/path/to/target/release/policy-document"
args = []
```

Then start with the copied profile. Omit `--database` unless isolating, and omit `artifact_root` (the reserved key is accepted and ignored; the provider is not required to write artifact files):

```sh
loop-engine --json --config /tmp/policy-document-providers.toml \
  start --id docs-audit policy-document @/tmp/readme.json "README audit"
loop-engine --json show docs-audit
loop-engine --json event docs-audit ready
loop-engine --json event docs-audit passed
```

Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`. `start` returns the run ID at `result.run.id`. Request `ready` after authoring the target, then checked `passed` for deterministic review. On `policy-document-nonconforming`, fix every reported violation, request check-free `revise`, and repeat from `prepare`.

## External evidence

Provider never invokes reviewer/model and never edits target. Compute digest over exact bytes (for example `shasum -a 256 /absolute/path/to/README.md`) and append one evidence record per configured semantic axis:

```sh
loop-engine --json append \
  --record-id product-fidelity-review docs-audit review-evidence \
  '{"gate":"semantic-review","policy_id":"product-fidelity","result":"pass","findings":"","author":{"name":"reviewer","kind":"agent"},"target_id":"README.md","target_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","profile_version":"readme-2"}'
```

Evidence requires exact frozen fields and enums. One current pass and no current standing fail are required per semantic axis. Malformed attributable evidence blocks until any later shape-conforming record for that axis. Wrong profile, target, or digest is stale and never satisfies current conformance. Any target byte change requires fresh evidence.

Reviewer identity, digest, and verdict remain caller claims, not signatures or provenance. Provider reads target once per evaluation but cannot lock it between evaluation and engine transition commit. Evaluation receives a fixed context snapshot; a concurrent append can be absent from an in-flight decision. Serialize `append` and `event` operations per run using one logical mutator.

## Validation

Run source journeys against shipped profile bytes:

```sh
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

Packaged archive smoke extracts `loop-engine` and `policy-document`, runs `policy-document data-dump` into an empty temporary root, then runs both modes from an empty working directory using only dumped profile bytes. macOS arm64 and Linux x86_64 archive smoke must pass before release publication.
