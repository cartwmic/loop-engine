# Agent usage

## Public surface

Primary commands: `start`, `list`, `show`, `append`, `event`, `history`, `terminate`. Additional commands: targeted `read`, terminal `explore`, passive `monitor`, explicit `reclaim`. Monitoring observes durable state, not external execution. Providers use `describe` and `evaluate` transport, plus optional `capture` and `admit`.

Providers that opt in (software-change, research and policy-document) also answer an optional `capture` operation: when a new run leaves an artifact's owning phase forward, by checked approval or owner override, the engine commits a `retained-artifacts` context record with that artifact's exact bytes in the same transaction. Full `show` and `history` expose these records after the run's files are gone; each entry has `name`, `sha256`, `size` and either `content` (`encoding` `utf-8` or `base64`), `same_as` (the record holding identical earlier content) or `absent: true`. Drafts between exits are not retained, and runs started before this capability have none.

`reclaim` previews, without writing, the bulky legacy work-slot invocation payloads that earlier releases stored for final and terminated runs; `reclaim --apply` clears them and runs VACUUM so the catalog file shrinks. Active runs are untouched and cleaned runs stay listed and showable; other commands may briefly wait or get a retryable busy error during the shrink. It never runs automatically.

Use an explicit catalog `--config`, inspect unintended LOOP/XDG/home redirects, and keep artifacts outside the checkout. Never use a user catalog for tests. Load CLI `--help` for exact arguments. Mutations require a fresh action/full observation; compact/status views do not arm them.
## External-driver loop

The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. The driver may use any agents or harness.

1. Inspect action/full `show` and provider `commission`. Early commissions return `missing_inputs`; supply those inputs before treating a commission as ready.
2. Do the work externally. Reviewers return judgments; the driver runs deterministic checks and owns progression.
3. Submit external judgments through `review-candidates`, external `command_results` through `prepare-validation`, and accepted context through engine `append`.
4. Read action/full `show`, then send the checked `event`; re-read after each transition. A successful process exit alone does not prove the deliverable.

For new workflows advertising provider record admission, supported provider-owned append/intake records are checked before storage. Named rule refusals and unavailable/incompatible/malformed validator responses fail closed; refused writes leave context/history unchanged. Deterministic validity is not semantic approval or aggregate readiness. Started runs keep frozen capabilities/runtime; no migration or bypass is implied.

Preserve review groups, self-review declarations, author counts and amendments. Declare evidence applicability explicitly; check ordinary review clearance before challenge review. Respect declared source scopes and current source identity. Record residual risks honestly and cover every applicable sufficiency axis; zero axes do not require invented reviews. Reuse evidence only while its declared applicability remains true. Typed advice answers are externally authored context of kind `advice-answer`.
## Software-change steering records

The engine's generic context examples are opaque storage examples, not a
software-change payload schema. For kind `user-steering`, this provider requires
a closed object with `target` and nonblank `instruction`:

```json
{"target":{"kind":"all"},"instruction":"The owner's exact instruction."}
```

Retain owner words unchanged in `instruction`. Do not put `text`, `question`,
`answer_verbatim`, `author` or arbitrary authorization metadata alongside it.
Keep other metadata externally or under a distinct opaque context kind; storing
it supplies no steering selection, review verdict or owner approval.

Supported targets are `{"kind":"all"}`,
`{"kind":"slots","ids":["intent-review"]}`, or
`{"kind":"tasks","plan_revision":"1","ids":["task-1"]}`.
Slot/task IDs must actually exist in the frozen workflow/current plan; examples
do not create them. ID lists must be nonempty, unique and nonblank.

The only optional top-level fields are:

- `supersedes`: unique earlier steering record IDs. It retires valid instructions,
  not malformed records.
- `proof_updates`: objects with `proof_id`, nonblank `reason`, and optional
  `owner`, `command`, `args`. Name an existing proof command; supply command and
  args together. Updates do not create criteria or waive proof obligations.
- `intent_baseline`: a closed object with `revision`, `locator`, `sha256` and
  `json_bytes`. Retain the exact prior intent JSON bytes as the string value;
  its revision must match `revision`. Use its actual `sha256:<64 lowercase hex>`
  digest, including the prefix. This prior-intent source is distinct from the
  intent artifact's pre-change Git baseline.

