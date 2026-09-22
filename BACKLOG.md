# Backlog

Wanted work and concrete leads. Completed work belongs in Git history. Items are grouped by topic, not implementation priority. Inclusion does not authorize implementation or establish requirements; [docs/PRD.md](docs/PRD.md) owns the engine contract.

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

### Complete Bookends coverage across the five phase artifacts

When Bookends is enabled, intent, design, plan, implementation report and validation report must each cite and completely cover the Bookends requirements in scope, appropriately for their phase. Citation presence alone does not establish meaningful coverage.

Assess current enforcement and close demonstrated gaps; this is a mandatory outcome, not a question of whether to add optional traceability. Keep this obligation distinct from implementation-task links to run-local acceptance criteria. This item does not extend the citation requirement to task packets, review commissions, review results or finding ledgers.

## Other wanted improvements

### Rethink the research workflow

Rethink the research workflow so the owner will actually use and trust it: investigate a query as thoroughly as reasonably practical, then synthesize findings that satisfy the research question and intent.

Establish what reasonable coverage and a satisfactory synthesis mean with the owner, and demonstrate them on representative real queries. Make material gaps and uncertainty visible; workflow completion alone does not establish research quality. The current bound-authoring gap is a limitation to consider, not the definition of this work. Detailed workflow design remains future work.

### Evaluate calibration and attestation needs across providers

Investigate whether and how providers beyond software-change merit calibration and attestation processes, or whether a more general evaluation workflow would serve them better. The purpose is to establish and maintain useful review judgments, not merely prove that commands run and verdicts are stored correctly.

Use software-change's experience to assess how representative acceptable and defective examples can reveal missed problems and false alarms. Establish who can justify the expected judgments, what is provider-specific, what could be shared, and the maintenance cost. Deliver an evidence-backed recommendation. Do not assume every provider needs the full software-change calibration process or that a shared framework must be built.

### Make work visibility useful for the driver and owner

The driver should be able to determine whether to wait, inspect a failure or seek help without repeatedly digging through process listings and scattered files. The owner should be able to understand what work is happening, what meaningful progress has been made and what is blocking it without interrogating the driver.

Provide both useful on-demand status and concise proactive chat updates for meaningful developments, blockers or owner decisions—not repetitive heartbeats. These views must agree and distinguish work in progress, execution completion and accepted results, with uncertainty explicit.

Assess the existing monitor, advisory summaries and driver guidance on real work first; change what fails these outcomes rather than assuming a new monitoring system or dedicated display is needed.

Status (2026-09-20): substantially delivered by run-1789764810283193000-1-10500 (commit 38f89d1, completed with zero overrides): status/visibility lanes separate execution, conformance, acceptance, evidence, freshness and uncertainty; acceptance projects explicit attributed decisions; passive owner updates per LE-143. AC-1..AC-5 pass with executed proof. Remaining: product-level review-commission pruning (see new item below).

### Make human status output digestible on active runs

Owner-observed pain (2026-09-21, second machine, brand-new active run): human `show --view status` renders lines and lines of wrapped text instead of the ~8-line digest demonstrated on the completed run-1789764810283193000-1-10500. Root cause, read from `render_show_compact` (`crates/loop-cli/src/lib.rs`): the human status output is the 8 digest lines **plus the entire status JSON packet appended inline** as one single-line blob (`status details ...`). On a finished run that packet is ~2 KB; on an active run it carries every latest durable evaluation with full feedback text, all nine visibility lanes with full reason prose, active invocation ownership/selection objects, and requestable-event details — so the terminal wraps one mega-line into pages and the digest scrolls away unseen. Related: `--view full --json` measured 9 GB on the completed run (~260k duplicated assignment/ledger-adjacent records); that view is machine-only and must never reach a terminal. Every status call also runs a live Dagu snapshot collection even when only the digest is wanted.

Possible directions (investigate, do not assume): drop or cap the `status details` tail in human mode since locators already point at the detail; gate it behind a flag; add a hard-capped digest view; skip the Dagu collection unless progress is requested; determine whether the 260k-record fan-out in full output is a cartesian duplication bug in change-report assembly or legitimate history rendered raw.

Directive: investigate first for the correct shape of human status — what an owner needs at a glance (current work, next action, blocker, evidence location, freshness, uncertainty) versus what belongs behind locators — against a live messy mid-implementation run, not a finished one. Fix what fails that shape; do not add a parallel monitoring system. Any CLI change keeps status non-arming and the GREEN/RED/BYPASS-style first-line contract intact.

Extend the investigation to every read primitive agents actually consume — show views (status/action/full, human and JSON), history, invocation-progress, monitor packets, list, and the instruction/guidance surfaces — and audit for duplicated information across them: the same evaluation text, lane prose, ledger content, or invocation detail repeated in multiple outputs or repeated per record. Machine consumers stay complete (agents read this output; nothing they need is removed), but each fact should have one authoritative home with locators elsewhere, so agents stop paying context-window rent on the same bytes N times. Demonstrate byte counts per primitive on a live run before and after; do not merely assert efficiency.

### Avoid unnecessary repeated work after plan changes

Identify ordinary plan-revision cases not adequately served by existing selected-task repair, ad-hoc repair and evidence applicability. Preserve completed work that remains valid under the revised plan, and repeat only work or review invalidated by the change.

Demonstrate a representative revised-plan execution reaching a valid completed outcome without rerunning unaffected work; do not assume that old success remains applicable merely because a task name is unchanged. Choose the smallest adequate change rather than prescribing delta-plan formats, generalized replay or cloned runtime state.

### Commission compact review packets by default

Review slots commission every record of their admitted kinds (observed: 511 records / ~1MB to an implementation-review worker, mostly other gates' ledgers). Small-window models fail; large ones never notice. Run-1789764810283193000-1-10500 proved current-gate-plus-references filtering (511 to ~125 records, preamble byte-identical, all reviews green) via run-scoped binding wrappers only — the product default is unchanged. Scope a provider change that commissions review-necessary records while preserving reuse/applicability references, LE-127 meaningful context, and full verification. Add a journey assertion on commissioned review-context size plus a small-window review probe so the gap cannot hide again.

### Make shipped profiles customizable without forking them wholesale

Owner dogfooding pain (2026-09-22): it is not clear how to take a shipped provider profile, change it slightly, and retain the rest of the defaults. Today the choice looks binary — use the profile as-is or own a full copy that silently drifts from future shipped improvements.

Investigate a better experience for profile layering/overlay: e.g. a documented copy-tweak-own flow, a named-profile-plus-overrides mechanism, or profile inheritance — whatever fits the provider architecture smallest. Outcomes: an owner can state "profile X with these N changes" in one obvious place; the unchanged remainder tracks shipped updates; drift/divergence is visible, not silent; validation still proves the effective profile, not just the base. Demonstrate on a real small customization, not a synthetic example.

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
