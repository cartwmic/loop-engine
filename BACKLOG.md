# Backlog

Wanted work and concrete leads. Completed work belongs in Git history. Items are grouped by topic, not implementation priority. Inclusion does not authorize implementation or establish requirements; [docs/PRD.md](docs/PRD.md) owns the engine contract.

Reconciled against the September 28 workflow delivery and October 1 review-recovery delivery through `a05dee75`. Removing delivered entries does not relabel `dogfood-review-teething-20261001` as ordinary GREEN: it completed with three scoped overrides for six original pre-lint proof fingerprints; its refusals and original evidence remain history. No fresh whole-matrix, hosted delivery or installed deployment is implied.

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

## Other wanted improvements

Observations marked as separate 0.22.0 dogfooding below came from another machine; their captures are not in this checkout. Recheck source evidence or reproduce current behavior before changing product contracts.

### Rethink the research workflow

Rethink the research workflow so the owner will actually use and trust it: investigate a query as thoroughly as reasonably practical, then synthesize findings that satisfy the research question and intent.

Establish what reasonable coverage and a satisfactory synthesis mean with the owner, and demonstrate them on representative real queries. Make material gaps and uncertainty visible; workflow completion alone does not establish research quality. The current bound-authoring gap is a limitation to consider, not the definition of this work. Detailed workflow design remains future work.

### Evaluate calibration and attestation needs across providers

Investigate whether and how providers beyond software-change merit calibration and attestation processes, or whether a more general evaluation workflow would serve them better. The purpose is to establish and maintain useful review judgments, not merely prove that commands run and verdicts are stored correctly.

Use software-change's experience to assess how representative acceptable and defective examples can reveal missed problems and false alarms. Establish who can justify the expected judgments, what is provider-specific, what could be shared, and the maintenance cost. Deliver an evidence-backed recommendation. Do not assume every provider needs the full software-change calibration process or that a shared framework must be built.

### Keep work visibility useful after externalizing execution

Preserve useful on-demand state, meaningful owner updates, evidence locators and explicit uncertainty as execution moves to the driver/harness. The existing visibility lanes and proactive-update guidance are delivered; remaining human readability and duplicated read output are covered below.

Do not add an external-work registry, automatic agent supervision or mandatory status-note lifecycle. Where an external driver provides progress or acceptance, distinguish it from engine-observed durable state; absence of a note is not proof that work finished. Execution-specific ownership/Dagu follow-ups cease to apply only when their machinery is actually retired. Preserve historical captures and do not relabel a retry as a demonstrated fix.

### Make human status output digestible on active runs

Owner-observed pain (2026-09-21, second machine, brand-new active run): human `show --view status` renders lines and lines of wrapped text instead of the ~8-line digest demonstrated on the completed run-1789764810283193000-1-10500. Root cause, read from `render_show_compact` (`crates/loop-cli/src/lib.rs`): the human status output is the 8 digest lines **plus the entire status JSON packet appended inline** as one single-line blob (`status details ...`). On a finished run that packet is ~2 KB; on an active run it can carry current-state latest evaluations with feedback text, visibility lanes with reason prose, active invocation ownership/selection objects, and requestable-event details — so the terminal can wrap one mega-line into pages and the digest scrolls away unseen. Related: `--view full --json` measured 9 GB on the completed run (~260k duplicated assignment/ledger-adjacent records); that view is machine-only and must never reach a terminal. Human status attempts invocation-progress collection when an invocation is selected; measure when this triggers live Dagu work and its cost.

Possible directions (investigate, do not assume): drop or cap the `status details` tail in human mode since locators already point at the detail; gate it behind a flag; add a hard-capped digest view; skip the Dagu collection unless progress is requested; determine whether the 260k-record fan-out in full output is a cartesian duplication bug in change-report assembly or legitimate history rendered raw.

Directive: investigate first for the correct shape of human status — what an owner needs at a glance (current work, next action, blocker, evidence location, freshness, uncertainty) versus what belongs behind locators — against a live messy mid-implementation run, not a finished one. Fix what fails that shape; do not add a parallel monitoring system. Any CLI change keeps status non-arming and preserves the applicable show output and exit-status contracts.

Owner-observed compliance signal (2026-09-22, other runs): agents pipe `show` output to null between transitions instead of reading it. Note the sharp edge: arming happens server-side, so `show ... > /dev/null` satisfies the observation interlock's letter (visit armed, mutation allowed) while defeating its purpose (acting informed). Output cost may contribute to this behavior, but the cause is unconfirmed. Investigate alongside duplication and byte cost: quantify what an agent needs from action/full per transition; consider machine-first shapes (digest plus locators by default, bulk on demand); and determine whether interlock compliance should mean anything beyond a server-side arm. Address the cause rather than assuming a punitive gate will make agents read the output.

