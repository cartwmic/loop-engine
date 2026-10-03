---
name: using-software-change-provider
description: Use when driving software-change intent, design, plan, implementation, reconciliation and validation with externally performed work, independent reviews and driver-owned finding disposition.
---

# Using the software-change provider

## Scoped discovery and delivery decisions

Discover the operator path, observable outcome, constraints, accepted risks and non-goals
before drafting. Ask about consequential ambiguity; do not turn a chosen mechanism into
intent. Budget the serial dependency closure, implementation report, proof and review.
Escalate a concrete budget blocker instead of silently retrying.
Simple-first/YAGNI/KISS apply: complexity needs a current requirement/failure and an
inadequate simpler alternative, not hypothetical future hardening.

software-change validates schemas, revision links, checkpoints and external evidence.
It never executes agents, advisors or proof commands and never judges findings for you.

## Required companion and driving minimum

Load repository skills/using-loop-engine/SKILL.md, especially Deterministic setup,
Exact profile confirmation and Choose an observation. Use explicit --config and
inspect LOOP_/XDG/home redirects. Production isolation requires current owner approval;
tests always use driver-owned catalogs/artifacts outside the checkout.

Action/full show arms mutation; status/compact, monitor, list and history do not.
Re-observe after each transition. Use full show as helper stdin, not a bounded view.
Parse workflow envelopes even on failure; only completed means success.
Global --timeout-ms bounds provider transport (default 30000), not external workers.
The engine never executes agents, advisors or proof commands.

## Exact profile confirmation

Display and hash the exact per-run profile, confirm its identity, all live policies,
groups/stages, self-review rules and author counts with the user, and rehash before
start. Separately confirm external role/model commands, write ownership, budget and
proof owner. Never substitute a model or silently reduce an axis/floor.
Started runs retain frozen policy; inspect effective author-counts and use only
explicit owner-attested amendments for supported count changes.

## Setup

Use `software-change data-dump DIR` for installed embedded data in an empty destination.
Choose a shipped minimal, standard or high-rigor profile; read its exact policy lists.
Generate a policy-only per-run file, explicitly choosing advice enablement:

```sh
software-change setup --rigor standard --output /absolute/profile.json --decline-advice
# Alternatively, use edited complete profile bytes:
software-change setup --profile /absolute/copied.json --output /absolute/profile.json --enable-advice
loop-engine --json --config /absolute/providers.toml start software-change @/absolute/profile.json "change"
```

`setup` edits policy only. Tools prepare or evaluate data, not external execution.
Setup starts no run or worker. Inspect output/explanation and effective policy;
--set, --add-axis and --remove-axis are explicit policy edits, not executor controls.
Confirm every changed obligation, author floor and advice/Bookends state before start.
Use --bookends only for an explicit opt-in. Read the resulting exact config/contract
versions; do not apply current contracts to historical runs or silently migrate them.
Register the absolute provider command under exact alias software-change.

## Cold reconstruction comparison

For enabled `intent-sufficient`, declare the pre-change `intent.baseline` and retain
it across revisions unless the owner explicitly selects a new baseline. Commission
unbriefed readers with only current intent and that source baseline. Retain every
counted reconstruction via `review-candidates`; intake snapshots the baseline.
Before ordinary approval, append the owner or explicitly owner-authorized
`intent-comparison` described in the reviewer protocol, addressing every counted
return and useful alternative. Clarify unwanted permitted alternatives in intent
and obtain a fresh cold exercise; private comparison comments are not constraints.
Prepare informed challenge only after this alignment clearance. Missing comparison,
partial coverage and changed reconstruction baseline block clearance. Scripted
journey declarations prove these mechanics only, never genuine semantic alignment.

## Frozen intent and operating boundary

Read current intent.json before authoring, commissioning, triaging or validating.
Share its operating_context: operators, environment, threat_boundary, accepted_risks
and outside_obligations. Accepted risks are residuals, never waivers of acceptance or
outside obligations. Do not demand excluded speculative hostile-user/multitenant controls.

Write one authoritative plain-English intent with exact technical references and
structured metadata. Explain specialist terms when needed, preserve qualifications,
and avoid a second narrative or readability-score rule. Acceptance entries are closed
{id, statement} records with AC-N IDs (positive integers, no leading zeros). Preserve
identity for unchanged meaning; replace IDs for changed meaning. Group inseparable
outcomes, split independently varying ones, and name a practical evidence path or
concrete reason proof is impractical. Word/conjunction counts are not findings.

