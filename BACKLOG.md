# Backlog

Open work and candidates only—not an implementation plan or requirement authority. `docs/PRD.md` owns requirements. Prefer small fixes and existing tools over new machinery.

## Recovery review follow-ups

Evidence and rationale: [recovery retrospective](docs/recovery-run-retrospective.md).

- **Cancellation identity:** Prevent resumed cancellation from confusing a reused PID/PGID with the original owned process. This risk remains unreproduced and unrepaired. Any investigation must respect the existing provider refusal; a simulation is not proof of actual OS reuse.
- **Reusable matrix/capture runner:** Extract the run-local executor's live output, atomic receipts, failure stopping, verified abort cleanup and valid-prefix resume. Reuse existing test runners and report checks; remove fixed run paths and command counts. Prove failure, stale-resume and descendant-cleanup paths, not just successful commands.
- **Deterministic progress monitor:** Cover software-change runs, invocations, bound/unbound plan graphs and fan-out, plus validation commands. Refresh human-readable status outside the model loop; notify the agent only on completion or when attention/judgment is needed. Reuse `show`, `invocation-progress` and structured captures. Keep workflow state, helper completion, worker outcome and review judgment distinct; report unknown data honestly. No automatic approval, advancement, retry or cancellation.
- **Shared packaged smoke:** Use one implementation for local archives, installed binaries and CI, parameterized by binaries/version/platform. Reuse the public journeys outside the checkout; reject bad checksums and versions.
- **Calibration and real v2 use:** Complete the 138 pending calibration attestations and an ordinary v2 software-change run. Synthetic journeys do not establish semantic review quality.
- **Current documentation:** Correct stale acceptance, audit and release labels after checking accepted versus committed wording. Explain why an implementation override can leave missing proof history and cause later denials; do not imply automatic downstream waivers. Leave historical reports unchanged.
- **Driver guidance:** Keep worker packets bounded, distinguish execution status from valid output and actual judgment, avoid rerun-until-pass reviews, and estimate serial work honestly. Use existing progress interfaces rather than repeated model-driven polling.

## Bookends correctness and coverage

- **Shallow-CI continuity — release blocker:** Fetch enough parent history and fail closed when a shallow checkout cannot supply it. Full and CI-equivalent shallow clones must reject live-ID disappearance, tombstone removal, reassignment and revival; genuine first adoption must remain valid.
- **Durable bypass evidence — release blocker:** Record invocation time, repository/revision, bypass class/reason and outcome locally. If recording fails, the bypass must not permit the push. Preserve normal GREEN/RED behavior and keep BYPASS distinct from GREEN.
- **Semantic proof audit:** Audit every live requirement and each conjunctive clause. Retain a revision-bound matrix linking requirement identity, citation, public scenario/call path, actual assertion and gap disposition. Mark unavailable historical proof unknown; later citation edits need ordinary review, not another exhaustive audit.
- **Push-range decision:** Either document immediate-parent-only continuity or accept bounded checks of every pushed transition at pre-push and CI. Do not claim intermediate commits are checked when only the tip is checked.
- **LE-38 decision:** Decide whether concurrent context appends should be guaranteed not to stale evaluation. The current exclusion is not that guarantee. Amend the requirement before demanding stronger public proof.
- **Citation-surface decision:** Consider rejecting citations outside approved proof paths, while distinguishing real citations from examples, fixtures, generated/vendor content and skip directives.
- **Public contract coverage:** Consider enabling the optional `contract` class for genuine public boundaries, with collected, ID-linked assertions—not duplicate journeys or internal-seam tests.

## Workflow and review improvements

- **Pre-review Git checkpoint:** Decide how the driver creates and verifies an owner-authorized commit after implementation triage and before review. A repository checkpoint is not a Git commit; workers must not commit a shared checkout independently.
- **Post-commit delivery pointer:** Separately link a completed run's reviewed report/tree to the matching landed commit and later hosted result. Keep terminal reports immutable and refuse content mismatches.
- **Required-author visibility:** Show normalized per-gate author counts before rejection, preferably in existing instructions/commission output. Distinguish omitted gates from insufficient review.
- **CLI consistency:** Decide whether `software-change checkpoint` should accept `--json` despite already returning JSON; fix `fan-out --help` to show when worker instructions are required.
- **Generate-PRD wording:** Replace the claimed contrary-evidence search with an honest description of the three predetermined extract lookups. No new search infrastructure is needed.
- **Circular-proof detection:** Calibrate existing review prompts against injected conclusions, unchanged observations and proof compatible with opposite outcomes. Require meaningful falsification and honest task dependencies before adding axes.
- **Policy-document finding delivery:** Forward driver-selected prior finding/evidence references into current review commissions, with explicit supersession. Durable storage alone does not deliver findings to workers.

## Larger decisions

- **Requirements versus specifications:** Separate enduring outcomes and intentional compatibility promises from encodings, implementation choices and test/release mechanics. Preserve accepted meaning and ID continuity; not every goal, rationale or journey deserves an ID.
- **PRD migration:** Agree source-preservation rules, normative authority and document structure before extracting IDs. Review actual candidate/source bytes for clause completeness, invented meaning and coverage feasibility—not merely a synthesis about them. Reuse existing profiles/axes where possible; test omissions, duplicates, non-requirements, contradictions and wrong-target reviews. Keep grammar validity, traceability and semantic acceptance separate.

## Conditional or deferred

- **Benchmark helper:** Extract collection/comparison only with a maintained comparable workload. Require repeated samples, declared concurrency and cleanup; do not freeze the historical workload or treat pilots as final evidence.
- **Generalized replay:** Revisit only if ordinary v2 work exposes repeated replacement traversal that existing repair, carry and backtracking cannot address. Do not clone runtime state.
- **Cross-provider calibration:** Finish existing calibration first; establish shared needs and gold-judgment ownership before building a neutral framework.
- **Prototype-to-spec workflow:** Explore a low-ceremony path for disposable prototypes only if existing research/orchestration cannot serve it. Prototype output is learning, not production code or frozen requirements by default.

## External owners

These follow-ups came from the recovery review but belong outside Loop Engine:

- **pi-subagents:** Fix `launchContractDigest` recovery compatibility; expose reliable selected-attempt results that separate execution, output validity, harness acceptance and judgment; independently review the lifecycle fixes. Validate through the public tool, including compaction, revival and partial parallel completion—without descriptor edits or disabled compaction.
- **Chezmoi:** Retain immutable-tag skill synchronization as dotfiles tooling, with exact-byte checks, orphan refusal, targeted apply and link verification.

## Reproduce before queueing

Unverified leads from external runs; inspect the current public path before proposing fixes:

- Multiline worker argv, hard-kill terminal capture, ad-hoc capture placement, graph help and timeout discoverability.
- Missing `author.kind`/malformed envelopes and verdict reversal on unchanged subject bytes. Any normalization must preserve identity and original judgment, never invent a verdict.
- Pi session mtime, hidden process arguments, Dagu completion visibility and shell-probe reliability claims from another machine.
- Upgrade compatibility beyond supported historical reads, and reviewer strictness/author-count effects. Establish the actual failure or calibration result first.
