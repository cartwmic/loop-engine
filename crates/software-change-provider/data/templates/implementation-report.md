# Implementation report

Report what implementation delivered against accepted plan. Preserve actual focused command outcomes and remaining proof honestly; a provisional report does not claim the driver-owned stable-tree matrix, document audits, requirement acceptance, benchmark comparison, hosted exact-commit checks or later live dogfood passed. Workers run assigned focused checks; reviewers consume retained evidence rather than repeat suites. The designated proof owner supplies complete final validation and repeats only invalidated checks.

Required metadata:

- non-empty `revision` and `author`;
- `plan_revision` matching current `plan.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered repository documents, each with `path` and declared `revision`.

Also record concise `summary`, `changed_surface`, and `validation` entries. Each validation entry is a closed `{proof}` record and may carry an optional `criterion_id` naming the current intent `AC-N` criterion it proves. Report completed work and meaningful deviations; do not use report claims to waive a configured review obligation. Coverage manifest makes repository and document scope explicit for external reviewers. With Bookends enabled, use the existing document manifest, summary, changed surface and validation links to explain what implementation delivered for the accepted in-scope requirements and applicable explicitly named documents; record unresolved mismatches for reconciliation instead of treating IDs or a document list as coverage. Bookends-off reports add no PRD-ID duty. Criterion references are optional; they must be well formed, locally duplicate-free, and present in the current intent, but they do not form a complete matrix or a second PRD-ID spine.
