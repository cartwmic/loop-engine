# Backlog

This file records candidate work that has not yet been accepted as a product requirement. [docs/PRD.md](docs/PRD.md) remains the sole requirement-ID authority. Promote an item through the normal intent and design process before treating it as binding.

The initial entries come from the 22-Aug-2026 blind review of Bookends commit `531532c` by Claude Fable 5, OpenAI Codex Sol, and Cursor Grok 4.6.

## Release blockers

### Restore continuity in shallow required-CI checkouts

**Context:** Required CI checks out the repository at the default depth of one. The checker cannot resolve `HEAD^`, maps the missing parent to first adoption, and therefore reports GREEN for continuity violations that report RED in a full clone. Fable reproduced this with the same invalid PRD transition in full and depth-one clones.

**Evidence:**

- `.github/workflows/preflight.yml` uses `actions/checkout@v6` without `fetch-depth`.
- `crates/bookends-check/src/git.rs::first_parent` maps an unavailable parent to `None`.
- `crates/bookends-check/src/continuity.rs` treats no parent as no continuity baseline.
- Superseded run `run-1787334584350890000-1-98697` had accepted the need for non-shallow CI history.

**Smallest scope:**

- Set `fetch-depth: 2` for the required preflight checkout.
- Make parent unavailability in a shallow repository RED instead of treating it as first adoption.
- Preserve legitimate first adoption in repositories that truly have no parent requirement state.

**Done when:**

- A full clone and a depth-limited CI-equivalent clone both reject live-ID disappearance, tombstone removal, title reassignment, and revival.
- A real first adoption remains GREEN.
- The public required-CI path proves the behavior, not only an internal seam test.

### Persist local bypass invocation evidence

**Context:** The accepted intent requires every `class:reason` bypass to be visible and durably recorded. The landed checker prints the bypass, but the local pre-push path leaves no later-retrievable record. Sol identified this as a literal intent gap; the same concern was accepted in superseded run `run-1787334584350890000-1-98697`.

**Evidence:**

- `scripts/bookends-check-gate.sh` forwards `BOOKENDS_BYPASS` but persists nothing.
- `.githooks/pre-push` executes the wrapper without a durable sink.
- `crates/bookends-check/src/main.rs` prints `BYPASS`, class, and reason to stdout.

**Smallest scope:**

- Choose the smallest durable destination appropriate for a trusted sole operator.
- Record invocation time, repository, checked revision or tree identity, bypass class, reason, and outcome.
- Fail closed if a requested bypass cannot be recorded.
- Do not add a service, registry, multi-user audit system, or generalized policy framework.

**Done when:**

- A black-box local pre-push invocation using `BOOKENDS_BYPASS=class:reason` succeeds only after leaving retrievable evidence.
- A recording failure prevents the bypass from permitting the push.
- Normal GREEN and RED invocations keep their current behavior.
- Software-change continues to treat BYPASS as non-GREEN.

## Follow-ups

### Make Generate-PRD verification wording honest

**Context:** `scripts/generate-prd-journey.py` performs three predetermined exact-line lookups but writes that no contrary repository extract was found “after the deterministic search.” No contrary-evidence search occurs. The candidate extracts, parser validity, provisional status, and human gate remain valid.

**Smallest scope:** Replace the unsupported claim with wording that states the exact extract is narrow and does not establish completeness. Do not add search infrastructure.

**Done when:** Generated verification evidence describes only work the deterministic journey actually performed, and the Generate-PRD self-test and source journey still pass.

### Decide push-range continuity behavior

**Context:** Immediate-parent continuity checks only the pushed tip against its parent. An invalid middle commit in a multi-commit push can escape if a later commit becomes the tip. This is a routine trusted-operator workflow, but immediate-parent-only continuity was an accepted minimal-design boundary.

**Decision:** Choose one:

1. Check each commit in the bounded pushed range at pre-push and required-CI boundaries; or
2. Retain immediate-parent-only behavior and explicitly document that intermediate commits in a multi-commit push are outside v1 continuity guarantees.

Do not turn this into an unbounded history-authority scan.

**Done when:** The chosen boundary is explicit in intent, schema documentation, tests, and public gate behavior, with no claim that invalid intermediate commits are caught unless they are actually checked.

### Retain exhaustive audit matrices

**Context:** Two implementation workers reported independent LE-1 through LE-90 citation-placement audits as 90/90, but their row-level scratch matrices were removed. Later auditors can see the result claim but must redo the mapping to falsify it.

**Smallest scope:** For future exhaustive semantic-placement audits, retain a compact run artifact containing requirement ID, requirement text or digest, citation location, exercised scenario, and observable assertion. Keep this as review evidence, not checker product machinery.

