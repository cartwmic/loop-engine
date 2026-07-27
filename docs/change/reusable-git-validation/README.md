# Reusable Git Validation Change

**Status:** Proposed

This change replaces the repository's large, project-specific validation framework with a narrow Rust `xtask`, typed repository configuration, tracked Git hooks, ordinary ecosystem commands, and publication-only semantic review.

The result is reusable inside this repository without becoming a general workflow engine. `xtask` owns Git and process mechanics. `quality/manifest.toml`, normal Rust tests, and semantic rubrics own loop-engine policy.

No OpenSpec artifacts belong to this change.

## Problem

Current validation combines several unrelated responsibilities:

- exact staged-tree and revision materialization;
- Git hook and push-range handling;
- process execution and evidence capture;
- hardcoded Cargo command dispatch;
- exhaustive dependency allowlists;
- documentation filename and terminology policy;
- source-text architecture scanning;
- operation/facet inventory scanning;
- initial-release acceptance-report generation;
- semantic-judge invocation;
- parent-policy bootstrap and anti-weakening rules.

The unusual Git mechanics are valuable. Much of the project-policy code is duplicate, brittle, or tied to the completed initial release. Evaluated hook managers (`pre-commit`, `prek`, Lefthook, and `hk`) can replace the small hook adapters, but none natively provides the required exact-index candidate, aggregate one-tip push verdict, semantic fan-out/coherence, and report-bound owner approval. Adding one would leave the hard part custom while adding another dependency.

## Settled context

- Repository is private, local-first, and maintained by a cooperative owner.
- Direct owner pushes are normal publication flow.
- Pre-push is the active local publication gate.
- CI independently verifies the pushed revision after publication; it cannot prevent an already accepted direct push without branch protection.
- Hosted authority, hostile-candidate isolation, multi-user approval, and branch protection are outside scope.
- macOS and Linux are supported.
- Checks never fix, rewrite, or stage source.
- Candidate commands and rubrics apply to the candidate immediately.
- Generic validation mechanics contain no Cargo, crate, operation, rubric, or documentation policy.

## Design principles

1. **Validate Git objects, not editor state.** Pre-commit validates the index tree; publication validates the resulting pushed tip.
2. **Configuration owns commands.** Runner executes typed `program` and argv declarations without a shell.
3. **Tests own product invariants.** Stable executable product rules belong in ordinary Rust tests reached by `cargo test`.
4. **Semantic policy stays semantic.** Documentation quality, internal architectural placement, and behavioral-evidence sufficiency belong to independent semantic axes.
5. **Reports remain failures.** Approval references a failed report; it never rewrites the report into a pass.
6. **Owner escape hatch is explicit.** Semantic failure can be approved with exact evidence and reason. Deterministic failure cannot.
7. **Keep `xtask` project-sized.** Do not create a plugin system, hook manager, remote service, policy language, or second workflow engine.

## Architecture boundary

### `xtask` owns

- locating repository and Git common directories;
- parsing hook input;
- resolving base, candidate commit, and candidate tree;
- materializing exact candidate trees;
- loading and validating typed configuration from the candidate;
- expanding a small fixed placeholder set;
- spawning programs directly with argv;
- environment addition/removal, cwd, timeout, process-group termination, and bounded output;
- stable changed-file delivery;
- deterministic result aggregation;
- semantic process scheduling and response validation;
- canonical report serialization, hashing, and Git-local storage;
- exact report-bound approval lookup;
- hook installation.

### Repository configuration owns

- concrete programs and arguments;
- check order;
- phases and scopes;
- timeouts and output bounds;
- project environment changes;
- Cargo, Go, and dependency-policy commands;
- semantic axis inventory;
- rubric paths;
- judge executable and model adapter configuration.

### Ordinary tests own

- inward product-crate dependency direction;
- CLI as sole product binary;
- operation catalog/driver/route/E2E/trace/facet equality;
- schemas and protocol behavior;
- all other executable product contracts.

### Semantic rubrics own