Check the provider shape before append. On a release with complete provider
admission, an invalid steering digest, revision or earlier-record reference is
refused before storage; correct the named field and retry. Required validation
being unavailable or incompatible is a write failure, not unchecked acceptance.
A stored record still grants no semantic authority.
No exception stores a refused record. Incomplete source builds without admission
still do not establish this guarantee from a successful direct append.

Earlier frozen runtimes (including 0.24.1) parse steering before supersession;
a malformed retained payload can therefore block even after a corrected record.
These runs gain no new recovery by editing profiles or replacing executables.
Preserve the failure and obtain owner authorization for any termination/replacement;
never delete history, bypass storage rules or overwrite the frozen runtime.

## Running advice and append-only recovery

Use a release implementing these record kinds on new runs. Do not apply current
contracts to runs frozen on an earlier executable. Full `show` preserves the original
input and complete context; action/full show must precede mutation.

### Immediate owner advice changes

`show.result.advice_policy` is the effective setting, overriding start-time advice
flags or frozen action-guidance prose. It contains `frozen`, `change_record_ids`,
`effective` and `enabled`; unavailable is not an enabled/stale fallback. Owner
attribution is trusted, not authentication. Append this closed `advice-policy-change`
shape using the current observed visit and actual owner decision:

```json
{
  "actor": {"name": "owner", "kind": "human"},
  "reason": "Use this advice requirement for the pending departure.",
  "state_visit": 4,
  "advice_departures": {
    "version": 1,
    "occasions": [{"state": "explore", "event": "intent-ready", "occasion_id": "execution-or-authority-issue:explore:intent-ready"}]
  }
}
```

Copy real state/event identities from the stored workflow; examples create no edge.
The complete map replaces the previous effective map immediately, including the
pending departure. Empty `occasions` disables new due advice. Removing A releases
A's unanswered prerequisite without releasing retained B; adding a requirement
needs a current declaration. Frozen software-change occasion families still bound
`advice-request`. The original map and every owner change remain in history.

For `advice-occasion`, provide `source_state`, `state_visit`, `event`, `occasion_id`,
object `target`, `triggered`, `trigger_source_ids`, `response_ids` and `reason`.
A triggered declaration names unique earlier trigger sources and earlier typed
advice-answer IDs (possibly empty while unanswered), with no non-trigger reason.
`triggered:false` needs a nonblank no-admissible-question reason and empty source/
response lists; it is not fabricated advice. An enabled effective map checks these
fields at write. An accidental duplicate is refused naming the existing ID.
Correct either direction by adding `revises` naming the latest declaration for
that exact occasion/state/visit/event and a nonblank `revision_reason`; originals
stay visible. Ordinary driver correction needs no owner ceremony or fresh-visit
revise. Removed declarations are historical/inactive for closure.

Every successful received answer still needs a reasoned `advice-disposition`, even
after policy removal/disablement or occasion revision. Use `response_id`, `answer_id`,
`occasion_id`, `state_visit`, matching object `target`, `disposition` (`accept`,
`partial`, `reject`) and nonblank `reason`. Changed targets need explicit valid
`advice-applicability` or fresh advice. The existing scoped `--advice-exception`
only excuses precisely named unanswered due occasions, never missing answer
dispositions, ordinary gates, proof or authority. Advice remains non-authoritative.

### Obsolete evidence and proof additions

Append software-change `record-withdrawal` as the closed object
`{record_ids:["earlier-record-id"],reason:"why obsolete",attesting_driver:{name,kind}}`.
IDs must be nonempty, unique, earlier same-run records, not already withdrawn;
reason/driver name must be nonblank. Only `review-evidence`, `evidence-applicability`
and supplemental `validation-command` records qualify. A failing review must
already have an exact-source entry in the latest valid finding ledger for its
gate/subject. Withdrawal never dispositions an accepted-unresolved finding,
received advice answer or already-taken transition. Frozen plan proof commands
are not context records and cannot be withdrawn.

