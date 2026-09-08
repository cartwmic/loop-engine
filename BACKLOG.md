# Backlog

Open work and candidates—not a delivery commitment or requirement authority. `docs/PRD.md` owns requirements. Inclusion preserves an idea; it does not mean it will be done.

The numbered groups rank expected impact on UX, friction, efficiency, output quality and time to finish a run, with higher-impact items first within each group. Favor repeated per-run gains and avoided rework over speculative machinery. This is not an execution sequence; effort and dependencies still matter. Evidence: [recovery retrospective](docs/recovery-run-retrospective.md).

## Correctness exceptions — separate from the UX ranking

These retain their existing significance regardless of convenience or expected speedup.

- **Shallow-CI continuity — release blocker:** Fetch enough parent history and fail closed when a shallow checkout cannot supply it. Full and CI-equivalent shallow clones must reject live-ID disappearance, tombstone removal, reassignment and revival; genuine first adoption must remain valid.
- **Durable bypass evidence — release blocker:** Record invocation time, repository/revision, bypass class/reason and outcome locally. If recording fails, the bypass must not permit the push. Preserve normal GREEN/RED behavior and keep BYPASS distinct from GREEN.
- **Cancellation identity:** Prevent resumed cancellation from confusing a reused PID/PGID with the original owned process. This risk remains unreproduced and unrepaired. Any investigation must respect the existing provider refusal; a simulation is not proof of actual OS reuse.

## 1. Biggest per-run gains

Reduce repeated context loading, unnecessary repair loops, monitoring tokens and manual proof handling.

- **Focused `show` output:** Separate concise status, agent-readable next-action context and explicit full inspection. Routine output should surface current instructions, available actions, current blockers and active work without repeating entire configuration, context and invocation histories or detailed change reports. Keep full evidence retrievable; reuse existing projections and provider-owned filtering rather than making core interpret opaque findings.
- **Surgical-first `show` guidance:** Bring actual bound and unbound instructions into line with the skill: prefer validation-local correction, selected task repair or honest no-task ad-hoc repair. Explain downstream invalidation costs and reserve deeper backtracking for materially wrong upstream obligations. Reduce repeated boilerplate without hiding these choices, current obligations or necessary guidance behind generic bound invocation instructions.
- **Deterministic progress monitor:** Cover software-change runs, invocations, bound/unbound plan graphs and fan-out, plus validation commands. Refresh human-readable status outside the model loop; notify the agent only on completion or when attention/judgment is needed. Reuse focused status projections, `invocation-progress` and structured captures rather than repeatedly consuming full `show` dumps. Keep workflow state, helper completion, worker outcome and review judgment distinct; report unknown data honestly. No automatic approval, advancement, retry or cancellation.
- **pi-subagents:** Fix `launchContractDigest` recovery compatibility; expose reliable selected-attempt results that separate execution, output validity, harness acceptance and judgment; independently review the lifecycle fixes. Validate through the public tool, including compaction, revival and partial parallel completion—without descriptor edits or disabled compaction.
- **Reusable matrix/capture runner:** Extract the run-local executor's live output, atomic receipts, failure stopping, verified abort cleanup and valid-prefix resume. Reuse existing test runners and report checks; remove fixed run paths and command counts. Prove failure, stale-resume and descendant-cleanup paths, not just successful commands.
- **Validation orchestration/finalization:** Consider combining named command execution, independent AC/goal review, driver-added validation and report-index finalization in the existing validation phase. Reuse current helpers and allow batched verdicts. Preserve driver judgment; neither a new state nor a new test runner is presumed necessary.

## 2. Better judgments and fewer review loops

Improve what passes review and prevent avoidable rejections, rediscovery and repeated commissioning.

- **Driver guidance:** Interview the owner for concrete outcomes, boundaries and testable acceptance criteria before freezing intent. Apply YAGNI/KISS within the explicitly stated project/run risk profile; do not import broader enterprise or hostile-actor assumptions. High rigor means stronger scrutiny, not expanded threat scope; accepted risks do not waive stated outcomes. Keep packets bounded, distinguish execution/output/judgment, avoid rerun-until-pass, and estimate serial work honestly. Use existing progress interfaces rather than model-driven polling.
- **Circular-proof detection:** Calibrate existing review prompts against injected conclusions, unchanged observations and proof compatible with opposite outcomes. Require meaningful falsification and honest task dependencies before adding axes.
- **Required-author visibility:** Show normalized per-gate author counts before rejection, preferably in existing instructions/commission output. Distinguish omitted gates from insufficient review.
- **Policy-document finding delivery:** Forward driver-selected prior finding/evidence references into current review commissions, with explicit supersession. Durable storage alone does not deliver findings to workers.
- **Calibration and real v2 use:** Complete the 138 pending calibration attestations and an ordinary v2 software-change run. Synthetic journeys do not establish semantic review quality.
- **Semantic proof audit:** Audit every live requirement and each conjunctive clause. Retain a revision-bound matrix linking requirement identity, citation, public scenario/call path, actual assertion and gap disposition. Mark unavailable historical proof unknown; later citation edits need ordinary review, not another exhaustive audit.
- **Current documentation:** Correct stale acceptance, audit and release labels after checking accepted versus committed wording. Explain why an implementation override can leave missing proof history and cause later denials; do not imply automatic downstream waivers. Leave historical reports unchanged.