- documentation impact and consistency;
- observability consequences;
- internal core layer direction;
- provider-process, persistence, composition, and dispatch placement;
- architecture/tenet/KISS judgment;
- behavioral-evidence sufficiency;
- final cross-axis coherence.

### Final `xtask` module boundary

Keep one flat module layer:

| File | Responsibility |
|---|---|
| `config.rs` | Manifest v2 parsing, validation, and typed configuration. |
| `git.rs` | Typed `/usr/bin/git` queries and hook-input parsing primitives. |
| `candidate.rs` | Candidate resolution, materialization, runner-input parity, and cleanup. |
| `process.rs` | Direct child execution, environment, timeout, process group, and lossless bounded streams. |
| `quality.rs` | Deterministic phase scheduling and result aggregation only. |
| `semantic_judge.rs` | Axis/coherence request construction, scheduling, and response validation. |
| `report.rs` | Canonical evaluation, approval, and publication-attempt records plus Git-local storage. |
| `publication.rs` | One-tip publication lifecycle and mechanical gate decision. |
| `hooks.rs` | Installation and thin pre-commit/pre-push entry points. |
| `lib.rs`, `main.rs` | CLI parsing and dispatch. |

Do not add runner traits, plugins, nested policy modules, check enums, project-specific command functions, or source scanners. Metadata architecture enforcement lives at `xtask/tests/workspace_architecture.rs`; it is test policy, not shipping runner behavior.

## Candidate model

### Pre-commit

1. Resolve `HEAD` as base; in an unborn repository normalize missing `HEAD` to Git's empty-tree object.
2. Run `/usr/bin/git write-tree` against the real index to obtain candidate tree. For this phase, `candidate_revision` is this tree object ID and equals `candidate_tree`.
3. Compute changed paths from normalized base object to candidate tree.
4. Materialize candidate tree into a temporary directory using a temporary Git index; reject absolute or escaping symlink targets.
5. Load candidate `quality/manifest.toml` from that directory and verify configured runner inputs match candidate content.
6. Make materialized source read-only and provide separate writable scratch, cache, and target directories.
7. Run the complete deterministic `pre-commit` phase from materialized content. After every spawned process and before deriving the final result, recompute and verify the materialized source tree and modes against `candidate_tree`; fail closed before starting another process if they differ.
8. Remove temporary state on success, failure, or interruption.

Unstaged tracked content and untracked files never enter the candidate. Every configured command starts beneath candidate root. Git-object checks receive explicit Git-directory and object operands; they never need real worktree cwd.

### Publication / pre-push

Git supplies update lines on stdin:

```text
<local-ref> <local-sha> <remote-ref> <remote-sha>
```

The runner parses every line before deciding disposition.

- Zero non-delete updates: allow without tree checks.
- One non-delete update: validate its resulting destination tip.
- More than one non-delete update: reject with instructions to split the push.
- Force push: allowed; validate resulting local tip rather than requiring ancestry.
- Duplicate updates resolving to the same destination tree still count as multiple content tips and are rejected for clarity.

For the accepted content update, the advertised remote SHA is base and local SHA is candidate. A new branch normalizes the advertised all-zero absence to Git's empty-tree object; that normalized object is `base_revision` for commands, semantic requests, reports, and approval binding. Deterministic and semantic execution perform the same post-process source-tree verification used by pre-commit. One push produces one aggregate publication verdict.

## Typed manifest

`quality/manifest.toml` changes incompatibly to schema version 2. Unknown keys and values fail closed.

Illustrative shape:

```toml
schema_version = 2

[defaults]
timeout_seconds = 900
max_output_bytes = 8388608

[defaults.environment]
unset = ["RUSTUP_TOOLCHAIN"]

[defaults.environment.set]
CARGO_TARGET_DIR = "{target_root}/cargo"
GOCACHE = "{cache_root}/go-build"
TMPDIR = "{scratch_root}/tmp"

[runner]
inputs = [
  ".cargo",
  ".githooks",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "xtask",
  "quality",
]

[[prerequisites]]
id = "cargo-deny"
program = "cargo"
args = ["deny", "--version"]
stdout_equals = "cargo-deny 0.20.2"
install_hint = "cargo install cargo-deny --locked --version 0.20.2"

[[prerequisites]]
id = "go-1.26.5"
program = "mise"
args = ["where", "go@1.26.5"]
install_hint = "mise install go@1.26.5"

[[checks]]
id = "diff-check"
phases = ["pre-commit", "publication"]
scope = "changed-files"
program = "/usr/bin/git"
args = ["--git-dir={git_directory}", "diff", "--check", "{base_revision}", "{candidate_revision}", "--"]
cwd = "{candidate_root}"

[[checks]]
id = "workspace-test"
phases = ["pre-commit", "publication"]
scope = "repository"
program = "cargo"
args = ["test", "--workspace", "--locked"]
cwd = "{candidate_root}"
timeout_seconds = 600

[semantic]
program = "{candidate_root}/quality/semantic-judge/v2/judge"
args = []
cwd = "{candidate_root}"
timeout_seconds = 900
response_schema = "quality/semantic-judge/v2/response.schema.json"

[semantic.environment]
unset = []

[semantic.environment.set]
TMPDIR = "{scratch_root}/tmp"

[[semantic.axes]]
id = "documentation"
rubric = "quality/rubrics/documentation.md"

[[semantic.axes]]
id = "observability"
rubric = "quality/rubrics/observability.md"

[[semantic.axes]]
id = "architecture"
rubric = "quality/rubrics/architecture.md"

[[semantic.axes]]
id = "behavioral-evidence"
rubric = "quality/rubrics/behavioral-evidence.md"

[semantic.coherence]
id = "coherence"
rubric = "quality/rubrics/coherence.md"
```

Final field names are frozen by the schema task in [tasks.md](tasks.md). The semantics below are mandatory. A deterministic-only invocation may parse a manifest without `[semantic]`; publication and explicit advisory invocation require one complete semantic program, exactly the configured axes, and coherence declaration. Final implementation has one v2 configuration path; no v1 compatibility dispatch or duplicate semantic registry remains.

### Check fields

| Field | Rule |
|---|---|
| `id` | Non-empty, unique stable identifier. |
| `phases` | Non-empty subset of `pre-commit`, `publication`. CI invokes `publication`. |
| `scope` | `repository` or `changed-files`. |
| `program` | One executable; no shell parsing. Bare names resolve through `PATH`; paths resolve after placeholder expansion. |
| `args` | Ordered argv values. Changed-file scope appends sorted repository-relative paths as separate argv entries. |
| `cwd` | Existing directory beneath candidate root. Escapes and real repository worktree cwd are rejected. |
| `timeout_seconds` | Positive bounded override. |
| `max_output_bytes` | Positive per-stream bound. Exceeding it terminates and fails the check. |
| environment additions/removals | Applied after inheriting caller environment; removal wins on duplicate key. |

Prerequisites are non-mutating direct process probes. `hooks install` and each validation entry point that loads a candidate manifest require every probe to pass. Deletion-only and rejected publication attempts load no manifest and run no probe, deterministic command, or semantic command; they may invoke only authoritative `/usr/bin/git rev-parse --git-common-dir` to locate evidence storage. `stdout_equals`, when present, compares one trailing-newline-trimmed UTF-8 line exactly. Failure prints `install_hint` but never executes it.

`runner.inputs` identifies tracked source/configuration that can change validation behavior. Before executing checks, pre-commit requires those working-tree paths to match candidate index content; pre-push requires them to match candidate commit content. Pre-push runner parity is supported only when candidate commit equals `HEAD`; pushing another checked-out branch's commit fails with exact instructions to check out that candidate tip and retry. Any mismatch is an unapprovable deterministic block: owner must restore or stash mismatching runner inputs and retry. An existing semantic approval remains reusable when base/candidate/config/rubric bindings are unchanged. This protects exact-candidate behavior from accidental unstaged runner edits without claiming hostile-candidate security.