Valid reviewer retirement still requires a reasoned `reviewer-manifest` roster
change (prior presence, current absence) plus `retired-author` ledger disposition.
Re-added authors are not still retired. Withdrawn/retired records and carries of
excluded origins do not count or block later coverage, including malformed records;
configured replacement floors remain. A new carry of an excluded origin is invalid.
Keep originals for immutable ledger-source identity; do not erase pending findings.

A commission reports current malformed records in `record_diagnostics` with ID/
reason instead of supplying fabricated clearance or aborting on their citations.
Gates remain blocked until correction by later conforming evidence or withdrawal/
valid retirement plus replacement coverage. Ledger review-source checks retain
stored identity/shape rather than re-resolving current cited files; unreadable
identity fields still invalidate a snapshot. Later checks see exclusions, while
passed transition history stays unchanged. Current counted cold reconstructions,
including eligible carries under their original IDs, all require intent comparison;
withdrawn/retired/noncounting returns remain history, not comparison obligations.

Provider admission is a separate required boundary: on supporting releases these
provider records are refused at write when invalid; without that capability a
completed append is not proof of provider validity. Do not describe incomplete
admission wiring as passing every-write validation or use a bypass to store a refusal.

## Software-change recording metadata and sources

`review-candidates` retains the genuine request/original and derives one stable
assessment_id from exact original-return bytes plus assignment when absent. All fresh
axes from that return share it. Consistent explicit IDs are preserved; contradictory
batch/row/projection identities refuse. Do not fabricate per-axis execution origins.
`assessment_missing`, `assessment_inconsistent` and `mixed_authors` are distinct
metadata/stage causes, not a substitute for genuine configured author-floor shortages.
Incomplete groups and ineligible self-review still block; do not weaken frozen policy.

A finding-ledger entry may omit string statement to derive exact immutable source
findings; a supplied value must match exactly. Review text stays exact, criterion/goal
arrays retain the literal two-character backslash-plus-n separator, not a newline.
Optional summary is driver explanation outside stable identity, not reviewer text.
Optional report residual accepted_by is {"name":"actual accepting actor","kind":"human|agent|script"}
(choose one kind), not a scalar. The implementation report schema is checked at the
first report-bearing reconciliation-ready hop; implementation-ready remains report-free.

Review citations may use contained file pointers/line ranges or
`record:ACTUAL_CONTEXT_ID#JSON-Pointer` over the complete stored ContextRecord,
e.g. `record:OWNER_ID#/data/instruction`. Allowed kinds: review-evidence,
intent-comparison, user-steering, finding-ledger, evidence-applicability,
criterion-verdict, goal-verdict, intent-correction. IDs must resolve to actual earlier
same-run records and pointers must exist. No invented nested IDs or arbitrary kinds.
Commission's review_record_sources names real delivered IDs. Applicable non-intent
steering follows existing target/supersession selection; unrelated/superseded steering
is excluded. Reviewer truth remains external; citation availability is not support.

### Driver intent correction

For a correction within accepted intent, kind `intent-correction` accepts this closed
data object (replace every placeholder with retained facts):

```json
{
  "subject":"intent.json",
  "subject_revision":"CORRECTED_REVISION",
  "prior_intent":{
    "revision":"PRIOR_REVIEWED_REVISION",
    "locator":"ACTUAL_RETAINED_PRIOR_LOCATOR",
    "sha256":"sha256:ACTUAL_64_LOWERCASE_HEX",
    "json_bytes":"EXACT_PRIOR_UTF8_JSON_TEXT"
  },
  "findings":[{"ledger_id":"ACTUAL_LEDGER_ID","finding_id":"F-actual"}],
  "author":{"name":"actual driver","kind":"script"},
  "reason":"Driver's attributed correction reason."
}
```

