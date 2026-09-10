# Operational UX implementation contracts

Reviewed implementation contract; not requirement-ID authority or a claim that
all commands below are implemented. Consumer tasks own public-path proof.
No existing stored workflow, profile, evaluation or capture is rewritten.

## Observation (T02)

`loop-engine --json show RUN --view action|status|full` defaults to action.
`--compact` aliases status, including JSON. Action/full arm only after loading
current instructions. Status uses `Persistence.load_status_data`, never
`load_show_data`; unsupported adapters refuse, not fall back. SQLite uses a
consistent read transaction with no observation update. T02 owns CLI routing.

`State.action_guidance` is optional opaque JSON, omitted for legacy states.
Providers author normalized obligations and repair text at describe time. Core
must not parse author policy or replace bound-invoke instructions with worker
body text. `ShowProjection.action_guidance` preserves the value. Absent means
unknown normalized detail, not zero required authors. Full's additive
`evaluation_history` preserves every durable checked allow/deny in semantic
sequence order, including exact transition, feedback, sequence and occurred_at.
`latest_evaluations` keeps its existing reduction and identities. Legacy full
JSON decodes with an empty history; that does not establish completeness.
Overrides remain separate history, never synthetic allows.

## Capture (T03)

Reviewed forms:

```
loop-engine capture-matrix --matrix FILE --working-directory ABS --output-dir ABS [--resume]
loop-engine capture-command --working-directory ABS --output-dir ABS -- EXECUTABLE ARG...
loop-engine capture-abort --output-dir ABS
```

The [capture schema](operational-ux-capture.schema.json) defines matrix rows,
finished receipts and selection indexes (`$defs/receipt` and `$defs/index`).
Unique row IDs, path resolution, digest agreement and success/cleanup rules
require executor checks beyond JSON shape. `matrix_identity` binds the full
ordered matrix, cwd and effective settings; index `receipts` is the ordered
selection array. The CLI executes this contract directly; the public `capture` journey covers
execution, refusal, abort and resume.

Matrix JSON is `{ "rows": [...] }`. Each row has unique nonempty `id`,
nonempty `argv` string array, `environment` non-secret string map (default `{}`),
`inherit_environment` name array (default `[]`), positive `timeout_ms`, and
`obligations` string array. Cwd is the single explicit existing directory.
Capture-command normalizes one row (`id: "command"`) into this same executor,
not a second path. Its default timeout is 3600000ms; `--timeout-ms N` overrides
it and repeatable `--inherit-environment NAME` declares inherited settings.
Use a one-row matrix to supply a proof command's existing ID, overrides and
obligation labels. Single-command also accepts explicit `--resume`.
Environment identity includes explicit overrides and declared inherited values
(or explicit absence); do not capture arbitrary environment/secrets.

Each immutable attempt directory contains `started.json`, `receipt.json` after
termination, `stdout` and `stderr`. Started records include row ID, actual argv,
resolved executable, cwd, settings, repository_before and epoch started_at.
Finished receipts retain those fields and add repository_after, finished_at,
wall_seconds (monotonic), exit_code, signal, timed_out, spawn_error,
cleanup status and stream SHA-256 values. Unknown terminal facts are null,
never successful defaults. Incomplete started attempts are not receipts.
The atomically replaced `index.json` selects `{id, receipt}` in declared order
and retains matrix/cwd/settings identity. Attempts are never overwritten.
Resume verifies the complete successful contiguous prefix, stream bytes,
settings, current repository identity and terminal cleanup before any spawn.
Failure or incomplete cleanup stops later admission. Abort controls only owned
execution, never observer-selected runs. Supported cleanup hosts are macOS and
Linux. A private inherited marker and verified current ancestry locate ordinary
detached descendants; environment inventories are never persisted. Observed PIDs
are paired with C-locale, UTC `ps lstart` stamps and rechecked before ancestry
expansion or signaling. A bare remembered PID or process-group number is not
ownership authority. Already-observed descendants retain their association after
marker or parent changes; an unobserved descendant that both detaches and discards
its marker cannot be reliably rediscovered by this mechanism.