Changed-file checks with an empty path set are recorded as skipped-success and are not spawned. Repository checks always run. Changed paths must be valid UTF-8; an unsupported path encoding fails closed with a diagnostic rather than entering a lossy report.

### Placeholders

Only these whole-value or embedded placeholders are supported:

- `{git_directory}`
- `{candidate_root}`
- `{scratch_root}`
- `{cache_root}`
- `{target_root}`
- `{base_revision}`
- `{candidate_revision}`
- `{candidate_tree}`

Unknown placeholders fail configuration validation. Placeholder expansion never invokes a shell. `{git_directory}` is absolute active-repository Git directory returned by authoritative `/usr/bin/git rev-parse --absolute-git-dir`; it is distinct from Git common directory used for evidence storage. Runner creates candidate-external writable scratch, cache, and target roots before checks; repository configuration—not Rust branches—maps tool-specific environment such as `CARGO_TARGET_DIR`, `GOCACHE`, and `TMPDIR` into them. Checks receive no writable source path. For pre-commit, `base_revision` is `HEAD` or the empty-tree object in an unborn repository, while `candidate_revision` and `candidate_tree` are both the index-derived tree object. For publication, base and candidate revisions follow the pre-push rules above and `candidate_tree` is the candidate commit's tree.

## Deterministic suite

Both pre-commit and publication run the complete suite in manifest order while collecting all failures:

1. `/usr/bin/git diff --check` over exact base and candidate objects;
2. `cargo fmt --all --check`;
3. `cargo check --workspace --locked`;
4. `cargo clippy --workspace --all-targets -- -D warnings`;
5. `cargo test --workspace --locked`;
6. `cargo doc --workspace --no-deps --locked`;
7. `cargo test --manifest-path test-support/providers/reference-provider/Cargo.toml --locked`;
8. `cargo test --manifest-path test-support/providers/scenario-provider/Cargo.toml --locked`;
9. Go reference-provider tests through `mise` with Go 1.26.5;
10. pinned `cargo deny check`.

The manifest—not Rust dispatch code—contains these commands.

Tool prerequisites are explicit and non-mutating:

- local and CI cargo-deny version is exactly `0.20.2`;
- Go 1.26.5 must already exist in `mise` before a hook runs;
- Go checks set both `MISE_AUTO_INSTALL=false` and `MISE_AUTO_INSTALL_DISABLE_TOOLS=go`;
- missing tools fail with setup instructions rather than installing during commit/push.

A deterministic result records:

- check ID;
- program and exact argv;
- cwd and declared environment changes;
- candidate binding;
- exit status or spawn/timeout/output-limit failure;
- lossless bounded stdout and stderr records;
- elapsed duration in integer milliseconds.

Each stream record contains encoding (`utf-8` or `base64`), exact captured bytes, and `complete`. Invalid UTF-8 uses base64. Crossing one stream's bound kills the process, sets `complete=false`, and records `output_limit`; the report never claims bytes emitted after termination were captured.

Any deterministic non-pass blocks. No approval command accepts such a report.

## Architecture validation

The current architecture checker contains valuable policy mixed with brittle source-text scanning and unrelated dependency governance. Replacement is deliberately narrower.

A small normal Rust test uses Cargo metadata to enforce:

- `loop-engine-core` has no normal dependency on integrations or CLI;
- `loop-engine-integrations` may depend inward on core but not outward on CLI;
- CLI may depend on both inward crates;
- CLI is the sole product crate with a binary target.

The test does not freeze an exact number of future product crates and does not enumerate allowed third-party libraries.

The following remain requirements but move to the publication architecture rubric:

- `model` must not depend on capabilities or operations;
- capabilities must not depend on operations;
- core must not construct provider processes or persistence integrations;
- integrations must keep process and SQLite construction in their owned adapters;
- CLI concrete construction stays in composition and operation-root dispatch stays in dispatch;
- raw integration details do not leak inward.