Inspect qualified user-steering originals and supersession, not incorporation summaries.
Later material intent revisions need an exact prior-intent owner source for byte-bound
delta; a driver paraphrase is not a baseline. Owner approval views from commission
are not owner approval. Keep drafter/reviewer/driver/owner identities separate.

## Gate map and artifact authoring

Author from embedded data/templates and exact frozen artifact schemas under artifact_root.

| Phase | Durable subject and obligation |
|---|---|
| Intent | intent.json: outcome, boundary and AC-N criteria |
| Design | design.json: current intent link and sufficient simple solution |
| Plan | plan.json: current design link, dependencies, observable task completion |
| Implement | implementation-report.json: actual changes and current plan link |
| Reconciliation | reconciliation.json: delivered meaning versus authoritative docs |
| Validation | validation-report.json: fixed current proof/criterion/goal index |

Contract-v3 plan tasks require nonempty unique current criterion_ids. Optional design,
implementation and proof references must name current criteria, not a second spine.
Use shown events (intent-ready, design-ready, plan-ready, implementation-ready,
reconciliation-ready and validation-ready where present), not a remembered topology.
Empty review lists omit rooms; zero axes require no fabricated evidence.
Checked drafts validate shape/links. Reconciliation has no report/checkpoint prerequisite.
Review states expose phase-owning revise routes. Validation-local report corrections
stay in validation; repository-state mismatch uses shown revise-implementation.

## Per-gate loop and external commissioning

1. Observe action/full. Perform the draft externally, then check schema/links before
   collecting judgments whose subject might otherwise change.
2. Pipe completed full show to `software-change commission --slot SLOT`; select
   --stage where needed. Read missing_inputs and supply them before launch.
   Preserve frozen operating context, selected steering, ledger and original sources.
3. Commission comprehensive first review across assigned axes/groups and exact author
   counts. For high rigor, honor individual then aggregate stages and their identities.
   First ordinary review is independent of peers. A parent pass is not challenge input
   until ordinary clearance; challenge must falsify its claim against frozen intent.
4. Preserve genuine request and original response. `software-change review-candidates`
   accepts stdin with show, assignment, request and original. assignment names gate,
   stage, group, author, subject and subject_revision. The original.text holds the
   actual external return. Structured returns carry assessment_id and judgments with
   axis, result, findings and grounds (reason plus evidence locators). Non-JSON returns
   need a driver-confirmed projection without invented outcomes. Inspect projected
   records and refusal diagnostics.
5. Triage before append or mutation: accept only in-scope material failures or genuinely
   supported passes. Preserve grounds/evidence and self-review declarations. Disputed
   substantive candidates need focused independent reconsideration; reject provably
   false claims with the contradicting source fact, not schedule or re-entry cost.
6. Append accepted review-evidence and a driver-authored finding-ledger snapshot, then
   observe and request approved/passed as shown. A process exit or projector success
   never supplies semantic judgment. Ordinary clearance precedes challenge clearance.

## Ledger, applicability and finding routing

Use data/templates/finding-ledger.json. The schema-version-1 snapshot has driver author,
gate, subject, subject_revision and stable F-... finding IDs. Sources use exact
context-record IDs. Accepted findings retain unresolved/resolved/stale status and an
owning phase; rejected/advisory entries use recorded/stale, null owner and empty routes.
For implementation findings, task_ids name honest frozen task owners; empty means no
honest task owner, not permission to dodge work. External driver routes that repair.
Revise the plan when decomposition is materially wrong.

An advisory-finding-proposal is inert; inspect, accept/edit/reject, then append the
separate authoritative ledger. Do not silently drop accepted unresolved findings on
revision change. Exact-source dispositions address current fails; they are not passes.
Reviewer-manifest and retired-author reasons preserve replacement history and unchanged
independent floors. Quiet/progress/thrash concern post-triage accepted-unresolved sets.

Confirmation reviews consume that durable set and inspect fixes plus fix-introduced
holes, not a second unrestricted search. Late findings need current evidence, violated
obligation, consequence, validation gap and provenance (newly exposed, fix-introduced,
previously overlooked). Known material defects are not waived by prior clearance;
comprehensive-first and materiality burdens prohibit drip-feeding or unrelated reopening.

Reuse requires explicit evidence-applicability with original context-record origin,
current target subject/revision/checkpoint, attesting_driver and reason. Preserve author,
result and source; do not copy stale identities into new evidence or invent a pass.
Respect declared source scopes. Inspect `software-change author-counts` with full show.
Owner amendments use `software-change author-counts --propose JSON`, exact targets,
after count, actor, authority reference and reason; inspect/append the candidate.
Counts do not rewrite policy, replace failed evidence or authorize self-review.

