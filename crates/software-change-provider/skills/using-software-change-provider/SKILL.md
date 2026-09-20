---
name: using-software-change-provider
description: Use when running the software-change workflow through Loop Engine with the software-change provider — confirming work-slot bindings and models with the user before start, selecting a config profile, authoring gate artifacts against frozen operating context, invoking bound implement workers or performing unbound rooms, appending concise review-evidence, evidence-applicability, and driver-authored finding-ledger snapshots, and clearing checked transitions.
---

# Using the software-change provider

## Scoped discovery and delivery decisions

Before drafting, discover the actual operator path, desired observable outcome, constraints, accepted risks and non-goals. Ask about consequential ambiguity; do not launder a predetermined mechanism into intent. Budget the complete serial dependency closure, summarizer, proof and review, not just one worker. Return a durable blocker and seek direction when the approved budget is insufficient; do not silently retry.

For bound and unbound review, use the shipped `data/review-worker-preamble.txt` framing: comprehensive first review across assigned axes; scoped confirmations of accepted fixes and fix-introduced holes; explicit original-evidence applicability for unaffected axes. Falsify circular proof by asking what observation distinguishes the claim from its opposite. Embedded conclusions and a report citing itself do not establish the outcome. Keep existing axes and frozen materiality limits.

After implementation capture triage, before independent review, obtain the owner's Git decision. Inspect staged names and diff; perform/confirm only the owner-authorized human/driver commit and verify its resulting identity with `git rev-parse HEAD`, or record pending/declined without changing Git or claiming a created commit. Workers do not independently commit. The reminder is not authorization. HEAD/index/status changes invalidate current identity-bound receipts, report and checkpoints: settle Git first, refresh invalidated proof through its existing owner (the graph summarizer owns its report), then keep source/Git identity stable through review and validation. Do not make the post-report checker its own prerequisite or normal terminal completion a pre-terminal proof obligation.

## Overview

`software-change` is Loop Engine's reference provider, distributed standalone with its shipped data embedded (`software-change data-dump DIR` materializes it); a repo checkout remains the development path. The live graph is the union of five draft rooms plus parent and adversarial review rooms when those policy lists are nonempty; `describe` omits empty lists. Draft ready events check their declared schemas, revision links and proof obligations; the v11 handoff into reconciliation has no report/checkpoint prerequisite. Parent semantic axes live on the parent review state; adversarial axes live on the distinct adversarial-review state. Validation-report-local corrections stay in the validation draft after edit/recheck for the next checked hop; the validation draft also exposes check-free `revise-implementation` when validation exposes a repository-state mismatch. Nearest `revise` from validation-review and validation-adversarial-review returns to that draft, and those states also expose `revise-implementation`. Phase-named owning routes (`revise-intent`, `revise-design`, `revise-plan`) handle upstream defects from review states and directly from the implement draft in newly described graphs. Reviewer convergence contract requires candidate triage before append or mutation, focused external reconsideration for disputed candidates, comprehensive first review, and bounded confirmation review. Quiet, progress, and thrash count per review state on the post-triage accepted-unresolved finding-ledger set. Confirmation consumes the durable set and does not search again except for fix-introduced holes. Bound workers do not use previously overlooked after that state's first comprehensive review of the subject; humans still may with full failure burden. Review packets carry the current ledger snapshot in immutable context; the frozen worker assignment identifies ordered axes, and the reviewer inspects entries whose `review_axes` include each assigned axis without changing policy or verdict authority. Known accepted material defects are never waived. Adversarial output is candidate data under the YAGNI/pragmatic append bar: extra mechanism, unlisted requirements, and hypothetical-future fails are not appended. Late findings still require current evidence, violated obligation, consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); prior visibility or overlook does not waive known material defects, while comprehensive-first and scope/materiality burdens block drip-feeding or unrelated reopening. Human-facing challenge review must meaningfully falsify a parent pass claim against frozen intent, not manufacture a second opinion.

The provider is deterministic only: it validates artifact schemas and revision links, then aggregates externally supplied review evidence. `describe` and `evaluate` never generate prompts, invoke a model, or judge findings. Bound workers, when frozen, are started by `loop-engine invoke`; you still triage outputs and append verdicts. Per-run obligations are frozen in immutable `initial_input`, and every later actor must inspect the run's frozen `intent.json` operating context rather than relying on chat memory or a profile default.

The criterion spine is run-local and intentionally small. Write each current intent acceptance entry as the closed `{id, statement}` record with an `AC-N` ID matching `^AC-[1-9][0-9]*$`; preserve an ID when its meaning is materially unchanged and use a new ID for replacement. Design coverage, implementation validation rows, and validation proof rows may carry optional `criterion_id` or `criterion_ids` references. Every contract-v3 plan task requires a nonempty unique `criterion_ids` set of current AC-N IDs; the provider checks membership while reviewers judge relevance. Other intermediate references remain optional. Contract v3 final validation requires complete criterion/goal coverage through the fixed index described below; the provider checks relationships, never semantic sufficiency.

## Required companion and engine driving minimum

`using-loop-engine` (`skills/using-loop-engine/SKILL.md`) is a **required companion**. This skill does not replace it. The closed driving minimum below is what you cannot skip when this skill is loaded alone; load the companion for full engine semantics.

**Run-state commands:** `start`, `list`, `show`, `append`, `event`, `history`, `terminate`, `invoke`, `amend-binding`, and `cancel-invocation`.

**Shared controls:** use the engine companion's **Choose an observation** and **Capture external commands** procedures for `monitor`, optional summaries, `capture-command`, `capture-matrix`, resume and `capture-abort`. These do not append provider evidence or advance a gate. Monitor emits JSONL; capture streams raw output, unlike workflow envelopes.

**Envelopes:** `completed`, `rejected`, `error`, and `invalid-invocation`. Parse JSON even on nonzero exit. Treat only `completed` as success.

**Bound versus unbound:** a catalog slot ID present in frozen `work_slot_bindings` is bound — `invoke` it; do not perform the stored work body. An absent key is unbound — perform the stored instructions yourself, then append and request the event.

