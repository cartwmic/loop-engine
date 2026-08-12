# Intent artifact

Write `intent.json` as product intent, not implementation plan.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when change is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `problem`: who or what is affected and why current behavior fails.
- `outcome`: externally observable result desired.
- `acceptance`: concrete outcomes an outside observer can check.
- `constraints`: limits imposed by outside obligations; do not turn preferences into constraints.
- `non_goals`: adjacent capabilities intentionally excluded.

Keep target separate from solution. Do not launder a chosen implementation into problem statement. Acceptance lines should state results, not work instructions. Reviewers judge materiality against this intent; minor blemishes, style, and invented norms do not decide a gate.