Use distinct current/prior revisions, exact bytes/digest/revision, and accepted
intent-owned findings in the latest matching earlier intent-gate ledger backed by
actual prior-revision review sources. Unknown/missing/wrong-source/stale references
refuse. Known later reviewed revisions without qualified exact prior bytes refuse
intent-ready and commission before review. Real scope changes still require owner
steering/authority; correction classification is a cooperative driver declaration,
not a prose/diff oracle, approval or finding discharge. Ordinary/challenge/checkpoint
packets expose intent_revision_baseline's actual record/source/delivery status;
owner intent_revision_view shows exact prior-byte delta and separate actors.
`record:CORRECTION_ID#/data/prior_intent/json_bytes` is a supported citation.
Driver correction provenance never becomes owner words or approval. Existing frozen
workflow context declarations are not rewritten.

Implementation reviewers inspect the actual declared-baseline-to-covered-commit
change, not report claims alone. Necessary inputs and tools are driver-selected;
no tool ban, mandatory bundled diff or new judging axis. Supplied-only calibration
uses supplied companions only, never live lookup of fictional path labels.

Default generated implementation/validation checkpoint JSON is multiline two-space
pretty JSON, no added final newline, readable with ordinary line readers. Internal
content/report/state digest meaning is unchanged; physical file-byte SHA-256 changes
with whitespace. Accepted history binds and retains actual verified bytes/name,
including compact/manual encodings, without rewriting or duplicating equivalent proof.

## Provider tools

Use `describe`/`evaluate`, `checkpoint`, `commission`, `review-candidates`, `prepare-validation`, `source-identity`, `author-counts`, and `data-dump`. `setup` edits policy only with `--set`, `--add-axis`, `--remove-axis`; use explain/drift to inspect policy changes. Tools prepare or evaluate data, not external execution. Keep frozen run obligations and original runtime; no active-run migration is authorized.

## Software-change external command results

At validation, `prepare-validation` consumes one JSON packet containing a completed
full `show`, absolute `working_directory`, report `revision`, actual report
`author`, and `command_results`. It is inert; valid output is not execution,
review or permission to advance.

Each result row uses this layout. This is a field example, not execution
evidence: replace all placeholders and the illustrated status with retained facts.

```json
{
  "id": "ACTUAL_PROOF_COMMAND_ID",
  "command": "ACTUAL_COMMAND",
  "args": ["ACTUAL_ARGUMENT"],
  "cwd": "/absolute/actual/working-directory",
  "exit_code": 0,
  "stdout": "ACTUAL_RETAINED_STDOUT",
  "stderr": "ACTUAL_RETAINED_STDERR",
  "producer": {"name": "ACTUAL_EXTERNAL_PRODUCER", "kind": "agent"},
  "source": {"head": "ACTUAL_HEAD", "state_sha256": "ACTUAL_STATE_DIGEST"}
}
```

- `id` matches the effective frozen `ProofCommand.id`; `proof_id` alone does not
  select a row. Supply exactly one row per required ID.
- `command` and `args` exactly match that effective named command. `cwd` is an
  existing absolute directory; `exit_code` is the actual integer process status,
  including failure, never a manufactured zero.
- `producer.name` and `producer.kind` are nonblank actual external attribution.
  Neither may claim engine execution.
- `source` is the nested object holding the actual `head` and `state_sha256`
  returned by `source-identity`. A field named `source_identity` alone is not this
  input shape. Do not substitute the different native repository-checkpoint digest.
  Optional `source.scope` is a string.
- Retain output in string `stdout`/`stderr`, or use nonblank string
  `stdout_ref`/`stderr_ref` naming existing files. References may be absolute or
  relative to the row's `cwd`, not the helper's cwd. Supply at least one output
  field; preserve empty output when that is the actual recorded result.

Inspect `commands_complete` and diagnostics. Missing/duplicate IDs, failed
statuses and source mismatch are not passing proof. Keep original captures intact
when adapting serialization; fixing `id` or `source` nesting does not justify
rerunning applicable execution or inventing missing facts. Inspect candidates and
the draft before any append, and keep final report/checkpoint and independent
criterion/goal duties unchanged.

## Software-change narratives and checkpoints

