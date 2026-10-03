---
name: using-research-provider
description: Use when scoping a research question, gathering sources externally, commissioning independent verification and synthesis review, and clearing research transitions in Loop Engine.
---

# Using the research provider

research validates artifacts, revision links and external judgments; it never searches,
fetches, writes conclusions or calls models. Scope → gather → verify → synthesize → end.

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

Prepare with `research setup --rigor standard --output /absolute/profile.json`. For installed use,
`research data-dump DIR` materializes embedded data into an empty destination.
Register an absolute research command under alias research in provider TOML.
Confirm the resolved profile; do not retrofit policy into an active run.

```sh
loop-engine --json --config /absolute/providers.toml start research @/absolute/profile.json "research question"
```

Read artifact_root from full show. Use data/templates and the frozen artifact schemas;
fixed subject files are brief.json, sources.json, verification.json and report.json.

## External research work

1. Scope: write a question, observable acceptance, constraints and non-goals, not
   a predetermined answer. Author brief.json and request scoped.
2. Gather: search/fetch externally; record stable source IDs, locators and exact
   extracts. sources.json must link current brief_revision; request gathered.
3. Verify: cite source_ids for claims, state support and genuine counterevidence or
   an honest unsuccessful challenge search. Link current sources_revision.
4. Synthesize: answer the brief using verified claims and claim_id/source_id citation
   pairs. Link current verification_revision; do not introduce unchecked material claims.

Material subject changes need new revisions; retaining revision asserts immateriality.
Do not hide known defects by bumping a revision.

## Gate map

| State/event | Checks and next step |
|---|---|
| scope / scoped | brief schema; enter gather |
| gather / gathered | sources schema and brief link; enter verify |
| verify / verified | verification schema/link and verify review; enter synthesize |
| synthesize / completed | report schema/link and synthesize review; enter end |

Use shown check-free correction routes: gather revise returns scope; verify revise
returns gather and revise-brief returns scope; synthesize revise returns verify,
revise-sources returns gather and revise-brief returns scope. Verification-local or
report-local corrections stay in their owning state and retry its checked event.

## Per-gate loop

1. Read action/full and author the subject externally. Request its checked event;
   schema/link denial requires repair before review. Evidence denial after valid
   shape is not a semantic reviewer failure.
2. Run `research commission verify` or `research commission synthesize` with completed
   full-show JSON on stdin. Supply missing_inputs before treating a packet as ready.
   Read exact policies/example prompts, selected sources and effective author counts.
3. Commission independent external reviewers against each applicable axis/group.
   Preserve the request and original return; retain declared self-review facts.
   First review is comprehensive; confirmations cover accepted fixes and new holes.
4. Submit a review-candidates stdin packet with show, assignment, author, request
   and result. assignment names gate, review_stage, subject, subject_revision and
   policy_ids; request.text and result.text retain the genuine exchange.
   `research review-candidates` projects candidates, not semantic approval.
5. Inspect diagnostics and triage candidates before append. Append actual accepted
   review-evidence; request verified or completed only after required coverage.

For shared intake, a JSON return uses items with policy_id, result and findings.
Multi-axis assignments must stay within one declared group and provide assessment_id;
one assessment is not extra independent authors. Non-JSON returns need a driver-confirmed
projection while retaining original text. Top-level self_review must be truthful and
explicitly enabled for eligible authors by the frozen profile. Eligible subject authors
then count once as labeled self-review, not independent or cold review. Required counts
change only through authorized durable author-count amendment.

## Evidence rules (condensed)

Evidence binds gate, policy_id, result, findings, author, subject, subject_revision
and config_version. Results are pass/fail; failures require actionable findings.
Use exact frozen config_version, not a guessed shipped version.
Latest conforming current verdict per axis/author stands; one standing failure blocks.
Distinct authors use name/kind identity; respect self-review exclusions and groups.
Wrong revisions/config are stale, never passing coverage. Malformed attributable
records need conforming supersession, not another author's fabricated pass.

Inspect `research author-counts` with full show on stdin. To change counts, use
`research author-counts --propose JSON` and inspect/append the returned candidate:
name exact targets, after count, actor, owner authority reference and reason.
A count change does not edit an axis, rewrite old judgments or excuse known failures.
Zero applicable axes require no invented reviewers. Explicit applicability is needed
for reuse; declare source scope and current identity rather than relabeling old evidence.

## Recovery and completion

Late findings need current evidence, violated obligation, consequence, validation gap
and provenance (newly exposed, fix-introduced or previously overlooked). Do not drip-feed
unrelated requirements or waive material defects because review was previously clear.
Driver owns external timeout/cleanup; verify no overlapping writer before retry.
Route upstream defects to their owning phase, then refresh invalidated subjects/review.

At end, hand off cited conclusion, source/claim mapping, terminal run observation,
artifact locators, genuine review decisions, unresolved uncertainty and residuals.
Synthetic journey passes prove mechanics, not source truth or semantic quality.

Small pre-start overrides use `--set review.GATE.AXIS.required_authors=N`,
`--set review.GATE.AXIS.self_review=true`, and `--set review.GATE.AXIS.group=NAME`.
Policy-document uses gate `semantic-review`; research uses `verify` or `synthesize`.
Use `--add-axis GATE.NEW=GATE.EXISTING` or `--remove-axis GATE.AXIS` for membership.
Inspect the output and `FILE.explain.json` (base identity, overrides, effective policy).
To select a newer base, replay the options with `--previous-explain OLD.explain.json`;
inspect drift and resolve refused missing targets before start. Raw prompt/schema
replacement remains a full custom file; preparation never launches work or changes a run.
