# Backlog

Wanted work and concrete leads. Completed work belongs in Git history. Items are grouped by topic, not implementation priority. Inclusion does not authorize implementation or establish requirements; [docs/PRD.md](docs/PRD.md) owns the engine contract.

Reconciled against released v0.25.0 (`144a40c072755893ebba97fff17842a50ffc252f`). Delivered external-work capabilities under LE-178–183 are no longer future work. Open issues retain concrete defects; related issue references below connect this backlog to them without making inclusion an implementation authorization. Removing delivered entries does not relabel `dogfood-review-teething-20261001` as ordinary GREEN: it completed with three scoped overrides for six original pre-lint proof fingerprints; its refusals and original evidence remain history. No fresh whole-matrix, hosted delivery or installed deployment is implied.

## PRD work

### Audit implementation coupling in the current PRDs

Audit all live requirements and normative prose in the engine and software-change PRDs—not just recently changed sections—for unnecessary coupling to implementation specifications.

Retain technical details when they are genuinely part of the intended product requirement, with an explicit rationale; existing wording or implementation alone is not justification. Otherwise propose rewriting, relocating to specifications, tombstoning or replacing the requirement as appropriate. Preserve required outcomes, intentional architectural constraints, genuine compatibility promises and requirement-ID history.

Account for the whole audited scope with concise dispositions and flag uncertain product decisions for the owner. Completion requires owner acceptance and application of the agreed changes, not merely an inventory or a few examples. This audit is independently actionable; it does not need to wait for the broader workflow below.

### Build a comprehensive PRD-centered workflow

Bring PRD creation, refinement, prototype-informed discovery, current-product backfill and faithful conversion/reconciliation into a coherent workflow. Consider the PRD-generation provider the owner is building at work alongside this repository's existing Generate-PRD research profile and PRD drafting/reconciliation procedures before deciding what to combine or replace. Do not assume that the answer is one new provider, a separate prototype provider or another parallel workflow.

Keep these outcomes explicit:

- **Define and refine PRDs:** Support product discovery and owner iteration, using suitable requirements-writing practices and templates. Produce a PRD that captures intended product outcomes and constraints rather than accidentally freezing implementation choices.
- **Use prototypes to inform requirements:** Allow a prototype step, subworkflow or accompanying provider where experimentation helps resolve product uncertainty. Iterate with the owner and carry the resulting learning into PRDs and specifications. Prototype code and incidental design choices do not automatically become production code or accepted requirements.
- **Codify current-product backfill:** Incorporate the current-product backfill process into the PRD workflow: inspect current intended product behavior for requirements missing from the PRD, distinguish enduring obligations from incidental implementation choices, and propose evidence-backed additions or amendments for explicit owner acceptance. This item codifies a reusable current-product process; it does not commission a new backfill audit or a search of historical runs. History may help explain a specific current behavior, but is not the audit target.
- **Preserve meaning during conversion and reconciliation:** Make conversion of an already accepted PRD into the living Bookends format preserve its accepted meaning. Prevent omitted obligations, invented requirements and automatic promotion of goals, examples or future-work ideas into binding requirements. Preserve genuine normative constraints, flag ambiguities or contradictions for the owner rather than silently resolving them, and require explicit acceptance of any intended change in meaning. Review the actual candidate against the accepted source, not merely a reassuring synthesis report. Demonstrate that representative defective conversions are caught even when grammar and identifier checks pass. Choose the smallest adequate procedure; do not assume a new ledger system or a fixed set of additional review stages.
- **Connect PRDs to delivery:** Provide a coherent handoff from owner-accepted requirements to Bookends-enabled software change. The five-artifact coverage obligation below remains separately actionable, not deferred until the entire PRD workflow is built.

Demonstrate representative paths through the workflow to an owner-accepted PRD usable for software change. A mechanically completed workflow or a schema-valid document alone does not establish a satisfactory PRD.

## Other wanted improvements

Observations marked as separate 0.22.0 dogfooding below came from another machine; their captures are not in this checkout. Recheck source evidence or reproduce current behavior before changing product contracts.

### Rethink the research workflow

Rethink the research workflow so the owner will actually use and trust it: investigate a query as thoroughly as reasonably practical, then synthesize findings that satisfy the research question and intent.