These forms use current source `software-change` with `-16` profiles; setup refuses
historical shipped IDs through -15 and reports minimal-16, standard-16, high-rigor-16.
The installed 0.24.1 predecessor lacks these options; existing `-14` runs retain their
frozen graph, obligations and original runtime. Load the
[software-change skill](../crates/software-change-provider/skills/using-software-change-provider/SKILL.md#optional-narratives-and-owner-checkpoints)
for the full procedure before driving either option.

Both options default off in `minimal-16`, `standard-16` and `high-rigor-16`. Setup
accepts repeated boolean phase settings; `false` removes a selected phase:

```sh
software-change setup --rigor standard --decline-advice --output /absolute/profile.json --set narrative.design=true --set checkpoint.design=true
```

Narrative phases are `design`, `plan`, `validation`; checkpoint phases are `intent`,
`design`, `plan`, `implementation`, `validation`. Use `--rigor high` for high-rigor.
Setup writes `extra.narratives` / `extra.checkpoints` lists and materializes the enabled
narrative gates from `extra.narrative_review`. Inspect `profile.json.explain.json`:
`overrides` records settings, and `effective` exposes both lists and review policies.
Confirm exact bytes, stages, groups and floors before start. Source-file edits cannot
change started runs.

After the artifact's configured ordinary/challenge reviews, enabled phases expose
`<phase>-narrative`, `<phase>-narrative-review` and
`<phase>-narrative-adversarial-review`, followed by `<phase>-checkpoint` when selected.
Reviewless policies omit their review rooms. Use the events in action/full show:
`narrative-ready` admits a draft; review/checkpoint exits use `approved`, or `passed`
on the final hop. Draft/report and final proof duties remain on checked boundaries.
Implementation report admission occurs after reconciliation, at `reconciliation-ready`;
its earlier `implementation-ready` handoff remains report-free.

Write each phase's standalone `<phase>-narrative.md` and canonical
`<phase>-narrative.json` under the run's `artifact_root`. The authoritative artifacts
are `design.json`, `plan.json` and `validation-report.json`. Read the
[narrative guide](../crates/software-change-provider/data/templates/narrative.md) and
[binding schema](../crates/software-change-provider/data/narrative-schema.json).
The binding names the exact approved artifact revision/hash, actual approved context
ID and markdown filename/hash; its revision is `<artifact revision>/<positive counter>`.
Approval facts can be nested in an `artifact-admitted` or `reconciliation-decision`
append: reference the real enclosing record ID. JSON stays authoritative. Each admitted
artifact/narrative identity is single-use; approving exits require the latest admitted
revision and matching bytes. A rewrite needs a new identity and current review.

With completed full show saved outside the checkout:

```sh
software-change commission --slot design-narrative-review --stage aggregate < /absolute/full-show.json
software-change commission --slot design-checkpoint < /absolute/full-show.json
```

Narrative commissions expose `review_target`, `review_sources`, `approved_artifact`
and `missing_inputs`. Supply the exact markdown, binding, artifact and frozen intent;
supply missing inputs before commissioning. Follow `review-candidates` intake for
actual external returns and append accepted evidence plus a driver finding ledger.
The three axes are `narrative-faithful`, `narrative-intent-aligned` and
`narrative-understandable`. Read materialized policies/effective author counts:
minimal has one aggregate author, standard two, high-rigor two at individual and
aggregate stages. Preserve author-set and peer-isolation rules. Ordinary findings
clear before challenge tests the clean conclusion. Zero-review fixtures supply no
high-rigor semantic proof.

### Owner decision records

Checkpoint commission exposes `owner_approval_view` with the current subject,
revision and source identities. `owner_approval.status: not-performed` is the view's
initial status; ask the owner and record the answer. Use the narrative binding as
subject when enabled, otherwise the artifact. Intent and implementation always use
`intent.json` and `implementation-report.json`.

This illustrative approval data assumes the view names design narrative revision
`3/1`. Save the actual decision as `/absolute/owner-decision.json`, copying identities
from the current view and replacing the example text with the owner's verbatim words:

```json
{
  "checkpoint": "design-checkpoint",
  "subject": "design-narrative.json",
  "subject_revision": "3/1",
  "decision": "approve",
  "target": null,
  "reason": null,
  "text": "Approve this design narrative.",
  "actor": {"name": "owner", "kind": "human"}
}
```

The closed payload permits no extra fields. Approval may omit `target` and `reason`;
rejection requires both as nonblank strings. `text`, identity fields and `actor.name`
must be nonblank. Preserve text bytes without trimming or paraphrase. Actor kinds
`human`, `agent` and `script` are identity conventions; the provider authenticates no
human. This illustrative narrative rejection uses the same configured subject:

```json
{
  "checkpoint": "design-checkpoint",
  "subject": "design-narrative.json",
  "subject_revision": "3/1",
  "decision": "reject",
  "target": "narrative",
  "reason": "The narrative omits the design's stated limitation.",
  "text": "Reject the narrative: keep the stated limitation visible.",
  "actor": {"name": "owner", "kind": "human"}
}
```

For an isolated catalog authorized by the owner (replace absolute paths and `RUN`):

```sh
loop-engine --json --database /absolute/catalog.sqlite --config /absolute/providers.toml show RUN --view full
loop-engine --json --database /absolute/catalog.sqlite --config /absolute/providers.toml append --record-id owner-design-3-1 RUN owner-decision @/absolute/owner-decision.json
loop-engine --json --database /absolute/catalog.sqlite --config /absolute/providers.toml show RUN --view action
loop-engine --json --database /absolute/catalog.sqlite --config /absolute/providers.toml event RUN approved
```

Use a new record ID for each decision. After a rejection, replace the last event with
the matching route below. At the final validation checkpoint, approval uses `passed`.
Append retains the decision as context; the checked event evaluates it. Read history
with the same database/config to inspect retained decisions, text and route outcomes.

| Decision | Allowed checked checkpoint route | Required correction |
|---|---|---|
| approve exact current subject/revision | `approved` (final `passed`) | Current bytes and remaining phase/proof obligations |
| reject, `target: artifact` | `revise` | New artifact revision through its phase draft/reviews; new enabled narrative and decision |
| reject, `target: narrative` (enabled phases only) | `revise-narrative` | New composite narrative revision and ordinary/challenge reviews; unchanged artifact reviews remain valid |

An artifact rejection still names the checkpoint's current subject (the narrative
binding when enabled). The `target` selects what to repair. Its route returns to
`explore`, `design`, `plan`, `implement` or `validation`. Pre-artifact-rejection judgments
cannot regain applicability through a later declaration. Re-appraise eligible post-rejection
work or collect fresh reviews. Narrative draft/review also exposes `revise-<phase>` for
JSON defects, and narrative review exposes check-free `revise` for prose corrections.