**Overlay meaning:** overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. You still triage worker output, append provider-shaped records, and request the shown event.

**Observation:** action `show` (default) gives current instructions and arms the visit; repeat after every transition before mutation. Use `show --view full` for frozen input, context, invocation/change reports and helper stdin. Status/compact and `monitor` never arm. Wait through passive `monitor`, using `invocation-progress` only for targeted graph/trace diagnosis. Helper `reaped`, worker exit, output conformance and provider judgment are distinct.

**Concurrency:** `loop-engine fan-out --max-active N` omitted stays uncapped; set N is at most N worker steps. `software-change run-plan-graph --working-directory ABS --max-active N` requires the driver's existing absolute directory; omitted stays 4 ordinary plan tasks, set N is at most N ordinary plan tasks, and the summarizer still runs after those tasks.

**Lock-in-before-start:** do not call `start` until the user confirms (1) bind or not (which slot IDs), (2) exact `{command, args}` per bound slot, and (3) model identity in those frozen args (nested `--worker` / `--task-worker` count) or explicit unpinned-default acceptance. Initial bindings freeze; owner-attested `amend-binding` changes future execution only, not initial input, policy or launched attempts. See the required engine companion for controls, preview, cancellation and override.

Run `loop-engine preview-bindings` on the JSON you will freeze before `start`. That report includes a `dagu` PATH check (minimum 2.14.0): ok with resolved path and version, or a warning naming the path or that PATH lookup found nothing; well-formed bindings still exit 0. `fan-out` and `software-change run-plan-graph` execute fail-close on the same condition before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator keys `dagu_home`, `dag_name`, and `run_name` (`plan-graph-<capture-dir-name>` for plan-graph). Provider packages do not ship `dagu`. Dagu is GPLv3: invoke the binary as a subprocess only; do not embed its Go API. Bound review `fan-out` joins mechanically (`fan-out-join` writes `summary.json`, invokes no model). Bound worker stdin is compact location JSON (absolute `artifact_root`, plus `context` when the catalog slot lists `stdin_context_kinds`); it does not dump `instruction_body`. All software-change slots forward eligible ledger/evidence/applicability/user-steering/incorporation context, with validation proof records additionally eligible at validation. The provider commission filter selects applicable recipients from the launch snapshot. Assigned axes are frozen in the review preamble; implementation tasks receive only exact-task findings and steering in their task packet, not dependant/summarizer spillover. Hidden stdin-exec colocates Pi sessions under each worker `capture_dir/sessions` via `PI_CODING_AGENT_SESSION_DIR` unless that variable is already inherited; do not add `--session-dir` to frozen argv, and do not switch bound Pi commands to `--mode json`. Provider contract details: `crates/software-change-provider/README.md`; frozen requirements: `crates/software-change-provider/docs/prd.md`.

## Frozen intent and operating boundary

Before authoring, commissioning, triaging, or validating any phase, read the current `intent.json` beneath the run's `artifact_root`. Treat its `operating_context` (`operators`, `environment`, `threat_boundary`, `accepted_risks`, and `outside_obligations`) as frozen and shared by every later actor. Do not demand speculative hostile-user or multi-tenant hardening excluded by `threat_boundary` unless the change invalidates that boundary or an outside obligation requires it. Accepted risks are residuals, not waivers: they never excuse a stated outcome, acceptance line, or outside obligation. Treat `AC-N` as the only criterion identity; optional downstream references must point to the current intent and must not create a second `LE-N` spine.

For v11 intent review, author one authoritative plain-English intent with exact technical references and structured metadata alongside the decision prose. Explain a specialist term when it first matters, preserve obligations and qualifications, and do not add a second narrative, readability score, or technical-language ban. Shape each acceptance criterion as one coherent bounded outcome with a practical evidence path: combine clauses sharing one decision, split independently varying outcomes, keep inseparable aspects together, and name a concrete reason when proof is impractical. The `acceptance-granularity` and `owner-comprehensible` axes are distinct external questions; style, sentence length, reading-grade scores, technical vocabulary, word counts, and conjunction counts are not findings.

Bound fan-out delivers compact context by default for all workers, with or without preamble: only each selected record's engine-owned top-level `data.loop_engine_origin` is omitted from a clone. Record IDs/order, concise origin, judgments, meaningful fields, assignment and controls remain; no model classification, opt-in flag or recursive stripping applies. Actual captured stdin is the truthful delivered input, not a full verification snapshot. The original selected full context is independently retained in per-worker `fan-out-spec.json` `routed_inputs` and summary routing. `capture_format: "bound-context-projection-v1"` requires that full snapshot for captured-commission verification; missing/malformed snapshots refuse. Unmarked legacy captures keep stdin-based verification. Core/provider history and existing selected-output, digest, assignment, attempt and applicability checks remain full-strength. Raw ad-hoc fan-out instruction bytes are unchanged. Graph task/findings/steering and summarizer packets remain their separate existing projection; no-task `ad-hoc-repair` still uses its closed repair packet and worker-owned report, not a graph summarizer. The observed oversized intent-draft setup path is classified against live LE-127; do not infer a new requirement without distinct semantics. Compact delivery remains the normal supported path and does not permit stripping generic protocol data needed by deterministic or non-model consumers.

The driver owns semantic disposition and Git lifecycle. Reviewer output is candidate evidence only. Preserve raw stdout/stderr, append an advisory `advisory-finding-proposal` only as an inert suggestion, and append the driver-edited `finding-ledger` snapshot that alone controls gate agreement and exact implementation routing. Do not let a proposal, report claim, or passing worker exit substitute for a driver decision.

The existing review-axis IDs remain unchanged, and the shipped v11 intent profiles add only the named `acceptance-granularity` and `owner-comprehensible` questions. High-rigor `plan-review` uses exactly `task-sized`, `context-sufficient`, `done-observable`, `decision-free`, `design-faithful`, and `dependencies-honest`; validation uses exactly `intent-delivered`, `docs-integrated`, and `requirement-proof-mapping`. Read every profile's exact axis lists; current v11 profiles share the same phase axis IDs, with different author floors and review stages. Do not add an axis to satisfy a finding or create another requirement axis.

