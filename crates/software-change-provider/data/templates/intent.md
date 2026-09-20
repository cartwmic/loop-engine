# Intent artifact

Write `intent.json` as product intent, not implementation plan.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when change is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `operating_context`: the frozen boundary later actors must use when judging the change:
  - `operators`: a non-empty array of non-empty strings naming who operates or reviews the change.
  - `environment`: a non-empty array of non-empty strings naming the supported execution environment.
  - `threat_boundary`: `{in_scope, excluded}`, each a non-empty array of non-empty strings. Record the trusted operating boundary; do not invent hostile-user or multi-tenant demands outside it.
  - `accepted_risks`: an array, possibly empty, of `{risk, rationale}` objects with non-empty strings. An accepted risk records a residual; it never waives a stated outcome or outside obligation.
  - `outside_obligations`: an array, possibly empty, of `{source, obligation}` objects with non-empty strings. These obligations remain binding even when a risk is accepted.
- `problem`: who or what is affected and why current behavior fails.
- `outcome`: externally observable result desired.
- `acceptance`: a non-empty array of closed `{id, statement}` criterion records. Each `id` is a stable run-local `AC-N` identity matching `^AC-[1-9][0-9]*$`; `statement` is the concrete outcome an outside observer can check. Preserve an ID while its meaning is materially unchanged and use a new ID for replacement.
- `constraints`: limits imposed by outside obligations; do not turn preferences into constraints.
- `non_goals`: adjacent capabilities intentionally excluded.

Every object in the artifact is closed: do not add fields outside the schema. Keep target separate from solution. Do not launder a chosen implementation into problem statement. Acceptance lines should state results, not work instructions. Use one authoritative intent surface: put the owner's plain-English problem, outcomes, scope boundaries, acceptance decisions, uncertainty, and qualifications in this artifact, and keep exact technical references and structured metadata alongside that prose rather than creating a second narrative. Explain a specialist term when it first matters; do not use a readability score, ban technical language, or hide a binding clause in another document. Shape acceptance criteria as coherent bounded outcomes with practical evidence: combine clauses that share one decision, split independently varying outcomes, keep inseparable aspects together, and do not promise proof that cannot realistically be obtained. Do not use word counts, conjunction counts, fixed criterion counts, style, or review ceremony as a substitute for that judgment. With Bookends disabled, AC-N is the only criterion spine; do not add PRD dispositions, candidate/liveness metadata, citations, or Green claims. Bookends-enabled runs add their one `prd_traceability` disposition per criterion through the overlay, not through this base template; that disposition may be linked-live, candidate, or not-applicable, and not-applicable does not waive or fulfill the criterion. For new semantic-coverage profiles, inspect the actual normative wording of every cited live requirement and every authoritative document it explicitly names. Record the sufficient-existing-wording, change-specific-proof, or missing/changed-enduring-meaning classification in the intent's existing traceability and concise author/driver notes; preserve substantive provisional wording and explicit owner-acceptance/application/commit status for a candidate. A related ID, shared topic, matching token, or parser-valid candidate is not semantic coverage. If delivered behavior is wrong under sufficient wording, retain an implementation-defect reclassification instead of inventing a requirement. This is drafting/review guidance, not an automatic semantic matcher. Later reviewers and operators must inspect this frozen operating context before judging; material outcomes and outside obligations are never waived.