## Reconciliation and Bookends

Read actual normative wording and authoritative documents explicitly cited by intent.
Classify delivered outcomes: sufficient existing wording (including code defects to fix),
change-specific proof, or missing/changed enduring meaning. A related ID, shared topic,
parser-valid candidate or command success is not semantic coverage.
Only changed enduring meaning needs exact owner acceptance, separately authorized
application/commit and independent inspection of resulting docs and public proof.
A justified no-document-change result is valid; retain concrete blockers otherwise.

Bookends is off by default. Enabled intent criteria each need prd_traceability:
linked-live, candidate or not-applicable. Candidate is provisional and blocks final
completion until accepted into committed PRD or reclassified; not-applicable is no waiver.
Use `software-change bookends-preview --working-directory ABS` before plan approval
for read-only prerequisite inspection, not implementation proof. Final gate needs real
GREEN, not BOOKENDS_BYPASS. Disabled runs reconcile relevant docs without inventing PRD IDs.

## Git and proof ownership

After implementation triage, obtain the owner's Git checkpoint decision before review.
Workers never stage/commit independently. Record pending/declined honestly; for an
authorized commit inspect staged names/diff and verify actual HEAD identity. A reminder
is not authorization. Settle Git before final identity-bound proof and keep tree stable.
The external proof owner runs full stable-tree checks; reviewers consume retained outcomes.
Repeat only checks invalidated by changes, never make a post-report checker its prerequisite.

Run evaluation from the intended repository checkout, not the artifact directory.
After complete reports, create read-only repository checkpoints:

```sh
software-change checkpoint --phase implementation --artifact-root ABS --working-directory ABS
software-change checkpoint --phase validation --artifact-root ABS --working-directory ABS
```

Both directories must exist and be absolute. This never changes Git. Checked admission
retains implementation-proof-history; validation cannot replace that proof with mutable
checkpoints. On stale proof, take shown revise-implementation, refresh reports/checkpoints
for the same current tree, and refresh affected independent evidence before retry.

## Prepare and finalize validation

Execute required proof commands externally, preserving real argv, cwd, exit_code,
stdout/stderr, producer and source identity. Obtain identity with
`software-change source-identity --working-directory ABS`; do not fabricate results.
Feed `software-change prepare-validation` stdin with show, working_directory, revision,
author, command_results and additions. Missing/failed/stale results need correction;
the helper launches nothing and supplies no passing verdicts.
Inspect diagnostics/commands_complete, append genuine addition_candidates and
command_candidates with their proposed IDs, then complete report_draft.

The fixed index links current implementation_revision, command_evidence_ids, exactly
one criterion row per current AC-N and separate goal_verdict_ids. Choose genuine unused
verdict IDs before checkpointing; no placeholders or rewriting index after judgment.
Criterion/goal authors exclude implementation/report authors by default. Explicit frozen
profile permission lets eligible authors count once as labeled self-review, not independent
or cold review. Required counts change only through authorized author-count amendment. Final coverage
needs all required genuine judgments, matching implementation history and no unresolved fail.
Challenge consumes that collection, not another proof run or criterion review.

After repair, append criterion-revalidation with subject_revision, affected_criteria,
change_kind material/report-index-only and reason. Materially affected judgments need fresh assessment;
criterion and whole-goal rows may carry only when the driver explicitly declares with a
reason that their complete scope and judging meaning remain applicable. Material repair
does not categorically require fresh goal judgment.

## External advice, recovery and final handoff

For enabled advice, inspect due occasion IDs. Prepare bounded, evidence-sufficient
atomic claims with `software-change advice-request` stdin; the driver sends questions
externally and appends actual typed advice-answer context. Every answer needs reasoned
advice-disposition. No admissible question uses honest advice-occasion triggered:false
with reason and empty trigger/response IDs. Timeout/invalid return is unanswered, not
negative advice. Scoped owner exceptions never satisfy review, proof or Bookends.

On external timeout/failure, retain outputs and inspect partial edits, verify cleanup
and no overlapping writer, then restore or deliberately incorporate changes. Route
correction to its owner and refresh only invalidated evidence. Out-of-scope/budget material
correction needs the owner's specific decision; visible residuals are not ordinary pass.
At actual end, return terminal observation, artifact/proof locators, criterion/goal
coverage, review/ledger decisions, residuals and pending owner Git/delivery decisions.
Synthetic journeys establish mechanics, not semantic approval or hosted exact-commit proof.

Preserve review groups, self-review declarations and effective author counts. Declare evidence applicability explicitly; check ordinary review clearance before challenge review.