Run checked implementation/validation evaluation from the intended repository checkout, not the artifact directory: verification resolves the repository from the provider's current working directory. The artifact root locates reports, not the evaluation repository. Checkpoint creation's explicit `--working-directory` does not change later evaluation CWD.

For implementation and validation, the report is not proof by itself. Run `software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS` after the report is complete. Both directories must already exist and be absolute. The command only reads repository state and writes the phase checkpoint; it never stages, commits, branches, pushes, creates, selects, merges, cleans, manages, or suggests worktrees. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, with or without implementation review. Validation requires the sole history entry for the current report revision to match the current report, document revisions, and repository state. Later context, regenerated mutable checkpoints, or overwritten history bytes cannot admit different bytes. If a checked event reports a stale checkpoint, use `revise-implementation` where shown, regenerate implementation report/checkpoint and validation report/checkpoint for the same current tree, then append fresh review evidence and ledger state before retrying. Validation cannot replace implementation proof.

## Contract v3, simplicity and final criterion proof

Shipped versions are `minimal-11`, `standard-11`, `high-rigor-11` with `contract_version: 3`. Independent criterion/goal floors are 1/2/2 for minimal/standard/high and remain separate from review-axis floors. Profiles remain unbound and Bookends-off by default. Intent review includes the distinct `acceptance-granularity` and `owner-comprehensible` questions at aggregate stage; high review evidence keeps `individual` and `aggregate` stages for its existing axes, while the new intent questions are aggregate-only. The same two author identities must cover high's existing stages, while standard and minimal use aggregate coverage only. Unsupported v2 semantic evaluation refuses explicitly; keep fixed original providers for historical runs. Bare describe is discovery; provider-free show/history preserves original records with no implied ownership/control capability. Do not migrate active bootstrap input, topology or bindings by catalog edits.

Simple-first/YAGNI/KISS apply product-wide. Complexity needs a meaningful current requirement/failure and inadequate simpler alternative in the ordinary design. Existing implementation/protocol/schema/dependency choices have no presumption of preservation; do not add speculative hardening or a justification bureaucracy.

Use the shipped `data/templates/validation-report.md` and `data/validation-report-schema.json` for exact v2 forms. The unbound command is `loop-engine --json show RUN --view full | software-change run-validation --engine ABS --working-directory ABS --revision REV [--commands ID,...] [--timeout-ms N]`. It executes named plan `proof_commands` (id, command, args, owner, obligation), retains actual command/cwd/exit/time/output/repository evidence and writes the report index plus inert command candidates. A subset never waives required final proof. It does not append, checkpoint, invoke reviewers or advance; do not emit a command-only bound validation draft that leaves its report duty to the driver.

### Prepare validation from retained commands

When commands already ran through common capture, use `prepare-validation` instead of rerunning them. This inert helper requires the validation draft state; it does not replace a bound validation worker's duty. First read `show --view full` and inspect `commission --slot validation-draft` for effective proof commands. Capture each required command with its exact ID, argv, obligation and declared settings, beneath the run artifact root and outside the checkout. Keep source/Git identity stable. See the engine companion for capture/resume.

Build one packet with `show` (the completed full envelope), absolute `working_directory`, fresh `revision`, `author: {name,kind}`, absolute `capture_indexes`, `execution_settings: {timeout_ms,environment,inherit_environment}`, and `additions: []`. Settings must match the captured rows; inspect repository `docs/operational-ux-contracts.md`, **Supplemental validation**, for exact forms.

```sh
software-change prepare-validation < preparation.json > prepared.json
```

Inspect `diagnostics` and require `commands_complete` before finalizing. This checks collection mechanics, not acceptance. Missing, failed, stale, duplicate or incomplete-cleanup receipts require correction or fresh execution; never infer a pass. The helper launches nothing, appends nothing, writes no report/checkpoint and supplies no passing verdicts.

For additional proof, supply unique `{id,command,args,owner,obligation}` entries in `additions` and corresponding real captures. They cannot replace frozen required IDs; `proof_updates` corrects existing executors, not adds commands. Append the returned `addition_candidates` as `validation-command` and `command_candidates` as `command-evidence`, preserving their proposed record IDs and data after inspection. Do not resubmit already-appended additions; later full observations expose them through commission. Use a fresh revision on a proposed-ID collision.

Install `report_draft` as `validation-report.json` only after completing the collection and choosing genuine unused verdict IDs. `judgment_batches` names pending AC/goal positions, not completed judgments; honor `excluded_authors`. Then follow the fixed-index procedure below. `run-validation` remains the execution alternative when fresh named plan-command execution is needed; neither helper upgrades a run frozen to an older provider.

### Finalize and judge the fixed index

Inspect and append genuine command candidates, finalize the index and prechosen unused verdict IDs, then checkpoint its bytes before review. No placeholders/reservations or report rewrite after judgment. The index names current implementation_revision, command_evidence_ids, exactly one `{criterion_id,verdict_ids}` per current AC-N, and separate goal_verdict_ids. Validation-ready can leave verdict IDs pending only with live review; ordinary approval or a reviewless final draft hop requires complete independent coverage and matching accepted implementation-proof-history. Criterion/goal authors cannot be report/implementation authors. Missing/duplicate/unknown/stale/self-authored/unsupported or unresolved failing coverage blocks. Ordinary validation workers may return `validation_verdicts` alongside axes; append actual rows with their prechosen IDs after triage. Challenge consumes that collection, not a second criterion review or proof run.

After repair append `criterion-revalidation: {subject_revision,affected_criteria,change_kind,reason}` (`change_kind` material or report-index-only). Affected criteria require fresh judgments; unaffected index rows may reference explicit evidence-applicability to original verdicts at the current report/checkpoint, retaining author/result/source and reason. Material repair requires fresh goal; only explained report-index-only correction can carry it. `commission --slot validation-review` exposes pending/fresh/carried rows. Failed criterion/goal sources use exact-source finding dispositions with policy_id AC-N or goal, never a fabricated pass.

