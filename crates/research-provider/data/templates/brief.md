# Brief artifact

Write `brief.json` as the research question, not a chosen answer.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when the question or scope is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `question`: the question to answer; do not present a chosen conclusion as the question.
- `scope`: what the run will cover and the time, population, or domain bounds.
- `acceptance`: observable results an outside observer can check when the report is complete.
- `constraints`: limits imposed by outside obligations; do not turn preferences into constraints.
- `non_goals`: adjacent questions intentionally excluded.

Search, fetch, and writing happen outside the provider. Reviewers judge materiality against this brief; minor blemishes, style, and invented norms do not decide a gate.
