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

`software-change` is Loop Engine's reference provider, distributed standalone with its shipped data embedded (`software-change data-dump DIR` materializes it); a repo checkout remains the development path. The live graph is the union of five draft rooms plus parent and adversarial review rooms when those policy lists are nonempty; `describe` omits empty lists. Draft ready events check schema and revision links only. Parent semantic axes live on the parent review state; adversarial axes live on the distinct adversarial-review state. Validation-report-local corrections stay in the validation draft after edit/recheck for the next checked hop; the validation draft also exposes check-free `revise-implementation` when validation exposes a repository-state mismatch. Nearest `revise` from validation-review and validation-adversarial-review returns to that draft, and those states also expose `revise-implementation`. Phase-named owning routes (`revise-intent`, `revise-design`, `revise-plan`) handle upstream defects from review states and directly from the implement draft in newly described graphs. Reviewer convergence contract requires candidate triage before append or mutation, focused external reconsideration for disputed candidates, comprehensive first review, and bounded confirmation review. Quiet, progress, and thrash count per review state on the post-triage accepted-unresolved finding-ledger set. Confirmation consumes the durable set and does not search again except for fix-introduced holes. Bound workers do not use previously overlooked after that state's first comprehensive review of the subject; humans still may with full failure burden. Review packets carry the current ledger snapshot in immutable context; the frozen worker assignment identifies ordered axes, and the reviewer inspects entries whose `review_axes` include each assigned axis without changing policy or verdict authority. Known accepted material defects are never waived. Adversarial output is candidate data under the YAGNI/pragmatic append bar: extra mechanism, unlisted requirements, and hypothetical-future fails are not appended. Late findings still require current evidence, violated obligation, consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); prior visibility or overlook does not waive known material defects, while comprehensive-first and scope/materiality burdens block drip-feeding or unrelated reopening. Human-facing challenge review must meaningfully falsify a parent pass claim against frozen intent, not manufacture a second opinion.

The provider is deterministic only: it validates artifact schemas and revision links, then aggregates externally supplied review evidence. `describe` and `evaluate` never generate prompts, invoke a model, or judge findings. Bound workers, when frozen, are started by `loop-engine invoke`; you still triage outputs and append verdicts. Per-run obligations are frozen in immutable `initial_input`, and every later actor must inspect the run's frozen `intent.json` operating context rather than relying on chat memory or a profile default.

The criterion spine is run-local and intentionally small. Write each current intent acceptance entry as the closed `{id, statement}` record with an `AC-N` ID matching `^AC-[1-9][0-9]*$`; preserve an ID when its meaning is materially unchanged and use a new ID for replacement. Design coverage, plan tasks, implementation validation rows, and validation proof rows may carry optional `criterion_id` or `criterion_ids` references. Design/plan/implementation references remain optional and are checked for syntax, local duplicates and current membership. Contract v2 final validation instead requires complete criterion/goal coverage through the fixed index described below; the provider checks relationships, never semantic sufficiency.

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

Bound fan-out delivers compact context by default for all workers, with or without preamble: only each selected record's engine-owned top-level `data.loop_engine_origin` is omitted from a clone. Record IDs/order, concise origin, judgments, meaningful fields, assignment and controls remain; no model classification, opt-in flag or recursive stripping applies. Actual captured stdin is the truthful delivered input, not a full verification snapshot. The original selected full context is independently retained in per-worker `fan-out-spec.json` `routed_inputs` and summary routing. `capture_format: "bound-context-projection-v1"` requires that full snapshot for captured-commission verification; missing/malformed snapshots refuse. Unmarked legacy captures keep stdin-based verification. Core/provider history and existing selected-output, digest, assignment, attempt and applicability checks remain full-strength. Raw ad-hoc fan-out instruction bytes are unchanged. Graph task/findings/steering and summarizer packets remain their separate existing projection; no-task `ad-hoc-repair` still uses its closed repair packet and worker-owned report, not a graph summarizer.

The driver owns semantic disposition and Git lifecycle. Reviewer output is candidate evidence only. Preserve raw stdout/stderr, append an advisory `advisory-finding-proposal` only as an inert suggestion, and append the driver-edited `finding-ledger` snapshot that alone controls gate agreement and exact implementation routing. Do not let a proposal, report claim, or passing worker exit substitute for a driver decision.