Workers run assigned focused checks; one driver/proof owner runs the full stable-tree matrix and repeats only invalidated checks. Reviewers consume retained outcomes. The public journey uses `--jobs 2` by default for independent isolated work; `--jobs 1` remains serial. Final benchmark comparisons, hosted exact-commit checks and later live dogfood are not proved by focused or synthetic runs.

## Setup

Before starting a run, load `skills/using-loop-engine/SKILL.md`, its **Deterministic setup** procedure, and the catalog/override rules. Use the normal user catalog. A database or artifact override needs an explicit isolation decision for this session.

Build the paired binaries with:

```sh
cargo build -p loop-cli -p software-change-provider
```

Use the provider binary's public setup command:

```sh
software-change setup \
  --rigor minimal|standard|high \
  --roster /absolute/path/roster.json \
  --engine /absolute/path/loop-engine \
  --provider /absolute/path/software-change \
  --output /absolute/path/run-profile.json \
  [--bookends] [--draft-worker /absolute/path/draft-worker.json] [--implementation /absolute/path/implementation.json]
```

The roster is an ordered, closed JSON array of `{ "author": "...", "command": "...", "args": [...] }` records. Author labels are distinct and nonempty. `command` and every argument are caller-owned bytes; setup preserves them and does not add model, effort, extension, or shell flags. `--draft-worker PATH` is an optional closed `{ "command": "...", "args": [...] }` object. When supplied, setup preserves it as the bound `intent-draft` binding; callers that need compact context normally make that binding the existing engine `fan-out` facade with one nested worker. The optional implementation file is one closed `{ "command": "...", "args": [...], "working_directory": "..." }` object. Its directory must already exist and be absolute.

Setup reads the selected profile, review preamble, and complete output schema from embedded provider data. It assembles every live review gate, exact policy order and stage, first-N author allocation, assignment-specific schema, and `{ "command": PROVIDER, "args": ["commission"] }` context filter. High-rigor workers run individual assignments before fresh aggregate assignments through the generic `fan-out --then` barrier. Aggregate preambles explicitly exclude individual captures and judgments. Review fan-out is frozen with `--max-active 2`. An implementation binding uses `run-plan-graph --max-active 1` and the supplied worker and directory.

The command invokes `ENGINE preview-bindings` and writes the generated profile only after input and preview validation succeed. The replacement is atomic. It starts no run, invokes no worker, selects no model, adapts no CLI, creates no worktree, and saves no preference. Its JSON stdout shows `effective_policy`, the exact roster, the output bytes, `output_byte_length`, `output_sha256`, `output_sha256_digest`, the generated profile path, and the binding preview. Warnings in the preview remain warnings; preview errors fail setup.

Setup refuses malformed or duplicate roster records, malformed draft-worker input, an insufficient author count for any configured stage, malformed implementation input, invalid shipped policy/schema data, non-absolute engine/provider paths, and incompatible output contracts. It never drops an axis or reduces an author floor to make input fit.

Before `start`, inspect the setup report and the exact output file. Confirm:

1. the rigor label, config version, contract version, criterion/goal author policy, every live gate and every normalized stage/axis author count;
2. Bookends state and any overlay axes;
3. every roster command and argument, every nested review worker, review concurrency, implementation concurrency, and commission filter;
4. the exact output bytes and SHA-256.

Hash that same output file immediately before `start` and abort on any mismatch. Start that unchanged file with the canonical engine procedure. A setup report or preview is not a run and does not authorize progression.

Shipped data remains unbound. The generated file is the per-run authority; later edits to shipped profiles, the skill, or a roster do not change a started run. Do not replace a confirmed file with a pristine profile after confirmation. Active runs keep their frozen policy, bindings, stages, and evidence. Use `show --view full` before a correction. In high rigor, select only affected individual assignments, create explicit applicability records for unaffected axes, and run both aggregate authors freshly across every axis. Aggregate authors remain the same identities as the individual stage. Accepted-unresolved findings still block. A binding amendment changes future execution only; it does not rewrite initial input or a launched attempt.

## Criterion spine and Bookends overlay

Shipped profiles leave Bookends off. With the overlay disabled, `AC-N` is the only criterion spine. To opt in, pass `--bookends` to setup; the generated profile freezes `extra.bookends.enabled: true`. Overlay-on requires exactly one `prd_traceability` object on every current intent criterion: `linked-live`, `candidate`, or `not-applicable`. A current candidate blocks final completion until the owner accepts it into a committed PRD or reclassifies it. `not-applicable` never waives or fulfills the criterion. Inspect `BOOKENDS_BYPASS` before final approval; a bypass remains visible and cannot satisfy final GREEN.

New semantic-coverage profile revisions retain the `ids-grounded` axis rather than adding a second requirement ledger. During intent/design drafting and review, read the actual normative wording of every cited live requirement and every authoritative document it explicitly names. Classify each promised enduring outcome into the three branches: sufficient existing wording, change-specific proof, or missing or changed enduring meaning. When delivered behavior is wrong under sufficient wording, record the implementation defect within the sufficient branch and correct the code; it is not a fourth requirement-meaning branch. A related ID, shared topic, matching token, parser-valid candidate, or command success is not coverage. Keep proposal wording substantive and provisional; record owner acceptance, application, and commit separately, and never treat a candidate as live. Bookends-disabled runs use the same semantic handoff for relevant repository documents without PRD IDs or overlay obligations. Existing frozen profile revisions keep their original ids-grounded rubric.

## Reconciliation state and proof ownership

New v11 `contract_version: 3` graphs expose the provider-owned `reconciliation` state after implementation editing and before the final implementation proof/review boundary. Its unbound `reconciliation-draft` slot authors `reconciliation.json`; the checked `reconciliation-ready` event validates it. The state does not write implementation or validation reports, checkpoints, or Git commits. Make the three-way branch conditional: sufficient existing wording, including an implementation defect corrected under sufficient wording, needs only the needed implementation and public proof and retains live traceability when Bookends is enabled; change-specific proof creates no requirement proposal or PRD commit; only missing or changed enduring meaning needs exact owner acceptance, separately authorized application and commit, updated traceability, and independent inspection of the resulting PRD text and associated public proof. A justified no-document-change result is valid, and a blocked result must retain concrete blockers.

