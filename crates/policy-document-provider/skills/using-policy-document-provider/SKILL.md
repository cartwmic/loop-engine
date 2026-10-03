---
name: using-policy-document-provider
description: Use when drafting or auditing README.md, AGENTS.md, or another policy document with deterministic checks and externally commissioned digest-bound semantic review.
---

# Using the policy-document provider

policy-document reads exact UTF-8 bytes, applies frozen deterministic checks and
aggregates digest-bound external verdicts. It never edits the document or calls reviewers.
Topology: prepare → deterministic-review → semantic-review → end.

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

Materialize installed data with `policy-document data-dump DIR` into an empty directory.
Prepare with `policy-document setup --rigor readme` (or `agents`) `--output /absolute/profile.json`. Set mode to draft or
audit and target.path to the absolute document. Preserve target.id and profile_version
unless intentionally making a custom profile. Both modes use the same checked topology.
artifact_root is accepted but ignored; this provider need not write artifacts.
Register absolute policy-document command under alias policy-document in provider TOML.

```sh
loop-engine --json --config /absolute/providers.toml start policy-document @/absolute/profile.json "document review"
```

Read data/target-guidance.md and data/reviewer-protocol.md for exact obligations.
Draft means externally author/revise; audit means assess existing bytes, not silently edit.
Keep guidance referential: do not invent a second product-policy authority.

## Profile map

README policies address the human overview and usable entry paths; AGENTS policies
address actionable agent guidance, source authority and safe ownership boundaries.
Use the actual selected policies, descriptions and example_prompt, not a generic
review checklist. Blind-first assessment must retain genuine independent returns;
a synthetic journey does not establish reviewer calibration.
Local references resolve beneath the target directory; normalized internal paths
are allowed but escaping that directory is not. Use prose checkout-root paths when
crate-local Markdown cannot link to root guidance.

## Run loop

1. Observe action/full. In prepare, author/revise externally or retain audited bytes.
   Request ready; it is check-free, not conformance proof.
2. In deterministic-review, request passed. On nonconformance, address every reported
   violation, use revise to prepare, and repeat ready/passed.
3. Compute lowercase SHA-256 of exact current target bytes. Keep those bytes stable
   through review/evaluation; the provider cannot lock them through engine commit.
   Append context kind `target-authorship` with data
   `{"target_sha256":"CURRENT_DIGEST","author":{"name":"ACTUAL_AUTHOR","kind":"agent"}}`.
   Declare every actual author (one record per author; kind is human, agent or script).
   A missing, malformed or stale declaration blocks semantic coverage and appears in
   commission `missing_inputs`. After any rewrite, declare authorship for the new bytes;
   `initial_input.target_author` is not supported.
4. Commission external semantic review using frozen mode, target, policies and project
   evidence. Cover all applicable groups/axes and effective independent author counts.
5. Intake original returns through `policy-document review-candidates` on stdin.
   Packet includes show, assignment, author, request and result; assignment subject
   and subject_revision both equal the current target digest. Preserve request.text
   and result.text, policy_ids and declared review_stage/self-review facts.
6. Inspect candidates/diagnostics and semantic fitness, then append review-evidence.
   The projector computes target identity; it does not judge the reviewer claim.
7. Observe and request semantic passed. This rechecks deterministic current bytes.
   On target edits, revise, rerun deterministic checks, recompute digest and obtain
   fresh current verdicts. Never reuse a stale digest as a pass.

For shared intake, a JSON return uses items with policy_id, result and findings.
Multi-axis assignments must stay within one declared group and provide assessment_id;
one assessment is not extra independent authors. Non-JSON returns need a driver-confirmed
projection while retaining original text. Top-level self_review must be truthful and
explicitly enabled for eligible authors by the frozen profile. Declared current target authors
then count once as labeled self-review, not independent or cold review. Required counts
change only through authorized durable author-count amendment.

## Explicit historical review context

No selection means no historical findings attachment. With full show on stdin,
`policy-document commission 'FROZEN_PROFILE_JSON'` returns selected original context,
current_target and diagnostics. Label attached judgments historical, compare target,
profile and digest, and never treat attachment as current coverage.

To select, add selection_data to the full-show envelope with target_id,
slot_id semantic-review, record_ids of original earlier review-evidence, and optional
supersedes referencing an earlier selection for the same target/slot. Pipe to commission,
inspect the calculated receipt and append returned data as review-context-selection.
Do not hand-author receipts. Latest selection controls; empty record_ids clears it.
After target edits, prepare a fresh selection without an old receipt. Preserve original
records and packet bytes; selection does not rewrite verdicts or establish relevance.

## Evidence record

Append the candidate data, not a wrapper with engine-generated timestamps:

```json
{
  "gate": "semantic-review",
  "policy_id": "product-fidelity",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "target_id": "README.md",
  "target_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "profile_version": "readme-4"
}
```

Use the frozen profile's version and actual digest, never this illustrative digest.
Author kind is human, agent or script; fail requires nonempty actionable findings.
Every axis needs its effective current pass count and no standing current fail.
Latest conforming verdict per policy/author stands; a later pass clears only that
same author's fail. Malformed attributable evidence needs conforming supersession.
Wrong target/profile/digest is stale. Identity claims are not signatures or proof of
reviewer independence; the driver owns truthful provenance and semantic triage.

## Author counts, recovery and handoff

Inspect `policy-document author-counts` with full show on stdin. Owner-directed
changes use `policy-document author-counts --propose JSON`: exact targets, after count,
actor, authority reference and reason. Inspect and append its candidate, never silently
change floors or remove inconvenient axes. Preserve groups/self-review declarations.

Do not invent reviews for zero applicable axes. Do not manufacture a failure from
word counts, style preferences or speculative obligations. Late material findings
still require evidence, violated obligation and concrete consequence.
Verify external cleanup and inspect partial edits before retry. End only on checked
terminal completion; return exact target digest/profile, run/catalog identity,
review decisions, residual risks and any pending owner application/Git authorization.

Small pre-start overrides use `--set review.GATE.AXIS.required_authors=N`,
`--set review.GATE.AXIS.self_review=true`, and `--set review.GATE.AXIS.group=NAME`.
Policy-document uses gate `semantic-review`; research uses `verify` or `synthesize`.
Use `--add-axis GATE.NEW=GATE.EXISTING` or `--remove-axis GATE.AXIS` for membership.
Inspect the output and `FILE.explain.json` (base identity, overrides, effective policy).
To select a newer base, replay the options with `--previous-explain OLD.explain.json`;
inspect drift and resolve refused missing targets before start. Raw prompt/schema
replacement remains a full custom file; preparation never launches work or changes a run.
