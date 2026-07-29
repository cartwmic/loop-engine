# Publication report v1

This directory freezes closed JSON Schemas for semantic evaluation, owner approval, and aggregate publication-attempt evidence. Runtime records never live here.

Records use compact canonical UTF-8 JSON emitted from closed typed structures. Object fields follow schema order; maps use lexicographically sorted keys; no trailing newline is written. External IDs are lowercase SHA-256 over exact stored bytes and are not embedded in those bytes.

Runtime records live below `/usr/bin/git rev-parse --git-common-dir`:

```text
loop-engine/validation/v1/reports/<report-digest>.json
loop-engine/validation/v1/approvals/<report-digest>/<approval-digest>.json
loop-engine/validation/v1/attempts/content/<candidate-tree>/<attempt-digest>.json
loop-engine/validation/v1/attempts/deletions/<attempt-digest>.json
loop-engine/validation/v1/attempts/rejected/<attempt-digest>.json
```

Writes use create-new temporary files, file synchronization, and atomic no-overwrite publication into digest path. Reads verify path digest, exact bytes, canonical encoding, closed shape, nullability, derived disposition, exact input projection, and candidate/config/rubric bindings. Linked worktrees share common-directory evidence.

Evaluation disposition is `pass`, `deterministic_block`, or `semantic_block`. Publication attempt gate decision is independently `pass`, `block`, or `approved`; approval never rewrites failed semantic report to pass. Every publication invocation stores one aggregate attempt, including empty/deletion, malformed input, invalid shape, multiple content tips, and content pass/block. Advisory semantic invocation stores evaluation only.

Owner approval command accepts only verified semantic-block report and non-empty reason:

```bash
cargo xtask validation approve --report <report-digest> --reason "<non-empty owner reason>"
```

Retry same Git push. Pre-push reruns prerequisites and full deterministic phase. Matching approval may skip semantic rerun only when base, candidate revision/tree, manifest digest, rubric digests, and report digest remain exact. Deterministic failure, malformed evidence, changed binding, or CI publication cannot use local approval. Repeated approval creates distinct immutable record.

See [`docs/development-policy.md`](../../../docs/development-policy.md) for owner workflow and [`xtask/tests/report.rs`](../../../xtask/tests/report.rs) for executable canonicality/binding coverage.