Establish what reasonable coverage and a satisfactory synthesis mean with the owner, and demonstrate them on representative real queries. Make material gaps and uncertainty visible; workflow completion alone does not establish research quality. Assess current external authoring and evidence handoffs as part of that outcome; retired bound-authoring machinery is not the definition of this work. Detailed workflow design remains future work.

### Evaluate calibration and attestation needs across providers

Investigate whether and how providers beyond software-change merit calibration and attestation processes, or whether a more general evaluation workflow would serve them better. The purpose is to establish and maintain useful review judgments, not merely prove that commands run and verdicts are stored correctly.

Use software-change's experience to assess how representative acceptable and defective examples can reveal missed problems and false alarms. Establish who can justify the expected judgments, what is provider-specific, what could be shared, and the maintenance cost. Deliver an evidence-backed recommendation. Do not assume every provider needs the full software-change calibration process or that a shared framework must be built.

### Make human status and read output useful on active runs

The external-work release removed the old human `status details` JSON tail and retired engine-owned invocation/Dagu execution. Earlier observations of enormous fan-out packets, live Dagu collection and wrapped status blobs remain historical leads, not established defects in v0.25.0. Do not rebuild retired execution monitoring to address them.

Measure the current supported status/action/full, history, list and instruction/guidance surfaces on a representative messy active run. Establish what the owner needs at a glance: current work, next action, blocker, evidence location, freshness and uncertainty. Distinguish external driver reports from engine-observed durable facts; an absent progress note is not proof of completion.

Find remaining duplication and byte cost using before/after measurements. Keep complete information accessible through authoritative records and locators, preserve non-arming status and existing observation/output contracts, and fix demonstrated readability problems. Do not assume that earlier multi-gigabyte measurements still describe the current path or add a parallel monitoring system.

The earlier observation that drivers discard `show` output remains a lead to understand, not proof that another punitive gate is needed. Address actual reading cost and instruction quality without claiming that a server-side observation proves an agent understood the result. External proof wait/deadline policy is tracked separately in #21; draft correction routes are tracked in #14.

### Preserve applicable work and evidence through correction

