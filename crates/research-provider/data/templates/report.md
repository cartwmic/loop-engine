# Report artifact

Write `report.json` as a cited conclusion grounded in verified claims. The provider does not judge conclusion quality.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when the conclusion is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `verification_revision`: must equal current `verification.json` revision when the `completed` gate is checked.
- `conclusion`: the cited answer to the brief question.
- `citations`: one or more `{claim_id, source_id}` pairs tying the conclusion to verified claims and gathered sources.

Report-local defects stay in synthesize: edit and recheck `report.json`, then retry checked `completed`. Nearest check-free `revise` is verification-owned only; use `revise-sources` for sources-owned defects or `revise-brief` for brief-owned defects. Do not waive known defects.