Start stamps have second resolution and checking before numeric signaling is not
atomic. This mitigates stale observations; it does not close the broader
PID-incarnation guarantee or cover same-second reuse. `pid-start-v1` ownership
snapshots are diagnostic, not an adoption protocol for interrupted captures.
Cleanup requires observed owned-process disappearance, including reaping, not
merely successful signals. A signal or identity-inspection error retains a failed
receipt and partial streams where possible, with cleanup pending—not approval.
An unavailable/lost controller or unverified cleanup remains pending and refuses
resume; `capture-abort` never guesses an acknowledgment. Inspect retained
`state.json`, `owned.json` and attempts before deciding how to resolve such a
blocked capture. No automatic retry or arbitrary PID cleanup is provided.

These commands are independent of run catalogs and provider evaluation. Put the
command first (no workflow-global options). Their stdout/stderr are flushed raw
child bytes, not a final JSON envelope; `index.json` is the machine-readable
selector. `state.json` exposes running/completed/failed, cleanup, selected_count
and row_count when a row finishes. An intermediate successful row is still
running. Receipt `aborted` and `capture_error` retain cancellation and stream
failures independently of the actual child exit. An abort can have child exit 0
and is still failed capture; the report reader preserves that distinction.

The existing report receipt contract in `implementation-report-proof.md` stays
unchanged. Its `argv` is the actual child proof command, not the enclosing
fan-out or validation-command argv. A report selector also carries plan_revision
and target_directory. Capture index and report selector are distinct formats;
an adapter validates and maps them, not renames one to the other. Native
provider evidence separately retains genuine fan-out invocation/assignment,
selected attempt/output identity and provider checkpoint identity. Neither an
outer exit nor a summary invents either receipt format.

## Supplemental validation (T06)

Context kind `validation-command` carries exactly the existing ProofCommand
fields `{id, command, args, owner, obligation}` (shared strict DTO in provider
`commission.rs`, reexported by `protocol.rs`). It is inert: no execution, acceptance criterion, waiver or
replacement. Preparation/evaluation consumers must reject blank fields,
duplicate supplemental IDs and collisions with frozen required command IDs.
`proof_updates` remains corrections for existing IDs, not additions. Missing,
failed or incomplete supplemental evidence cannot count as passed. This new
record is for candidate source behavior only; never amend the frozen production
profile or assume released provider support. T06 owns effective-collection
integration and unchanged released-provider/candidate-command interoperability.

`software-change prepare-validation` reads one JSON packet on stdin:
`{show, working_directory, revision, author, capture_indexes, execution_settings, additions}`.
`show` is the completed full validation observation; paths are absolute. Settings
contain positive `timeout_ms`, optional explicit `environment` and non-secret
`inherit_environment` names. `additions` is an array of the DTO above, and indexes
select real common-capture attempts beneath the run artifact root. Preparation
returns diagnostics, additive/command record candidates, a complete report draft,
and separate pending AC and goal positions with excluded report/implementation
authors. It launches nothing, writes no report/checkpoint, appends nothing, and
never generates passing verdicts. The driver must finalize/checkpoint the index
before commissioning executable independent judgments. `commands_complete` is
mechanical collection completeness, not acceptance. The new candidate evidence
form explicitly names `common-capture-v1`, the original index and execution
settings; its report-proof identity is not a provider checkpoint identity.

The hidden native `validation-command` keeps exactly four worker argv fields:
`[validation-command, ABS_CWD, SERIALIZED_PROOF_COMMAND, TIMEOUT_MS]`. Its optional
stdin JSON supplies `artifact_root`, `environment` and `inherit_environment`.
Without a packet it retains a fresh capture under the system temporary directory;
production callers supply the run artifact root. It uses the shared in-process
capture executor, forwards both live streams to stderr (retaining separate raw
files), and reserves stdout for one native JSON result with `capture_index` and
`capture_receipt` locators. Native repository fields use the actual provider
identity collected around execution, while the referenced child receipt uses the
report identity. Never launch the enclosing ad-hoc fan-out from inside the proof
checkout: its own output files would change the measured tree. Run it from the
external artifact directory. Frozen `run-validation` remains validation-only.

## Shared packaged smoke

`python3 scripts/packaged-smoke.py --mode archive|installed --expected-version VERSION --platform TARGET --output-root ABS --package-identity ABS`