The existing review-axis IDs remain unchanged. High-rigor `plan-review` uses exactly `task-sized`, `context-sufficient`, `done-observable`, `decision-free`, `design-faithful`, and `dependencies-honest`; validation uses exactly `intent-delivered`, `docs-integrated`, and `requirement-proof-mapping`. Standard uses its existing intent, design, and validation IDs; minimal uses only `intent-delivered` on validation. Do not add an axis to satisfy a finding.

Run checked implementation/validation evaluation from the intended repository checkout, not the artifact directory: verification resolves the repository from the provider's current working directory. The artifact root locates reports, not the evaluation repository. Checkpoint creation's explicit `--working-directory` does not change later evaluation CWD.

For implementation and validation, the report is not proof by itself. Run `software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS` after the report is complete. Both directories must already exist and be absolute. The command only reads repository state and writes the phase checkpoint; it never stages, commits, branches, pushes, creates, selects, merges, cleans, manages, or suggests worktrees. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, with or without implementation review. Validation requires the sole history entry for the current report revision to match the current report, document revisions, and repository state. Later context, regenerated mutable checkpoints, or overwritten history bytes cannot admit different bytes. If a checked event reports a stale checkpoint, use `revise-implementation` where shown, regenerate implementation report/checkpoint and validation report/checkpoint for the same current tree, then append fresh review evidence and ledger state before retrying. Validation cannot replace implementation proof.

## Contract v2, simplicity and final criterion proof

Shipped versions are `minimal-9`, `standard-9`, `high-rigor-9` with `contract_version: 2` and `criterion_policy: {required_authors:1,goal_required_authors:1}` independent of review-axis floors. Profiles remain unbound. Unsupported older semantic profiles refuse explicitly; keep their fixed original providers for execution. Bare describe is discovery; provider-free show/history preserves original records with no implied ownership/control capability. Do not migrate active bootstrap input, topology or bindings by catalog edits.

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

Before start, load repository `skills/using-loop-engine/SKILL.md`, **Deterministic setup**, and follow its catalog/override inspection procedure. Use the normal user catalog; only an explicit human isolation request in this session authorizes database or artifact overrides. Other runs and old preferences do not authorize isolation.

```sh
cargo build -p loop-cli -p software-change-provider
```

Register `target/debug/software-change` under alias `software-change` (absolute `command` path) in an uncommitted machine-local `providers.toml`.

