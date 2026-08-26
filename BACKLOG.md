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

#### Add a concise CLI status and progress view

- **Observed:** `show` and `invocation-progress` expose durable state and bound-work progress, but following a long software-change run still requires reading large JSON envelopes and correlating graph details manually. The latest run also suggested that Dagu-backed task state was not always visible or reliable through `invocation-progress`; direct `dagu status` and `dagu history` are underlying implementation details rather than the driver-facing contract.
- **Candidate:** Add a deterministic human-readable CLI view for one run that concisely presents lifecycle, current state, requestable events, latest checked outcome, active or latest invocation, and available inner task counts/statuses. Derive it from existing engine operations rather than creating a second state authority, preserve machine-readable JSON, and first reproduce and repair any confirmed `invocation-progress` mapping defect.
- **Open questions:** Decide whether this is a new `progress` command or a compact `show` mode, which provider-neutral fields are stable, how unavailable inner progress is displayed, and whether completed, failed, overrun, reused, and skipped tasks need distinct presentation. Black-box CLI proof must cover a running graph, a completed graph, and unavailable progress without requiring the operator to invoke Dagu directly.

#### Add honest public proof for concurrent context appends

- **Observed:** LE-38 says concurrent context appends alone do not make an in-flight evaluation stale. An internal concurrency test exercises that behavior, but the Bookends citation in the public software-change journey only says its state-race fixture makes no guarantee about context appends. The tagged scenario therefore does not semantically prove the requirement.
- **Candidate:** Add a public CLI scenario that blocks a checked evaluation, appends context concurrently, releases the evaluation, and proves the evaluation may commit from its original context snapshot while both the append and transition remain durable. Keep the scenario focused on the declared v0.1 boundary rather than broadening concurrency semantics.
- **Open questions:** Confirm the expected public outcome and diagnostics for both `allow` and `deny`, and whether one representative result is sufficient when internal tests retain the complete matrix.

#### Reframe adversarial review terminology

- **Observed:** “Adversarial” and repeated “attack/falsify” framing can push reviewers toward security-style overreach even though the reviewer protocol limits findings to material failures against frozen intent.
- **Candidate:** Use “challenge review,” “devil's-advocate review,” or “weakness-exposure review” in human-facing titles and instructions. Preserve existing state and gate IDs in the first iteration to avoid a topology/config migration for a wording change.
- **Open questions:** Pick one term, test whether it changes review quality, and decide later whether internal IDs merit migration.

### Moderate candidates

#### Deliver steering mechanically to later software-change work

- **Observed:** LE-51 was tombstoned because the implementation proves only that durable `user-steering` reaches `show` and checked evaluation requests. Software-change draft slots still do not receive steering; implementation and review slots receive the current `finding-ledger` context, but the cited journey did not prove that steering changed later work or authorization.
- **Candidate:** Define a workflow-level steering-delivery contract that names intended later recipients, carries applicable durable steering to bound workers or evaluations without relying on driver memory, and proves an observable later result changes because of it. Keep context opaque to Loop Engine core and do not expand this into live interruption, which remains deferred.
- **Open questions:** Define recipient selection, ordering and supersession, whether steering applies to already-frozen artifacts or only later phases, and how a driver records that steering was incorporated when work remains unbound.

#### Route policy-document findings into later review work

- **Observed:** Policy-document review evidence and denial diagnostics are durable and visible to the driver and provider, but bound semantic-review workers receive no forwarded context. LE-58 and LE-59 therefore describe possible reuse of previous findings without mechanically delivering those findings to the later external reviewer.
- **Candidate:** Give each later semantic review or revision commission a durable, current-target finding packet derived from prior accepted findings or review evidence. Preserve raw evidence and digest identity; the delivery mechanism must not reinterpret findings or let stale evidence satisfy current conformance.
- **Open questions:** Decide whether to forward selected `review-evidence`, introduce a normalized workflow-specific finding record, or have the driver construct a digest-bound packet; define supersession and handling of findings against changed target bytes.

#### Normalize mechanically exposed reviewer candidates for driver triage <<<

- **Observed:** Completed bound review invocations now always expose stable assignment identity, selected attempt or coverage gap, selected-output digest, and originating path through durable `show`; those records remain inert, and provider checks bind driver-authored claims to the exact selected bytes. The driver still reads each selected output and hand-authors provider-shaped `review-evidence` plus the authoritative `finding-ledger` snapshot. No provider-level triage view extracts normalized `result` and `findings` fields from the already exposed selected output.
- **Candidate:** Optionally derive an immutable, nonauthoritative provider candidate view from each selected schema-conforming output, carrying its existing run/gate/subject/axis/assignment/attempt provenance plus normalized result and finding text. Present it as a convenience bundle for driver accept/edit/reject/ownership/routing; never auto-promote it into review evidence, ledger authority, carry, routing, or checked-transition satisfaction. Keep Loop Engine core opaque to finding semantics and leave unbound/external review on its current hand-authored path unless separately accepted.
- **Open questions:** Decide whether normalization saves enough driver work beyond reading the durable selected outputs; if so, prefer a software-change provider command or projection rather than a generic engine finding model. Define malformed/exhausted representation and retry deduplication. Black-box proof must preserve exact selected-attempt provenance, show normalized candidates appear without hand-authored finding JSON, remain inert before driver confirmation, and prove only the later driver-authored record can affect a checked transition.

#### Add a driver-owned implementation commit checkpoint

