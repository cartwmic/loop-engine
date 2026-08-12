# Implementation report

Report what implementation delivered against accepted plan.

Required metadata:

- non-empty `revision` and `author`;
- `plan_revision` matching current `plan.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered repository documents, each with `path` and declared `revision`.

Also record concise `summary`, `changed_surface`, and `validation` entries. Report completed work and meaningful deviations; do not use report claims to waive a configured review obligation. Coverage manifest makes repository and document scope explicit for external reviewers.
