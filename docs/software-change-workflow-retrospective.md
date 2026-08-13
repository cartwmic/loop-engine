# Software-Change Workflow Calibration Retrospective

Date: 2026-08-13

## Situation

The `meaningful-backlog-hardening` run reached its implementation phase, but progress stalled in calibration work. Calibration envelopes advanced through v18. Each reviewer-visible input change invalidated prior attestations, bumped the neutral corpus revision, rehashed all 70 rows, and triggered another fresh 70-call round.

This produced useful corrections early. Later rounds mostly optimized benchmark purity after the corpus had already become reliable enough for its purpose. v18 now has:

- 70/70 expected verdict matches;
- 35 PASS results with empty findings;
- 35 FAIL results with non-empty findings;
- independent semantic agreement on all 70 verdicts;
- no reviewer-visible expected class or target verdict;
- a few non-blocking wording and construction-context caveats.

No product release changed. v0.2.2 remains immutable. No implementation commit, tag, push, publication, or run attestation occurred during this calibration loop.

## How We Got Here

The workflow had strong correctness rules but weak stopping rules.

1. We treated every calibration concern as potentially blocking before classifying its consequence.
2. We coupled corpus edits to global revision and attestation invalidation. A one-line fixture correction forced all 70 rows through fresh review.
3. We used full-corpus review rounds while still developing the corpus. Targeted tests and targeted reviews did not become the default after failures were localized.
4. We broadened “oracle resistance” from preventing expected-class leakage into hiding generic calibration construction. The latter does not tell a reviewer which verdict to return.
5. We required findings to approach ideal axis wording even when their verdict and material core were independently correct.
6. Review feedback had no enforced severity threshold. “Could be cleaner” competed with wrong verdicts and real contract defects.
7. The implementation run became the place where the calibration benchmark itself matured. Product hardening and benchmark development became one critical path.

Early rounds found meaningful defects: incorrect profile-transport semantics, a dropped evidence decision, incomplete task sources and ownership, and reviewer-visible text that disclosed selected conflict mechanics. Fixing those defects improved validity. Later rounds showed diminishing returns: generic envelope labels, harmless QA context, and unnecessary clauses in otherwise correct findings became reasons to consider resetting the corpus again.

## Why This Is Bad

The process violated KISS and YAGNI even though individual steps followed the written rules.

- **It optimized the instrument instead of shipping the change.** Calibration exists to establish that semantic review is trustworthy enough. It is not the product.
- **It made small edits globally expensive.** Exact-byte integrity is valuable, but global invalidation turned local corrections into repeated full campaigns.
- **It rewarded ceremonial work.** More rounds and more evidence looked like rigor without proportionate reduction in product risk.
- **It obscured material risk.** Time spent refining already-correct findings displaced final implementation validation and review.
- **It created an unreachable quality bar.** Natural-language review will always have variance and imperfect wording. Requiring perfect outputs guarantees endless corpus churn.
- **It weakened operator judgment.** Rules replaced the decision about whether a finding could change a verdict, permit a defect, or cause a realistic regression.

Rigor without a materiality threshold becomes bureaucracy. Perfect calibration is neither achievable nor required for a useful workflow.

## Changes for Future Software-Change Runs

### 1. Define the calibration threat model before execution

Calibration blocks only for:

- wrong semantic verdict;
- reviewer-visible expected class or target verdict;
- materially false or unsupported finding;
- missing evidence that makes the verdict indeterminate;
- a defect that could realistically admit bad work or reject good work systematically.

Generic calibration framing, neutral QA context, stylistic weakness, and removable secondary clauses are non-blocking unless they reveal the answer or alter the material judgment.

### 2. Require materiality classification

Every review finding must be classified before any edit:

- **Blocker:** invalidates verdict, independence, or evidence integrity.
- **Material:** exposes a realistic workflow or product failure and merits correction.
- **Advisory:** wording, cleanliness, or hypothetical concern without demonstrated decision impact.

Only blockers and material findings may reset calibration. Advisory findings are recorded and deferred.

### 3. Add explicit stopping criteria

A corpus is good enough when:

- all expected verdicts match in one fresh round;
- PASS rows contain no material findings;
- FAIL rows contain at least one correct, material, axis-scoped reason;
- exact input identity and reviewer identity validate;
- independent audit finds no blocker under the stated threat model.

Once these conditions hold, stop. Do not reopen the corpus for advisory improvements during the run.

### 4. Separate corpus development from implementation gating

Develop and stabilize calibration fixtures before starting a full-rigor implementation run. Freeze the corpus revision at run start. During implementation, change it only for a demonstrated blocker. Broader corpus improvements belong in a separate maintenance change with their own review and budget.

### 5. Localize invalidation and review

When a defect affects specific packet bytes, rerun and re-audit only affected rows unless shared system instructions, shared prompts, shared schemas, or pairing identities changed. Preserve global exact-byte checks, but do not equate one local fixture edit with evidence that every unaffected semantic decision became stale.

If current attestation format cannot represent safe row-level reuse, improve that mechanism separately. Do not work around it with repeated full campaigns.

### 6. Budget calibration rounds

Set a default budget before execution:

- one baseline round;
- one correction round;
- one final confirmation round.

Exceeding the budget requires a written blocker: affected rows, demonstrated consequence, why targeted rerun is insufficient, and owner approval. Round count is not a quality metric.

### 7. Make owner judgment an intentional gate

Automation verifies mechanics. Reviewers provide semantic evidence. The owner decides whether remaining risk is material. Workflow documentation must state that independently correct verdicts do not fail because prose could be cleaner.

### 8. Keep synthetic evidence in its lane

Synthetic calibration demonstrates deterministic transport, separation, and broad reviewer behavior. It cannot prove universal semantic quality. Production confidence must also come from focused implementation review, executable validation, and observed workflow behavior. Do not make the synthetic corpus carry the whole assurance burden.

## Decision for the Current Run

Freeze v18/r15. Do not create v19 for advisory wording or generic construction-context concerns. Treat the 70/70 round and independent verdict agreement as sufficient calibration evidence. Proceed to attestation and final implementation validation, subject only to a newly demonstrated blocker under the threat model above.