With Bookends enabled, read the actual accepted text of every cited requirement and every authoritative document it explicitly names. Preserve live traceability and reject related IDs, shared topics, matching tokens, parser-valid candidates, and command success as semantic coverage. With Bookends disabled, inspect relevant repository documents against approved intent and delivered behavior without PRD IDs, Bookends citations, candidate machinery, or overlay obligations. After reconciliation, finalize the implementation report, create the repository checkpoint, conduct any configured implementation review, validation, and final proof against the resulting tree. If an authorized reconciliation edit makes an earlier report or checkpoint stale, both reviewful and reviewless graphs expose the check-free `revise-implementation` return to `implement`; invoke the existing bound implementation/report owner through its supported selection (or perform the unbound correction), preserve the authorized document edits, and return through `implementation-ready` without reapplying them before finalizing proof. The current run and older stored graphs are not migrated; perform equivalent reconciliation through the existing workflow and evidence conventions.

The coordinating assistant owns the passive owner update: after reading existing status or monitor output and observing a meaningful development, it posts a concise source-backed update naming the observed change and needed action or decision in the active conversation before the next wait, inspect, or help decision. Machine completion/attention is not that update. Workers run assigned focused checks; the coordinating driver is the designated proof owner for the complete final stable-tree matrix and repeats only checks invalidated by later changes.

## Work-slot policy (confirm before start)

Cataloged slots are `intent-draft`, `intent-review`, `intent-adversarial-review`, `design-draft`, `design-review`, `design-adversarial-review`, `plan-draft`, `plan-review`, `plan-adversarial-review`, `implement`, `implementation-review`, `implementation-adversarial-review`, `validation-draft`, `validation-review`, and `validation-adversarial-review`; v11 additionally catalogs `reconciliation-draft` in `reconciliation`. Bindings are sparse and freeze at `start`; a binding must name a slot in the snapshotted catalog. Setup emits live review bindings, preserves an optional caller-supplied `intent-draft` binding from `--draft-worker`, and may emit an optional `implement` binding. A slot with no configured policy axes must remain unbound.

Before `start`, run `loop-engine preview-bindings` on the exact `work_slot_bindings` map and confirm the report. Confirm the exact outer commands, every nested worker/task worker, author/stage assignment, concurrency, commission filter, and any model/effort arguments supplied by the caller. An unbound slot remains driver-performed. Owner-attested binding amendments affect future execution only; they do not rewrite initial input or an already launched attempt.

## Exact profile confirmation

Treat the generated profile as immutable after confirmation. Recompute and record its SHA-256, inspect the full `work_slot_bindings` map, and run `loop-engine preview-bindings` on that same map. A profile hash covers the generated file and its embedded assignment bytes. It does not cover work performed by unbound actors or a later replacement roster. If any byte, command, argument, stage, author, or model/effort argument changes, repeat the report, preview, and owner confirmation before starting.

Unbound launches use a separate owner-confirmed role-to-model manifest. Keep its exact bytes and hash outside the profile, verify each requested model with `pi --list-models`, and pass that exact model when the role launches. Stop on an unavailable or substituted model. A generated setup roster remains the authority for bound worker command and argument bytes; it does not supply an implicit external-fleet choice.

For an active run, never retrofit new profile obligations into the frozen input. Read the current show, use the stored graph and bindings, and choose the owning correction route. Preserve valid evidence explicitly; obtain fresh evidence for affected stages and subjects. A failed or overrun attempt does not authorize an overlapping retry. The driver triages captures and appends evidence; setup never appends, retries, dispositions, or progresses a gate.

The implement binding remains a separate opt-in `run-plan-graph --working-directory ABS --task-worker` pattern. Leave task selection out of frozen argv. Omit invoke input for the full plan. When an existing frozen task owns a correction, use `loop-engine invoke RUN_ID implement --input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'`; the provider requires that closed shape, current revision, unique known roots, and same-revision standing results for every prerequisite outside the roots-plus-dependants selection. Only when a current accepted unresolved implementation finding has no honest frozen task owner, leave its `task_ids` empty and invoke the same binding with exact `--input '{"repair_finding_ids":["FINDING_ID"]}'`. Repair requires unique current forwarded ledger entries that are accepted, unresolved, implementation-owned, current for the verified implementation checkpoint, and no-task-routed. Malformed, empty, unknown, stale, wrong-owner/status/disposition, task-routed, or absent-context requests refuse before Dagu resolution or mutation. Packet input combined with frozen `--task`/`--tasks` also refuses.

A valid repair runs one `ad-hoc-repair` assignment under the frozen task worker and checkout. Its compact packet contains the exact finding objects, frozen plan revision, provider-derived pre-report and repository-state identity, and the narrow report-writing obligation. It runs no plan task or summarizer and does not alter `plan-task-results.json`. The provider accepts the result only when `implementation-report.json` is schema-valid, links the frozen plan, and uses a revision absent from the pre-proof and all accepted implementation-proof history; only then does it create a new checkpoint. Inspect `summary.json` generic worker/output/routed-input data and `repair` pre/post metadata. A failed worker or report creates no post checkpoint but may leave partial checkout edits; restore or deliberately incorporate them before another mutation. After success, append a later ledger snapshot resolving the finding and reconfirm affected independent implementation review and validation. There is no direct unbound repair form.

Direct callers may still select roots with repeated `--task ID` or one `--tasks ID,ID,...`; omitted selection remains full execution. The implementation binding, whether supplied through setup or configured explicitly, must freeze one existing absolute directory selected and maintained by the driver, must not pass `--no-context-files`, and must freeze its model before start. Task mode projects only current accepted unresolved findings with `owner_phase: implementation` and an exact matching `task_ids` entry under that task's `finding_context`; stale, resolved, rejected, advisory, and unrelated entries are absent. Omitted, relative, nonexistent, and non-directory working directories are rejected before workers; the same graph-level cwd reaches every plan task and summarizer or the repair worker. Successful execution requires that directory to be a Git working tree for checkpoint generation, and the provider does not create, discover, select, reuse, merge, clean, manage, or suggest worktrees. Optional `--max-active N` may live in that frozen argv (omitted stays 4 ordinary plan tasks; set N is at most N ordinary plan tasks). Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only.