The generic runner knows none of these names.

## Semantic topology

Hooks and CI run semantic review only for publication. An owner may invoke the same pipeline explicitly with:

```text
cargo xtask validate --semantic --base <sha> --candidate <sha>
```

The advisory command requires candidate commit=`HEAD` and applies the same runner-input parity rule as pre-push; base may be any revision. It runs the complete manifest `publication` deterministic phase first, stores an evaluation report, writes no publication-attempt record, and does not consult approvals.

1. Deterministic suite must pass first.
2. Every configured axis runs, concurrently, against the same exact candidate.
3. Each axis receives only its own rubric plus shared revision, diff/resulting content, and deterministic evidence.
4. Malformed output receives one correction attempt within the original timeout.
5. Each invocation normalizes to one of:
   - `pass`
   - `block`
   - `indeterminate`
   - `unavailable`
6. Final coherence judge receives normalized results from every axis and the same candidate binding.
7. Coherence may identify an additional blocker but cannot upgrade or erase any axis non-pass.
8. Runner derives final disposition mechanically; model output never chooses override eligibility.

The current single aggregate semantic call is replaced by a versioned v2 request/response contract supporting axis and coherence request kinds. Exactly four initial axes remain configured: documentation, observability, architecture, and behavioral evidence.

`quality/manifest.toml` is the sole authoritative axis/rubric inventory. `quality/rubrics/manifest.json` and semantic-v1 dispatch are removed from active validation rather than retained as compatibility inputs. The semantic executable receives one canonical JSON request on stdin and returns one JSON response on stdout. Only fixed typed `args` from candidate config are used; requests never alter argv. `cwd` must resolve beneath candidate root. Environment inherits caller values then applies typed semantic set/unset values. For each invocation, `{scratch_root}` resolves to a distinct writable subdirectory: one per axis and another for coherence. Axis scratch paths are never shared.

After each semantic process exits, the runner verifies materialized source content and modes against `candidate_tree`. A mismatch terminates in-flight axis/correction process groups, starts no further child, and mechanically records unfinished axes plus coherence as `unavailable` with source-mutation evidence. Those synthetic normalized results satisfy report completeness but force `semantic_block`; coherence executable does not run against changed source.

## Evaluation and publication records

Tracked schemas and executable protocol contracts live at:

- `quality/validation/v2/` — manifest schema and runner contract;
- `quality/semantic-judge/v2/` — semantic executable contract;
- `quality/publication-report/v1/` — evaluation, approval, and attempt schemas.

Runtime records remain untracked under Git common-directory paths described below; tracked schema directories never contain runtime evidence.

A semantic evaluation report contains at least:

```text
schema_version
base_revision
candidate_revision
candidate_tree
manifest_digest
rubric_digests
deterministic_results
axis_results
coherence_result | null
derived_disposition
```

For `deterministic_block`, `axis_results` is empty and `coherence_result` is null because semantic execution did not start. Content publication stores this evaluation report and references it from its blocked attempt. For semantic dispositions, all configured axis results and one coherence result are required.

`derived_disposition` is one of:

- `pass`
- `deterministic_block`
- `semantic_block`

The typed-config module owns binding digest computation: `manifest_digest` is SHA-256 of exact candidate `quality/manifest.toml` bytes; `rubric_digests` is a stable path-sorted map from each manifest-declared repository-relative rubric path to SHA-256 of its exact candidate bytes. Evidence and publication modules consume this API and never reimplement hashing.

Rules:

- any deterministic non-pass yields `deterministic_block` and no semantic invocation;
- otherwise any axis or coherence non-pass yields `semantic_block`;
- only all-pass results yield `pass`.

A publication-attempt record contains:

```text
schema_version
update_kind
input_kind
input_evidence
updates
rejection_code | null
base_revision | null
candidate_revision | null
candidate_tree | null
manifest_digest | null
rubric_digests | null
fresh_deterministic_results
evaluation_report_digest | null
approval_digest | null
derived_disposition
gate_decision
created_at
```