Extend the investigation to every read primitive agents actually consume — show views (status/action/full, human and JSON), history, invocation-progress, monitor packets, list, and the instruction/guidance surfaces — and audit for duplicated information across them: the same evaluation text, lane prose, ledger content, or invocation detail repeated in multiple outputs or repeated per record. Machine consumers stay complete (agents read this output; nothing they need is removed), but each fact should have one authoritative home with locators elsewhere, so agents stop paying context-window rent on the same bytes N times. Demonstrate byte counts per primitive on a live run before and after; do not merely assert efficiency. A separate 0.22.0 run reported 211 context records / 18.8 MB, a 47 MB fan-out spec and `show` taking over 100 seconds; repeated full-intent steering and per-worker context copying are leads to measure, not established causes here.

### Refuse unusable evidence at intake and let a later valid record replace it

Observed 2026-10-04 on 0.24.0 (run-1791141043221020000-1-57519, high-rigor-14-1author): reviewers cited context records as `owner-steering-1`. `software-change review-candidates` accepted those locators, but the intent-review gate later refused them ("needs a JSON Pointer or line selector"; "not found beneath artifact_root or repository cwd"). A malformed `review-evidence` record keeps its axis blocked: a later valid record from the same author, a finding-ledger disposition or an intent revision does not clear it, so the only way forward was an owner override. Make intake refuse anything the gate will refuse. Then either let a later valid record from the same author, gate, stage and subject replace a malformed one, or give a supported way to cite context records (for example the owner-source steering that intent reviewers are told to compare against). Also note that a multi-axis aggregate batch needs a shared `assessment_id` that the shipped worker preamble never mentions.

### Make high-rigor intent review clear without overrides

Observed 2026-10-04 on 0.24.0 (run-1791151179920088000-1-87013, high-rigor-14-1author). Every judgment passed, but the run still needed overrides:
- **Same-author rule caught late.** High-rigor requires each axis to have the same reviewer identity in the individual and aggregate stages (`mixed_authors`). Nothing at commission or intake warned when separate identities were used; it surfaced only at `approved`. The only repair is a `reviewer-manifest` roster change plus `retired-author` ledger entries and replacement reviews.
- **One override forces another.** After ordinary `intent-review` is overridden, the challenge gate denies `challenge-before-ordinary-clearance` even when every challenge judgment passes, so a second override is unavoidable.
- **Challenge commission omits the owner's comparison.** `commission --slot intent-adversarial-review` did not deliver the `intent-comparison` record, although the `intent-sufficient` challenge prompt requires it.
- **Challenge context is mostly duplicated packets.** The challenge commission context was ~1.4 MB, mostly each parent record's duplicated `external_review.request` packet (~130 KB × 8).

Warn about stage/author mismatches at commission or intake, include the comparison in challenge commissions, and stop repeating request packets in delivered context.

### Preserve applicable work and evidence through correction

LE-150 and the shipped selected-work/applicability paths already support meaningful same-run reuse after plan reshaping. The external-work refactor extends this to flexible commissions and proportionate aggregate/goal reuse. Close only demonstrated remaining gaps: preserve original work and failed attempts, inspect real dependency/scope effects, refresh affected proof and finish normally without replaying unaffected work for ceremony. Do not build generalized replay or a second criterion ledger merely because older notes described repeated work.

### Validate compact review packets and blinded perspectives

LE-149 and the current commission filter deliver relevant context and accessible originals without arbitrary content quotas. A subsequent intent revision still exposed indirect individual-stage applicability conclusions in first aggregate preparation; the driver removed them before model launch. Reproduce and close any remaining product-path leak without relying on a private wrapper, while preserving required combined perspective and relevant applicability/findings.

Measure actual delivered context and test the declared independence/blindness purpose. Necessary originals must remain accessible; filtered or smaller output is not itself proof that the reader received correct material. Keep read-primitive duplication/human digest work in the separate item above.

### Maintain review quality against actual misses

Grounded pass reasons, locator-only source references, truthful intent approval, merit-based triage, explicit partial/output-only recovery, invalid-reuse refusal, factual evidence preview/apply and per-criterion distinguishing proof are delivered baseline duties under LE-151/152/154–159. Do not recommission their implementation wholesale.

