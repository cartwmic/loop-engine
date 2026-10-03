# Policy document provider

## Overview

`policy-document` is the release- and source-distributed external provider for PRD section 11. It never edits the target, invokes a reviewer, or judges semantic quality. It reads exact UTF-8 target bytes, applies run-frozen deterministic policies, and aggregates externally supplied semantic verdicts bound to the current digest.

Fixed topology is `prepare` → `deterministic-review` → `semantic-review` → `end`; both revision edges are check-free. Initial input is closed JSON containing `schema_version`, `profile_version`, `mode` (`draft` or `audit`), absolute target `{id,path}`, non-empty deterministic policies, and non-empty semantic policies. Reserved `artifact_root` is accepted and ignored. Optional `target_author: {name,kind}` identifies the document author. Execution and reviewer delivery belong to the external driver. The provider is not required to write artifact files. Other unknown `initial_input` keys still fail. Agent procedure for this crate is [AGENTS.md](AGENTS.md). Drive a run with [skills/using-policy-document-provider/SKILL.md](skills/using-policy-document-provider/SKILL.md).

README profile `readme-4` keeps the title, purpose, onboarding, usage, validation, command, and local-reference deterministic floors. Its semantic axes also cover honest fitness, verifiable claims, and troubleshooting; onboarding review traces prerequisites through the first useful result, including compatible external-service setup or an explicit bring-your-own contract. AGENTS profile `agents-4` keeps the scope/authority, workflow/validation, completion/handoff, command, and local-reference floors, without a title or exact heading spelling requirement. Its semantic axes also cover non-discoverable sharp edges, ambiguity resolution, signal density, and living config; operational review checks fresh-checkout and post-edit command dependencies, and risk review checks cleanup/installation boundaries when relevant. Neither revision changes existing run-frozen inputs or adds a semantic axis. The skill describes an optional blind-first document assessment before independent repository-backed review; it is not a provider gate.

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
- `/tmp/policy-data/crates/policy-document-provider/data/target-guidance.md`

Copy chosen JSON profile, set `mode`, and replace target path with an absolute path. Keep target ID `README.md` or `AGENTS.md` for shipped profiles. Semantic policies accept `required_authors` (integer >= 1, default 1), `group` (non-empty string), and `self_review` (boolean, default false). Distinct exact `(name,kind)` identities count once; target-author evidence is excluded unless the policy allows self review. Shared groups are scoped to semantic-review and combined intake assignments carry one `assessment_id`.

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

Current-source `policy-document commission "$FROZEN_PROFILE_JSON"` consumes a completed full-show envelope (or provider context packet) to prepare explicitly selected historical review context, not a verdict. See [Explicit historical review context](skills/using-policy-document-provider/SKILL.md#explicit-historical-review-context) for selection and receipt freshness. Historical findings never become current proof by attachment.

For digest-bound evidence shape and recording, use [Evidence record](skills/using-policy-document-provider/SKILL.md#evidence-record) and [Evidence rules](skills/using-policy-document-provider/SKILL.md#evidence-rules). The provider never invokes a reviewer/model or edits the target. Candidate focused views and default compact delivery with independent full verification do not change document evidence rules or upgrade released v0.19.0 runs. Monitor/advisory output cannot approve a document; source integration is not a semantic audit.

Evidence requires exact frozen fields and enums. Each semantic axis requires its effective author floor and no current standing fail. Malformed attributable evidence blocks until any later shape-conforming record for that axis. Wrong profile, target, or digest is stale and never satisfies current conformance. Prior evidence may be reused only through explicit `evidence-applicability`; deterministic checks always rerun on current bytes.

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


Packaged archive smoke extracts `loop-engine` and `policy-document`, runs `policy-document data-dump` into an empty temporary root, then runs both modes from an empty working directory using only dumped profile bytes. macOS arm64 and Linux x86_64 archive smoke must pass before release publication.

## External review commands

`commission FROZEN_PROFILE_JSON < full-show.json` works before semantic review and reports `missing_inputs` instead of requiring a target to exist. Existing explicit historical selections remain context, not proof.

`review-candidates < packet.json` accepts `{show, assignment, author, request, result, projection?, self_review?}`. Assignment contains `gate`, `review_stage`, `subject` and `subject_revision` (both current target digest), `policy_ids`, and `assessment_id` for combined groups. Request/result contain exactly one `text` or file `reference`. Prose needs driver `projection: {items:[{policy_id,result,findings}]}`. Original request/result are retained, malformed siblings are isolated, and candidates are never appended automatically. Direct self review requires `self_review: true` and permitted policies.

`author-counts [--propose JSON] < full-show.json` inspects base/effective counts and append-only amendments. Amendment data is `{targets:[{gate:"semantic-review",axis:POLICY_ID}],before,after,actor,authority:{kind,reference},reason,corrects?}`. Authority kind is `owner` or `standing-delegation`; `after` is >= 1. Invalid amendments block named targets (unknown targets block all) until a valid correction references the invalid record ID. Count changes never remove standing failures.

Applicability data is `{origin:{kind:"context-record",id:REVIEW_RECORD_ID},target:{subject:TARGET_ID,revision:CURRENT_DIGEST},attesting_driver:{name,kind},reason}`. Origin must be earlier review-evidence from the same target/profile. Reviewer identity and verdict are retained.