On the bound implement path, invoke persists the exact optional `invocation_input` on the invocation view and also supplies generic `standing_assignment_ids` from the same provider-free `show` projection. The graph treats a recorded prerequisite as standing only when its sidecar revision/success agrees and its assignment ID is in that engine list; graph-local exit 0 alone cannot admit stale work. A driver who wants to reuse a non-standing task must use the appropriate check-free correction route and then provide fresh proof. Direct ad-hoc `run-plan-graph` without an engine packet retains its sidecar-only contract.

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--working-directory",
    "/absolute/path/to/driver-selected-checkout",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"--model\",\"MODEL\"]}"
  ]
}
```

Add an existing extension path only when the selected model provider requires it. Keep the chosen model and its effort argument in the frozen worker args; setup does not infer extensions or models.

## Proportional late-finding guide

A late material finding remains actionable, but the driver should choose the narrowest honest shipped response. This is guidance, not an automatic router or semantic dependency closure. If a correction is material, bump the owning subject revision; its standing evidence becomes stale by design. Preserve valid upstream work and append-only history rather than re-clearing it for ceremony.

| Defect owner | Shipped response | Ordinary cost |
|---|---|---|
| Validation-report-only | Edit `validation-report.json` in the validation draft; retry `validation-ready` or terminal `passed`, using nearest `revise` only from a review state. | Recheck the report and validation checkpoint; fresh validation evidence follows a material report revision. Earlier implementation proof and work remain unless the repository checkpoint is stale. |
| Implementation, frozen task owns defect | Use `revise-implementation`, then bound `{plan_revision,task_roots}` selection (or direct `--task` only outside a bound run). | Rerun that task plus dependants, regenerate implementation and validation reports/checkpoints, and reconfirm affected downstream reviews. |
| Implementation, no frozen task honestly owns defect | Use `revise-implementation`, keep accepted unresolved finding `task_ids` empty, then invoke the same frozen slot with `{repair_finding_ids}`. | Run one captured repair worker, resolve the ledger finding, regenerate proof, and reconfirm affected implementation and validation reviews without replaying plan tasks. |
| Plan | Use `revise-plan` from implement or implementation/validation review. | Plan revision normally invalidates downstream implementation/validation artifacts, checkpoints, and reviews; re-author and re-clear forward. |
| Design | Use `revise-design` from implement or plan/implementation/validation review. | Design revision normally invalidates downstream plan, implementation, and validation work/proof/reviews; intent remains valid. |
| Intent | Use `revise-intent` from implement or design/plan/implementation/validation review. | Intent revision normally invalidates all downstream artifacts, checkpoints, and evidence; re-author and re-clear the downstream path. |

From implement, choose the owning route yourself: `revise-plan` targets `plan`, `revise-design` targets `design`, and `revise-intent` targets `explore`. These check-free routes require no implementation report or checkpoint for rejected work. First observe with `show` and wait for owned work to finish, or use `cancel-invocation RUN INVOCATION` and verify cleanup. Elapsed-but-live work and cleanup-pending cancellation block both departure and retry. Artifacts, repository edits, invocation/capture evidence, and denial history remain; there is no rollback or automatic defect classification. Consult the stored graph's available events: upgrading binaries does not add routes to an older run.

Deeper backtracking is exceptional: use an owning-phase event only when that phase's accepted obligation is materially wrong. Captured ad hoc repair is shipped only for the no-honest-frozen-task case; it is not a substitute for task selection or plan revision. Owner-attested `event --override` is an exceptional engine operation: only one available edge after observation/quiescence, permanently labeled `completed-with-overrides` on final completion. It never fabricates evidence or changes frozen policy; later edges retain their obligations. Load the engine companion's exact attestation and cancellation rules before use. Evidence applicability is an explicit driver declaration, not a work-routing or waiver mechanism. If frozen obligations or the operating boundary cannot be corrected in place, replacement is the existing fresh-run escape; retain the old run and its history.

## Gate map

Draft ready/passed events check their declared schemas and revision links. In v11, `implementation-ready` enters reconciliation without requiring an implementation report/checkpoint; `reconciliation-ready` checks the reconciliation result. Finalize the report and checkpoint against the reconciled tree before commissioning implementation review (or entering validation when no implementation review is live). Historical graphs without reconciliation keep their original implementation-ready report/checkpoint check. Implementation review and validation checked hops require their current provider-generated repository checkpoint. Live review `approved` or `passed` rechecks the subject, validates that gate's `review-evidence`, and requires a well-formed current `finding-ledger` snapshot with fresh subject and checkpoint state. Its exact-source dispositions must discharge current failures; accepted-unresolved findings still block across revisions. Discharged fails count as independent judgments, not rewritten passes. The live last hop into `end` is `passed`; earlier live review hops are `approved`.

| Event (from state) | Subject checked | Evidence gate |
|---|---|---|
| `intent-ready` (explore) | `intent.json` | schema/links only |
| `approved` / `passed` (`intent-review`) | `intent.json` | `intent-review` |
| `approved` / `passed` (`intent-adversarial-review`) | `intent.json` | `intent-adversarial-review` |
| `design-ready` (design) | `design.json` | schema/links only |
| `approved` / `passed` (`design-review`) | `design.json` | `design-review` |
| `approved` / `passed` (`design-adversarial-review`) | `design.json` | `design-adversarial-review` |
| `plan-ready` (plan) | `plan.json` | schema/links only |
| `approved` / `passed` (`plan-review`) | `plan.json` | `plan-review` |
| `approved` / `passed` (`plan-adversarial-review`) | `plan.json` | `plan-adversarial-review` |
| `implementation-ready` (implement → reconciliation, v11) | no artifact prerequisite | checked handoff; bound execution must still finish |
| `reconciliation-ready` (reconciliation, v11) | `reconciliation.json` | result schema and mode/branch obligations; a direct hop to validation also requires the post-reconciliation implementation report/checkpoint |
| `implementation-ready` (implement, historical graph without reconciliation) | `implementation-report.json` | schema/links + current implementation checkpoint |
| `approved` / `passed` (`implementation-review`) | `implementation-report.json` | `implementation-review` |
| `approved` / `passed` (`implementation-adversarial-review`) | `implementation-report.json` | `implementation-adversarial-review` |
| `validation-ready` / `passed` (validation) | `validation-report.json` | schema/links + current validation checkpoint matching accepted `implementation-proof-history/` entry |
| `approved` / `passed` (`validation-review`) | `validation-report.json` | `validation-review` |
| `passed` (`validation-adversarial-review`) | `validation-report.json` | `validation-adversarial-review` |
| `revise-plan` / `revise-design` / `revise-intent` (implement) | — check-free, owned work must be quiescent | no implementation report required |
| `revise-implementation` (reconciliation, validation draft/review states) | — check-free | regenerate implementation, report, review, and proof; preserve authorized reconciliation edits and re-enter reconciliation |
| `revise` (any review state) | — check-free | — |

## Per-gate loop

1. Read action `show` for instructions, obligations, events and work locators; read full for frozen input, complete context and invocation/change reports. Action/full arms the visit: repeat after every transition before mutation. Read `artifact_root/intent.json` operating context before phase work; never infer missing legacy author counts as zero.
2. If **bound**, do not author the room yourself. `invoke` it (`loop-engine --json --timeout-ms N invoke RUN_ID SLOT_ID`; choose an allowance above the 30s default), then monitor until completion or attention. On overrun, wait or cancel owned work and verify cleanup, then observe before retry. Inspect `capture_dir/summary.json`, selected attempts and stdout before stderr; overlay succeeded is only bound CLI exit 0. For implement, follow the selection/repair procedure above: tasks and summarizer share the frozen checkout; full/selected mode's summarizer owns the report, while no-task repair's single worker owns it. Use `invocation-progress` only for targeted diagnosis.
3. If this state is **unbound**, author or revise the subject artifact in `artifact_root` using its template from `crates/software-change-provider/data/templates/`. Material content changes require a revision bump — a bump makes prior raw verdicts stale for coverage, but does not resolve accepted-unresolved ledger findings; keeping the revision asserts the edit was immaterial. Preserve the frozen operating context, stated outcomes, and outside obligations; do not turn accepted risks into waivers or add excluded hostile/multi-tenant requirements. Every contract-v3 plan task must carry a nonempty unique current `criterion_ids` set; design and intermediate implementation references remain optional and use only current `AC-N` IDs. Final validation requires the fixed complete criterion/goal index, not a parallel PRD-ID spine. For unbound implementation or validation work, create the matching checkpoint only after the report is complete: `software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS`. Both directories must already exist and be absolute; the command is read-only with respect to Git.
4. For evidence gates, obtain each configured stage/axis's `required_authors` count of distinct external judgments (minimal aggregate 1; standard aggregate 2; high individual and aggregate 2): fresh context, not the artifact's author, each judging every assigned axis separately using its exact `example_prompt`. High aggregate authors must be the same identities as the individual stage. Default to one aggregate commission per used author per gate and one individual commission per axis and used author; focused confirmation carries only explicitly unaffected judgments. Follow `crates/software-change-provider/data/reviewer-protocol.md`. Reviewers must judge within frozen `operating_context`; do not append speculative hostile or multi-tenant demands outside `threat_boundary`, and do not treat `accepted_risks` as permission to waive outcomes or `outside_obligations`. For plan review, require affected user/operator paths, observable outcomes, pragmatic black-box proof or a concrete impracticality reason, sufficient context, and implementation freedom. For validation review, reject activity-only evidence and inspect every new or changed Bookends citation semantically rather than accepting its requirement token. Unbound: you commission those reviewers. Bound review: read the captures, then you still triage and append; `fan-out` does not write records. Preserve each axis's first-N author allocation in frozen review `--worker` args; individual-stage assignments are singleton axes, while aggregate-stage assignments group all axes for each author. Adversarial output is candidate data; extra mechanism, unlisted requirements, and hypothetical-future fails are not appended.
5. Append one record per accepted axis judgment — after triaging against the frozen intent and operating context, `kind` is `review-evidence`, `data` is the stage-aware object. Contract-v3 requires `review_stage` plus the eight original judgment fields; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` is non-empty on `fail`; and `config_version` must match the run's frozen config. For a bound worker, add only the concise origin reference:

```json
{
  "gate": "design-review",
  "policy_id": "intent-faithful",
  "review_stage": "aggregate",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "design.json",
  "subject_revision": "3",
  "config_version": "standard-11",
  "origin": {"kind": "selected-assignment-output", "id": "INVOCATION_ID", "assignment_id": "ASSIGNMENT_ID"}
}
```

```sh
loop-engine --json append "$RUN_ID" review-evidence @verdict.json
```

Core resolves the invocation and assignment from the same run and adds `loop_engine_origin`; do not copy its selected attempt, digest, path, capture directory, command, or binding. The provider verifies the raw selected bytes and compares `axis`, `author`, `result`, and `findings`. Missing, changed, unavailable, or disagreeing bytes are `unverified`. Omit `origin` only for genuinely external hand-authored evidence.

Before authoring any record, a fresh driver may run the exact read-only candidate pipe over the explicit `show --view full` envelope:

```sh
"$ENGINE" --json show "$RUN_ID" --view full | "$PROVIDER" review-candidates
```

The provider expands each completed contracted review batch into fresh per-axis candidates in durable invocation/assignment then frozen axis order. Authorized reuse rows are separately labeled `carried` references with their exact `applicability_id`, not new reviewer verdicts. `ready` is a mechanical selected-byte/contract result with only stable invocation/assignment origin plus normalized `axis`, `author`, `result`, and `findings`; `malformed`, `unavailable`, `missing-selection`, and `exhausted` are mechanical diagnostics with no judgment fields. The command does not open the catalog, retry, deduplicate distinct durable invocations, rewrite raw attempts, append context, route findings, or satisfy a gate. Inspect the raw attempts and then explicitly **accept, edit, or reject** each candidate. Candidate output is not `review-evidence` and is not semantic review; use the unchanged ordinary append path for driver-authored `review-evidence` and `finding-ledger`, then request the shown checked event.