Pick a profile from `crates/software-change-provider/data/configs/` — `minimal.json` (validation-review only, no challenge lists), `standard.json` (intent-review, design-review, validation-review plus 1:1 challenge counterparts), or `high-rigor.json` (all parent review gates plus 1:1 challenge counterparts; two distinct reviewers on design-review and validation-review parent axes). Copy it to a run-specific file. Shipped profiles omit `work_slot_bindings`. Do not `start` that copy until the user has approved the work-slot policy below. Follow the required canonical setup above for normal-catalog start and owner-only isolation. The engine allocates the durable directory and records that absolute path in object `initial_input` (`show` reveals it). `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers.

## Exact profile and external-fleet preflight

Treat the selected profile and an external role-to-model manifest as two separate authorities. The profile is authoritative for profile-derived obligations; the manifest is authoritative only for unbound external launches and is never merged into `work_slot_bindings`.

Before `start`, set a per-run `PROFILE_NAME` label and display facts derived from the exact profile file that will be passed with `@PROFILE`, not from a shipped default:

```sh
set -eu
: "${PROFILE:?absolute per-run profile path}"
: "${PROFILE_NAME:?owner-facing profile label}"
PROFILE_SHA256=$(shasum -a 256 "$PROFILE" | awk '{print $1}')
printf 'PROFILE_NAME=%s\n' "$PROFILE_NAME"
jq --arg name "$PROFILE_NAME" '{profile_name:$name, config_version, contract_version, criterion_policy, live_review_states: [(.review_policies // {} | to_entries[] | select((.value | type) == "array" and (.value | length) > 0) | {id:.key, axes:[.value[] | {id, required_authors:(.required_authors // 1)}]})], bookends_enabled:(.extra.bookends.enabled // false), work_slot_bindings:(.work_slot_bindings // {})}' "$PROFILE"
printf 'PROFILE_SHA256=%s\nExact resulting PROFILE bytes:\n' "$PROFILE_SHA256"
cat "$PROFILE"
```

The summary must show every live review state, axis ID, normalized `required_authors` (missing means one), Bookends enabled/disabled state, exact sparse binding map, and any model values frozen in nested binding arguments. Run `preview-bindings` on that same binding map. After any constructor rewrite, repeat the summary, exact-byte display, preview, and hash; obtain explicit owner confirmation of the profile label, derived facts, bytes, hash, and bindings. Rehash this same file immediately before `start` and abort on any mismatch. A profile hash does not cover assignments absent from its bindings.

For unbound work, create a separate owner-confirmed manifest containing only roles and exact model IDs, for example:

```json
{"analyst":"MODEL_ID","writer":"MODEL_ID","ordinary-reviewer":"MODEL_ID","challenge-reviewer":"MODEL_ID"}
```

Keep the manifest outside the profile, print its exact bytes and SHA-256, and obtain a separate owner confirmation. For every role actually launched, run `pi --list-models`, require the exact manifest ID to be present, pass that exact ID in the launch's model argument, and preserve launch evidence containing the role, manifest hash, requested model, and actual model argument. If the requested model is unavailable or the actual launch differs, stop and obtain a new confirmation; never fall back, substitute, or silently use a CLI default. With an all-unbound profile, the driver performs every slot and this external manifest—not profile-derived bindings—governs those launches.

## Criterion spine and Bookends overlay

Shipped profiles leave Bookends off. With the overlay disabled, `AC-N` is the only criterion spine: do not add PRD disposition, candidate/liveness metadata, Bookends citations, or Green claims to authored artifacts. To opt in, copy one profile and set `extra.bookends.enabled` to JSON `true`; do not edit the shipped bytes. Overlay-on requires exactly one `prd_traceability` object on every current intent criterion, with one of:

- `linked-live`: nonempty `live_ids` containing only live repository `LE-N` IDs;
- `candidate`: one parser-valid `proposed_id` plus its `record_markdown`; or
- `not-applicable`: a nonempty reason about PRD traceability only.

Before requesting final approval with the Bookends overlay enabled, inspect `BOOKENDS_BYPASS` and unset an unintended value in the evaluation environment. A malformed nonempty value errors; a deliberate bypass is visible but cannot satisfy required final GREEN. Run evaluation from the intended repository checkout, not the artifact directory.

`not-applicable` never waives or fulfills the associated criterion. Downstream artifacts continue to use only optional current-intent `AC-N` references; they do not carry a parallel PRD-ID projection. Missing, malformed, duplicate, or tombstoned live IDs are denied mechanically. A current candidate can proceed through authoring but blocks final completion for that Bookends-enabled run until the owner accepts it into a committed PRD or honestly reclassifies it. At each durable `e2e/journey` or declared `contract` test boundary, the driver or bound worker cites the applicable live ID as `bookends:LE-<n>`; the driver triages the capture and appends the resulting evidence. The overlay adds only `ids-grounded` and validation `bypass-not-green`, calls the checker in-process without inventing a bypass (an externally supplied `BOOKENDS_BYPASS` is surfaced as `BYPASS`), and refuses validation `passed` on checker `RED` or `BYPASS`.

## Work-slot policy (confirm before start)

Cataloged slots: `intent-draft`, `intent-review`, `intent-adversarial-review`, `design-draft`, `design-review`, `design-adversarial-review`, `plan-draft`, `plan-review`, `plan-adversarial-review`, `implement`, `implementation-review`, `implementation-adversarial-review`, `validation-draft`, `validation-review`, `validation-adversarial-review`. End has no slot. Bindings are sparse and freeze at `start`. A binding whose slot is not in the snapshotted catalog fails at `start`.

Shipped profiles omit `work_slot_bindings` (or `{}`). Every slot stays driver-performed until the caller opts in. A review slot with no configured policy axes must not be bound. Copying a profile is not model lock-in. Skill and constructor do not emit draft bindings. `start` still accepts a hand-written draft binding.

Before `start`, show the exact selected-profile summary/bytes/hash and the expanded `preview-bindings` report, plus the separate external role-to-model manifest when any work is unbound. Recheck the profile hash immediately before using that same file. Wait for the owner to confirm all of these authorities:

1. **Profile:** label, `config_version`, every live review state/axis and normalized author count, Bookends state, sparse bindings, exact bytes, and SHA-256.
2. **Bound slots:** which sparse slot IDs, if any, and exact command/args including every nested worker.
3. **External fleet:** separate manifest bytes/hash and every exact role-to-model assignment for unbound launches.
4. **Models:** every model-bearing CLI receives the confirmed exact model, or an explicitly accepted unpinned default only where the owner chose that policy; no fallback or substitution.

### Deterministic review-binding constructor

Do not hand-author one generic worker. The executable constructor below accepts the same per-run `PROFILE` that will be frozen, one live review `SLOT_ID`, and an ordered caller-confirmed `ROSTER` JSON file. `ROSTER` must be a non-empty array of exact `{ "author": "...", "model": "..." }` objects with pairwise-distinct, non-empty author labels and non-empty models. Supported slots are every live review slot: `intent-review`, `validation-review`, the other parent review slots, and all challenge slots (the unchanged `*-adversarial-review` IDs). Skill and constructor do not emit draft bindings (`intent-draft`, `design-draft`, `plan-draft`, `implement`, `validation-draft`). `start` still accepts a hand-written draft binding. Use the same roster file for parent and challenge slots; challenge identity has no disjoint-author floor and there is no second roster file. Bind parent and challenge as separate slots; same-slot mixed parent and challenge fan-out is not the enabled path.

Set every path explicitly. `DATA_ROOT` is either the checkout root or a root populated by `software-change data-dump`. `ENGINE`, `SOFTWARE_CHANGE_COMMAND` and `PI_COMMAND` become frozen command bytes. The binding uses the provider's shared `commission` context filter to select applicable steering while retaining the review/applicability source records. Review workers keep `--no-skills --no-extensions`, load only the two explicit extensions, include `--tools read,grep,find,ls`, and do not pass `--no-context-files`.

```sh
set -eu
: "${PROFILE:?path to the per-run profile being frozen}"
: "${SLOT_ID:?live review slot id}"
: "${ROSTER:?path to ordered caller-confirmed roster JSON}"
: "${DATA_ROOT:?checkout root or data-dump root}"
: "${ENGINE:?absolute loop-engine command}"
: "${SOFTWARE_CHANGE_COMMAND:?absolute software-change command for commission selection}"
: "${PI_COMMAND:?absolute pi command}"
: "${CURSOR_EXTENSION_PATH:?absolute cursor-provider extension path}"
: "${CLAUDE_BRIDGE_EXTENSION_PATH:?absolute claude-bridge extension path}"