The latest well-formed decision for the checkpoint across all revisions must match
this subject/revision. A newer decision for another revision leaves it pending;
malformed records are ignored with diagnostics. Older approval cannot revive after
rejection/rewrite. If files were edited early or made malformed, checkpoint rejection
routes still evaluate the recorded subject identity; take the matching route and repair
in the draft. Approving exits continue to refuse stale bytes/identities.

`intent-comparison` grants no checkpoint approval; checkpoint approval grants no intent
comparison. Overrides remain exceptional and supply no owner approval. The final
`passed` into end, including a reviewless narrative/checkpoint route, still checks the
complete validation index, criterion/whole-goal judgments, commands, current proof and
reconciliation/Bookends obligations where applicable. Preserve artifact/history evidence
and last reviewed markdown for each phase. Deleting the artifact directory loses its
narratives. Narrative review does not establish redaction or share safety.

## Workspace validation

From the checkout root, the final-proof owner runs `scripts/baseline.sh` plus the union of boundary commands in checkout-root `AGENTS.md`. Workers use `cargo test -p <crate> <filter>` for focused unit tests or `python3 scripts/run-central-tests.py --filter NAME` for focused central integration tests. Inspect active catalogs before rebuilding frozen provider paths; retain exact argv, source identity and outcomes. Missing required proof blocks completion.

Draft or audit README/AGENTS through policy-document under the root guide's document procedure; target edits require fresh digest-bound evidence. Release generation uses only `python3 scripts/generate-release.py` and read-only `python3 scripts/generate-release.py --check`, followed by the independent strict graph assertions in the root release procedure. Git, dispatch and publication remain separately owner-authorized.