For review reuse, append one distinct applicability declaration. It references the original evidence context record and names only the current target, attesting driver, and short reason; semantic applicability remains the driver's judgment:

```sh
loop-engine --json append "$RUN_ID" evidence-applicability @applicability.json
```

The provider retains the original evidence author, verdict, findings, subject revision, and config identity. It checks the named context record and current target/checkpoint mechanically and does not infer applicability from repository changes.

After triage, append one driver-authored `finding-ledger` snapshot from the shipped template. Its finding `source` is a context-record reference to the original review evidence. Keep every earlier snapshot and raw capture unchanged; `show --view full` exposes the full context history.

```sh
TEMPLATE="$DATA_ROOT/crates/software-change-provider/data/templates/finding-ledger.json"
jq \
  --arg gate "$GATE" \
  --arg subject "$SUBJECT" \
  --arg rev "$SUBJECT_REVISION" \
  --argjson findings "$FINDINGS_JSON" \
  '.data.gate=$gate | .data.subject=$subject | .data.subject_revision=$rev | .data.findings=$findings' \
  "$TEMPLATE" >finding-ledger.envelope.json
KIND=$(jq -r .kind finding-ledger.envelope.json)
loop-engine --json append "$RUN_ID" "$KIND" "$(jq -c .data finding-ledger.envelope.json)"
```

The closed snapshot uses `schema_version: "1"`, a driver `author`, and a unique `F-...` ID for each finding. Each source is exactly `{"kind":"context-record","id":"REVIEW_EVIDENCE_ID"}`; the provider resolves that record and its engine origin. Accepted findings use `unresolved`, `resolved`, or `stale` plus an owning phase; rejected/advisory findings use `recorded` or `stale` with null owner and empty routing arrays. For an accepted unresolved implementation finding, nonempty `task_ids` means the named frozen task owns correction and enables focused task selection; empty `task_ids` is the driver's explicit no-honest-frozen-task judgment and is eligible for exact `{repair_finding_ids}` selection. Do not leave the array empty merely to avoid task execution, and revise the plan when decomposition is materially wrong. The provider rejects invalid current subject/checkpoint/routing and changed stable source identities. Resolved historical source revisions and routing remain valid history without false applicability. Accepted-unresolved findings cannot be silently dropped or resolved by a revision bump. It derives current checkpoint identity; the snapshot does not copy repository-state, path, digest, attempt, command, binding, or changed-input fields.

### Advisory classification and routing proposal

A semantic classifier may write an advisory context record from `data/templates/advisory-finding-proposal.json` with kind `advisory-finding-proposal`. It can suggest candidate source IDs, a disposition, reason, owner phase, task IDs, review axes, and rationale. The driver must inspect each proposal and **accept, edit, or reject** it. A proposal never satisfies a gate, changes a reviewer packet, or routes an implementation task. Only the resulting driver-authored `finding-ledger` snapshot is authoritative; append that snapshot separately after triage.

6. Request the event. Interpret the outcome:
   - **Schema denial** (`rejected`) — artifact shape or link failed; evidence was not judged: fix shape first.
   - **Evidence denial** (`rejected`) — names unsatisfied policy axes and diagnostics for nonconforming/ignored records.
   - **Error** — invalid or inaccessible `artifact_root`, or provider failure; nothing advanced.

## Evidence rules (condensed)

- Latest conforming verdict per `(axis, subject_revision, author)` stands. Evidence is not a vote; an undispositioned standing `fail` blocks even when others pass. Reasoned rejection/resolution discharges only its exact source and counts as judgment, not pass.
- Distinct-author counts use exact `(name, kind)`; the subject's author never counts toward its own review.
- Stale `subject_revision` never satisfies; wrong `config_version` counts as neither pass nor fail.
- Nonconforming records block the axis with a malformed diagnostic until a later conforming record supersedes them.
- Normal gates retain accepted-unresolved findings across revisions until explicit disposition. Override is a separately visible owner exception, never reviewer pass or ledger repair.
- After triage, a well-formed fresh `finding-ledger` snapshot is required before live-review `approved` or `passed`; current fails require exact-source nonblocking dispositions, and accepted-unresolved findings remain blocking. The provider does not judge statements, reasons, dispositions, owners, quiet/progress/thrash, or provenance.
- Confirmation consumes the durable finding-ledger set and does not search again except for fix-introduced holes. Bound workers do not use previously overlooked after that state's first comprehensive review of the subject; humans still may with full failure burden.
- Late findings remain actionable when they provide current evidence, violated obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked; timing, prior visibility, or reviewer overlook does not waive materiality. Comprehensive-first review and scope/materiality burdens still bar drip-feeding and unrelated reopening.

`retired-author` requires a nonempty reason and ordered per-gate `reviewer-manifest` snapshots (`{gate,authors:[{name,kind}],reason}`) showing the source author present then absent. Retired authors do not count; replacements must meet the unchanged independent-author floor. Rejected/advisory/retired findings have null owner, empty routes and recorded/stale status; advisory/stale alone never discharges a current raw fail.

Inspect later steering with `loop-engine --json show RUN --view full | software-change commission --slot SLOT [--task TASK]`. The required engine companion defines user-steering targets, supersession, proof_updates and steering-incorporation. Configure the shared commission filter for bound selection; task-only instructions do not reach dependants or summarizer, and launched attempts keep their original commission.

## Production proof boundary

Use `scripts/software-change-journey.py` for repository and archive checks. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. Source `full` mode drives separate Loop Engine processes across provider TOML, SQLite, production provider, shipped high-rigor artifacts, deterministic denials, evidence aggregation, and terminal state. After the high-rigor run reaches `end`, it starts a second run from shipped `minimal.json` and walks the stitched hops (empty review lists omitted, last-hop `passed`). Packaged `checked-prefix` mode starts extracted binaries, materializes embedded data with `data-dump`, and runs one checked transition from that dump. Synthetic pass records prove schema/evidence shape, independence, routing, aggregation, and persistence only; they are not semantic review judgments.
