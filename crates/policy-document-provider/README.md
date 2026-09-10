# Policy document provider

## Overview

`policy-document` is the release- and source-distributed external provider for PRD section 11. It never edits the target, invokes a reviewer, or judges semantic quality. It reads exact UTF-8 target bytes, applies run-frozen deterministic policies, and aggregates externally supplied semantic verdicts bound to the current digest.

Fixed topology is `prepare` → `deterministic-review` → `semantic-review` → `end`; both revision edges are check-free. Initial input is closed JSON containing `schema_version`, `profile_version`, `mode` (`draft` or `audit`), absolute target `{id,path}`, non-empty deterministic policies, and non-empty semantic policies. Reserved `artifact_root` and `work_slot_bindings` are accepted and ignored by this provider. This provider-only treatment does not bypass the engine's binding validation at normal `start`; arbitrary binding JSON is not a supported engine-start contract. The provider is not required to write artifact files. Other unknown `initial_input` keys still fail. Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-policy-document-provider/SKILL.md](skills/using-policy-document-provider/SKILL.md).

README profile `readme-2` supplies title, purpose, onboarding, usage, validation, command, and local-reference deterministic floors; those floors are unchanged. Semantic axes add honest fitness, verifiable claims, and troubleshooting sharp edges, and tighten audience navigation so README does not impersonate AGENTS.md. AGENTS profile `agents-2` supplies scope/authority, workflow/validation, completion/handoff, command, and local-reference floors; those floors are unchanged, and no title or exact heading spelling is required. Semantic axes add non-discoverable sharp edges, ambiguity resolution, signal density, and living config, and tighten operational precision, authority resolution, and risk-boundary sufficiency.

## Setup

Run from the repository root with [rustup](https://rustup.rs/) and native C compiler/linker tools installed (macOS Command Line Tools or the Linux build toolchain). The checkout pins Rust 1.98.0, not a public minimum supported version.

```sh
cargo build --release -p loop-cli -p policy-document-provider
./target/release/policy-document data-dump /tmp/policy-data
```

Dump refuses to overwrite any directory entry, including dangling symlinks. On write failure, rollback removes only files created by that invocation; caller-owned files and destination entries remain untouched. Shipped files appear at:

- `/tmp/policy-data/crates/policy-document-provider/data/readme.json`
- `/tmp/policy-data/crates/policy-document-provider/data/agents.json`
- `/tmp/policy-data/crates/policy-document-provider/data/reviewer-protocol.md`
- `/tmp/policy-data/crates/policy-document-provider/data/semantic-review-worker-preamble.md`
- `/tmp/policy-data/crates/policy-document-provider/data/semantic-review-worker-output-schema.json`
- `/tmp/policy-data/crates/policy-document-provider/data/target-guidance.md`

Copy chosen JSON profile, set `mode`, and replace target path with an absolute path. Keep target ID `README.md` or `AGENTS.md` for shipped profiles. The shipped skill uses the dumped worker files and that same selected profile to construct assigned semantic-review workers, then previews and hash-confirms the resulting profile before starting it unchanged. Current semantic policies reject `required_authors` as an unknown field: the provider requires at least one current pass and no standing fail per axis, not a configurable author floor. The generic constructor's roster/count handling does not extend that public input contract.

## Usage

Create `/tmp/policy-document-providers.toml`:

```toml
[providers.policy-document]
command = "/absolute/path/to/target/release/policy-document"
args = []
```

Before starting with the copied profile, load [Setup](skills/using-policy-document-provider/SKILL.md#setup), including canonical engine Deterministic setup. Use the normal user catalog: no database or artifact override unless the human explicitly requests isolation in this session; other runs and old preferences confer no permission. The engine owns durable run storage and records/injects `artifact_root`; this provider ignores that reserved field and reads absolute `target.path`.

```sh
./target/release/loop-engine --json --config /tmp/policy-document-providers.toml \
  start --id docs-audit policy-document @/tmp/readme.json "README audit"
./target/release/loop-engine --json show docs-audit
```

`start` returns the run ID at `result.run.id`. Draft means authoring/revision intent; audit means assessment of existing bytes. Both have identical provider mechanics and neither enforces caller read-only work. Corrections remain caller-owned and require authorization. Follow the skill's [Run loop](skills/using-policy-document-provider/SKILL.md#run-loop) for observation, review and revision.

## External evidence

Current-source `policy-document commission "$FROZEN_PROFILE_JSON"` consumes a completed full-show envelope (or bound filter packet) to prepare explicitly selected historical review context, not a verdict. See [Explicit historical review context](skills/using-policy-document-provider/SKILL.md#explicit-historical-review-context) for selection and receipt freshness. Historical findings never become current proof by attachment.

For digest-bound evidence shape and recording, use [Evidence record](skills/using-policy-document-provider/SKILL.md#evidence-record) and [Evidence rules](skills/using-policy-document-provider/SKILL.md#evidence-rules). The provider never invokes a reviewer/model or edits the target. Candidate focused views and default compact delivery with independent full verification do not change document evidence rules or upgrade released v0.19.0 runs. Monitor/advisory output cannot approve a document; source integration is not a semantic audit.

Evidence requires exact frozen fields and enums. One current pass and no current standing fail are required per semantic axis. Malformed attributable evidence blocks until any later shape-conforming record for that axis. Wrong profile, target, or digest is stale and never satisfies current conformance. Any target byte change requires fresh evidence.

Reviewer identity, digest, and verdict remain caller claims, not signatures or provenance. Provider reads target once per evaluation but cannot lock it between evaluation and engine transition commit. Evaluation receives a fixed context snapshot; a concurrent append can be absent from an in-flight decision. Serialize `append` and `event` operations per run using one logical mutator.

## Limitations

The bounded Markdown parser recognizes ATX headings outside fences, and closed fenced blocks containing a nonblank, noncomment line—not executable commands. It checks inline links/images and line-leading `@` imports, not reference-style or HTML links. It does not execute commands or validate remote links, anchors, or every Markdown reference. Local references must remain within the target directory; percent-encoded and absolute paths are rejected. The resolver can normalize `sub/../file` inside that directory, but crate authoring rules prohibit `..` in Markdown links outright.

## Validation

Run source journeys against shipped profile bytes:

```sh
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/release/loop-engine \
    --provider target/release/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

The software-change journey `--self-test` executes this crate's semantic-review constructor against the shipped readme and agents profile shapes (temporary absolute targets) and prints `worker-data skill/root policy assertions passed` only after those fixtures, required keys/data bytes/preview visibility, and fail-closed invalid cases pass.

Packaged archive smoke extracts `loop-engine` and `policy-document`, runs `policy-document data-dump` into an empty temporary root, then runs both modes from an empty working directory using only dumped profile bytes. macOS arm64 and Linux x86_64 archive smoke must pass before release publication.