Supply each of `loop-cli`, `software-change-provider`, `policy-document-provider`
and `research-provider` as `--archive APP=ABS --checksum APP=SHA256` in archive
mode, or `--binary APP=ABS` in installed mode. Identity JSON contains `version`,
`platform` and `sha256` (an application-to-digest object for archives or installed
executables respectively). Obtain identity from the package build/download;
installed identity must bind the actual installed bytes. It is explicit caller
provenance, not cryptographic publisher authentication. Policy-document has no
`--version`: its identity comes from that package evidence and its public
profile dump and draft/audit journeys. The other three executables additionally
verify their public version output. No Bookends package is assumed.

The wrapper requires the native supported release target and creates a fresh
attempt and empty working directory outside checkout. It runs the existing
software-change packaged checked-prefix, policy-document draft and audit, then
research packaged journeys. `commands.json`, stdout/stderr and `outcome.json`
remain under the supplied output root on success or failure. Argument-parser
errors return nonzero on stderr before an attempt exists. CI supplies the dist
plan version and downloaded archive checksum sidecars to this same entry point.
The `packaged` fixture builds real local cargo-dist outputs, copies them outside
checkout and exercises both modes and identity/input/journey refusals; it never
publishes. Other source journeys and the final matrix remain separate proof.

## Bookends history and bypass

Bookends still compares only the immediate baseline: HEAD for a dirty PRD,
otherwise the first parent of HEAD. A verified root or a resolved tree without
the PRD permits first adoption. Missing required commit/tree/blob objects return
RED, including a depth-one shallow boundary. CI checks out depth two; standalone
callers must fetch needed history or accept RED, not silently adopt.

The gate wrapper delegates bypass permission to `bookends-check --bypass
<class>:<reason>`. On a red check, the CLI first publishes and syncs a write-once
YAML receipt outside the repository, then emits BYPASS and exit zero. Receipts
include UTC Unix invocation seconds, canonical repository, revision (or explicit
unavailable diagnostic), class, reason and BYPASS outcome. Default storage is
`$XDG_STATE_HOME/bookends-check/bypasses`, falling back to
`$HOME/.local/state/bookends-check/bypasses`; `--receipt-root ABS` overrides it.
Recording failure emits RED and nonzero, never permission. Existing receipts
are not overwritten. GREEN checks remain GREEN and do not consume a bypass.
These are local runtime artifacts, not committed evidence or semantic approval.

## Passive monitor

```
loop-engine monitor --run RUN_ID --run OTHER_ID --capture-dir /absolute/captures --json --attention-seconds 300
loop-engine monitor --engine /absolute/released/loop-engine --run RUN_ID --json
```

Put `monitor` first. Repeat `--run` and `--capture-dir`; at least one is required.
`--invocation ID` scopes exactly one selected run to one invocation. Optional
`--database PATH` selects an explicitly authorized fixture/other catalog; otherwise
normal engine defaults apply. Native subprocess reads use `show --view status`,
`history`, and `invocation-progress`, never an arming show. `--engine ABS` always
selects the restricted compatibility adapter: only `list`, `history` and
`invocation-progress`, never released show or candidate catalog access. Compatibility
capture bodies are read only when explicitly attached with `--capture-dir`.
Each backend read has a five-second timeout; a stalled read becomes uncertainty,
not stalled work control. Polling defaults to one second (`--poll-seconds N`).

`--json` writes one flushed JSON object per line to stdout. Without it, human
rendering goes to stderr. Both follow until interrupted; neither cancels observed
work. A pipe consumer should read until the selected source's `event` is
`completion` or `attention`, then inspect `boundary.reason` and the separate
`workflow`, `helper`, `worker`, `conformance`, and `judgment` lanes. `source` names
`run:ID` or `capture:ABS`; packets include sample time and available invocation,
attempt, receipt, history sequence/time and engine/catalog locators. Completion is
selected execution/workflow termination, **not semantic approval**. One peer's
completion is never promoted to another source. Unchanged boundary identities are
not repeatedly notified. Restart rereads retained evidence and can notify again;
there is no exact-once/replay contract or new event journal.

Capture matrix completion requires completed state, matching nonzero selected and
expected row counts, identified successful receipts and intact streams. Running
state is not matrix completion. Fan-out completion checks declared worker coverage,
selected output integrity and declared mechanical conformance separately. Native
plan-graph summaries add generic `expected_assignment_ids` and `auxiliary_workers`
execution receipts while retaining the ordinary-task-only `workers` array. The
monitor includes the summarizer auxiliary receipt in its worker lane, requires the
whole declared inventory and intact selected outputs, and leaves semantic judgment
unknown. Helper completion is separate from this execution evidence. Legacy summaries
without inventory and missing graph nodes remain unknown. A terminal invocation whose graph coverage is unknown
gets attention rather than inferred worker success. Failures and unverified cleanup
produce attention. `--attention-seconds N` also emits a source-identified elapsed
observer deadline; it is not ETA and never authorizes retry or cancellation.
Sampling and read timeouts can delay a deadline notification.

