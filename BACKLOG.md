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

### Audit semantic Bookends proof and retain the matrix

**Context:** Bookends GREEN proves that a live-ID token appears in an eligible tracked proof file collected by required CI; it does not prove that the neighboring scenario or assertion materially demonstrates the requirement. Two implementation workers reported independent LE-1 through LE-90 citation-placement audits as 90/90 but removed their row-level matrices. The known LE-38 mismatch and the LE-104 mismatch accepted during `run-1787626154593632000-1-54142` show that a structurally eligible citation can still point at the wrong or incomplete observable proof. A read-only audit also demonstrated that `scripts/software-change-journey.py::assert_semantic_outcome_proof_contract` accepts one arbitrary requirement row linked to an unrelated known scenario.

**Smallest scope:** Perform one exhaustive semantic audit of every live PRD requirement and its conjunctive clauses. Retain a revision-bound matrix containing requirement ID and digest, citation location, containing public scenario and call path, observable assertion, material proof verdict, and disposition for every gap. Backfill relevant prior software-change runs only from evidence that still exists; mark unavailable historical proof unknown rather than rewriting immutable reports or inferring success. Future exhaustive audits must retain the same matrix as review evidence, not checker-owned semantic truth.

**Done when:** A fresh reviewer can inspect or sample the retained matrix without reconstructing mappings; every live requirement has a semantic verdict; every incomplete or unrelated proof has an explicit repair, citation correction, or accepted-gap disposition; and the artifact names the audited `git rev-parse HEAD`. Later changed citations continue to receive the ordinary LE-97 semantic review; they do not require rebuilding this matrix or adding checker-owned semantic judgment.

### Add a post-commit run-to-commit pointer

**Context:** Final implementation and validation reports correctly describe the pre-commit state as `939c7d6+uncommitted-worktree`. After commit, no durable pointer links that exact reviewed surface to landed commit `531532c`. Historical reports should not be rewritten.

**Smallest scope:** Add a separate post-commit record that names the run ID, report revisions, landed commit, and proof that the committed pathname set and bytes match the reviewed worktree. Keep the run reports immutable.

**Done when:** A later auditor can move from the final run evidence to the landed commit without relying on chat history, while the original reports remain unchanged.

## Software-change run roadmap

These items came from operating the Bookends run and subsequent software-change runs. Each entry is a candidate for later intent/design work, not an accepted requirement. The packages below order the work by dependency and dogfood value. Candidate details remain in rough size bands afterward only to avoid obscuring their evidence and boundaries.

Apply one provenance rule across this section: Loop Engine serves a trusted sole owner and well-intentioned, instruction-following drivers. Provenance is durable memory for resume, focused invalidation, honest bypass, and debugging; it is not a hostile-agent security system or a generic semantic dependency graph. Store work and captures once under engine-owned IDs, then reference those IDs instead of asking drivers to repeat commands, paths, digests, and binding fields. Rich metadata is acceptable when generated mechanically and hidden from ordinary operation; driver-authored metadata must stay small. Trust explicit driver materiality and carry declarations, reject cheap mechanical mismatches such as a wrong run, state visit, artifact revision, checkpoint, task, criterion, or invocation, and do not replay otherwise valid work merely to produce a denser provenance trail.

### Recommended run packages

Use the high-rigor profile with unbound review and implementation work slots while dogfooding these packages. Every run uses at least two distinct ordinary review authors plus a third distinct adversarial-review author. Ask the owner to select every implementation, subagent, and reviewer model before start; this roadmap prescribes no model. Unless a package says otherwise, treat each linked candidate as a separate surgical software-change run and confirm its intent before combining it with adjacent work.

