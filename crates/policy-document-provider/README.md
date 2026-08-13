# Policy document provider

`policy-document` is source-distributed external provider for PRD section 11. Fixed topology is `prepare` → `deterministic-review` → `semantic-review` → `end`; both revision edges are check-free. Initial input is closed JSON containing `schema_version`, `profile_version`, `mode` (`draft` or `audit`), absolute target `{id,path}`, non-empty deterministic policies, and non-empty semantic policies.

## Build and materialize profiles

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

## Register and start

Create `/tmp/policy-document-providers.toml`:

```toml
[providers.policy-document]
command = "/absolute/path/to/target/release/policy-document"
args = []
```

Then use fresh SQLite store and copied profile:

```sh
loop-engine --database /tmp/policy-document.sqlite --json \
  --config /tmp/policy-document-providers.toml \
  start --id docs-audit policy-document @/tmp/readme.json "README audit"
loop-engine --database /tmp/policy-document.sqlite --json show docs-audit
loop-engine --database /tmp/policy-document.sqlite --json event docs-audit ready
loop-engine --database /tmp/policy-document.sqlite --json event docs-audit passed
```

README profile supplies title, purpose, onboarding, usage, validation, command, and local-reference deterministic floors. AGENTS profile supplies scope/authority, workflow/validation, completion/handoff, command, and local-reference floors; no title or exact heading spelling is required.

## External evidence

Provider never invokes reviewer/model and never edits target. Compute digest over exact bytes (for example `shasum -a 256 /absolute/path/to/README.md`) and append one evidence record per configured semantic axis:

```sh
loop-engine --database /tmp/policy-document.sqlite --json append \
  --record-id product-fidelity-review docs-audit review-evidence \
  '{"gate":"semantic-review","policy_id":"product-fidelity","result":"pass","findings":"","author":{"name":"reviewer","kind":"agent"},"target_id":"README.md","target_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","profile_version":"readme-1"}'
```

Evidence requires exact frozen fields and enums. One current pass and no current standing fail are required per semantic axis. Malformed attributable evidence blocks until any later shape-conforming record for that axis. Wrong profile, target, or digest is stale and never satisfies current conformance. Any target byte change requires fresh evidence.

Reviewer identity, digest, and verdict remain caller claims, not signatures or provenance. Provider reads target once per evaluation but cannot lock it between evaluation and engine transition commit. Evaluation receives a fixed context snapshot; a concurrent append can be absent from an in-flight decision. Serialize `append` and `event` operations per run using one logical mutator.

Run source journey with `python3 scripts/policy-document-journey.py --engine target/debug/loop-engine --provider target/debug/policy-document --mode draft` (or `audit`).