Optional repeatable `--observation ABS` attaches driver-supplied JSON containing
`run_id`, `sampled_at_ms`, and nonempty `attesting_driver`, plus opaque supplied
content. Matching observations appear attributed and dated in the judgment lane;
current applicability is unknown. Mismatched/malformed observations remain unknown.
They do not override engine evidence or create completion. Deterministic observation
neither interprets provider verdict semantics nor makes model calls.

### Optional advisory summaries

Add `--summary-config FILE --output-dir ABS` to `monitor`. Without it there are
zero completion calls. Config JSON has exactly these fields (the executable and
arguments are operator supplied, never a shell string or built-in model API):

```json
{"executable":"/absolute/completion-command","args":["--example"],"timeout_seconds":60,"minimum_interval_seconds":300,"max_calls":24}
```

Both second values must be finite and positive; `max_calls` is a nonnegative
integer. One output directory identifies one retained monitoring session: reuse
it on restart to preserve attempted-call budget and cadence. Do not delete its
state to retry. A file lock prevents two observers from sharing its summary
session; the second still observes deterministically with summaries disabled.
Do not change sources/config mid-session; use a separately chosen directory for
an intentionally new budget. The product does not select a model or approve spend.

Each attempt receives one JSON object on stdin: `selected_sources`,
`current_new_evidence`, `omitted_source_count`, `omitted_source_locators`,
`evidence_digest`, `previous_summary`, and an instruction that prior prose is
fallible, not evidence. Source packets total at most 65536 serialized bytes;
oversized packets are omitted whole with their locators, not silently truncated.
The digest covers all selected normalized evidence, including omitted packets.
Observation/elapsed clock fields alone do not trigger calls. Current selected
packets, rather than a lossy text diff, allow correction against the prior input
digest. The prior usable output remains retained with its input digest and path.

Stdout must be exactly one JSON object with nonempty string `developments`,
`significance`, `uncertainty`, and `corrections` array. Each correction has exactly
nonempty strings `prior_claim` and `correcting_evidence` (an evidence locator).
Optional `usage` and `cost` each contain exactly nonnegative numeric `amount` and
nonempty string `unit`. Other fields, empty output, nonzero exits, missing commands,
timeouts and malformed output fail conformance. This shape check does not verify
truth, grounding, or the validity of a correction.

The flushed `summary` JSONL lane is advisory and separate from snapshot,
completion and attention. It displays attempts, budget, input digest and the last
usable summary; changed evidence or failures label that output older. Usage/cost
are only the command's supplied values, otherwise unknown. No summary approves,
advances, retries or cancels work. Each `attempt-NNNN` retains `command.json`,
`stdin.json`, raw `stdout`/`stderr` and `exit.json`. Earlier attempts are not replaced.

Calls run serially without blocking deterministic polling; changes during a call
or cadence coalesce to the latest evidence. All attempts are reserved in
`session.json` before spawn, including failures. Unchanged evidence never retries;
cap exhaustion only stops summaries. Invalid config or persistence failure disables
summaries, not observation. A restart with an interrupted attempt lacking exit
evidence disables further automatic summaries rather than risking overlapping
calls or pretending an unknown exit succeeded. Its raw files and reserved call
remain; stopping an observer is not cancellation of observed workflow work.

## Fixture entry point

```
python3 scripts/operational-ux-journey.py --case CASE --binary-dir ABS --released-root ABS --output-root ABS
```

Inputs are explicit existing absolute roots; output must be external to checkout.
Cases: show (T02), capture (T03), monitor (T04), summary (T05), validation (T06),
policy-selection (T07), guidance (T08), packaged (T09), delivery (T10), bookends
(T11). Each owner implements its case through real public commands with scripted
backends/isolated fixture catalogs, using `test_contract`; do not duplicate
existing journey implementations or create another suite runner. The initial
entry point refuses every unimplemented case and retains failure outcomes in
fresh attempt directories. Argument/refusal checks do not prove these cases.
