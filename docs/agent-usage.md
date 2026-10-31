# Agent usage

## Public surface

Primary commands: `start`, `list`, `show`, `append`, `event`, `history`, `terminate`. Additional commands: targeted `read`, terminal `explore`, passive `monitor`, explicit `reclaim`. Monitoring observes durable state, not external execution.

`list` accepts optional `--lifecycle active|final|terminated`, `--label SUBSTRING` and `--provider NAME` filters; with none, every run is listed. `read` kinds are `history`, `delta`, `context`, `record`, `assignment`, `error`, `attempt`, `stdout` and `stderr`. `read RUN --kind context [--record-kind KIND] [--cursor N] [--limit N]` pages headers of the run's context records (`id`, `kind`, `sequence`, `data_bytes` and top-level scalar fields, strings cut at 80 characters) without transcripts; `read RUN --kind record --id ID [--pointer POINTER]` returns that record exactly as full show lists it, or a JSON Pointer slice of it (for example `/data/findings`). `history` entries that add context carry `context_kind`. None of these reads arm mutation, and a closed pipe (`| head`) ends output quietly. Providers use `describe` and `evaluate` transport, plus optional `capture` and `admit`.

Providers that opt in (research and policy-document) also answer an optional `capture` operation: when a new run leaves an artifact's owning phase forward, by checked approval or owner override, the engine commits a `retained-artifacts` context record with that artifact's exact bytes in the same transaction. Full `show` and `history` expose these records after the run's files are gone; each entry has `name`, `sha256`, `size` and either `content` (`encoding` `utf-8` or `base64`), `same_as` (the record holding identical earlier content) or `absent: true`. Drafts between exits are not retained, and runs started before this capability have none.

`reclaim` previews, without writing, the bulky legacy work-slot invocation payloads that earlier releases stored for final and terminated runs; `reclaim --apply` clears them and runs VACUUM so the catalog file shrinks. Active runs are untouched and cleaned runs stay listed and showable; other commands may briefly wait or get a retryable busy error during the shrink. It never runs automatically.

Use an explicit catalog `--config`, inspect unintended LOOP/XDG/home redirects, and keep artifacts outside the checkout. Never use a user catalog for tests. Load CLI `--help` for exact arguments. Mutations require a fresh action/full observation; compact/status views do not arm them.
## External-driver loop

The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. The driver may use any agents or harness.

1. Inspect action/full `show` and provider `commission`. Early commissions return `missing_inputs`; supply those inputs before treating a commission as ready.
2. Do the work externally. Reviewers return judgments; the driver runs deterministic checks and owns progression.
3. Submit external judgments through the provider tools its skill names (for example research `review-candidates`) and accepted context through engine `append`.
4. Read action/full `show`, then send the checked `event`; re-read after each transition. A successful process exit alone does not prove the deliverable.

For new workflows advertising provider record admission, supported provider-owned append/intake records are checked before storage. Named rule refusals and unavailable/incompatible/malformed validator responses fail closed; refused writes leave context/history unchanged. Deterministic validity is not semantic approval or aggregate readiness. Started runs keep frozen capabilities/runtime; no migration or bypass is implied.

Preserve review groups, self-review declarations, author counts and amendments. Declare evidence applicability explicitly; check ordinary review clearance before challenge review. Respect declared source scopes and current source identity. Record residual risks honestly and cover every applicable sufficiency axis; zero axes do not require invented reviews. Reuse evidence only while its declared applicability remains true. Typed advice answers are externally authored context of kind `advice-answer`.
## Ending a run the owner has decided to stop

`terminate` takes only the run ID and records one history entry. It carries no reason and
does not verify the decision or any cited proof, so put the decision on the run first:

1. Observe with `show`. Confirm no owned work is live; cancel and verify cleanup first if any.
2. Append the owner's decision with the owner's exact words, in the record kind the
   provider's skill names for owner words (lean-change: `owner-decision`; otherwise an
   opaque kind such as `owner-stop` carrying the words and an actor).
3. Append one record of an opaque kind (for example `external-close-note`) authored by the
   driver. It names what the driver checked and cites it (merge commits, review verdicts,
   gate output, owner proofs), and lists what was not proven. It is driver provenance,
   not owner words, and grants no authority.
4. Observe again, then `terminate`.

The run ends `terminated`, never `final`, and nothing claims completion. The engine does
not check the cited records. Walk the remaining edges with `event --override` only if the
owner wants a completed-with-overrides outcome instead.

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
needs a current declaration. The original map and every owner change remain in history.

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

### Provider admission

Provider admission is a separate required boundary: on supporting releases these
provider records are refused at write when invalid; without that capability a
completed append is not proof of provider validity. Do not describe incomplete
admission wiring as passing every-write validation or use a bypass to store a refusal.

## Model bindings

The engine keeps a provider-neutral model-binding library: a run's frozen ~model_bindings~
input names a model and thinking level per role, ~model-binding-amendment~ records change
them with the owner's words, and ~session-declaration~ records state what actually ran.
Full and status ~show~ report the bindings as ~model_bindings~ (or state ~never-bound~).
No shipped provider currently binds models; the reference fixture
~tests/fixtures~ ~model-binding-provider~ proves the contract. Declared values are the
driver's statement, not a verified model identity.

## Provider tools

Each provider lists its own commands (`research --help`, and the provider `AGENTS.md`). Lean-change has `setup`, `verify`, `intent-packet`, `review-packet` and `outcome-map`; research has `commission`, `review-candidates`, `author-counts` and `data-dump`; policy-document is described in its operator guide. Tools prepare or evaluate data, not external execution. Keep frozen run obligations and original runtime; no active-run migration is authorized.

## Workspace validation

From the checkout root, the final-proof owner runs `scripts/baseline.sh` plus the union of boundary commands in checkout-root `AGENTS.md`. Workers use `cargo test -p <crate> <filter>` for focused unit tests or `python3 scripts/run-central-tests.py --filter NAME` for focused central integration tests. Inspect active catalogs before rebuilding frozen provider paths; retain exact argv, source identity and outcomes. Missing required proof blocks completion.

Draft or audit README/AGENTS through policy-document under the root guide's document procedure; target edits require fresh digest-bound evidence. Release generation uses only `python3 scripts/generate-release.py` and read-only `python3 scripts/generate-release.py --check`, followed by the independent strict graph assertions in the root release procedure. Git, dispatch and publication remain separately owner-authorized.
