# Development policy

**Status:** Dependency policy and C1 empty-runtime closure prepared by T030 (2026-07-18). Authoritative CI adapter landed by T029 (2026-07-18). Semantic judge contract and provisioning names frozen by T012 (2026-07-17).

Related documents:

- [Testing doctrine](testing.md)
- [Code architecture](architecture.md)
- [Decision D012](change/initial-implementation/decisions.md#d012--semantic-judge-provisioning)
- [Semantic judge v1 contract](../quality/semantic-judge/v1/README.md)
- [Foundation seed rubric manifest](../quality/rubrics/manifest.json)
- [Dependency policy (`deny.toml`)](../deny.toml)
- [Checkpoint evidence schema](../quality/evidence/v1/checkpoint.schema.json)

## Documentation coherence

Every commit must independently leave relevant documentation coherent with behavior, architecture, contracts, testing policy, and development policy it introduces ([I47](invariants.md#i47-every-commit-is-documentation-coherent)).

Before committing during Phase 0 contract freeze (through T016), run the configured semantic judge against the exact parent-to-candidate diff using the parent revision's rubric.

Before pushing, every unpublished commit must receive a determinate semantic-judge `pass`. `fail`, `unavailable`, and `indeterminate` block publication.

## Bootstrap and parent rubric

Foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` consumed the one-time bootstrap publication exception. No second bootstrap exception is permitted.

Until focused rubric files land in T025, the parent rubric for every post-foundation commit is the foundation seed manifest at [quality/rubrics/manifest.json](../quality/rubrics/manifest.json) anchored to that parent revision. Changed rubric content applies only to the following commit.

No implementation commit may be pushed before T029. The first post-foundation push must show determinate judgment for every C0/C1 commit in the unpublished range.

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

After T024, the canonical wrapper becomes:

```bash
cargo run -p xtask -- judge --staged
```

The versioned hook adapters clear Git's repository-local environment variables before tests spawn nested fixture repositories, then clear any caller-provided `RUSTUP_TOOLCHAIN` override before invoking Cargo so the tracked `rust-toolchain.toml` pin is authoritative. The pre-commit adapter refuses to run when hook, `xtask`, quality-manifest, dependency-policy, toolchain, or semantic-judge implementation paths contain unstaged changes. This prevents working-tree gate code from differing from its staged candidate while unrelated unstaged product/document edits remain excluded by exact-index materialization.

After T028, the canonical publication / pre-push command is:

```bash
cargo run --locked -p xtask -- publication --from <exclusive-start-rev>
```

Optional inclusive end defaults to `HEAD`:

```bash
cargo run --locked -p xtask -- publication --from <exclusive-start-rev> --to <inclusive-end-rev>
```

The command enumerates every commit in `<exclusive-start-rev>..<inclusive-end-rev>` (oldest-first, actual Git parent per commit, merge commits rejected). For a new branch, the pre-push adapter passes Git's exact destination remote name and URL, queries that destination's currently advertised refs, fetches its commit graph into an isolated temporary bare repository backed by read-only local object alternates, and excludes commits reachable from those advertised tips; stale or forged local remote-tracking refs and another remote can never suppress judgment. It loads **`quality/manifest.toml` from each candidate revision's detached worktree** (never the tip manifest for historical commits), enforces monotonic manifest evolution once a parent manifest exists, runs the candidate's currently-implemented quality checks, injects exact command evidence into the semantic-judge request, emits each per-commit judge JSON response before applying blocking disposition, and requires a determinate parent-rubric semantic-judge `pass` in publication mode. Pre-manifest historical commits run the immutable built-in baseline (`git diff --check` plus semantic judge only). A manifest override exists only in the in-process Rust test API; publication and pre-push CLIs expose no override, so normal history always uses each candidate's committed manifest. Gate logic stays in `xtask`; hooks and CI must not reimplement it.

T029 wires that same publication command in GitHub Actions (`.github/workflows/quality.yml`). Pull-request jobs explicitly check out `pull_request.head.sha`; GitHub's synthetic merge ref is never passed to the linear exact-parent gate.

## Local vs publication disposition

| Verdict | Local pre-commit (T027+) | Publication / pre-push / CI |
|---|---|---|
| `pass` | allow | allow |
| `fail` | block | block |
| `indeterminate` | warn; allow | block |
| `unavailable` | warn; allow | block |

Deterministic documentation, architecture, schema, and quality checks remain separate from semantic judgment.

## Formatting and Clippy policy (T020)

The workspace toolchain is pinned in `rust-toolchain.toml` (Rust 1.95.0). Formatting options are frozen in repository-root `rustfmt.toml`. Inherited lint levels are defined in workspace-root `Cargo.toml` under `[workspace.lints]`; workspace members opt in with `[lints] workspace = true` in their own `Cargo.toml`.

Canonical verification commands (also consumed by hooks, CI, and `cargo run -p xtask -- quality` after T028):

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
cargo run --locked -p xtask -- dependencies
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

## T029 GitHub Actions provisioning owner

T029 provisions the same adapter without duplicating gate logic in workflow YAML. Workflow authority is `.github/workflows/quality.yml`.

| Name | Kind | Owner | Purpose |
|---|---|---|---|
| `LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE` | repository variable | T029 | Path to generic judge executable (`quality/semantic-judge/v1/judge`) |
| `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` | repository secret | T029 | Redacted `pi` auth JSON for `openai-codex` only; written to a temp file at runtime, never logged |

Owner setup (values never committed):

1. Set repository variable `LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE` to `quality/semantic-judge/v1/judge`.
2. Set repository secret `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` to the operator's redacted `pi` `auth.json` contents for `openai-codex` only.
3. Do not store secret values in tracked files, workflow logs, or artifacts.

Workflow steps must:

1. fail closed when the variable or secret is missing;
2. materialize `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` into an ephemeral `auth.json` under `PI_CODING_AGENT_DIR` for the job only (never echo contents);
3. invoke the canonical publication command once for the exact unpublished range (`--from <exclusive-start-rev>`); the command itself gates every commit;
4. store per-commit judge responses under `target/quality-artifacts/ci/<job>/<candidate_revision>/response.json` and upload them as CI artifacts;
5. rely on the publication command exit status to fail closed on `fail`, `indeterminate`, or `unavailable` (no YAML verdict branching).

Supported CI matrix (authoritative host/target policy from [technology.md](technology.md)):

| Required check name | Host image | Target triple |
|---|---|---|
| `quality / linux-x86_64` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` |
| `quality / linux-aarch64` | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |
| `quality / macos-aarch64` | `macos-15` | `aarch64-apple-darwin` |
| `quality / macos-x86_64` | `macos-15-intel` | `x86_64-apple-darwin` |

### Required branch rule / check

Until T198 applies server-side protection, document the intended rule:

- protect `main` from direct writes;
- require branch current with `main` before integration;
- require linear history and disable merge-commit integration; only fast-forward an already reviewed commit chain so landed commit SHAs are the exact SHAs judged by publication CI (squash/rebase rewriting is not an authoritative substitute);
- require all four status checks above (`quality / linux-x86_64`, `quality / linux-aarch64`, `quality / macos-aarch64`, `quality / macos-x86_64`) before merge and before release;
- prevent bypass where the hosting platform supports it.

Those exact check names are the T029 → T198 handoff for required CI.

### Local dry-run (same command CI runs)

Uses the operator's existing local `pi` auth (`~/.pi/agent/auth.json` or `PI_CODING_AGENT_DIR`). Does not read GitHub Actions secrets:

```bash
export LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE="${LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE:-quality/semantic-judge/v1/judge}"
cargo run --locked -p xtask -- publication --from origin/main
```

To dry-run the first post-foundation unpublished range locally:

```bash
cargo run --locked -p xtask -- publication --from 7552af5968b4a2c10aefd01fbfa6c351817e1b8b
```

Capture per-commit stdout JSON locally when useful:

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
