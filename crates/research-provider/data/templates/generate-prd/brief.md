# Brief artifact (generate-PRD)

Write `brief.json` as the research question, not a chosen answer.

The question is to extract a schema-valid living markdown PRD candidate for the current repository. Scope the run to this checkout: tracked documents, tests, and other reviewable repository evidence. Do not present a finished PRD as the question.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when the question or scope is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `question`: extract a schema-valid living markdown PRD candidate for the current repository; do not present a chosen conclusion as the question.
- `scope`: this repository's living product claims that can be backed by reviewable repository evidence.
- `acceptance`: observable results an outside observer can check when the report is complete, including that each proposed requirement carries reviewable repository evidence.
- `constraints`: proposed IDs stay inside the published grammar; a human must accept or reject before any commit to `docs/PRD.md`; this extract is not a software-change precondition.
- `non_goals`: certifying semantic completeness, auto-committing the candidate, calling software-change evaluate, or inventing a fourth Loop Engine provider.

Search, fetch, and writing happen outside the provider. Reviewers judge materiality against this brief; minor blemishes, style, and invented norms do not decide a gate.