**Done when:** A fresh reviewer can inspect or sample the retained matrix without reconstructing all mappings, and the artifact is bound to the reviewed repository revision.

### Add a post-commit run-to-commit pointer

**Context:** Final implementation and validation reports correctly describe the pre-commit state as `939c7d6+uncommitted-worktree`. After commit, no durable pointer links that exact reviewed surface to landed commit `531532c`. Historical reports should not be rewritten.

**Smallest scope:** Add a separate post-commit record that names the run ID, report revisions, landed commit, and proof that the committed pathname set and bytes match the reviewed worktree. Keep the run reports immutable.

**Done when:** A later auditor can move from the final run evidence to the landed commit without relying on chat history, while the original reports remain unchanged.

## Latest-run candidate triage

These items came from operating the Bookends software-change run. They are ordered by perceived implementation complexity, not priority. Each entry is a candidate for later intent/design work, not an accepted requirement.

### Bounded candidates

#### Reject citations outside approved Bookends surfaces

- **Observed:** Bookends indexes citations only in the class pathspecs from `bookends.toml`. A `bookends:LE-<n>` token in another tracked file is ignored rather than rejected.
- **Candidate:** Scan citation tokens across the tracked textual tree and reject tokens outside approved proof surfaces. Keep the configured class pathspecs as the authority for where citations may count.
- **Open questions:** Define treatment for the PRD itself, generated files, fixtures, vendored content, skipped files, and binary/non-text files. Compare the final rule with Compass before choosing syntax or exclusions.

#### Diagnose Dagu status through `invocation-progress`

- **Observed:** The latest run suggested that Dagu state was not visible or reliable through Loop Engine inspection. The documented driver surface is `invocation-progress`; direct `dagu status` and `dagu history` are underlying implementation details.
- **Candidate:** Reproduce inspection while a plan graph is running and after it completes, then fix the smallest confirmed break in locator discovery, Dagu invocation, status mapping, or presentation.
- **Open questions:** The defect may depend on locator timing, invocation lifecycle, Dagu version, or completed-run behavior. Do not design a replacement inspection surface before reproducing it.

#### Add honest public proof for concurrent context appends

- **Observed:** LE-38 says concurrent context appends alone do not make an in-flight evaluation stale. An internal concurrency test exercises that behavior, but the Bookends citation in the public software-change journey only says its state-race fixture makes no guarantee about context appends. The tagged scenario therefore does not semantically prove the requirement.
- **Candidate:** Add a public CLI scenario that blocks a checked evaluation, appends context concurrently, releases the evaluation, and proves the evaluation may commit from its original context snapshot while both the append and transition remain durable. Keep the scenario focused on the declared v0.1 boundary rather than broadening concurrency semantics.
- **Open questions:** Confirm the expected public outcome and diagnostics for both `allow` and `deny`, and whether one representative result is sufficient when internal tests retain the complete matrix.

#### Reframe adversarial review terminology

- **Observed:** “Adversarial” and repeated “attack/falsify” framing can push reviewers toward security-style overreach even though the reviewer protocol limits findings to material failures against frozen intent.
- **Candidate:** Use “challenge review,” “devil's-advocate review,” or “weakness-exposure review” in human-facing titles and instructions. Preserve existing state and gate IDs in the first iteration to avoid a topology/config migration for a wording change.
- **Open questions:** Pick one term, test whether it changes review quality, and decide later whether internal IDs merit migration.

### Moderate candidates

#### Calibrate plan and validation review axes

- **Observed:** Plan review has `done-observable` and `decision-free`, while validation review has `intent-delivered` and `requirement-proof-mapping`. The latest run still admitted plans without an explicit pragmatic black-box/E2E bar, packets that could over-specify implementation, and validation evidence that could describe completed work without proving the change works as intended. In a Bookends-enabled repository, a citation can also be mechanically present while the tagged journey or contract assertion does not semantically cover the cited requirement.
- **Candidate:** Revise and calibrate the shipped axes so plans require validation gates and user-path proof where realistic, reject both under-specified and over-specified task packets, and require validation to demonstrate the frozen outcome rather than internal activity. For every new or changed Bookends citation, validation review should inspect whether the tagged scenario and observable assertion actually prove the cited requirement instead of accepting token placement as coverage. This also covers the broader review-axis coverage follow-up.
- **Open questions:** “Where realistic” needs concrete fixtures so reviewers do not demand impossible E2E tests or invent infrastructure. Determine whether existing axes should be sharpened or new axes added.

#### Deliver steering mechanically to later software-change work