`input_kind` is `git_update_lines` or `ci_push_event`. `input_evidence` always losslessly stores exact source bytes as UTF-8/base64. For `git_update_lines`, source is exact stdin; both pre-push and direct fixtures use `validate --publication --updates-stdin`. For a valid `ci_push_event`, source bytes must be canonical JSON with exactly `before`, `after`, and `ref`, supplied through `validate --publication --ci-event <path>`; malformed CI files retain their exact bytes in rejected evidence. CI deterministically projects a valid object into one update tuple with local/remote ref=`ref`, local SHA=`after`, and remote SHA=`before`. `updates` is the canonical parsed, stable-ordered update-tuple set when parsing succeeds. No third input projection is allowed.

`update_kind` is `content`, `deletion_only`, or `rejected`. The report contract owns this frozen exhaustive mapping and every producer uses it unchanged:

| `rejection_code` | Exact condition |
|---|---|
| `malformed_update_input` | Git-update stdin is non-UTF-8 or cannot be tokenized as newline-separated four-field records. |
| `invalid_update_shape` | Four fields parse, but ref names, object IDs, zero-ID placement, or delete/new/update combination is invalid. |
| `multiple_content_tips` | Parsed valid input contains more than one non-delete destination tip. |
| `malformed_ci_event` | CI event file is non-UTF-8, invalid JSON, has unknown/missing fields, or has invalid `before`/`after`/`ref` shape. |

For `content`, revision/tree/config fields are non-null, `rejection_code` is null, and existing evaluation/approval rules apply. `invalid_update_shape` retains tokenized four-field tuples in `updates`; only `malformed_update_input` and `malformed_ci_event` may have an empty `updates` array. For `deletion_only`, `rejection_code` and all revision/tree/config/rubric/evaluation/approval fields are null, deterministic results are empty, `derived_disposition=pass`, and `gate_decision=pass`; no candidate manifest or semantic executable is loaded. Empty Git-update stdin is represented by this same variant with an empty `updates` array. For `rejected`, `rejection_code` is non-null, revision/tree/config/rubric/evaluation/approval fields are null, deterministic results are empty, `derived_disposition=deterministic_block`, and `gate_decision=block`; malformed input may have an empty `updates` array because exact bytes remain in `input_evidence`. Any other nullability combination fails schema validation.

`gate_decision` is `pass`, `approved`, or `block`. It is `approved` only when fresh deterministic checks pass and an exact valid approval authorizes the referenced report's `semantic_block`. Thus failed semantic judgment stays failed while publication authorization remains explicit.

All records serialize deterministically as UTF-8 JSON. SHA-256 of exact bytes is each record's external identifier; digest is not embedded in hashed bytes. Local records live under:

```text
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/reports/<report-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/approvals/<report-digest>/<approval-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/content/<candidate-tree>/<attempt-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/deletions/<attempt-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/rejected/<attempt-digest>.json
```

One push invocation writes one aggregate publication-attempt record, including malformed and multi-tip rejections. Evaluation and approval records are immutable supporting evidence. CI uploads its evaluation and attempt records as workflow artifacts and creates no local approval.

## Owner approval

Command:

```text
cargo xtask validation approve --report <report-digest> --reason <non-empty text>
```

A valid approval records:

- schema version;
- freshly generated UUIDv7 `approval_id`;
- report digest;
- base revision;
- candidate revision and tree;
- manifest and rubric digests;
- non-empty owner reason;
- creation timestamp.

Approval bytes receive their own SHA-256 digest and are written atomically under the report-digest directory above. Creation retries if either approval ID or canonical digest already exists, so repeating the command with identical binding, reason, and clock tick still creates distinct immutable evidence. It does not modify previous evidence. Approval records are local evidence, not credentials or hostile-user authorization.

Approval can bypass `semantic_block`, including explicit block, indeterminate, or unavailable results. It can never bypass `deterministic_block`, malformed stored reports, digest mismatch, changed candidate/config/rubrics, or changed advertised base.

On retry, pre-push:

