# Agent usage

## Public surface

Primary commands: `start`, `list`, `show`, `append`, `event`, `history`, `terminate`. Additional commands: targeted `read`, terminal `explore`, passive `monitor`, explicit `reclaim`. Monitoring observes durable state, not external execution. Providers use `describe` and `evaluate` transport, plus optional `capture`.

Providers that opt in (software-change, research and policy-document) also answer an optional `capture` operation: when a new run leaves an artifact's owning phase forward, by checked approval or owner override, the engine commits a `retained-artifacts` context record with that artifact's exact bytes in the same transaction. Full `show` and `history` expose these records after the run's files are gone; each entry has `name`, `sha256`, `size` and either `content` (`encoding` `utf-8` or `base64`), `same_as` (the record holding identical earlier content) or `absent: true`. Drafts between exits are not retained, and runs started before this capability have none.

`reclaim` previews, without writing, the bulky legacy work-slot invocation payloads that earlier releases stored for final and terminated runs; `reclaim --apply` clears them and runs VACUUM so the catalog file shrinks. Active runs are untouched and cleaned runs stay listed and showable; other commands may briefly wait or get a retryable busy error during the shrink. It never runs automatically.

Use an explicit catalog `--config`, inspect unintended LOOP/XDG/home redirects, and keep artifacts outside the checkout. Never use a user catalog for tests. Load CLI `--help` for exact arguments. Mutations require a fresh action/full observation; compact/status views do not arm them.
## External-driver loop

The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. The driver may use any agents or harness.

1. Inspect action/full `show` and provider `commission`. Early commissions return `missing_inputs`; supply those inputs before treating a commission as ready.
2. Do the work externally. Reviewers return judgments; the driver runs deterministic checks and owns progression.
3. Submit external judgments through `review-candidates`, external `command_results` through `prepare-validation`, and accepted context through engine `append`.
4. Read action/full `show`, then send the checked `event`; re-read after each transition. A successful process exit alone does not prove the deliverable.

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

Check the provider shape before append. A completed engine append does not
validate it. In 0.24.1, commission and final validation parse every retained
steering record before supersession. A malformed payload or invalid baseline
therefore blocks the run even after a corrected superseding record. Preserve
the failure and stop; no documented append-only repair exists. Do not delete
history, bypass checks or replace a frozen executable. Obtain owner authorization
to preserve/terminate the affected run and prepare a correctly shaped replacement.

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

These forms require the current source `software-change` provider with `-15` profiles.
The installed 0.24.1 predecessor lacks these options; existing `-14` runs retain their
frozen graph, obligations and original runtime. Load the
[software-change skill](../crates/software-change-provider/skills/using-software-change-provider/SKILL.md#optional-narratives-and-owner-checkpoints)
for the full procedure before driving either option.

Both options default off in `minimal-15`, `standard-15` and `high-rigor-15`. Setup
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