case "$SLOT_ID" in
  intent-review|intent-adversarial-review|design-review|design-adversarial-review|plan-review|plan-adversarial-review|implementation-review|implementation-adversarial-review|validation-review|validation-adversarial-review) ;;
  intent-draft|design-draft|plan-draft|implement|validation-draft)
    printf 'constructor does not emit draft bindings: %s\n' "$SLOT_ID" >&2; exit 1 ;;
  *) printf 'unsupported review SLOT_ID: %s\n' "$SLOT_ID" >&2; exit 1 ;;
esac

PREAMBLE_FILE="$DATA_ROOT/crates/software-change-provider/data/review-worker-preamble.txt"
SCHEMA_FILE="$DATA_ROOT/crates/software-change-provider/data/review-worker-output-schema.json"
for required_file in "$PROFILE" "$ROSTER" "$PREAMBLE_FILE" "$SCHEMA_FILE"; do
  test -f "$required_file" || { printf 'missing required file: %s\n' "$required_file" >&2; exit 1; }
done

TMP_PROFILE=$(mktemp "$(dirname "$PROFILE")/.software-change-profile.XXXXXX")
trap 'rm -f "$TMP_PROFILE"' EXIT HUP INT TERM
jq \
  --arg slot "$SLOT_ID" \
  --arg engine "$ENGINE" \
  --arg provider "$SOFTWARE_CHANGE_COMMAND" \
  --arg pi "$PI_COMMAND" \
  --arg cursor "$CURSOR_EXTENSION_PATH" \
  --arg bridge "$CLAUDE_BRIDGE_EXTENSION_PATH" \
  --arg separate_axes_reason "${SEPARATE_AXES_REASON:-}" \
  --rawfile base_preamble "$PREAMBLE_FILE" \
  --slurpfile output_schema "$SCHEMA_FILE" \
  --slurpfile roster "$ROSTER" '
  def required_author_count:
    if has("required_authors") then .required_authors else 1 end;
  . as $profile
  | ($roster[0]) as $entries
  | if ($separate_axes_reason != "" and ($separate_axes_reason | test("\\S") | not)) then
      error("SEPARATE_AXES_REASON must explain the one-axis commission")
    elif (($entries | type) != "array" or ($entries | length) == 0) then
      error("ROSTER must be a non-empty array")
    elif (all($entries[]; type == "object") | not) then
      error("every ROSTER entry must be an object")
    elif (all($entries[]; ((keys | sort) == ["author", "model"])) | not) then
      error("every ROSTER entry must contain exactly author and model")
    elif (all($entries[]; ((.author | type) == "string" and (.author | length) > 0 and (.model | type) == "string" and (.model | length) > 0)) | not) then
      error("ROSTER author and model values must be non-empty strings")
    elif (($entries | map(.author) | unique | length) != ($entries | length)) then
      error("ROSTER author labels must be pairwise distinct")
    elif (($output_schema | length) != 1 or $output_schema[0].required != ["author", "judgments"] or ($output_schema[0].properties.judgments.items.oneOf | length) != 2) then
      error("provider review-worker complete output schema is missing or unsupported")
    elif (($profile.work_slot_bindings // {} | type) != "object") then
      error("PROFILE work_slot_bindings must be absent or an object")
    elif (
      $slot == "intent-draft" or $slot == "design-draft" or $slot == "plan-draft"
      or $slot == "implement" or $slot == "validation-draft"
    ) then
      error("constructor does not emit draft bindings")
    elif (([
        "intent-review","intent-adversarial-review",
        "design-review","design-adversarial-review",
        "plan-review","plan-adversarial-review",
        "implementation-review","implementation-adversarial-review",
        "validation-review","validation-adversarial-review"
      ] | index($slot)) == null) then
      error("unsupported review slot")
    else . end
  | ($profile.review_policies[$slot]) as $policies
  | if (($policies | type) != "array" or ($policies | length) == 0) then
      error("selected review slot has an unsupported or empty policy list")
    elif (all($policies[]; (type == "object" and (.id | type) == "string" and (.id | length) > 0)) | not) then
      error("every selected policy must have a non-empty id")
    elif (($policies | map(.id) | unique | length) != ($policies | length)) then
      error("selected policy IDs must be unique")
    elif (all($policies[]; ((.example_prompt | type) == "string" and (.example_prompt | length) > 0)) | not) then
      error("every selected policy must have a non-empty example_prompt")
    elif (all($policies[]; ((required_author_count | type) == "number" and (required_author_count | floor) == required_author_count and required_author_count > 0)) | not) then
      error("required_authors must normalize to a positive integer")
    elif (([$policies[] | required_author_count] | max) > ($entries | length)) then
      error("ROSTER has too few entries for selected required_authors")
    else . end
  | [
      range(0; ($entries | length)) as $roster_index
      | $entries[$roster_index] as $entry
      | [$policies[] | select(required_author_count > $roster_index)] as $author_policies
      | (if $separate_axes_reason == "" then [$author_policies] else [$author_policies[] | [.]] end)[] as $assigned
      | select(($assigned | length) > 0)
      | {
          command: $pi,
          args: [
            "--print", "--no-skills", "--no-extensions",
            "-e", $cursor, "-e", $bridge,
            "--tools", "read,grep,find,ls",
            "--model", $entry.model
          ],
          preamble: (
            $base_preamble
            + "FROZEN REVIEW ASSIGNMENT\n"
            + "provider: software-change\n"
            + "slot_id: " + $slot + "\n"
            + "assigned_policies: " + ($assigned | tojson) + "\n"
            + "required_author_claim: " + $entry.author + "\n"
            + "separate_axes_reason: " + $separate_axes_reason + "\n"
            + (if $profile.contract_version == 2 and $slot == "validation-review" then
                "criterion_author_number: " + (($roster_index + 1) | tostring) + "\nRead the frozen validation-report index and checkpoint. Return assigned criterion/goal judgments in validation_verdicts: [{record_id,kind,data}], using the prechosen IDs for your author number under criterion_policy. Do not create placeholders, edit the index, or run commands. Consume retained command evidence. For focused repair, leave unaffected applicability rows alone; judge affected rows freshly. Axis judgments consume this collection.\n"
              elif $slot == "validation-adversarial-review" then "Consume the existing criterion/goal collection; do not commission it again or run proof commands.\n" else "" end)
          ),
          full_output_schema: (
            $output_schema[0]
            | (if $profile.contract_version == 2 and $slot == "validation-review" then
                .properties.validation_verdicts = {type:"array", items:{type:"object", additionalProperties:false, required:["record_id","kind","data"], properties:{record_id:{type:"string",minLength:1},kind:{type:"string",enum:["criterion-verdict","goal-verdict"]},data:{type:"object"}}}}
              else . end)
            | .properties.author.const = {name: $entry.author, kind: "agent"}
            | .properties.judgments.minItems = ($assigned | length)
            | .properties.judgments.maxItems = ($assigned | length)
            | .properties.judgments.items.oneOf[].properties.axis.enum = [$assigned[].id]
            | .properties.judgments.allOf = [$assigned[] | {contains: {type: "object", required: ["axis"], properties: {axis: {const: .id}}}}]
          )
        }
    ] as $workers
  | (reduce $workers[] as $worker
      (["fan-out"]; . + ["--worker", ($worker | tojson)])) as $fan_out_args
  | .work_slot_bindings = (.work_slot_bindings // {})
  | .work_slot_bindings[$slot] = {command: $engine, args: $fan_out_args, context_filter: {command: $provider, args: ["commission"]}}
' "$PROFILE" >"$TMP_PROFILE"
jq -e . "$TMP_PROFILE" >/dev/null
mv "$TMP_PROFILE" "$PROFILE"
trap - EXIT HUP INT TERM

profile_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
PROFILE_SHA256=$(profile_sha256 "$PROFILE")
printf '%s\n' 'Exact resulting PROFILE bytes:'
cat "$PROFILE"
printf '\n'
printf '%s\n' 'Resulting work_slot_bindings:'
jq .work_slot_bindings "$PROFILE"
printf 'PROFILE_SHA256=%s\n' "$PROFILE_SHA256"

BINDINGS_PREVIEW=$(mktemp "${TMPDIR:-/tmp}/software-change-bindings.XXXXXX")
trap 'rm -f "$BINDINGS_PREVIEW"' EXIT HUP INT TERM
jq -e .work_slot_bindings "$PROFILE" >"$BINDINGS_PREVIEW"
"$ENGINE" preview-bindings "@$BINDINGS_PREVIEW"
rm -f "$BINDINGS_PREVIEW"
trap - EXIT HUP INT TERM
printf 'Confirm this resulting profile, bindings, models, and SHA-256 before start.\n'
```

The constructor preserves other sparse bindings. It allocates each policy to its first normalized `required_authors // 1` roster entries, emitting one batch per used author in roster order with exact profile axis order/prompts, author/model and complete output schema. There is no batching policy knob; parent and challenge gates remain separate.

A nonconforming first output gets one identical-command/model schema correction with exact errors; both raw attempts remain under `attempts/` and `attempts.json`. Exhaustion fails closed. Mixed pass/fail output is valid, not approval. The engine inserts `{artifact_root, context}` (and requested force-fresh controls) into the frozen preamble. The schema's `x-loop-engine-force-fresh` constraint makes reuse rows fail conformance on force-fresh invocations, with the same single correction and preserved attempts. Process/schema conformance does not verify semantic applicability: provider candidate/evidence checks additionally reject unauthorized or stale reuse against the captured commission.

For confirmation, append reasoned `evidence-applicability` records for unaffected original per-axis evidence before invocation. Retain the frozen full axis set: affected rows are fresh, unaffected rows name those exact applicability IDs. No binding amendment is needed. `review-candidates` emits inert fresh per-axis candidates sharing one invocation/assignment origin and separately labeled carried references. Triage before append; copy a fresh candidate into per-axis review-evidence with its policy_id and current target, never fabricate per-axis invocation IDs or append a reuse row as a new verdict. A failed required axis still needs its exact-source driver disposition.

When evidence size, specialization or observed review failure justifies separate one-axis commissions, set `SEPARATE_AXES_REASON` to that nonempty reason before running this same constructor. It splits each used author's assigned policy list into singleton workers, freezing singleton min/maxItems, axis enums and contains list while retaining first-N allocation, author/model and exact prompt. Record the reason before owner confirmation; do not change the selected profile's policies or required author counts. This is an explicit binding construction choice, not a new profile setting.

After the caller confirms, copy the displayed hash into `CONFIRMED_PROFILE_SHA256`. Immediately before `start`, fail if the same profile's bytes changed, then start that unchanged file:

```sh
set -eu
: "${CONFIRMED_PROFILE_SHA256:?exact SHA-256 confirmed by the caller}"
: "${PROVIDER_CONFIG:?absolute provider TOML path}"
: "${LABEL:?run label}"
if command -v sha256sum >/dev/null 2>&1; then
  CURRENT_PROFILE_SHA256=$(sha256sum "$PROFILE" | awk '{print $1}')
else
  CURRENT_PROFILE_SHA256=$(shasum -a 256 "$PROFILE" | awk '{print $1}')
fi
test "$CURRENT_PROFILE_SHA256" = "$CONFIRMED_PROFILE_SHA256" || {
  printf 'PROFILE changed after confirmation: expected %s, got %s\n' \
    "$CONFIRMED_PROFILE_SHA256" "$CURRENT_PROFILE_SHA256" >&2
  exit 1
}
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@$PROFILE" "$LABEL"
```

The implement binding remains a separate opt-in `run-plan-graph --working-directory ABS --task-worker` pattern. Leave task selection out of frozen argv. Omit invoke input for the full plan. When an existing frozen task owns a correction, use `loop-engine invoke RUN_ID implement --input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'`; the provider requires that closed shape, current revision, unique known roots, and same-revision standing results for every prerequisite outside the roots-plus-dependants selection. Only when a current accepted unresolved implementation finding has no honest frozen task owner, leave its `task_ids` empty and invoke the same binding with exact `--input '{"repair_finding_ids":["FINDING_ID"]}'`. Repair requires unique current forwarded ledger entries that are accepted, unresolved, implementation-owned, current for the verified implementation checkpoint, and no-task-routed. Malformed, empty, unknown, stale, wrong-owner/status/disposition, task-routed, or absent-context requests refuse before Dagu resolution or mutation. Packet input combined with frozen `--task`/`--tasks` also refuses.

A valid repair runs one `ad-hoc-repair` assignment under the frozen task worker and checkout. Its compact packet contains the exact finding objects, frozen plan revision, provider-derived pre-report and repository-state identity, and the narrow report-writing obligation. It runs no plan task or summarizer and does not alter `plan-task-results.json`. The provider accepts the result only when `implementation-report.json` is schema-valid, links the frozen plan, and uses a revision absent from the pre-proof and all accepted implementation-proof history; only then does it create a new checkpoint. Inspect `summary.json` generic worker/output/routed-input data and `repair` pre/post metadata. A failed worker or report creates no post checkpoint but may leave partial checkout edits; restore or deliberately incorporate them before another mutation. After success, append a later ledger snapshot resolving the finding and reconfirm affected independent implementation review and validation. There is no direct unbound repair form.

Direct callers may still select roots with repeated `--task ID` or one `--tasks ID,ID,...`; omitted selection remains full execution. The binding is not produced by the review constructor, must freeze one existing absolute directory selected and maintained by the driver, must not pass `--no-context-files`, and must freeze its model before start. Task mode projects only current accepted unresolved findings with `owner_phase: implementation` and an exact matching `task_ids` entry under that task's `finding_context`; stale, resolved, rejected, advisory, and unrelated entries are absent. Omitted, relative, nonexistent, and non-directory working directories are rejected before workers; the same graph-level cwd reaches every plan task and summarizer or the repair worker. Successful execution requires that directory to be a Git working tree for checkpoint generation, and the provider does not create, discover, select, reuse, merge, clean, manage, or suggest worktrees. Optional `--max-active N` may live in that frozen argv (omitted stays 4 ordinary plan tasks; set N is at most N ordinary plan tasks). Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only.

On the bound implement path, invoke persists the exact optional `invocation_input` on the invocation view and also supplies generic `standing_assignment_ids` from the same provider-free `show` projection. The graph treats a recorded prerequisite as standing only when its sidecar revision/success agrees and its assignment ID is in that engine list; graph-local exit 0 alone cannot admit stale work. A driver who wants to reuse a non-standing task must use the appropriate check-free correction route and then provide fresh proof. Direct ad-hoc `run-plan-graph` without an engine packet retains its sidecar-only contract.

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--working-directory",
    "/absolute/path/to/driver-selected-checkout",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--model\",\"MODEL\"]}"
  ]
}
```

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

Draft ready/passed events check schema and revision links; implementation and validation checked hops also require a provider-generated current repository checkpoint. Live review `approved` or `passed` rechecks the subject, validates that gate's `review-evidence`, and requires a well-formed current `finding-ledger` snapshot with fresh subject and checkpoint state. Its exact-source dispositions must discharge current failures; accepted-unresolved findings still block across revisions. Discharged fails count as independent judgments, not rewritten passes. The live last hop into `end` is `passed`; earlier live review hops are `approved`.

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
| `implementation-ready` (implement) | `implementation-report.json` | schema/links + current implementation checkpoint |
| `approved` / `passed` (`implementation-review`) | `implementation-report.json` | `implementation-review` |
| `approved` / `passed` (`implementation-adversarial-review`) | `implementation-report.json` | `implementation-adversarial-review` |
| `validation-ready` / `passed` (validation) | `validation-report.json` | schema/links + current validation checkpoint matching accepted `implementation-proof-history/` entry |
| `approved` / `passed` (`validation-review`) | `validation-report.json` | `validation-review` |
| `passed` (`validation-adversarial-review`) | `validation-report.json` | `validation-adversarial-review` |
| `revise-plan` / `revise-design` / `revise-intent` (implement) | — check-free, owned work must be quiescent | no implementation report required |
| `revise-implementation` (validation draft/review states) | — check-free | regenerate implementation, report, review, and proof |
| `revise` (any review state) | — check-free | — |

## Per-gate loop

1. Read action `show` for instructions, obligations, events and work locators; read full for frozen input, complete context and invocation/change reports. Action/full arms the visit: repeat after every transition before mutation. Read `artifact_root/intent.json` operating context before phase work; never infer missing legacy author counts as zero.
2. If **bound**, do not author the room yourself. `invoke` it (`loop-engine --json --timeout-ms N invoke RUN_ID SLOT_ID`; choose an allowance above the 30s default), then monitor until completion or attention. On overrun, wait or cancel owned work and verify cleanup, then observe before retry. Inspect `capture_dir/summary.json`, selected attempts and stdout before stderr; overlay succeeded is only bound CLI exit 0. For implement, follow the selection/repair procedure above: tasks and summarizer share the frozen checkout; full/selected mode's summarizer owns the report, while no-task repair's single worker owns it. Use `invocation-progress` only for targeted diagnosis.
3. If this state is **unbound**, author or revise the subject artifact in `artifact_root` using its template from `crates/software-change-provider/data/templates/`. Material content changes require a revision bump — a bump makes prior raw verdicts stale for coverage, but does not resolve accepted-unresolved ledger findings; keeping the revision asserts the edit was immaterial. Preserve the frozen operating context, stated outcomes, and outside obligations; do not turn accepted risks into waivers or add excluded hostile/multi-tenant requirements. Keep design/plan/implementation criterion references optional and use only current `AC-N` IDs. Final validation requires the fixed complete criterion/goal index, not a parallel PRD-ID spine. For unbound implementation or validation work, create the matching checkpoint only after the report is complete: `software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS`. Both directories must already exist and be absolute; the command is read-only with respect to Git.
4. For evidence gates, obtain the axis's configured `required_authors` count of distinct external judgments (default 1; high-rigor design-review and validation-review parent axes require 2; adversarial axes require 1): fresh context, not the artifact's author, each judging every assigned axis separately using its exact `example_prompt`. Default to one commission per used author per gate; focused confirmation carries only explicitly unaffected judgments. Follow `crates/software-change-provider/data/reviewer-protocol.md`. Reviewers must judge within frozen `operating_context`; do not append speculative hostile or multi-tenant demands outside `threat_boundary`, and do not treat `accepted_risks` as permission to waive outcomes or `outside_obligations`. For plan review, require affected user/operator paths, observable outcomes, pragmatic black-box proof or a concrete impracticality reason, sufficient context, and implementation freedom. For validation review, reject activity-only evidence and inspect every new or changed Bookends citation semantically rather than accepting its requirement token. Unbound: you commission those reviewers. Bound review: read the captures, then you still triage and append; `fan-out` does not write records. Preserve each axis's first-N author allocation when grouping frozen review `--worker` args; a multi-axis batch does not collapse distinct-author obligations. Adversarial output is candidate data; extra mechanism, unlisted requirements, and hypothetical-future fails are not appended.
5. Append one record per accepted axis judgment — after triaging against the frozen intent and operating context, `kind` is `review-evidence`, `data` is the eight-field object. All eight judgment fields are required; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` is non-empty on `fail`; and `config_version` must match the run's frozen config. For a bound worker, add only the concise origin reference:

```json
{
  "gate": "design-review",
  "policy_id": "intent-faithful",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "design.json",
  "subject_revision": "3",
  "config_version": "standard-9",
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
