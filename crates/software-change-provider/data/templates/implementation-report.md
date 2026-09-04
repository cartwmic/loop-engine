# Implementation report

Report what implementation delivered against accepted plan.

Required metadata:

- non-empty `revision` and `author`;
- `plan_revision` matching current `plan.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered repository documents, each with `path` and declared `revision`.

Also record concise `summary`, `changed_surface`, and `validation` entries. Each validation entry is a closed `{proof}` record and may carry an optional `criterion_id` naming the current intent `AC-N` criterion it proves. Report completed work and meaningful deviations; do not use report claims to waive a configured review obligation. Coverage manifest makes repository and document scope explicit for external reviewers. Criterion references are optional; they must be well formed, locally duplicate-free, and present in the current intent, but they do not form a complete matrix or a second PRD-ID spine.
