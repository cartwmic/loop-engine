# Implementation report

Report what implementation delivered against accepted plan. Preserve actual focused command outcomes and remaining proof honestly; a provisional report does not claim the driver-owned stable-tree matrix, document audits, requirement acceptance, benchmark comparison, hosted exact-commit checks or later live dogfood passed. Workers run assigned focused checks; reviewers consume retained evidence rather than repeat suites. The designated proof owner supplies complete final validation and repeats only invalidated checks.

Required metadata:

- non-empty `revision` and `author`;
- `plan_revision` matching current `plan.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered repository documents, each with `path` and declared `revision`.

Also record concise `summary`, `changed_surface`, and `validation` entries. Each validation entry is a closed `{proof}` record and may carry an optional `criterion_id` naming the current intent `AC-N` criterion it proves. Report completed work and meaningful deviations; do not use report claims to waive a configured review obligation. Coverage manifest makes repository and document scope explicit for external reviewers. With Bookends enabled, use the existing document manifest, summary, changed surface and validation links to explain what implementation delivered for the accepted in-scope requirements and applicable explicitly named documents; record unresolved mismatches for reconciliation instead of treating IDs or a document list as coverage. Bookends-off reports add no PRD-ID duty. Criterion references are optional; they must be well formed, locally duplicate-free, and present in the current intent, but they do not form a complete matrix or a second PRD-ID spine.

## Review inputs and first report check

Implementation reviewers inspect the actual declared-baseline-to-covered-commit change against frozen intent/plan, not the report as its own proof. The driver supplies necessary meaningful inputs and chooses tools; no tool ban or mandatory bundled diff is introduced. Preserve accessible source originals and retained command outcomes. Supplied-only calibration never resolves fictional repository labels against a live checkout.

The first implementation report schema check is at report-bearing `reconciliation-ready`; the earlier `implementation-ready` handoff remains report-free. Generated repository checkpoints are readable multiline JSON. Internal content/state hashes keep their existing meaning; the physical file checksum changes with whitespace. Accepted history retains actual verified bytes/checksum rather than rewriting older compact proof.

## Residuals

Optional `residuals` records name `kind`, a concrete `statement`, and `references`; optional `accepted_by` is the object `{"name":"actual accepting actor","kind":"human|agent|script"}` (choose one kind), not a scalar. Record actual attribution, not inferred acceptance. Kinds: `accepted-boundary`, `unresolved-defect`, `proof-gap`, `unsupported-operation`, `exception`. An accepted risk never waives an outcome. Never rename an unresolved defect as an accepted boundary; disclose proof gaps and unsupported operations honestly.

Optional `source_coverage` lists `{ "scope": "primary", "commit": "<current HEAD>" }` for every declared source scope. Multiple scopes are retained in checkpoint `repositories`; a stale secondary repository refuses approval.
