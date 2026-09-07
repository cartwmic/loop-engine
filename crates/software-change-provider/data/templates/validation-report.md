# Validation report (contract version 2)

Use the frozen intent's existing AC-N identities and explicit `criterion_policy`, independently of review-axis counts. Inspect its operating context, accepted risks and outside obligations. With Bookends enabled, reviewers inspect each `prd_traceability` disposition: a candidate still blocks final completion, and not-applicable never waives or fulfills a criterion. A live requirement token is not semantic proof. With Bookends disabled, do not invent PRD coverage or GREEN claims. Retain required document-audit and repository proof. Earlier profiles retain their original report contract and require their fixed original provider; this template does not migrate them.

From the validation draft, pipe ordinary `loop-engine --json show RUN` into:

```sh
software-change run-validation --engine ABS --working-directory ABS --revision REV
```

The unbound helper executes the plan's named `proof_commands` through engine ad-hoc capture. `--commands ID,...` selects execution only; it does not waive unselected final obligations. Explicit applicable steering may replace argv or owner with a reason, not the obligation. Shell prose is never executed. Commands run serially, with `--timeout-ms N` (default 1200000 per command). Nonzero, timeout, missing executable, changed repository identity and unavailable/incomplete captures cannot pass. Inspect all retained failures; helper exit 1 retains candidates/report but means command failure. The helper does not append, advance, invoke reviewers or create a bound draft binding.

The report is a fixed index, not a freehand outcome database:

- `revision`, truthful script `author`, and current `implementation_revision`;
- `command_evidence_ids`, referencing genuine command-evidence records for every final named proof;
- `criteria: [{criterion_id, verdict_ids}]`, exactly once per current AC-N;
- `goal_verdict_ids`, separate whole-intent judgment(s), never AC-0.

The schema is `validation-report-schema.json`. Inspect inert command candidates and append them with their caller-named IDs. They reference `capture: {summary, assignment_id}` under this run's artifact root, not a fabricated invocation. Existing command evidence for the same effective command and checkpoint tree can be explicitly indexed without rerunning it.

Before review, check that the prechosen verdict IDs are unused, finish the index, and create `validation-checkpoint.json`. Keep its bytes fixed. Normal validation still requires matching accepted implementation-proof-history. Append actual later external judgments with existing `append --record-id`; never insert reservation/placeholder records. Missing names may be pending at `validation-ready`, but ordinary approval (or a reviewless draft-to-end hop) requires complete independent evidence.

Each `criterion-verdict` names `criterion_id`, `subject: validation-report.json`, `subject_revision`, `checkpoint: validation-checkpoint.json`, `author`, `result`, `findings` (empty array for pass, nonempty strings for fail), and earlier `evidence_context_ids` naming retained command evidence. A `goal-verdict` has the same fields without criterion_id. Neither report nor implementation authors count. Bound combined commissions may return `{record_id,kind,data}` entries in `validation_verdicts` alongside axis judgments; `review-candidates` emits inert `verdict-ready` candidates. Append their concise origin unchanged; core resolves capture identity and the provider verifies the exact raw row. Ordinary axes consume this collection; challenge consumes it without commissioning another criterion review or rerunning commands.

For repair, append `criterion-revalidation: {subject_revision, affected_criteria, change_kind, reason}`. `change_kind` is `material` or `report-index-only`. Affected criteria require fresh judgments. Explicit unaffected `evidence-applicability` names the original verdict, current report/checkpoint, attesting driver and short reason. Index the applicability ID instead of a fresh verdict ID. Material repair requires a fresh goal; only an explicitly explained report-index-only correction can carry it. `commission --slot validation-review` shows pending, fresh and carried rows with original author/result/source.

Failures remain failures. The validation-review finding ledger may disposition exact criterion/goal sources (`policy_id` is the source AC-N, or `goal` for the whole-intent source). Accepted-unresolved findings block across revisions; rejection/resolution does not rewrite the original judgment. Carry never aliases unknown criteria or silently declares affected judgments unchanged. Independent semantic review must judge whether retained command outputs establish the outcome and whole intent; the deterministic provider checks relationships, not truth.
