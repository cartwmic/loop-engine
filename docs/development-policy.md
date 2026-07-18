# Development policy

**Status:** Semantic judge contract and provisioning frozen by T012 (2026-07-17).

Related documents:

- [Testing doctrine](testing.md)
- [Code architecture](architecture.md)
- [Decision D012](change/initial-implementation/decisions.md#d012--semantic-judge-provisioning)
- [Semantic judge v1 contract](../quality/semantic-judge/v1/README.md)
- [Foundation seed rubric manifest](../quality/rubrics/manifest.json)

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

After T028, publication range evaluation becomes the canonical pre-push gate. T029 wires the same publication command in GitHub Actions.

## Local vs publication disposition

| Verdict | Local pre-commit (T027+) | Publication / pre-push / CI |
|---|---|---|
| `pass` | allow | allow |
| `fail` | block | block |
| `indeterminate` | warn; allow | block |
| `unavailable` | warn; allow | block |

Deterministic documentation, architecture, schema, and quality checks remain separate from semantic judgment.

## T029 GitHub Actions provisioning owner

T029 must provision the same adapter without duplicating gate logic in workflow YAML.

| Name | Kind | Owner | Purpose |
|---|---|---|---|
| `LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE` | repository variable | T029 | Path to generic judge executable (`quality/semantic-judge/v1/judge`) |
| `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` | repository secret | T029 | Redacted `pi` auth JSON for `openai-codex` only; written to a temp file at runtime, never logged |

Workflow steps must:

1. materialize `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON` into an ephemeral `auth.json` for the job only;
2. invoke the same publication command documented here for each commit in the exact unpublished range;
3. store per-commit judge responses as CI artifacts;
4. fail closed on `fail`, `indeterminate`, or `unavailable`.

## Verification commands (T012)

```bash
quality/semantic-judge/v1/verify-contract
quality/semantic-judge/v1/build-smoke-request | quality/semantic-judge/v1/judge
```

Real publication-mode smoke against parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` and candidate `581c23bb085718d37d994d77d59d3d70b7ea309f` must return determinate `pass` or `fail`. `indeterminate`, `unavailable`, or empty output block T012 completion.

Untracked smoke transcripts may be kept locally under `target/quality-artifacts/` when useful; tracked contract artifacts must remain stable and credential-free.