## 3. Easier startup and delivery

Reduce setup and handoff work; these help less frequently than the per-state and per-review changes above.

- **Multi-run coordinator/prompter skill:** Distill the practice of one coordinator agent delegating several concurrent software-change runs to separate driver agents. Make bound/unbound startup prompts, work ownership, monitoring, escalation and handoff easy to establish. Reuse per-run skills rather than duplicate their workflow; stay independent of Herdr or any particular multiplexer.
- **CLI consistency:** Decide whether `software-change checkpoint` should accept `--json` despite already returning JSON; fix `fan-out --help` to show when worker instructions are required.
- **Generate-PRD wording:** Replace the claimed contrary-evidence search with an honest description of the three predetermined extract lookups. No new search infrastructure is needed.
- **Shared packaged smoke:** Use one implementation for local archives, installed binaries and CI, parameterized by binaries/version/platform. Reuse the public journeys outside the checkout; reject bad checksums and versions.
- **Pre-review Git checkpoint:** Decide how the driver creates and verifies an owner-authorized commit after implementation triage and before review. A repository checkpoint is not a Git commit; workers must not commit a shared checkout independently.
- **Post-commit delivery pointer:** Separately link a completed run's reviewed report/tree to the matching landed commit and later hosted result. Keep terminal reports immutable and refuse content mismatches.
- **Chezmoi:** Retain immutable-tag skill synchronization as dotfiles tooling, with exact-byte checks, orphan refusal, targeted apply and link verification.

The pi-subagents and chezmoi items belong to their respective projects, not Loop Engine core.

## 4. Broader requirement-policy decisions

Potentially valuable, but wider in scope and less directly tied to routine run friction. Establish the desired contract before adding enforcement.

- **Requirements versus specifications:** Separate enduring outcomes and intentional compatibility promises from encodings, implementation choices and test/release mechanics. Preserve accepted meaning and ID continuity; not every goal, rationale or journey deserves an ID.
- **PRD migration:** Agree source-preservation rules, normative authority and document structure before extracting IDs. Review actual candidate/source bytes for clause completeness, invented meaning and coverage feasibility—not merely a synthesis about them. Reuse existing profiles/axes where possible; test omissions, duplicates, non-requirements, contradictions and wrong-target reviews. Keep grammar validity, traceability and semantic acceptance separate.
- **Traceability breadth:** Decide whether downstream AC references should be mandatory and which additional intent obligations need PRD mapping. Keep unmatched requirements as honest proposals for human acceptance, not forced links or automatic IDs for every datum.
- **Historical requirement backfill:** Consider auditing selected past runs for enduring obligations missing from the current PRD. Propose evidence-backed additions or amendments for owner review; do not rewrite historical records or infer requirements from every old implementation choice.
- **Push-range decision:** Either document immediate-parent-only continuity or accept bounded checks of every pushed transition at pre-push and CI. Do not claim intermediate commits are checked when only the tip is checked.
- **LE-38 decision:** Decide whether concurrent context appends should be guaranteed not to stale evaluation. The current exclusion is not that guarantee. Amend the requirement before demanding stronger public proof.
- **Citation-surface decision:** Consider rejecting citations outside approved proof paths, while distinguishing real citations from examples, fixtures, generated/vendor content and skip directives.
- **Public contract coverage:** Consider enabling the optional `contract` class for genuine public boundaries, with collected, ID-linked assertions—not duplicate journeys or internal-seam tests.

## 5. Conditional or exploratory

Revisit after the simpler improvements reveal what is still worth building.

- **Optional semantic status summaries:** Let a dedicated summarizer explain an ongoing run using new evidence and its previous summary, without requiring the driver to narrate progress. Keep this separate from deterministic monitoring: it spends model tokens and must not turn interpretation into workflow authority.
- **Benchmark helper:** Extract collection/comparison only with a maintained comparable workload. Require repeated samples, declared concurrency and cleanup; do not freeze the historical workload or treat pilots as final evidence.
- **Delta-only revised plans:** Assess whether a genuinely necessary plan revision can describe only remaining work while referencing already-delivered work and proof, without false incompleteness findings. This is not established by current proof. Prefer existing selected/ad-hoc repair when the plan itself is still sound; do not presume a generalized replay mechanism.
- **Cross-provider calibration:** Finish existing calibration first; establish shared needs and gold-judgment ownership before building a neutral framework.
- **Generalized replay:** Revisit only if ordinary v2 work exposes repeated replacement traversal that existing repair, carry and backtracking cannot address. Do not clone runtime state.
- **Prototype-to-spec workflow:** Explore a low-ceremony path for disposable prototypes only if existing research/orchestration cannot serve it. Prototype output is learning, not production code or frozen requirements by default.

## Reproduce before ranking

Unverified leads from external runs; inspect the current public path before proposing fixes:

- Multiline worker argv, hard-kill terminal capture, ad-hoc capture placement, graph help and timeout discoverability.
- Missing `author.kind`/malformed envelopes and verdict reversal on unchanged subject bytes. Any normalization must preserve identity and original judgment, never invent a verdict.
- Pi session mtime, hidden process arguments, Dagu completion visibility and shell-probe reliability claims from another machine.
- Upgrade compatibility beyond supported historical reads, and reviewer strictness/author-count effects. Establish the actual failure or calibration result first.