- **Observed:** `run-plan-graph` completes tasks and a summarizer, but the resulting work may remain uncommitted. Having each parallel task worker commit would conflict with the shared working directory and create merge/ownership policy inside the workflow.
- **Candidate:** Add a driver-owned checkpoint after graph summary/triage and before implementation review. A repository-specific deterministic finalizer may create and verify the commit; Loop Engine and the software-change provider remain unaware of Git mechanics and consume only bounded completion evidence.
- **Open questions:** Determine whether this is an operator-configured finalizer binding, external evidence required by the implementation transition, or only a stronger driver instruction. The design must preserve explicit commit authorization and must not treat command exit zero as proof of the expected repository state.

#### Add public contract tests with Bookends IDs

- **Observed:** Bookends supports an optional `contract` class, but this repository currently configures only `e2e_journey`. Public contracts therefore have no separate allowlisted proof surface carrying `bookends:LE-<n>` citations.
- **Candidate:** Identify real public contract-test boundaries, configure the Bookends contract class and required CI collection, and thread living PRD IDs through those tests.
- **Open questions:** Define which boundaries are contracts rather than journeys, avoid duplicating the same assertion in both classes, and keep citations at durable public assertions rather than internal unit-test seams.

#### Speed up workspace tests and black-box journeys

- **Observed:** Loop Engine catalogs and Python-journey work-roots are already isolated: each scenario gets its own SQLite file, provider TOML, artifact directory, and dummy workers. Independent `work_slot_journey.prove_*` functions in `scripts/software-change-journey.py::_run_dummy_worker_proofs` still run one after another, each with a distinct `work_dir`. Source full mode then concatenates a single-run workflow walk, successor routes, stitched source, engine-boundary scenarios, dummy proofs, checkpoint scenarios, and the Bookends overlay. Implement-plan workers independently re-run `cargo test`, clippy, and the same public journeys against the shared checkout `target/`, which serializes on Cargo's build-directory lock. One `software-change-journey.py --mode source --traversal-depth full` plus the sibling policy-document, research, and Generate-PRD source journeys plus workspace cargo is hour-class wall clock on a machine that can run isolated processes in parallel. Isolation is present and unused for scheduling. One active caller per run is a product constraint and is not the defect.
- **Candidate:** Keep public coverage and fail-closed assertions. Parallelize independent black-box proofs that already use distinct work-roots. Give concurrent cargo and journey processes isolated compiler directories so they do not wait on one `target/` lock. Do not parallelize hops that must share one run's journal. Do not treat graph-local same-revision `exit 0` as remainder-carry. Measure and record wall-clock for `cargo test --workspace`, clippy, and each public source journey before and after.
- **Open questions:** Which `prove_*` functions are independent versus sharing fixtures or profile mutation; whether CI preflight should batch journeys; whether implement-plan task workers should be forbidden from re-running the full journey suite when a designated `docs-prd-journeys` node owns that proof; and what rustc/linker RAM bound is safe for parallel cargo on this machine.

### High-complexity candidates

#### Separate enduring product requirements from implementation specifications

- **Observed:** The living PRD now gives requirement IDs to a heterogeneous set of product outcomes, architecture boundaries, exact CLI and JSON contracts, Dagu/helper details, journey implementation, CI wiring, and cargo-dist assertions. The policy-document and research sections mix enduring workflow behavior with provider mechanics and test/release requirements; the work-slot section is especially coupled to current commands, helpers, paths, packet fields, and Dagu. Several requirements would therefore need edits after an implementation replacement that preserved product behavior, while Section 16 says exact CLI grammar, field names, framing, and internal representation belong to technical design.
- **Candidate:** Reshape the requirement catalog around stable user/operator/integrator outcomes and intentional product constraints across core, software-change, policy-document, research, and work-slot delegation. Move exact public protocol encodings into a versioned interface contract, implementation choices into technical design, and test/release mechanics into validation and release policy. Do not copy research's explicit “blackbox tests exist” requirement into policy-document; require public-boundary journey proof through repository validation policy instead. Preserve Bookends traceability from those journeys and contract tests to the smaller living requirement set.
- **Open questions:** Identify which exact interfaces are intentional compatibility promises, choose authoritative homes for extracted material, and plan tombstone/new-ID continuity without weakening current public proof or losing historical rationale.

#### Reuse completed plan work across plan revisions <<<

- **Observed:** Within one frozen plan, the driver can now select task roots plus dependants, rely only on engine-standing prerequisite results, explicitly carry clean or overridden prior results, skip unselected workers observably, and still regenerate the summary and repository checkpoint for the resulting tree. Full execution remains the default, and graph-local exit 0 cannot silently carry stale bound work. A new complete `plan.json` revision intentionally does not reuse prior-plan results.
- **Candidate:** Consider exact-node reuse across plan revisions only when normalized task content, dependencies, routed findings, worker binding, task-recorded repository effect, and repository preconditions all match; otherwise rerun. Keep each plan revision as the complete authoritative DAG and keep execution history separate rather than creating a mutable union plan. Reuse must remain an explicit driver act, not automatic validity judgment.
- **Open questions:** Define cross-revision task/dependency fingerprints, proof that remains valid after later shared-checkout mutations, overlap and partial-side-effect handling, report revision identity, provenance, and force-clean rerun. Black-box proof must use worker counters or fail-if-invoked sentinels to show an explicitly reused node in a later complete plan revision is genuinely not invoked and every stale or incomplete identity fails closed.

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
