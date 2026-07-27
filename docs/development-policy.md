# Development policy

**Status:** Publication-checkpoint coherence superseded per-commit scheduling by owner decision R001 (2026-07-18). Aggregate hook/publication/CI repair R002–R005, dependency policy, workspace boundaries, and fail-closed pre-commit/pre-push enforcement are implemented. Current candidate exposes complete frozen 21-operation catalog; final publication uses one advertised-remote-tip-to-candidate gate.

Related documents:

- [Testing doctrine](testing.md)
- [Code architecture](architecture.md)
- [Decision D012](change/initial-implementation/decisions.md#d012--semantic-judge-provisioning)
- [Semantic judge v1 contract](../quality/semantic-judge/v1/README.md)
- [Foundation seed rubric manifest](../quality/rubrics/manifest.json)
- [Dependency policy (`deny.toml`)](../deny.toml)
- [Checkpoint evidence schema](../quality/evidence/v1/checkpoint.schema.json)

## Publication-checkpoint coherence

Every accepted push must leave the candidate destination tip coherent with behavior, architecture, contracts, testing policy, and development policy introduced by the aggregate change from the exact remote destination tip ([I47](invariants.md#i47-every-publication-checkpoint-is-documentation-coherent)). Commits inside one unpublished range may be incomplete and may repair one another.

Default pre-commit runs bounded fast deterministic checks against the exact staged tree. Explicit staged semantic judgment remains available for early feedback but is not a commit requirement.

Before pushing, the exact remote-base-to-local-head range must receive one determinate semantic-judge `pass`. `fail`, `unavailable`, and `indeterminate` block publication.

## Base rubric

Foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` remains the initial rubric seed. Its frozen rubric encodes superseded per-commit scheduling, so it cannot judge the owner-directed scheduling migration faithfully. R001 therefore authorizes exactly one policy-migration rubric for an aggregate publication whose exact base is that foundation revision. This is not a second bootstrap: deterministic quality and one fail-closed semantic judgment remain mandatory.

Owner must explicitly set `LOOP_ENGINE_OWNER_MIGRATION_RUBRIC` to [`quality/semantic-judge/v1/migrations/publication-checkpoint-v1.md`](../quality/semantic-judge/v1/migrations/publication-checkpoint-v1.md) for that one publication. Tooling requires regular-file SHA-256 `5c06777499f87a46923ac9423f274b70d152d5e356d8378d939e76f6dc2a5d9a`, rejects this override for every other base, and never selects candidate content implicitly. After migration, exact remote destination tip supplies the rubric. A changed rubric applies only to the following push, so a candidate range cannot weaken its own review.

## Semantic judge executable

| Setting | Value |
|---|---|
| Contract | [quality/semantic-judge/v1/README.md](../quality/semantic-judge/v1/README.md) |
| Executable | `quality/semantic-judge/v1/judge` |
| Config | `quality/semantic-judge/v1/config.json` |
| Model | `openai-codex/gpt-5.6-sol` through `pi` (`--no-tools`) |
| Default timeout | 900 seconds |

Credentials are never committed. Local runs use the operator's existing `pi` authentication. Override the `pi` binary with `LOOP_ENGINE_SEMANTIC_JUDGE_PI` when needed.

## C0 local judge command (through T023)

Build a local-mode request for the exact staged index tree against `HEAD` parent, then invoke the generic executable:

```bash
quality/semantic-judge/v1/build-exact-staged-request \
  | quality/semantic-judge/v1/judge
```

The staged request builder sets `parent_revision` to `HEAD`, `candidate_revision` to the index tree (`git write-tree`), `diff` to `git diff --cached HEAD`, and `relevant_docs` to staged resulting markdown from the Git index only. It never reads unstaged working-tree content or candidate working-tree rubrics.

Judge responses are self-binding publication evidence: every stdout JSON document echoes `parent_revision` and `candidate_revision` from the request. Store the response artifact alongside the request when recording C0 evidence; a revision mismatch in model output is rejected as `unavailable`.

Parent rubrics load from the parent revision's committed `quality/rubrics/manifest.json` when present. When that manifest is absent, the builder composes the foundation-seed rubric only from immutable Git blobs at foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` (`git show <sha>:docs/invariants.md` for I47, `docs/testing.md` for Git enforcement direction, `docs/tenets.md` for tenet 27, `docs/architecture.md` for composition/enforcement). Frozen source paths and blob digests live in `quality/semantic-judge/v1/request_builder.py`; composed rubric content must match digest `3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd`. Request evidence exposes per-source provenance in `deterministic_evidence`. The bundled file at `quality/semantic-judge/v1/frozen/foundation-seed.v1.md` is a non-authoritative reference only.

T012 publication smoke remains a separate fixed-revision command and is not the C0 pre-commit gate:

```bash
quality/semantic-judge/v1/build-smoke-request \
  | quality/semantic-judge/v1/judge
```

R002 removes semantic judgment from default pre-commit execution. The versioned hook adapters clear Git's repository-local environment variables before tests spawn nested fixture repositories, then clear any caller-provided `RUSTUP_TOOLCHAIN` override before invoking Cargo so the tracked `rust-toolchain.toml` pin is authoritative. Runner-input parity covers hook, `xtask`, quality-manifest, dependency-policy, toolchain, and formatting-policy paths. This prevents working-tree gate code from differing from its staged candidate while unrelated unstaged product/document edits remain excluded by exact-index materialization.

After T028, the canonical publication / pre-push command is:

```bash
cargo run --locked -p xtask -- publication --from <exclusive-start-rev>
```

Optional inclusive end defaults to `HEAD`:

```bash
cargo run --locked -p xtask -- publication --from <exclusive-start-rev> --to <inclusive-end-rev>
```

R003 changes this command to evaluate exactly one range: `<exclusive-start-rev>..<inclusive-end-rev>`. The exclusive start is the remote destination tip and the inclusive end is the candidate local head. Existing-branch publication requires the base to be an ancestor of the head and rejects unsupported nonlinear replacement. Pre-push uses Git's exact advertised destination SHA rather than a mutable remote-tracking ref. For a new branch, Git's hook input has no integration-ref SHA, so adapter performs fresh exact `ls-remote` query for destination `refs/heads/main` and requires candidate head to descend from that snapshot. Server branch-current protection remains authority if integration branch advances after query.

The command creates one detached worktree at candidate head, runs `git diff --check` across the aggregate range, loads candidate-head `quality/manifest.toml`, proves it did not remove or weaken checks present at the base, runs candidate quality once, injects that evidence into one base-rubric semantic request, emits one response, and requires determinate `pass`. The one-time foundation migration replaces the failed 2.2 MB duplicated request while still binding the complete foundation range by SHA-256/size/path-count evidence and semantically projecting the exact reviewed-C1-to-candidate governance repair delta; ordinary checkpoints continue to send the exact destination-base-to-candidate diff and resulting changed documents. Intermediate commits and their manifests are not separate authorities. Pre-push accepts at most one non-delete ref update and requires its local SHA to equal checked-out `HEAD`; ambiguous or differently checked-out pushes must be split. Gate logic stays in `xtask`; hooks and CI must not reimplement it.

R004 wires two canonical phases into protected-`main` `pull_request_target` GitHub Actions (`.github/workflows/quality.yml`); candidate workflow edits cannot authorize themselves. A credential-free runner executes trusted base `xtask` against exact candidate objects; trusted gate process owns source and evidence while candidate-influenced subprocesses run as unprivileged UID 65534 with writes confined to dedicated cargo/target directories. Gate rejects tracked, untracked, or ignored non-target source changes before and after each check, then uploads revision-bound deterministic evidence plus an inert Git bundle. A fresh runner imports only the exact hash-bound bundle, deletes it before credentials exist, builds trusted base gate code, then provisions judge credentials and performs semantic-only publication. Candidate-controlled code and candidate checkout paths never exist on credential-bearing runners. Pull-request jobs explicitly bind `pull_request.base.sha` and `pull_request.head.sha`; GitHub's synthetic merge ref is never passed to the gate. Push CI remains credential-free because protected pull-request checks are remote semantic authority. Foundation-to-R001 migration predates this remote workflow and therefore relies on owner-authorized local aggregate pre-push; installed protected-default-branch workflow governs subsequent pull requests.

## Local vs publication disposition

| Verdict | Explicit staged judge | Publication / pre-push / CI |
|---|---|---|
| `pass` | advisory pass | allow |
| `fail` | advisory fail | block |
| `indeterminate` | advisory warning | block |
| `unavailable` | advisory warning | block |

Default pre-commit has no semantic disposition; any fast deterministic failure blocks commit. Deterministic documentation, architecture, schema, and quality checks remain separate from semantic judgment.

## Formatting and Clippy policy (T020)

The workspace toolchain is pinned in `rust-toolchain.toml` (Rust 1.95.0). Formatting options are frozen in repository-root `rustfmt.toml`. Inherited lint levels are defined in workspace-root `Cargo.toml` under `[workspace.lints]`; workspace members opt in with `[lints] workspace = true` in their own `Cargo.toml`.

Canonical verification commands declared by [`quality/manifest.toml`](../quality/manifest.toml):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Policy:

- `-D warnings` treats all `rustc` and Clippy warnings as errors for the invoked command.
- No broad `[workspace.lints.*]` allow lists and no crate-level `#![allow(...)]` or `#[allow(...)]` attributes that hide unfinished behavior.
- Formatting and Clippy checks remain separate from semantic judgment.

## Dependency policy (T030)

Repository-root [`deny.toml`](../deny.toml) is the authoritative dependency policy for licenses, sources, advisories, and bans. It must stay compatible with the dual MIT/Apache-2.0 project license (D003 / T003).

Allowlist summary:

- **Licenses:** deny-by-default permissive allowlist (`MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `0BSD`, `Zlib`, `Unlicense`, `Unicode-3.0`, `Unicode-DFS-2016`, `CC0-1.0`). Copyleft-only expressions are rejected. SPDX `OR` alternatives that include an allowed license remain acceptable.
- **Sources:** crates.io registry only; `unknown-registry` and `unknown-git` are `deny`; `allow-git` is empty.
- **Advisories:** vulnerabilities and unsound advisories fail closed (cargo-deny 0.20.2 behavior); `yanked = "deny"`; unmaintained advisories are limited to workspace-direct edges for C1.
- **Bans:** `wildcards = "deny"`; `multiple-versions = "warn"` (transitive duplicates are recorded, not hard-failed, at C1).
- **Lockfile:** workspace `Cargo.lock` must exist; non-crates.io and git lockfile sources fail the gate.

Canonical selected-equivalent command (licenses, sources, lockfile, policy shape):

```bash
cargo deny check
```

Complementary advisory-database scan against the same `deny.toml` (pinned `cargo-deny` 0.20.2; missing tool or advisory DB failure blocks publication):

```bash
cargo deny check
```

CI installs the pinned release before invoking the canonical publication command. The quality manifest runner `cargo-deny` invokes the same pinned check in detached worktrees.

Stop condition: if a selected dependency can only be satisfied by a license expression incompatible with the dual MIT/Apache-2.0 allowlist, do not land it; redesign or replace the dependency first.

## C1 closure — empty runtime surface (T030)

Checkpoint C1 closes the workspace/governance skeleton only. The currently implemented product CLI remains an empty composition-root placeholder: root help exposes **zero** application routes, and no runtime operation catalog entries are published. Application operations arrive with C2+.

C1 candidate evidence uses [`quality/evidence/v1/checkpoint.schema.json`](../quality/evidence/v1/checkpoint.schema.json) with `checkpoint_id = "C1"` and `application_operations = []`. Orchestrator commit message for this boundary: `build: establish workspace governance`.

## R004 GitHub Actions provisioning owner

T029 originally provisioned per-commit publication on every platform leg. R004 replaces that schedule: one credential-free `publication quality evidence` job, one fresh-runner `publication / aggregate` semantic job, and four credential-free `quality / ...` jobs for supported platforms. Workflow authority is `.github/workflows/quality.yml`.

| Name | Kind | Owner | Purpose |
|---|---|---|---|
| `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` | repository secret | T029 | Redacted `pi` auth JSON for `openai-codex` only; written to a temp file at runtime, never logged |

Owner setup: set `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` to operator's redacted `pi` `auth.json` contents for `openai-codex` only. Never store secret value in tracked files, workflow logs, or artifacts. Judge executable is pinned to trusted base path `quality/semantic-judge/v1/judge`, not a candidate-controlled setting.

Workflow steps must:

1. run trusted base publication quality phase on a credential-free runner with root-owned source/evidence and unprivileged candidate-influenced subprocesses, then upload exact base/head-bound evidence plus a hash-bound Git bundle;
2. use a fresh runner for semantic phase, import candidate Git objects only from that inert bundle, remove the bundle, and build trusted base `xtask` before credentials are provisioned;
3. fail closed when secret is missing, then materialize it into ephemeral `auth.json` under `PI_CODING_AGENT_DIR` for semantic step only;
4. consume the bound evidence through `publication --quality-report-in`, invoke trusted base judge exactly once, and store the response under `<base_revision>..<candidate_revision>/response.json`;
5. rely on publication exit status to fail closed on `fail`, `indeterminate`, or `unavailable` (no YAML verdict branching).

Required CI jobs (authoritative host/target policy from [technology.md](technology.md)):

| Required check name | Host image | Target triple | Scope |
|---|---|---|---|
| `publication / aggregate` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | consume trusted bound evidence plus one base-to-head semantic judgment |
| `quality / linux-x86_64` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | candidate-head deterministic quality |
| `quality / linux-aarch64` | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | candidate-head deterministic quality |
| `quality / macos-aarch64` | `macos-15` | `aarch64-apple-darwin` | candidate-head deterministic quality |
| `quality / macos-x86_64` | `macos-15-intel` | `x86_64-apple-darwin` | candidate-head deterministic quality |

### Required branch rule / check

Until T198 applies server-side protection, document the intended rule:

- protect `main` from direct writes;
- require branch current with `main` before integration;
- require linear history and disable merge-commit integration; only fast-forward an already reviewed commit chain so landed commit SHAs are the exact SHAs judged by publication CI (squash/rebase rewriting is not an authoritative substitute);
- require `publication / aggregate` plus all four `quality / ...` status checks above before merge and before release;
- prevent bypass where the hosting platform supports it.

Those exact check names are the T029 → T198 handoff for required CI.

### Local dry-run (same command CI runs)

Uses the operator's existing local `pi` auth (`~/.pi/agent/auth.json` or `PI_CODING_AGENT_DIR`). Does not read GitHub Actions secrets:

```bash
export LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE="${LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE:-quality/semantic-judge/v1/judge}"
cargo run --locked -p xtask -- publication --from origin/main
```

To run the one owner-authorized post-foundation policy migration:

```bash
LOOP_ENGINE_OWNER_MIGRATION_RUBRIC=quality/semantic-judge/v1/migrations/publication-checkpoint-v1.md \
  cargo run --locked -p xtask -- publication \
  --from 7552af5968b4a2c10aefd01fbfa6c351817e1b8b
```

Tooling rejects this environment override once destination base differs from foundation.

Capture aggregate publication stdout JSON locally when useful:

```bash
mkdir -p target/quality-artifacts/local-dry-run
cargo run --locked -p xtask -- publication --from origin/main \
  | tee target/quality-artifacts/local-dry-run/publication.stdout
```

## Verification commands (T012)

```bash
quality/semantic-judge/v1/verify-contract
quality/semantic-judge/v1/build-smoke-request | quality/semantic-judge/v1/judge
```

Real publication-mode smoke against parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` and candidate `581c23bb085718d37d994d77d59d3d70b7ea309f` must return determinate `pass` or `fail`. `indeterminate`, `unavailable`, or empty output block T012 completion.

Untracked smoke transcripts may be kept locally under `target/quality-artifacts/` when useful; tracked contract artifacts must remain stable and credential-free.

## Planning and execution lessons (2026-07-22)

Distilled from the initial-implementation over-engineering assessments ([fable](change/initial-implementation/overengineering-assessment.md), [sol](change/initial-implementation/overengineering-assessment-sol.md)) and the plan amendment they produced. These are binding planning policy for future changes in this repository.

1. **Walking skeleton first.** When end-to-end tests are the sole behavioral authority, sequencing must produce a runnable production driver and harness before breadth. A plan that defers the driver to a late phase guarantees days of unaccepted inventory regardless of code quality. See [testing.md § Sequencing doctrine](testing.md#sequencing-doctrine).
2. **Governance proportional to executors.** Validation/repair ceremony scales with the number of independent, mutually untrusted executors. For one owner orchestrating subagents, validation runs at work-package boundaries, not per microtask. Roughly one third of the original plan's execution units governed work instead of doing work; that ratio is the failure signal to watch for.
3. **Task granularity has a ceiling.** Plans of hundreds of microtasks with dense dependency edges make plan maintenance its own workload, and the bookkeeping drifts anyway (the amended plan replaced a header/marker contradiction observed in practice). Prefer six to ten vertical work packages with internal checklists; reserve task-level granularity for genuinely independent, delegable units.
4. **Prove shared machinery once.** Failure modes that flow through one shared adapter (process failures, protocol violations, bounds) are proven exhaustively on one representative operation, plus one representative row per other consumer. Requiring the full cross-product on every operation costs quadratically and yields linearly.
5. **Separate product invariants from process policy.** Encoding review/validation process into numbered system invariants makes the plan self-justifying — ceremony appears mandatory because documents written alongside the plan mandate it. Runtime guarantees belong in invariants.md; process policy belongs here, where the owner can amend it without invariant ceremony.
6. **Checkpoint cadence for long-running agents.** Multi-day unattended execution accumulates unexamined direction risk. The orchestrator stops for explicit owner review at every work-package boundary; no plan may authorize unattended multi-package runs.
7. **Bound uncommitted tranches.** Uncommitted work never spans more than one work package; large uncommitted line counts are a stop-and-commit signal, not a milestone in progress.