Retain real misses and sound judgments for the required fresh/applicable-history calibration when judging meaning changes. The external-work refactor adds the owner-alignment and causal acceptance-sufficiency questions and strengthens granularity/plain-English guidance; its manual prompt trials are diagnostic, not complete calibration or universal model-quality proof. First ordinary reviews remain comprehensive, affected confirmation preserves valid evidence, and challenge tests a cleared conclusion rather than adjudicating open ordinary failures.

### Evaluate external advisory backends and judgment quality

The generic typed advisory primitive and software-change's bounded evidenced questions are delivered under LE-163–166. The external-work refactor preserves enabled advice as driver-performed work, with genuine typed values and reasoned dispositions, not engine-owned dispatch or gate authority.

Remaining investigation concerns useful judgment quality and feasible backends: compare sufficient-evidence bounded cases against reasoning judgments; inspect misses, false alarms and confidently wrong answers; measure real interface limits, latency, resource contention, installation and licensing. Evaluate a local model only if useful, with an owner-selected endpoint or host and honest unavailability; no model, download, deployment or default change is selected here.

Do not revive the earlier proposed automatic cascade or send investigation, planning, missing-fact inference, cross-artifact or whole-run synthesis through the bounded advice interface. Advice does not replace independent reviews, alter author floors, accept requirements or authorize exceptions. Driver-owned reasoning/reconsideration remains outside that classifier contract. The general cross-provider evaluation/framework item above is separate.

### Make small profile changes without a wholesale fork

Standalone export, selected-file authority and effective preview/validation are delivered by LE-160. The owner now selected the remaining small-customization outcome for the external-work refactor: specify a shipped base plus common policy changes through supported preparation, inspect overrides versus defaults and visible drift, and resolve/confirm the real effective profile before launch. Guaranteed knobs cover author counts, explicit self-review permissions, groups/axes/stages, and advice/Bookends switches; raw schema/prompt replacements can remain a complete-custom-profile path.

For future preparation against an updated base, unoverridden values must not remain a stale copied fork. Started runs do not silently track shipped changes. Choose the smallest recipe/options/overlay path; no inheritance framework, editor or preference store is assumed.

The owner also requested explicit durable amendments of review, criterion and goal author counts without restarting supported active runs. Record before/after counts and effective obligations, preserve original judgments/findings/history, retain eligibility/blindness/group meaning and require genuine extra coverage when raising a floor. Old pinned runtimes are not retrofitted; the current delivery may use a separately confirmed replacement run. These are selected outcomes, not claims of implemented capability.

### Report final residuals honestly

Selected for the external-work refactor: existing final report/review surfaces make remaining limitations, accepted residual risks, unresolved defects, unsupported operations and proof uncertainty understandable. Distinguish accepted scope from undelivered outcomes and visible owner exceptions. Do not add a new phase or duplicate residual ledger by default, fabricate owner attribution, or rename a blocking defect residual to obtain a clean completion.

### Record externally executed command proof

Selected for the external-work refactor: the driver/harness runs test and proof commands. The engine records supplied results, output references and deterministic metadata; its optional command/matrix launch and execution-control surfaces are retired too. Normal proof can be fulfilled from genuine sufficient external results without rerunning solely for engine capture, while preserving current-source identity, actual assertions and original attribution. Deterministic provider calls and metadata checks remain. Historical captures stay original history, not newly labeled external work.

### Complete one workflow across repositories

Selected for the external-work refactor: support a representative change spanning multiple repositories through one ordinary workflow. Sources, coverage, command proof, identities and applicable Bookends authorities must identify the correct repository; matching filenames or local IDs are not interchangeable. Prove both outcomes and a wrong/stale-secondary-source refusal, without assuming one evaluation cwd is sufficient.

Drivers retain repository/worktree/clone/merge/commit authority. No repository manager, automatic semantic-impact classifier or mandatory external-work registry follows. This item is not delivered merely by externalizing agent execution.

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
- **Early worker termination after a failed invocation:** If a newly invoked worker is again killed (`signal: terminated`) seconds after start, before its model runs, and shortly after a failed or cancelled invocation on the same slot, retain both invocations' ownership records, captures and timestamps and establish whether cleanup of the earlier invocation reached the new one. Do not assume an immediate successful rerun explains it.
- **Older software-change/CLI dogfood leads:** Defer minor `show`/`@PATH`/payload-less `event` diagnostics unless repeated ordinary use makes them material. Recheck the earlier `proof_id` documentation complaint and contract-v2 `run-validation` refusal against current supported contracts before treating either as an active defect; 0.22.0 `run-validation` emits `proof_id` in command-evidence candidates and its recovery contract accepts contract versions 2 and 3, so only template/schema documentation of `proof_id` for hand-written reports may remain.