- **Observed:** LE-51 was tombstoned because the implementation proves only that durable `user-steering` reaches `show` and checked evaluation requests. Software-change draft and implementation slots do not receive it, review slots receive only `accepted-findings`, and the cited journey did not prove that steering changed later work or authorization.
- **Candidate:** Define a workflow-level steering-delivery contract that names intended later recipients, carries applicable durable steering to bound workers or evaluations without relying on driver memory, and proves an observable later result changes because of it. Keep context opaque to Loop Engine core and do not expand this into live interruption, which remains deferred.
- **Open questions:** Define recipient selection, ordering and supersession, whether steering applies to already-frozen artifacts or only later phases, and how a driver records that steering was incorporated when work remains unbound.

#### Route policy-document findings into later review work

- **Observed:** Policy-document review evidence and denial diagnostics are durable and visible to the driver and provider, but bound semantic-review workers receive no forwarded context. LE-58 and LE-59 therefore describe possible reuse of previous findings without mechanically delivering those findings to the later external reviewer.
- **Candidate:** Give each later semantic review or revision commission a durable, current-target finding packet derived from prior accepted findings or review evidence. Preserve raw evidence and digest identity; the delivery mechanism must not reinterpret findings or let stale evidence satisfy current conformance.
- **Open questions:** Decide whether to forward selected `review-evidence`, introduce a normalized workflow-specific finding record, or have the driver construct a digest-bound packet; define supersession and handling of findings against changed target bytes.

#### Freeze operating threat model and risk posture in intent

- **Observed:** The intent schema freezes constraints and non-goals but has no explicit operating threat model or accepted risk posture. Security-focused reviewers can therefore assume a stronger environment than the owner accepted and drive extra mechanisms.
- **Candidate:** Carry a concise operating context, threat boundary, and explicit risk acceptance from frozen intent through design, plan, implementation, and validation review packets. Reviewers must judge within that boundary unless the change itself invalidates it.
- **Open questions:** Decide whether this belongs in new intent fields or a stricter convention over existing constraints/non-goals. Keep real outside obligations distinct from owner preferences, and do not let accepted risk become a waiver for failing stated acceptance.

#### Add a driver-owned implementation commit checkpoint

- **Observed:** `run-plan-graph` completes tasks and a summarizer, but the resulting work may remain uncommitted. Having each parallel task worker commit would conflict with the shared working directory and create merge/ownership policy inside the workflow.
- **Candidate:** Add a driver-owned checkpoint after graph summary/triage and before implementation review. A repository-specific deterministic finalizer may create and verify the commit; Loop Engine and the software-change provider remain unaware of Git mechanics and consume only bounded completion evidence.
- **Open questions:** Determine whether this is an operator-configured finalizer binding, external evidence required by the implementation transition, or only a stronger driver instruction. The design must preserve explicit commit authorization and must not treat command exit zero as proof of the expected repository state.

#### Add reviewer-output conformance recovery

- **Observed:** Fan-out checks only declared top-level output keys. A reviewer that emits malformed or incomplete JSON can fail the whole fan-out before the semantic result reaches the summarizer.
- **Candidate:** Preserve the raw output, run deterministic schema validation, and allow a focused shape-repair worker before summarization. The repair step may recover representation but may not change the reviewer's semantic verdict or invent findings.
- **Open questions:** Define the full schema, how repair provenance is retained, when to retry the original reviewer instead, and what remains a hard failure. A generic “schema fixer” must not silently become a second reviewer.

#### Add public contract tests with Bookends IDs

- **Observed:** Bookends supports an optional `contract` class, but this repository currently configures only `e2e_journey`. Public contracts therefore have no separate allowlisted proof surface carrying `bookends:LE-<n>` citations.
- **Candidate:** Identify real public contract-test boundaries, configure the Bookends contract class and required CI collection, and thread living PRD IDs through those tests.
- **Open questions:** Define which boundaries are contracts rather than journeys, avoid duplicating the same assertion in both classes, and keep citations at durable public assertions rather than internal unit-test seams.

### High-complexity candidates

#### Separate enduring product requirements from implementation specifications

- **Observed:** The living PRD now gives requirement IDs to a heterogeneous set of product outcomes, architecture boundaries, exact CLI and JSON contracts, Dagu/helper details, journey implementation, CI wiring, and cargo-dist assertions. The policy-document and research sections mix enduring workflow behavior with provider mechanics and test/release requirements; the work-slot section is especially coupled to current commands, helpers, paths, packet fields, and Dagu. Several requirements would therefore need edits after an implementation replacement that preserved product behavior, while Section 16 says exact CLI grammar, field names, framing, and internal representation belong to technical design.
- **Candidate:** Reshape the requirement catalog around stable user/operator/integrator outcomes and intentional product constraints across core, software-change, policy-document, research, and work-slot delegation. Move exact public protocol encodings into a versioned interface contract, implementation choices into technical design, and test/release mechanics into validation and release policy. Do not copy research's explicit “blackbox tests exist” requirement into policy-document; require public-boundary journey proof through repository validation policy instead. Preserve Bookends traceability from those journeys and contract tests to the smaller living requirement set.
- **Open questions:** Identify which exact interfaces are intentional compatibility promises, choose authoritative homes for extracted material, and plan tombstone/new-ID continuity without weakening current public proof or losing historical rationale.