1. **Authorize the product constraint.** Run [Make the minimum-provenance product mandate authoritative](#make-the-minimum-provenance-product-mandate-authoritative) first. It gives every later design a binding YAGNI/KISS burden and is the first dogfood run.
2. **Make the current workflow easier to drive.** Compose [Confirm the rigor profile before start](#confirm-the-rigor-profile-before-start), [Make late-phase backtracking proportional and explicit](#make-late-phase-backtracking-proportional-and-explicit), and [Reframe adversarial review terminology](#reframe-adversarial-review-terminology) only if intent confirms one documentation-and-procedure surface. These changes reduce avoidable operator mistakes without waiting for engine mechanics.
3. **Add bound surgical repair in two runs.** First implement only invocation-time plan-task selection from [Support surgical implementation rework without replaying completed work](#support-surgical-implementation-rework-without-replaying-completed-work). After that composed path reaches terminal completion, add captured ad hoc repair for findings with no honest existing task. Do not pull criterion redesign, carry redesign, binding correction, or generalized replay into either run.
4. **Expose the resulting state.** Implement [Add a concise CLI status and progress view](#add-a-concise-cli-status-and-progress-view) after task selection so the compact view covers the durable selection and inner-task behavior actually shipped.
5. **Simplify evidence handling.** Implement [Reduce driver-authored provenance to stable references](#reduce-driver-authored-provenance-to-stable-references), then decide whether [Normalize mechanically exposed reviewer candidates for driver triage](#normalize-mechanically-exposed-reviewer-candidates-for-driver-triage) still saves meaningful work. Keep normalization inert until the driver accepts it.
6. **Create one criterion spine.** Implement [Thread acceptance criteria through live or candidate PRD requirements](#thread-acceptance-criteria-through-live-or-candidate-prd-requirements) before [Orchestrate final validation per frozen criterion](#orchestrate-final-validation-per-frozen-criterion). The second run must reuse the first run's keyspace rather than create another.
7. **Deliver prior findings where they are needed.** Treat [Deliver steering mechanically to later software-change work](#deliver-steering-mechanically-to-later-software-change-work) and [Route policy-document findings into later review work](#route-policy-document-findings-into-later-review-work) as separate provider runs sharing the stable-reference rule, not as a generic engine finding subsystem.
8. **Add exceptional continuation only after ordinary repair works.** Implement [Keep run policy frozen and execution controls simple](#keep-run-policy-frozen-and-execution-controls-simple) before [Add a signed owner override](#add-a-signed-owner-override). Then revisit [Defer generalized replay for replacement runs](#defer-generalized-replay-for-replacement-runs) only from observed remaining replacement-run cost.
9. **Improve repository handoff and proof cost independently.** [Add a driver-owned implementation commit checkpoint](#add-a-driver-owned-implementation-commit-checkpoint) and [Speed up workspace tests and black-box journeys](#speed-up-workspace-tests-and-black-box-journeys) are independent runs after the minimum-provenance mandate. Neither should block surgical repair.
10. **Keep public-proof corrections in a parallel lane.** Run [Reject citations outside approved Bookends surfaces](#reject-citations-outside-approved-bookends-surfaces), [Reconcile LE-38 requirement wording and public proof](#reconcile-le-38-requirement-wording-and-public-proof), and [Add public contract tests with Bookends IDs](#add-public-contract-tests-with-bookends-ids) separately as their product decisions become ready. They do not belong in provider-repair runs.
11. **Leave strategic reshaping until operating evidence justifies it.** [Separate enduring product requirements from implementation specifications](#separate-enduring-product-requirements-from-implementation-specifications), [Generalize provider evaluation and calibration](#generalize-provider-evaluation-and-calibration), and [Create a prototype-to-spec workflow](#create-a-prototype-to-spec-workflow) are later standalone initiatives, not cleanup to bundle into the packages above.

### Candidate detail: bounded scope

#### Make the minimum-provenance product mandate authoritative

- **Observed:** The machine-global Pi agent policy already directs agents to use YAGNI, KISS, black-box user-path proof, and the personal trusted-owner risk model. The repository root and nested provider `AGENTS.md` files do not yet state the minimum-provenance rule, and `BACKLOG.md` is non-binding. The living engine and provider requirements can still accrete identifiers, digests, lineage, gates, and replay rules without an explicit product requirement that each added burden solve an observed ordinary-use failure more simply than existing state, history, invocation capture, or driver judgment.
- **Candidate:** Through the normal human-accepted PRD process, add a concise paramount product requirement covering the engine and software-change provider: assume a trusted sole owner and well-intentioned, instruction-following drivers; use provenance only for resume, focused invalidation, honest bypass, and debugging; capture facts once under stable engine-owned IDs and reference them; keep driver-authored metadata small; trust explicit materiality and carry declarations except for cheap mechanical mismatches; recommend the narrowest honest correction; retain actual failures and override labels; and do not replay valid work merely to satisfy workflow ceremony. Give every proposed provenance field, identity dimension, gate, state, or replay rule a YAGNI/KISS burden to show the observed ordinary-use failure it prevents and why a smaller mechanism is insufficient. Rich internal metadata remains acceptable when generated automatically and kept out of the ordinary path.
- **Open questions:** Decide the smallest split between the engine PRD and provider requirements without duplicating one rule into many IDs. After human acceptance, add short operational summaries with authoritative requirement references to the root and relevant nested `AGENTS.md` files so planning, implementation, and review agents apply the rule; AGENTS guidance must not become a competing requirement authority. Do not add a meta-framework, scoring rubric, new progression gate, or separate synthetic proof run. Apply the mandate while designing and reviewing the surgical-rework candidate: prefer the smallest repair path that preserves resume, evidence, failure visibility, and black-box terminal behavior.

#### Reject citations outside approved Bookends surfaces

- **Observed:** Bookends indexes citations only in the class pathspecs from `bookends.toml`. A `bookends:LE-<n>` token in another tracked file is ignored rather than rejected.
- **Candidate:** Scan citation tokens across the tracked textual tree and reject tokens outside approved proof surfaces. Keep the configured class pathspecs as the authority for where citations may count.
- **Open questions:** The current tree already contains explanatory `bookends:` strings in documentation, skills, checker tests and schema docs, calibration fixtures, and other non-proof surfaces. Define their non-citation spelling or exclusions, the interaction with the existing `bookends:skip` marker, and treatment for the PRD itself, generated files, fixtures, vendored content, skipped files, and binary/non-text files before enabling rejection. Compare the final rule with Compass before choosing syntax or exclusions.

#### Add a concise CLI status and progress view

- **Observed:** `show` and `invocation-progress` expose durable state and bound-work progress, but following a long software-change run still requires reading large JSON envelopes and correlating graph details manually. The latest run also suggested that Dagu-backed task state was not always visible or reliable through `invocation-progress`; direct `dagu status` and `dagu history` are underlying implementation details rather than the driver-facing contract.
- **Candidate:** Add a deterministic human-readable CLI view for one run that concisely presents lifecycle, current state, requestable events, latest checked outcome, active or latest invocation, and available inner task counts/statuses. Derive it from existing engine operations rather than creating a second state authority, preserve machine-readable JSON, and first reproduce and repair any confirmed `invocation-progress` mapping defect.
- **Open questions:** Decide whether this is a new `progress` command or a compact `show` mode. A new command requires amending LE-72's other-command enumeration; a compact `show` mode does not. Decide which provider-neutral fields are stable, how unavailable inner progress is displayed, and whether completed, failed, overrun, reused, and skipped tasks need distinct presentation. Black-box CLI proof must cover a running graph, a completed graph, and unavailable progress without requiring the operator to invoke Dagu directly.

#### Reconcile LE-38 requirement wording and public proof

- **Observed:** Live LE-38 says concurrent context appends are outside v0.1 evaluation-staleness guarantees; it does not positively guarantee that an in-flight evaluation remains current. The public software-change journey honestly says its state-race fixture makes no guarantee about context appends, while an internal concurrency test demonstrates the current snapshot behavior. Treating that internal behavior as the meaning of LE-38 would create the same requirement/citation mismatch this backlog is trying to remove.
- **Candidate:** Decide the product contract first. If concurrent appends should positively remain non-staling, explicitly amend LE-38 through the normal human-accepted PRD process, then add a public CLI scenario that blocks a checked evaluation, appends context, releases the evaluation, and proves the amended behavior for both the append and transition. If the v0.1 non-guarantee remains intentional, keep the journey wording at that boundary and do not claim that a positive snapshot-commit scenario is required proof of LE-38.
- **Open questions:** Decide whether the positive behavior is an enduring public guarantee, expected diagnostics for `allow` and `deny` if it becomes one, and whether one representative public result is sufficient when internal tests retain the complete matrix.

#### Make late-phase backtracking proportional and explicit

- **Observed:** The graph already preserves nearest and owning-phase routes, but `show` instructions and the software-change skill mostly enumerate them as peers. They do not explain that intent or design rollback becomes exceptional after plan approval, that most implementation or validation findings should take the narrowest surgical repair path, or which artifact revisions, judgments, ledgers, and checkpoints each deeper route retires. In `run-1787626154593632000-1-54142`, two validation findings used `revise-implementation` and forced broad forward re-clear; the second was a citation-placement correction.
- **Candidate:** Add plain driver-facing late-finding guidance to the skill, reviewer protocol, and relevant `show` instructions. Recommend, in order, a validation-report-local correction, existing-task surgical repair, captured ad hoc repair, plan revision, design revision, intent revision, signed owner override, or replacement run. For each path show the defect owner and ordinary invalidation cost. This is guidance for a well-intentioned driver, not an engine router, deterministic classifier, new state machine, or policy subsystem. After plan approval, return to plan only when desired work or decomposition is materially wrong, to design only when a frozen design decision is invalid, and to intent only when the accepted outcome or boundary itself is wrong.
- **Open questions:** Calibrate the guidance against R7 claim-trust and the simplified carry candidate. A well-intentioned driver may carry a prior judgment to a new artifact revision with the originating evidence ID and a short materiality reason; the engine need not infer semantic dependency closure or demand an inventory of every changed input. Documentation fixtures must cover validation-report-only, implementation, plan, design, and intent defects, but they do not substitute for the composed bound-run repair journey required by the surgical-rework candidate.

#### Reduce driver-authored provenance to stable references

- **Observed:** Engine-authored invocation records already retain frozen binding inputs, selections, capture paths, selected attempts, output digests, and status. Provider records and current carry/ledger procedures can require the driver to restate or reconcile much of the same information through raw-source links, changed-input inventories, exact accepted-unresolved snapshots, and separate `unchanged-carry` and `override-carry` acts. This retained detail did not prevent the latest run from taking the wrong repair route or accepting semantically weak proof; it increased the amount the driver had to correlate.
- **Candidate:** Keep rich invocation and capture metadata mechanically generated, but make ordinary provider records reference engine-owned invocation, attempt, checkpoint, evidence, and finding IDs instead of duplicating their fields. Replace LE-103 and LE-104's two carry acts with one declaration containing the prior evidence ID, current subject revision or checkpoint, attesting driver, and a short reason it remains applicable. Mechanically reject only a missing or wrong-run evidence reference and a non-current named subject, revision, checkpoint, task, criterion, or invocation; do not require a changed-input inventory or infer semantic applicability. Keep finding records to stable ID, concise description, source reference, owning phase or task, status, and disposition. Preserve raw captures and append-only history for inspection without making their complete provenance vocabulary a progression precondition.
- **Open questions:** Inventory which duplicated fields are still required at the provider boundary and remove those that resolve unambiguously from a stable reference. Reassess whether the generic covered-input change-report fields remain useful after the two carry acts are replaced; current requirements are a baseline to amend or tombstone, not a compatibility constraint. Black-box proof should show that a fresh driver can resume, identify open findings, inspect original evidence, and carry or rerun it without reconstructing commands, paths, digests, or changed-input closures by hand.

#### Confirm the rigor profile before start

- **Observed:** The mandatory pre-start confirmation covers bound slots, exact commands and arguments, and models. It does not require the owner to confirm `minimal`, `standard`, or `high-rigor`, even though that choice freezes which review states, axes, and author counts exist and cannot be patched. `preview-bindings` reports binding mechanics rather than the profile's semantic obligation surface.
- **Candidate:** Make the selected profile a separate preflight decision. Present its profile name and `config_version`, live review states, axis IDs, required-author counts, and whether Bookends is enabled, then require explicit owner confirmation alongside the existing binding/model confirmation. Start with skill and root-policy procedure; add a deterministic preview surface only if hand-summarizing the exact profile proves error-prone.
- **Open questions:** Decide whether extending `preview-bindings` is honest or whether a provider-owned `preview-profile` projection is cleaner. Done proof must show the confirmed summary was derived from the exact bytes later frozen at `start`, not from the currently shipped profile.

#### Reframe adversarial review terminology

- **Observed:** “Adversarial” and repeated “attack/falsify” framing can push reviewers toward security-style overreach even though the reviewer protocol limits findings to material failures against frozen intent.
- **Candidate:** Use “challenge review,” “devil's-advocate review,” or “weakness-exposure review” in human-facing titles and instructions. Preserve existing state and gate IDs in the first iteration to avoid a topology/config migration for a wording change.
- **Open questions:** Pick one term, test whether it changes review quality, and decide later whether internal IDs merit migration.

### Candidate detail: moderate scope

#### Orchestrate final validation per frozen criterion

- **Observed:** `validation-report.json` is driver-authored. Its schema requires only a nonempty array of free-text requirement/proof rows, while high-rigor supplies one aggregate `requirement-proof-mapping` axis for the whole report. The source journey preloads a passing fixture and performs its extra proof-content helper only after the run has reached `end`; a read-only probe showed that helper accepts an unrelated one-row mapping. During `run-1787626154593632000-1-54142`, the owner had to commission criterion-bundled Kimi and Sol checks manually; they found a public `--assignments=[]` defect and later a fix-introduced LE-104 citation mismatch that prior activity and token checks had missed.
- **Candidate:** After the acceptance-criteria-threading candidate establishes the sole run-local criterion keyspace, turn validation draft work into explicit orchestration over the frozen intent and selected final repository checkpoint. Run the deterministic commands named by the accepted plan, require an independent evidence-backed verdict for every acceptance criterion under an explicitly accepted criterion-author policy, and commission a separate whole-intent goal check. Do not infer that policy from the current per-axis author counts. One reviewer output may cover multiple criteria when it reports a distinct verdict for each; configured validation axes consume those selected verdicts rather than commissioning duplicates. Identify a verdict by criterion ID, validation-report revision, repository checkpoint, reviewer, result, and stable evidence or finding references. Generate the validation report as a concise index over engine-owned command, review, criterion-verdict, and checkpoint IDs rather than repeating their commands, paths, digests, or captures. Let the driver add ad hoc validation and retain disposition authority, but do not rely on an unsupported freehand proof claim. Surface only deterministic checks that are actually named and runnable.
- **Open questions:** Decide whether the existing `validation-draft` binding plus a deterministic constructor is sufficient or whether the report/checkpoint and profile contracts should be replaced. Preserve external semantic judgment, R22 materiality, and one repository checkpoint without creating criterion-specific covered-input or proof-digest graphs. After a focused repair, the driver names affected criteria; prior verdicts for unaffected criteria may be carried to the new report revision by referencing the old verdict and recording a short reason. Black-box proof for this later candidate must reject omitted, duplicate, and unknown criterion IDs; show different verdicts for two criteria; block on the failing one; and correct one criterion without recommissioning the unaffected criterion. Do not make the first surgical task-selection increment depend on this validation redesign.

#### Thread acceptance criteria through live or candidate PRD requirements

- **Observed:** The optional Bookends overlay injects one top-level nonempty `requirement_ids` array into intent, design, plan, and validation artifacts. It checks syntax and liveness but does not bind an ID to each acceptance criterion, and it never mints IDs. An unmatched requirement discovered while freezing intent therefore has no honest in-run representation: forcing an existing ID launders the mismatch, while inventing a live ID violates the PRD's human acceptance and continuity rules.
- **Candidate:** Give every acceptance criterion one stable run-local ID. Map that ID to one or more semantically matching live PRD IDs or to an explicit candidate requirement when Bookends is enabled. Plan tasks name the criteria they serve, and final criterion verdicts use the same IDs; other artifacts may reference those IDs when useful but need not duplicate a complete trace matrix. Candidate requirements remain proposals, are checked with `bookends-check candidate`, and become live only after explicit human acceptance and commit. Mechanical checks enforce ID completeness and referential consistency while external review judges semantic fit.
- **Open questions:** Keep the first scope to acceptance criteria rather than every problem statement, constraint, non-goal, accepted risk, and outside obligation. Decide how run-local and candidate IDs are named, whether unresolved candidates block high-rigor completion, and how acceptance or rejection remains visible. Black-box proof must show that an unrelated live ID cannot satisfy semantic review, an unmatched criterion can travel honestly as a candidate, and validation does not create a second criterion keyspace.

#### Deliver steering mechanically to later software-change work

- **Observed:** LE-51 was tombstoned because the implementation proves only that durable `user-steering` reaches `show` and checked evaluation requests. Software-change draft slots still do not receive steering; implementation and review slots receive the current `finding-ledger` context, but the cited journey did not prove that steering changed later work or authorization.
- **Candidate:** Define a workflow-level steering-delivery contract that names intended later recipients, carries applicable durable steering to bound workers or evaluations without relying on driver memory, and proves an observable later result changes because of it. Keep context opaque to Loop Engine core and do not expand this into live interruption, which remains deferred.
- **Open questions:** Define recipient selection, ordering and supersession, whether steering applies to already-frozen artifacts or only later phases, and how a driver records that steering was incorporated when work remains unbound.

#### Route policy-document findings into later review work

- **Observed:** Policy-document review evidence and denial diagnostics are durable and visible to the driver and provider, but bound semantic-review workers receive no forwarded context. LE-58 and LE-59 therefore describe possible reuse of previous findings without mechanically delivering those findings to the later external reviewer.
- **Candidate:** Give each later semantic review or revision commission a durable, current-target finding packet that references prior accepted findings or review evidence by stable ID. Keep raw evidence at its engine-owned origin instead of copying its path and digest into each packet. The delivery mechanism must not reinterpret findings or let stale evidence satisfy current conformance.
- **Open questions:** Decide whether to forward selected `review-evidence` references or introduce a small workflow-specific finding record; define driver-declared supersession and handling of findings against a revised target.

#### Normalize mechanically exposed reviewer candidates for driver triage

- **Observed:** Completed bound review invocations already expose assignment and selected-attempt identity, capture location, output digest, and status through durable `show`. The driver still reads each selected output and hand-authors provider-shaped `review-evidence` plus the authoritative `finding-ledger` snapshot. No provider-level triage view extracts normalized `result` and `findings` fields from the selected output.
- **Candidate:** Optionally derive a nonauthoritative provider candidate view containing the source invocation/attempt ID plus normalized result and finding text. Present it as a convenience bundle for driver accept/edit/reject/ownership/routing; never auto-promote it into review evidence, ledger authority, carry, routing, or checked-transition satisfaction. Resolve detailed provenance from the source ID rather than copying it into the candidate. Keep Loop Engine core opaque to finding semantics and leave unbound/external review on its current hand-authored path unless separately accepted.
- **Open questions:** Decide whether normalization saves enough driver work beyond reading the durable selected outputs; if so, prefer a software-change provider command or projection rather than a generic engine finding model. Define malformed/exhausted representation and retry deduplication. Black-box proof must show normalized candidates appearing from the correct selected attempt, remaining inert before driver confirmation, and only a later driver-authored record affecting a checked transition.

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

### Candidate detail: high complexity

#### Keep run policy frozen and execution controls simple

- **Observed:** Snapshotting provider configuration at `start` gives a durable run a stable workflow graph, rigor profile, obligation set, and interpretation after resume. Treating every captured field as permanently immutable conflates that useful policy envelope with operating mechanics. The blanket rule makes a binding typo require a replacement run and contributed to treating later plan-task selection as frozen binding configuration rather than invocation input. Correcting bindings requires replacing the current immutable-initial-input and frozen-`work_slot_bindings` requirements. LE-101 is the useful precedent for invocation-time subset selection without rewriting the binding, not the source of binding immutability.
- **Candidate:** Keep provider/protocol identity, described topology, profile bytes and semantic obligations, review axes and author counts, Bookends policy, and initial run input immutable. Snapshot work-slot bindings at start, but after the requirement amendment permit an owner-authorized correction for future invocations. Record the old binding, new binding, affected slot, reason, and owner act once; each invocation already retains the binding it actually used. Treat task or review subsets, routed findings, retry or force-fresh choice, concurrency, and timeout as ordinary per-invocation inputs. Do not add executable-byte, environment, or model-content attestations.
- **Open questions:** Use the same minimal owner-authentication envelope as the signed-override candidate. A correction makes no automatic semantic claim about old output: the well-intentioned driver either reruns affected work or carries a prior result with its evidence ID and a short reason. Do not compute a dependency closure through tasks, reports, checkpoints, reviews, and criteria. Black-box proof must show old invocations retaining their original binding, later invocations using the correction, task selection creating no binding correction, and topology/profile edits refusing through this path.

#### Separate enduring product requirements from implementation specifications

- **Observed:** The living PRD now gives requirement IDs to a heterogeneous set of product outcomes, architecture boundaries, exact CLI and JSON contracts, Dagu/helper details, journey implementation, CI wiring, and cargo-dist assertions. The policy-document and research sections mix enduring workflow behavior with provider mechanics and test/release requirements; the work-slot section is especially coupled to current commands, helpers, paths, packet fields, and Dagu. Several requirements would therefore need edits after an implementation replacement that preserved product behavior, while Section 16 says exact CLI grammar, field names, framing, and internal representation belong to technical design.
- **Candidate:** Reshape the requirement catalog around stable user/operator/integrator outcomes and intentional product constraints across core, software-change, policy-document, research, and work-slot delegation. Move exact public protocol encodings into a versioned interface contract, implementation choices into technical design, and test/release mechanics into validation and release policy. Do not copy research's explicit “blackbox tests exist” requirement into policy-document; require public-boundary journey proof through repository validation policy instead. Preserve Bookends traceability from those journeys and contract tests to the smaller living requirement set.
- **Open questions:** Identify which exact interfaces are intentional compatibility promises, choose authoritative homes for extracted material, and plan tombstone/new-ID continuity without weakening current public proof or losing historical rationale.

#### Support surgical implementation rework without replaying completed work

- **Observed:** Commit `41b10a4` added standing-aware review-assignment subsets, direct `run-plan-graph --task` roots plus dependants, explicit carry acts, observable skipped work, and routing of accepted implementation findings into matching later task packets. Those pieces do not compose through the usual bound `implement` slot. `--task` / `--tasks` must be frozen into the `run-plan-graph` binding argv at `start`, while later `loop-engine invoke --assignment` selection enumerates fan-out review workers rather than plan tasks. A driver can therefore run the full bound implementation, accept review findings, return to `implement`, and see those findings routed to named tasks, but cannot re-invoke that same bound slot for only those tasks; the full plan runs again. The black-box journey proves finding routing and direct `run-plan-graph --task` subsets separately, not that real review/fix loop. Running an unbound `run-plan-graph --task` by hand is a workaround, but it is not a second invocation of the same frozen bound slot and does not preserve that invocation record. In `run-1787626154593632000-1-54142`, a late local defect led to a full bound graph re-invocation, two implementation/validation loops, and eventual off-engine completion with the run stranded at `implement`.
- **Candidate:** First expose invocation-time plan-task selection for the same bound `implement` slot, resolving selected roots against the current frozen-plan revision without rewriting the binding. Record selected task IDs, routed finding IDs, the ordinary engine-authored invocation, and the resulting repository checkpoint; the invocation already owns command, binding, output, and capture provenance. When no existing task is an honest fit, provide a captured ad hoc repair using the same invocation record plus accepted finding IDs and pre/post checkpoint IDs. When desired work or decomposition is materially wrong, use the plan-revision route. Keep this first increment independent of the later criterion-ID, criterion-validation, and carry redesigns; use the accepted validation path to prove terminal completion. Full execution and a force-fresh complete plan remain available.
- **Open questions:** Choose the smallest task-selection CLI and packet shape. Do not introduce cumulative-plan identity, automatic exact-node reuse, or engine-computed invalidation lineage in the first increment. Black-box proof must drive one bound run through full implementation, an accepted task-routed finding, check-free return to `implement`, a second invocation of the same bound slot selecting only affected later task roots, focused confirmation through the accepted review path, existing validation, and terminal completion. Fail-if-invoked sentinels must prove unrelated tasks did not start; selected packets contain the accepted findings; `show` retains the unchanged binding and selected task IDs; and the checkpoint describes the resulting tree. A second composed case must take a no-honest-task finding through a captured ad hoc repair and terminal validation. Empty, duplicate, unknown, stale-plan, or missing-prerequisite selections must fail before Dagu, any plan-task worker, the summarizer, or an ad hoc repair worker starts; the ordinary engine invocation and bound provider process may exist and record the mechanical failure. A direct unbound runner call does not satisfy this proof.

#### Add a signed owner override

- **Observed:** Existing runs expose only provider-described events and checked transitions; a frozen obligation cannot be waived. The practical escape hatch has been to abandon the run and finish outside it, losing the evidence and history the guardrails were meant to preserve. Four cataloged software-change runs are still active at `implement`, including `run-1787626154593632000-1-54142`. The current provider contract forbids cryptographic provenance and mid-run waivers (`crates/software-change-provider/docs/prd.md` R20 and Section 5), so the override requires an explicit owner-approved requirement amendment. The signature is an authority label that distinguishes the human owner from a well-intentioned agent using the same CLI, not general hostile-operator hardening.
- **Candidate:** After amending those requirements, add one owner-only override act. The owner supplies or confirms a public key at preflight and keeps the private key outside model and run access. Sign only the run ID, current engine state visit, exact transition or skipped gate, reason, and replay-resistant nonce. Record the request, skipped checks, and resulting state in durable history. Every provider or bound-work blocker needed for terminal progression has an override path; the terminal result is permanently labeled completed-with-overrides or equivalent. Actual denial, failed review, Bookends RED/BYPASS, and skipped checks remain visible and are never rewritten as provider allow, passed review, or GREEN. `show` and `history` expose the override and a concise run-level summary. Reuse the same small envelope for an accepted future-binding correction without making ordinary task selections owner-only.
- **Open questions:** Define canonical signed bytes, replay prevention, the terminal representation, and the disposition of runs created before owner keys exist. Do not add enterprise enrollment, trust chains, or signatures over duplicated policy, binding, artifact, evidence, and checkpoint inventories. Black-box proof must show a valid signature advancing only the named current state visit while preserving the underlying failure, invalid or replayed requests refusing without mutation, and `show` plus `history` distinguishing override from ordinary success.

#### Defer generalized replay for replacement runs

- **Observed:** Correcting immutable policy, topology, or profile data requires a new run and re-traversal. Generalized safe replay would need compatibility rules across policy, profile, topology, artifact revisions, authorship, repository state, and evidence freshness. That is a provenance-heavy subsystem. Invocation-time task selection, captured ad hoc repair, simple future-binding correction, driver-attested carry, and signed owner override remove the common reasons current runs become stranded.
- **Candidate:** Do not implement generalized replay until those narrower paths ship and production use still demonstrates repeated costly replacement-run traversal. For an occasional immutable-policy replacement, repeating affected gates is preferable to introducing an unchecked fast-forward or a semantic compatibility engine.
- **Open questions:** Measure how often replacement runs remain necessary after the narrower paths exist and how much still-valid work they actually repeat. If replay later becomes justified, start with explicit driver-selected prior evidence references re-evaluated under the new frozen obligations and stop at the first failed gate; do not clone runtime state or infer a universal compatibility graph.

#### Generalize provider evaluation and calibration

- **Observed:** The software-change provider ships a calibration procedure and fixtures, while other providers may rely mainly on deterministic journeys and ad hoc semantic review. Those prove mechanics but not reviewer/prompt quality.
- **Candidate:** Evaluate whether a provider-neutral workflow should compare policy prompts, expected findings, false positives, and model behavior across software-change, policy-document, research, and future providers.
- **Open questions:** Determine which semantics are actually shared, what remains provider-specific, who owns gold judgments, and whether the existing software-change calibration can be reused without turning evaluation into provider runtime behavior.

#### Create a prototype-to-spec workflow

- **Observed:** Software-change freezes relatively strong intent/design/plan artifacts before implementation. Early product work may instead need a disposable prototype, owner iteration, and learning before a build-quality specification is possible.
- **Candidate:** Explore a lower-ceremony workflow that scopes a question, builds and iterates a prototype with the owner, records what was learned, then produces a candidate intent/design/spec for a normal quality implementation workflow. Prototype output is evidence, not production code by default.
- **Open questions:** Decide whether this is a research profile, a new provider, or orchestration across existing providers; when requirements become frozen; what prototype code may survive; and how the final specification avoids laundering accidental prototype choices into requirements.