1. resolves the same exact base and candidate;
2. reruns prerequisite probes and all deterministic checks;
3. finds every approval whose verified report and bindings match current base, candidate, tree, manifest, and rubrics;
4. selects newest valid `created_at`, breaking ties by lexicographically smallest approval digest;
5. skips semantic rerun;
6. writes an attempt with unchanged `derived_disposition=semantic_block`, `gate_decision=approved`, and exact report/approval digests.

No matching approval means semantic judges run normally. This makes approval reusable across transport retries without pretending failed semantic review passed. CI ignores approvals and runs semantic review independently.

## Hooks and installation

Tracked hooks contain only portability setup and `xtask` invocation. Runner-input parity checks live in Rust. Hooks do not parse policy, materialize candidates, run quality commands directly, or duplicate publication logic.

A tracked Cargo alias makes project commands concise:

```text
cargo xtask hooks install
cargo xtask validate --staged
cargo xtask validate --publication --updates-stdin
cargo xtask validate --publication --ci-event <path>
cargo xtask validation approve --report <sha256> --reason <text>
```

`cargo xtask hooks install` requires an existing `HEAD`; an unborn repository fails before Git configuration changes. It materializes `HEAD` as the read-only candidate, verifies runner-input parity, creates distinct candidate-external scratch/cache/target roots, loads the candidate manifest, and cleans auxiliaries on every exit. Probes use candidate-root cwd, manifest default environment/placeholder expansion, and normal configured timeout/output bounds.

It then:

- validates repository and manifest;
- runs non-mutating prerequisite probes and prints configured install hints for missing tools;
- confirms tracked hook files exist and are executable;
- sets local `core.hooksPath=.githooks` through `/usr/bin/git config --local`;
- is idempotent;
- reports any conflicting local value and leaves it unchanged;
- performs no global Git configuration.

Fresh clones require this explicit command. Git provides no repository-controlled pre-hook capable of installing itself.

## CI

Push CI writes the exact three-field canonical event projection described above when event shape is valid; if projection cannot be formed, it passes original event-file bytes so rejected evidence remains lossless. It invokes `cargo xtask validate --publication --ci-event <path>` with local approvals disabled. Pre-push and direct publication fixtures instead forward raw Git update lines through `--updates-stdin`.

- Deterministic and semantic inputs bind pushed commit/tree.
- For ordinary and force pushes, `github.event.before` is base and `github.sha`/`github.event.after` is candidate.
- For a new branch (`before` is the all-zero object ID), the empty-tree object is normalized as `base_revision` for deterministic and semantic evidence.
- A deletion (`after` is the all-zero object ID) performs no tree checks and emits deletion disposition/evidence only.
- Any event whose before/after/ref shape cannot satisfy these rules fails closed before candidate execution.
- Candidate manifest and rubrics apply immediately.
- Report uploads even when validation fails.
- Semantic credentials remain outside repository.
- CI does not claim to prevent direct push.
- Owner-approved local publication may produce red CI; report shows why.

Existing pull-request hostile-candidate isolation and branch-protection language is removed because it does not match repository operation.

## Final-state implementation

Tasks build the final validation shape directly. Intermediate task boundaries require focused tests and explicit handoffs, but do not preserve duplicate v1/v2 entry points or prove every partially assembled tree as an independently operable release. Package gates validate cohesive capability groups after their final callers and configuration are connected.

Git provides rollback. This change pack does not record baseline restoration commands, maintain a compatibility inventory, rehearse reverts, or preserve obsolete dispatch solely for phased rollout. Superseded code may remain temporarily only when a later task still needs it to compile; the owning final-state package removes it before its cumulative gate.

Final tree has exactly one active path for each surface:

- pre-commit invokes v2 staged deterministic validation;
- pre-push invokes v2 aggregate publication validation;
- CI invokes the same v2 publication lifecycle through its canonical event projection;
- `quality/manifest.toml` is the only active deterministic and semantic registry.

## Removal scope

### Delete from active validation

