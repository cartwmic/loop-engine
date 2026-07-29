# Development policy

**Status:** Exact-candidate Git validation v2 is active. Pre-commit runs deterministic checks over staged tree. Pre-push runs one aggregate deterministic and semantic publication decision. Push CI independently re-evaluates pushed revision and ignores local approvals.

Related documents:

- [Testing doctrine](testing.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [Validation manifest v2](../quality/validation/v2/README.md)
- [Semantic judge v2 contract](../quality/semantic-judge/v2/README.md)
- [Publication report v1](../quality/publication-report/v1/README.md)
- [Dependency policy (`deny.toml`)](../deny.toml)

## Owner setup

Repository requires Rust 1.95.0, cargo-deny 0.20.2, and Go 1.26.5 already installed through `mise`. Validation never installs prerequisites. Go commands run with `MISE_AUTO_INSTALL=false` and `MISE_AUTO_INSTALL_DISABLE_TOOLS=go`.

Install cargo-deny and Go when absent, then install tracked hooks from an existing committed `HEAD`:

```bash
cargo install cargo-deny --locked --version 0.20.2
mise install go@1.26.5
cargo xtask hooks install
```

Installer validates candidate manifest, runner-input parity, hook executability, and prerequisite versions before setting repository-local `core.hooksPath=.githooks`. Repeating command is safe. Conflicting local `core.hooksPath` remains unchanged and must be resolved by owner.

## Exact staged validation

Run same command used by pre-commit:

```bash
cargo xtask validate --staged
```

Candidate is exact index tree. `HEAD` is base; unborn repository uses Git empty tree. Unstaged tracked content and untracked files do not enter candidate. Runner inputs such as hooks, `xtask`, manifest, rubrics, toolchain, and policy files must match staged candidate even when other unstaged product files are allowed. Every command starts beneath read-only materialized candidate root and writes only to candidate-external scratch, cache, and target roots.

Complete deterministic suite comes only from [`quality/manifest.toml`](../quality/manifest.toml) and runs in declared order while collecting failures. Any prerequisite, configuration, process, output-limit, timeout, source-mutation, parity, or check failure blocks commit. No approval bypass exists.

## Advisory semantic validation

Run publication-phase deterministic and semantic validation before push when early feedback helps:

```bash
cargo xtask validate --semantic --base <base-revision> --candidate HEAD
```

`--base` may name any revision. `--candidate` is required and must resolve to currently checked-out `HEAD`; another commit or branch fails with checkout-and-retry guidance. Command runs complete deterministic `publication` phase first, then four configured axes and coherence. It writes one evaluation report and prints report digest. It writes no publication-attempt record and never consults approvals.

Candidate manifest and rubrics apply immediately through sole v2 policy path.

## Publication and pre-push

Normal owner command remains ordinary Git push:

```bash
git push <remote> <refspec>
```

Installed pre-push hook forwards exact Git update lines to:

```bash
cargo xtask validate --publication --updates-stdin
```

Do not manufacture update lines for normal use; Git supplies advertised destination SHA as base and resulting local SHA as candidate. Force pushes validate resulting local tip without ancestry requirement. New branches normalize zero advertised base to Git empty tree. Candidate commit must equal checked-out `HEAD`, and runner inputs must match that commit.

One invocation yields one aggregate verdict:

- Empty input or deletion-only updates pass without loading manifest or running prerequisites, deterministic checks, or semantic judges.
- Exactly one content update runs fresh complete deterministic suite, all semantic axes, then coherence.
- More than one non-delete update, including duplicate updates to same tree, blocks with `multiple_content_tips`; split push.
- Malformed bytes block with `malformed_update_input`; tokenized but invalid ref/OID/delete/new/update shapes block with `invalid_update_shape`.
- A valid force push is treated as content and judged normally.

All configured axes run concurrently even when another blocks. Malformed successful judge output receives exactly one correction attempt within original timeout. Coherence may add blocker but cannot erase focused non-pass. `block`, `indeterminate`, and `unavailable` all derive `semantic_block`.

## Reports and approvals

Evidence root is shared Git common directory, including linked worktrees:

```text
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/reports/<report-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/approvals/<report-digest>/<approval-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/content/<candidate-tree>/<attempt-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/deletions/<attempt-digest>.json
$(/usr/bin/git rev-parse --git-common-dir)/loop-engine/validation/v1/attempts/rejected/<attempt-digest>.json
```

Each completed reportable advisory run stores evaluation. Each reportable publication invocation stores one aggregate attempt, including pass, semantic block, deterministic block, deletion, malformed input, and multi-tip rejection. Content attempt references evaluation report. Records are immutable canonical JSON addressed by SHA-256 of exact bytes; reports remain failed after approval.

Owner may approve exact verified `semantic_block` report with non-empty reason:

```bash
cargo xtask validation approve \
  --report <report-digest> \
  --reason "<non-empty owner reason>"
git push <remote> <refspec>
```

Retry same push. Pre-push reruns prerequisites and every deterministic check. Exact matching approval lets retry skip semantic execution and records unchanged `derived_disposition=semantic_block` with `gate_decision=approved` plus report/approval digests.

Approval deterministically cannot bypass `deterministic_block`, malformed report, digest mismatch, or changed base, candidate commit/tree, manifest, or rubric. When several exact approvals exist, newest `created_at` wins with lexicographically smallest digest tie-break. Repeating approval command creates distinct immutable evidence.

## Push CI authority

[`.github/workflows/quality.yml`](../.github/workflows/quality.yml) runs only on push. It projects exact canonical `before`, `after`, and `ref` into same publication lifecycle:

```bash
cargo xtask validate --publication --ci-event <path>
```

CI independently evaluates pushed commit/tree using candidate manifest and rubrics immediately. It does not read or honor local approvals. Ordinary, force, new-branch, deletion, and malformed event behavior follows same aggregate contract. Workflow always uploads command stdout/stderr plus available Git-common-dir evaluation and attempt evidence, including blocked runs.

CI verifies after publication. Direct push cannot be prevented by this workflow or by local hooks on owner-controlled machine. Repository makes no branch-protection or server-side prevention claim. Owner-approved local push may therefore produce red CI with independent report explaining block.

## Policy changes and rollback

`quality/manifest.toml` is sole active deterministic and semantic registry. Generic runner executes candidate policy immediately and contains no project-specific command or rubric dispatch. Semantic process uses generic v2 JSON-over-stdio protocol.

Validation never fixes, rewrites, stages, or rolls back source. Use ordinary Git commits, revert, reset, or ref movement appropriate to repository state when rollback is needed. Git is sole rollback mechanism.

## Formatting and Clippy policy

Workspace toolchain is pinned in `rust-toolchain.toml` (Rust 1.95.0). Formatting options are frozen in repository-root `rustfmt.toml`. Inherited lint levels are defined in workspace-root `Cargo.toml` under `[workspace.lints]`; workspace members opt in with `[lints] workspace = true`.

Canonical manifest commands:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

`-D warnings` treats all Rust and Clippy warnings as errors. Broad allow lists and crate/item attributes that hide unfinished behavior are prohibited. Formatting and Clippy remain deterministic checks separate from semantic judgment.

## Dependency policy

Repository-root [`deny.toml`](../deny.toml) is authoritative for licenses, sources, advisories, and bans. It remains compatible with dual MIT/Apache-2.0 project license.

- Licenses use deny-by-default permissive allowlist. Copyleft-only expressions reject; SPDX `OR` including allowed license may pass.
- Sources allow crates.io only. Unknown registry, unknown Git, and Git dependencies reject.
- Vulnerability, unsound, yanked, lockfile, and advisory-database failures block.
- Wildcard dependencies reject. Multiple transitive versions warn and remain recorded.
- Workspace `Cargo.lock` must exist.

Canonical command, pinned to cargo-deny 0.20.2:

```bash
cargo deny check
```

If dependency cannot satisfy license/source policy, redesign or replace it before landing.

## Planning and execution policy

1. **Walking skeleton first.** Produce runnable production driver and black-box harness before breadth.
2. **Governance proportional to executors.** Run validation/repair at coherent work-package boundaries, not every microtask.
3. **Bound task granularity.** Prefer vertical packages with internal checklists over hundreds of coupled microtasks.
4. **Prove shared machinery once.** Exhaust shared failure path on one representative operation plus representative consumers.
5. **Separate product invariants from process policy.** Runtime guarantees belong in invariants; owner validation policy belongs here.
6. **Checkpoint long-running work.** Stop for owner review at each work-package boundary.
7. **Bound uncommitted tranches.** Do not span more than one work package without commit boundary.