#### Add a revision-scoped finding ledger and selective review routing

- **Observed:** `accepted-findings` is durable per review gate/revision and is supplied to review workers, but implementation workers do not receive a normalized finding packet. A revised artifact can also trigger the whole fan-out even when only some axes need confirmation. Churn/thrash remains an operator judgment outside the bound review work.
- **Candidate:** Produce a durable routing record that names each accepted finding, owning phase, affected task IDs, affected review axes, subject revision, and disposition. Add a bound semantic meta-review that categorizes candidate findings against frozen intent, configured policy, pragmatism, YAGNI, KISS, maintainability, correctness, and UX. The driver still inspects that judgment before append. Use the record to select confirmation reviewers, reuse same-revision passing axes where valid, and escalate detected churn to the human.
- **Open questions:** Define authority between raw reviewers, the meta-reviewer, and the driver; stale-record rules; what counts as a meaningful revision; and when a prior pass is invalidated. Mechanical propagation/audit and semantic classification must remain separate.

#### Add incremental plan-graph retries

- **Observed:** Re-invoking `run-plan-graph` traverses the whole DAG with model calls. A failed node, an accepted implementation finding, or a later review revision can therefore repeat successful tasks that do not own the defect.
- **Candidate:** Make task results addressable by task ID, plan revision, dependency inputs, and repository state. Feed routed accepted findings into affected implementation packets, rerun the invalidated subgraph, and regenerate the implementation summary from reused plus new task evidence.
- **Open questions:** Define safe reuse when tasks mutate one shared checkout, dependency invalidation, partial side effects, changed plan packets, report revision identity, and the operator's ability to force a clean rerun. A successful process exit alone cannot make a node reusable.

#### Add an explicit in-run operator override

- **Observed:** Existing runs expose only provider-described events and checked transitions; a frozen obligation cannot be waived. There is no owner-only way to bypass one state after an explicit decision.
- **Candidate:** Explore a separate operator override control that records the requested state/event, reason, skipped checks, actor, and resulting history without pretending the skipped transition passed.
- **Open questions:** Determine which states can ever be overridden, whether final safety gates remain non-overridable, how providers represent the resulting run, and whether termination/restart should remain the only valid answer. This must not weaken ordinary event selection or silently rewrite history.

#### Add evidence-backed replay for replacement runs

- **Observed:** Restarting with corrected frozen bindings/profile data requires a new run and re-traversal. Copying databases, artifact roots, captures, or frozen bindings is unsafe, while manually omitting states changes the workflow being proved.
- **Candidate:** Add a distinct replay/fast-forward path for a replacement run that references qualifying prior artifacts and evidence, re-evaluates each transition under the new run's frozen obligations, and stops at the first incompatible or failed gate.
- **Open questions:** Define artifact/revision identity, policy compatibility, evidence freshness, author independence, binding changes, and audit history. Replay must not clone runtime state or convert historical success into an unchecked current pass.

#### Generalize provider evaluation and calibration

- **Observed:** The software-change provider ships a calibration procedure and fixtures, while other providers may rely mainly on deterministic journeys and ad hoc semantic review. Those prove mechanics but not reviewer/prompt quality.
- **Candidate:** Evaluate whether a provider-neutral workflow should compare policy prompts, expected findings, false positives, and model behavior across software-change, policy-document, research, and future providers.
- **Open questions:** Determine which semantics are actually shared, what remains provider-specific, who owns gold judgments, and whether the existing software-change calibration can be reused without turning evaluation into provider runtime behavior.

#### Create a prototype-to-spec workflow

- **Observed:** Software-change freezes relatively strong intent/design/plan artifacts before implementation. Early product work may instead need a disposable prototype, owner iteration, and learning before a build-quality specification is possible.
- **Candidate:** Explore a lower-ceremony workflow that scopes a question, builds and iterates a prototype with the owner, records what was learned, then produces a candidate intent/design/spec for a normal quality implementation workflow. Prototype output is evidence, not production code by default.
- **Open questions:** Decide whether this is a research profile, a new provider, or orchestration across existing providers; when requirements become frozen; what prototype code may survive; and how the final specification avoids laundering accidental prototype choices into requirements.