LE-150/171/177/182 and the selected-work/applicability paths are delivered baseline. Close demonstrated remaining gaps without replaying unaffected work for ceremony: malformed or replaced immutable records (#7, #17, #20), duplicate advice occasions (#15), and stage/order-sensitive ordinary clearance with fresh or carried evidence (#16, #18).

Preserve original work, failed attempts, source identities and genuine unresolved findings. Support auditable correction/retirement and inspect actual dependency/scope effects; refresh affected proof and finish normally. Keep #17 and #18's distinct acceptance cases even where their causes overlap other issues. Reappending the same early challenge assessment must not make it a new post-clearance judgment. Do not build generalized replay, silently ignore malformed active obligations or add a second criterion ledger.

### Make review packets complete, citeable and usable

LE-149 and current commission filtering deliver relevant context and accessible originals without arbitrary content quotas. Existing tests cover initial aggregate blindness and retained confirmation evidence. The historical report of indirect individual-stage applicability conclusions reaching an initial aggregate remains a reproduction lead; do not claim a current leak or recommission the delivered baseline without evidence.

Close current packet gaps: applicable non-intent steering and confirmation context (#4), readable checkpoints/scoped implementation changes (#9), and a supported way to cite required reconstructions, comparisons and owner sources (#19). Necessary originals must remain accessible; compact output alone does not prove the reviewer received correct material. Preserve required combined perspectives, applicable findings and blindness.

Align intake with those packets and the effective evaluator contract: source-locator checks (#5), shared assessment identity and useful diagnostics (#6, #11), Bookends overlay axes (#10), residual shape and pre-review validation (#8), source-faithful finding statements (#12), and intent-baseline/correction routing (#13). The current baseline check applies before ordinary intent commissions too; do not retain the older challenge-only description.

The v3 contract removes reviewer hash and rationale-length requirements and permits faithful driver-confirmed projection of non-JSON returns. This supersedes #1 and #3's bound-worker contract problems, not every possible diagnostic defect. Preserve useful source returns; investigate any current nested-schema/format diagnostic concern only when reproduced, rather than silently truncating judgments or reviving retired output repair. Keep read-surface duplication and human digest work in the separate item above.

### Maintain review quality against actual misses

Grounded pass reasons, locator-only source references, truthful intent approval, merit-based triage, explicit partial/output-only recovery, invalid-reuse refusal, factual evidence preview/apply and per-criterion distinguishing proof are delivered baseline duties under LE-151/152/154–159. Do not recommission their implementation wholesale.

Retain real misses and sound judgments for the required fresh/applicable-history calibration when judging meaning changes. The shipped external-work contract includes owner-alignment and causal acceptance-sufficiency questions and granularity/plain-English guidance; its manual prompt trials are diagnostic, not complete calibration or universal model-quality proof. First ordinary reviews remain comprehensive, affected confirmation preserves valid evidence, and challenge tests a cleared conclusion rather than adjudicating open ordinary failures.

### Evaluate external advisory backends and judgment quality

The generic typed advisory primitive and software-change's bounded evidenced questions are delivered under LE-163–166. The shipped external-work contract preserves enabled advice as driver-performed work, with genuine typed values and reasoned dispositions, not engine-owned dispatch or gate authority.

Remaining investigation concerns useful judgment quality and feasible backends: compare sufficient-evidence bounded cases against reasoning judgments; inspect misses, false alarms and confidently wrong answers; measure real interface limits, latency, resource contention, installation and licensing. Evaluate a local model only if useful, with an owner-selected endpoint or host and honest unavailability; no model, download, deployment or default change is selected here.

Do not revive the earlier proposed automatic cascade or send investigation, planning, missing-fact inference, cross-artifact or whole-run synthesis through the bounded advice interface. Advice does not replace independent reviews, alter author floors, accept requirements or authorize exceptions. Driver-owned reasoning/reconsideration remains outside that classifier contract. The general cross-provider evaluation/framework item above is separate.

## Cross-project work

### pi-subagents: verify recovery, result retrieval and lifecycle fixes

First check current public-tool behavior for three reported gaps:

- Recovery handles rejected because of `launchContractDigest` compatibility.
- Retrieving the correct completed result without private-directory parsers.
- Independent acceptance of the shipped compaction/shutdown fixes.

Close cases already resolved and fix only demonstrated remaining problems in pi-subagents. Result retrieval must distinguish execution state, output validity, harness acceptance and domain judgment, preserving usable completed results even when a later harness failure occurs.

Exercise interrupted/revived attempts, compaction, missing or malformed results and partial parallel completion with a scripted backend. Do not edit recovery descriptors or disable compaction to make tests pass.

### Chezmoi: reusable release-tag skill import

Turn the repeated import of Loop Engine skill bundles from an immutable release tag into a small reusable dotfiles utility. Reuse the established canonical layout and existing deployment tooling rather than introducing another skill tree or deployment system.

Verify the tag and exact imported bytes, detect unexpected orphans, and support reviewable changes followed by targeted apply and deployed-byte/harness-link verification. Demonstrate a real release import through the deployed result; preserve older installed tool versions needed by frozen runs.

## Investigate only if it recurs

These are concrete recurrence triggers, not active investigations. [Investigation dispositions](investigations.md) retains supporting context separately from this backlog.

- **Recovery-test failure:** If `recovery_composed_public_terminal_and_outer_failure` fails again without reaching its intended missing-evidence rejection, retain the actual stdout, stderr and work captures, then investigate the cause. Do not assume that a passing rerun explains or fixes the failure.
- **Premature worker exit:** If a worker again stops before finishing its task but returns a successful exit code, retain its transcript, stdout/stderr, task result and the decision about whether the work was complete. Establish the cause and responsible component before proposing a fix.
- **Early worker termination after a failed invocation:** If a newly invoked worker is again killed (`signal: terminated`) seconds after start, before its model runs, and shortly after a failed or cancelled invocation on the same slot, retain both invocations' ownership records, captures and timestamps and establish whether cleanup of the earlier invocation reached the new one. Do not assume an immediate successful rerun explains it.
- **Older software-change/CLI dogfood leads:** Defer minor `show`/`@PATH`/payload-less `event` diagnostics unless repeated ordinary use makes them material. Recheck the earlier `proof_id` documentation complaint and contract-v2 `run-validation` refusal against current supported contracts before treating either as an active defect; 0.22.0 `run-validation` emits `proof_id` in command-evidence candidates and its recovery contract accepts contract versions 2 and 3, so only template/schema documentation of `proof_id` for hand-written reports may remain.
