# Reviewer protocol

Provider checks evidence shape and aggregation. A review worker decides truth externally and returns one judgment for its assigned axis. The driver triages that judgment and records one `review-evidence` context record per accepted axis judgment. `review-evidence` remains binary: `result` is exactly `pass` or `fail`; this protocol adds no verdict, severity, owner-override, or round-state fields.

## Review worker deliverable

A bound review worker is read-only and returns only one JSON object with top-level keys `axis`, `author`, `result`, and `findings`. The shipped `review-worker-preamble.txt` defines the worker role, artifact lookup through the mechanically forwarded `artifact_root`, and the authority of its frozen assignment. The shipped `review-worker-output-schema.json` declares the mechanically required keys used by opt-in bindings.

The judgment is candidate data, not provider evidence. The worker does not conduct web research, author artifacts, run deterministic checks, call `show`, append evidence, request an event, or progress the run. Those are driver duties. Exit 0 and mechanical key presence do not establish a valid deliverable; the driver must compare the values to the frozen assignment, inspect reviewer independence, and triage the captured judgment before append.

## Evidence record

```json
{
  "kind": "review-evidence",
  "data": {
    "gate": "verify",
    "policy_id": "claim-grounded",
    "result": "pass",
    "findings": "",
    "author": {"name": "reviewer-sol", "kind": "agent"},
    "subject": "verification.json",
    "subject_revision": "3",
    "config_version": "research-1"
  }
}
```

All eight fields are required. `result` is exactly `pass` or `fail`; `findings` is a string and is non-empty for `fail`. Author identity is exact `(name, kind)`. `subject` must match gate subject. `subject_revision` and `config_version` must name what was reviewed and which frozen config judged it.

Evidence gates are `verify` (subject `verification.json`) and `synthesize` (subject `report.json`). Scope and gather checked events are schema and revision-link only.

## Failure burden and scope

A failing review finding carries a **mandatory failure burden** and **consequence proof**. It must identify:

1. the violated obligation in the supplied original brief, acceptance, constraint, non-goal, or current-phase contract;
2. grounded evidence from supplied artifacts;
3. a concrete failure scenario and its consequence for research success; and
4. why existing validation does not already resolve the problem.

Judge two independent questions for every candidate concern: **materiality** (could it plausibly affect success against the brief?) and **scope** (is it within the original brief or introduced by this research run?). The matrix is:

| Materiality | Scope | Treatment |
|---|---|---|
| material | in scope | accepted blocking finding; append conforming `fail`, fix it, and review the fix |
| material | out of scope | follow-up; do not reopen current research unless current work introduced it |
| non-material | in or out of scope | advisory; do not make it block current research |

Do not use style, silence, length/count proxies, invented norms, or bounded omissions outside named obligations as findings. A candidate that cannot meet failure burden is not a blocking failure. A material finding within original brief or introduced by current work cannot be deferred as follow-up.

## Pre-append candidate triage and reconsideration

Reviewer output is candidate data until owner inspection. **Before append or mutation**, triage each candidate against failure burden, independent **scope and materiality**, evidence integrity, and current subject revision. Do not append a candidate merely because reviewer output calls it a failure, and do not mutate an artifact to evade a finding.

Append an accepted in-scope material failure as conforming binary evidence; provider aggregation then blocks normally. There is no waiver: accepted material failure remains blocking until fixed or a subject revision changes the reviewed work. Unsupported, advisory, unrelated, or burden-deficient candidates do not authorize an owner pass and are not appended as blocking failures. If owner disputes a candidate's evidence, consequence, materiality, or scope classification, request **focused external reconsideration** of that candidate only. Give reconsideration the disputed evidence and original-brief linkage; append only returned conforming evidence that meets this protocol. Focused reconsideration does not silently turn a disputed material concern into approval.

## Review rounds

The **comprehensive first review** is the first ordinary review: inspect all supplied evidence and report all material findings visible within configured axis scope. Do not spend the first round on only one preferred concern.

After accepted findings are fixed, a **confirmation review** is bounded: verify each accepted fix, affected-scope behavior, downstream consistency, and regressions introduced by the fix. Confirmation does not reopen unrelated advisory concerns or repeat a full search for already-settled material claims.

A late material finding remains actionable and is not waived because it arrived after approval or confirmation. A **late material finding** proof names **current supplied evidence**, violated in-scope obligation, concrete consequence, **validation gap**, and provenance explaining whether the issue was **newly exposed**, **fix-introduced**, or **previously overlooked**. Provenance explains timing; it is not an exclusion test: **previous visibility or reviewer overlook does not waive** a known material defect. When that burden is met, accept the finding and route it to its owning phase; timing never changes its materiality. Comprehensive first review remains mandatory, so this rule does not permit drip-feeding findings. Unrelated reopening still carries the independent scope and materiality burden above.

The default **three-round circuit breaker** limits ordinary review rounds. After three rounds, owner consolidates unresolved material cases and either commissions one bounded review using the consolidated case or routes work directly to the phase owning the defect. Budget exhaustion changes review method, never verdict: it **never waives a known defect**, and an unresolved accepted in-scope material finding remains blocking.

## Owning-phase routing

A review operator selects the phase that owns an accepted material defect. Use phase-named check-free events exposed by the static graph:

| Review state | Nearest `revise` | Direct owning-phase events |
|---|---|---|
| `verify` | `revise` → `gather` | `revise-brief` → `scope` |
| `synthesize` | `revise` → `verify` | `revise-sources` → `gather`; `revise-brief` → `scope` |

Local corrections stay in-state. Verification-local `verification.json` corrections stay in verify: correct the artifact and retry checked `verified`. Report-local `report.json` corrections stay in synthesize: correct the artifact and retry checked `completed`. In `verify`, use nearest `revise` only for a sources-owned defect. In `synthesize`, use nearest `revise` only for a verification-owned defect. Use `revise-sources` for sources-owned defects and `revise-brief` for brief-owned defects. After any fix, confirmation covers affected scope and downstream regressions before the review gate is attempted again.

## Convergence

The research run terminates only when **no unresolved accepted in-scope material finding** remains and the cited conclusion is independently evidenced. **Zero advisory comments is not required**. Provider validates and aggregates evidence; external reviewers and owners perform semantic judgment, candidate triage, round accounting, and route selection. Round state stays outside provider runtime.

## How to judge

Judge only configured axis. Deny only for a defect plausibly affecting research success against its brief and meeting failure burden. Minor blemishes, style preferences, length/count proxies, silence, and invented norms are not findings. Do not hunt bounded omissions outside axis scope. Do not waive material finding: evidence is not a vote, and a material defect remains material until fixed or revision changes.