- custom documentation checker;
- source-text architecture scanner and scanner fixtures;
- selected-equivalent dependency checker;
- operation-coverage scanner;
- initial-release acceptance-report generator;
- hardcoded quality runner enum and dispatch;
- parent-manifest monotonic evolution;
- foundation bootstrap and one-time migration policy;
- trusted-base/unprivileged-candidate CI machinery;
- stale single aggregate semantic scheduling;
- callable semantic-v1 adapter/dispatch and `quality/semantic-judge/v1/config.json`;
- legacy `quality/rubrics/manifest.json` registry.

Historical initial-implementation documents and evidence schemas may remain as history, but no active command, configuration, test, or owner documentation may reference retired validation paths.

### Retain and rewrite narrowly

- `xtask` package and command entry point;
- hook installation;
- exact staged/revision candidate mechanics;
- pre-push update parsing;
- command evidence capture;
- semantic adapter contract and response validation;
- Git-local reports;
- tracked `.githooks`;
- current focused rubric content;
- operation-catalog equality tests.

### Dependency reduction target

After deleted modules are removed, audit `xtask/Cargo.toml`. `syn`, `walkdir`, `camino`, `cargo_metadata`, `serde_json`, or other dependencies remain only when replacement code directly needs them. Cargo metadata may instead live solely in the small architecture test if production `xtask` does not need it.

## Failure behavior

Fail closed for:

- malformed candidate manifest;
- failed or version-mismatched prerequisite probe;
- missing configured executable;
- configured cwd outside candidate root;
- unsupported phase/scope/placeholder;
- candidate materialization mismatch;
- source mutation attempt or changed materialized source;
- runner input differing from candidate content;
- absolute or candidate-escaping symlink;
- non-UTF-8 changed path that cannot be represented losslessly in report JSON;
- timeout or output limit;
- malformed Git hook input;
- multiple content tips;
- missing semantic axis result;
- duplicate axis ID;
- invalid response or citation;
- coherence result attempting to erase an axis blocker;
- corrupt evaluation, approval, or attempt record;
- digest/revision/tree/config/rubric mismatch.

Deletion-only pushes, empty changed-file checks, and successful idempotent hook installation are not failures.

## Non-goals

- hosted or distributed validation service;
- general-purpose hook manager;
- plugin ecosystem;
- Bash or Make command orchestration;
- shell command strings;
- automatic formatting, fixing, or staging;
- same-run collaboration;
- multi-user approval or signatures;
- hostile-candidate credential defense;
- branch protection;
- multi-tip content pushes;
- Windows support;
- parent-policy bootstrap;
- anti-self-weakening machinery;
- preserving initial-release evidence generation as an evergreen gate.

## Acceptance

Change is complete when:

- fresh clone installs hooks through one documented idempotent command;
- pre-commit proves unstaged and untracked contamination cannot affect checks or runner inputs;
- every configured command starts beneath candidate root;
- missing Go/cargo-deny prerequisites fail without installing tools;
- all configured deterministic checks run against exact index tree;
- force-push candidate is judged by resulting content;
- deletion-only and multi-tip behavior match contract;
- runner contains no hardcoded Cargo or loop-engine quality command dispatch;
- ordinary tests enforce metadata architecture rules and operation catalog equality;
- all four semantic axes execute despite another axis blocking;
- coherence cannot erase an axis non-pass;
- malformed judge output gets exactly one correction attempt;
- semantic approval succeeds only for exact failed report and reason;
- approved retry retains semantic-block disposition and records `gate_decision=approved`;
- deterministic approval attempt is rejected;
- changed base/candidate/tree/config/rubric invalidates approval;
- CI ignores local approval and uploads independent evaluation/attempt records;
- macOS validation passes and Linux passes through owner-authorized CI or a named recorded Linux-container equivalent;
- no active v1 compatibility dispatch or duplicate policy registry remains;
- obsolete validation code and stale policy documentation are removed;
- `xtask` dependency and line-count reduction is recorded in final change evidence.
