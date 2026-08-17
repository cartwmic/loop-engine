# Bound-work UX findings and suggestions

Written 2026-08-16 against loop-engine 0.9.0. Re-triaged 2026-08-17 against 0.11.0.

Source run: `shared-catalog-agent-default-0.9` (software-change `standard`), abandoned at `plan`
and terminated once its successor `shared-catalog-agent-default-0.11` reached `end` on 0.11.0.

This note is operator/driver analysis. It is not a software-change artifact and does not
bind the provider, the engine, or any run.

## Status against 0.11.0

The 0.11.0 release closed four findings. The re-triage below comes from driving
`shared-catalog-agent-default-0.11` end to end on 0.11.0, not from reading the changelog.

Closed:

- **F3** — `show` now projects `inner_workers` with `command`, `args`, and `exit_code` per
  worker in argv order once the bound CLI finishes. The collector summary is no longer discarded.
- **F5** — partially subsumed by F3: worker identity is still positional, but argv is now
  reported alongside the exit code, so a driver can tell which worker failed.
- **F6** — the capture path now carries the invocation id
  (`work-slot-captures/{slot_id}/invocation-{id}`), so retries no longer clobber.
- **F9** — `preview-bindings [JSON|@FILE]` inspects `work_slot_bindings` without starting a run,
  expands nested worker JSON, and warns on `--no-extensions` without `-e`.

Still open, each re-observed on 0.11.0:

- **F1** — `--timeout-ms` still defaults to 30000 ms and `--help` still describes it as
  "Provider operation timeout" while `invoke` reuses it as `allowed_time_ms`. Driving this run
  still meant guessing 1200000 for review slots and 3600000 for implement.
- **F2** — mitigated but not closed: `show` now carries an `overlay_meaning` string
  ("Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work"),
  which states the contract but does not surface whether the work was any good.
- **F4** — confirmed live. The `implementation-review` overlay reported `succeeded` with
  `exit_code` 0 while inner worker 0 had written only a Cursor context-length error to stderr
  and produced zero stdout. Collector exit status still hides inner worker failure.
- **F7** — confirmed live. One reviewer returned a JSON verdict, one returned markdown prose,
  and one returned an apology that it lacked write tools. Bound reviewer stdout has no contract.
- **F8**, **F11**, **F12** — unchanged; F11 and F12 were re-observed as manual `show` polling
  with no wait and no pause.
- **F10** — unchanged, and sharper than first written. See the addendum below.

## Addendum: bound slots fail closed on zero-axis gates

F10 observed that empty gates still look like bound reviews. 0.11.0 makes the consequence
concrete. `standard-5` defines zero `implementation-review` axes, which invites the reading
that the bound invoke is ceremonial and the checked edge can be requested directly. It cannot:

```
$ loop-engine --json event RUN_ID approved
{"code": "bound-slot-invocation-required",
 "message": "bound work slot `implementation-review` requires a succeeded invocation matching
             slot id, instruction digest, and current visit subject; overlay was none",
 "status": "rejected"}
```

A bound slot requires a succeeded overlay whether or not the gate defines review axes. Axis
count and overlay requirement are independent. `bound-slot-invocation-required` is currently
named in no skill, README, or `docs/agent-usage.md`, so an agent that hits it has nothing to grep.

## What bound work currently is

`invoke RUN_ID SLOT_ID` starts the frozen `{command, args}` for that slot. Hidden `wait-invocation` is parent of the worker, waitpids it, and writes terminal overlay `succeeded` or `failed`. Overlay `running` means the waiter is still alive and elapsed time is under `allowed_time_ms`. Overlay `overrun` means the waiter is still alive past `allowed_time_ms`; it is terminal for retry: invoke the same slot again.

Those statuses are process-level. Overlay `succeeded` means the bound CLI exited 0. It does not mean the review passed, the plan is approved, or evidence was appended. The driver still triages worker output, appends provider-shaped records, and requests the shown event.

That split is the right product contract. The UX problem is that almost everything needed after `invoke` is off the `show` path.

## Findings from the executed `design-review` invocation

### F1. Default invoke timeout is the provider timeout

`--timeout-ms` defaults to 30000 ms and is documented as a provider `describe`/`evaluate` timeout. `invoke` reuses it as `allowed_time_ms`. The design-review fan-out here took on the order of 10 minutes. The driver had to guess `--timeout-ms 600000`.

Help text still says "Provider operation timeout". Skills say "raise N above the 30s default" but do not suggest a number.

This run's recorded invocation:

- `invocation_id`: `invocation-1786888178774313000-1-17246`
- `slot_id`: `design-review`
- `status`: `succeeded`
- `exit_code`: `0`
- `allowed_time_ms`: `600000`
- elapsed roughly 10 minutes (`started_at` 1786888178774 → `completed_at` 1786888783956)

### F2. Overlay `succeeded` is collector exit 0, not semantic approval

After overlay `succeeded`, the driver still had to:

1. Find `artifact_root/fan-out/design-review/{0,1}/stdout`
2. Parse mixed worker output
3. Reconstruct eight-field `review-evidence` records
4. `append` them
5. Request the shown event

`show` reported overlay success and the frozen binding blob. It did not name capture paths, per-worker identity, or the next action.

### F3. Fan-out already builds the missing summary, then `invoke` discards it

`loop-cli` `fan-out` collector returns `FanOutSummary` with `output_dir` and per-worker `command`, `args`, `exit_code`, `stdout_path`, `stderr_path`. In ad-hoc mode that JSON is printed to stdout.

In bound mode, `wait-invocation` spawns the frozen CLI with stdout and stderr pointed at `/dev/null`. The summary never lands in `show`, `history`, or the artifact directory.

Durable captures for this run:

```
$artifact_root/fan-out/design-review/0/{stdout,stderr}
$artifact_root/fan-out/design-review/1/{stdout,stderr}
```

No `summary.json`, no `worker.json`, no model id, no invocation id, no exit code file.

### F4. Inner worker nonzero exit can still yield overlay `succeeded`

Collector tests prove a dummy nonzero worker exit still produces collector success. Combined with F3, overlay `succeeded` can mean "every reviewer process was reaped," including crashes. Per-worker exit codes exist only in the discarded summary.

### F5. Worker identity is positional argv order

Index `0` vs `1` is `--worker` order. Mapping "Grok vs Sol" required reading frozen `work_slot_bindings` argv. `show.work_slot_invocations[].binding` holds the whole fan-out argv, not per-worker identity.

This run's frozen review workers, in order:

0. `pi --print --model cursor/cursor-grok-4.6`
1. `pi --print --model openai-codex/gpt-5.6-sol`

### F6. Capture path clobbers on retry

Bound fan-out writes to `artifact_root/fan-out/<slot-id>/<index>/`. A later invoke of the same slot overwrites the previous capture. `invocation_id` is not in the path.

### F7. Reviewer stdout has no contract

Worker 0 (Grok) wrote markdown, including a sidecar JSON path. Worker 1 (Sol) wrote a JSON object. The reviewer protocol specifies the eight-field `review-evidence` append record, not worker stdout. The driver had to parse both and mint records.

Sol's earlier intent review also failed once by trying to read missing `progress.md`. Bound instruction bodies did not fence the readable file set.

### F8. Catalog/binary skew is a panic, not a version error

After 0.9.0 wrote `invocation_started` history actions, PATH `loop-engine` 0.7.0 failed with:

```
persistence-failure: sqlite-deserialization: could not deserialize history action: unknown variant invocation_started
```

Workaround: call the absolute 0.9.0 binary for `show` and `history`. Skills do not say "use the same absolute `loop-engine` that started the run." Binding construction independently required absolute paths to `loop-engine`, `software-change`, and `pi` to avoid PATH skew at invoke time.

### F9. Binding confirmation is escaped nested JSON

Usable review bindings are `loop-engine fan-out --worker '{command,args}' --worker '{command,args}'`, with inner model flags. Implement is `software-change run-plan-graph --task-worker '{command,args}'`. Stock `standard.json` freezes review slots to `fan-out` with **zero** `--worker` entries, so stock `invoke` fails closed.

The freeze cannot be patched. Confirmation UX is a giant JSON blob. Easy to freeze the wrong binary or an unpinned `pi --print`.

### F10. Empty gates still look like bound reviews

On shipped `standard.json`:

- `review_policies.design-review`: 4 axes
- `review_policies.plan-review`: `[]`
- `review_policies.implementation-review`: `[]`

Those empty-policy slots are still bound to `fan-out`. Overlay can succeed, captures can exist, and `approved` is still schema-only. An operator can believe reviewers gated a phase that the frozen profile does not gate.

### F11. `invoke` returns immediately; there is no user wait

The driver polls `show` until overlay is terminal. There is no public `wait` command, no remaining-time field, and no inner-worker progress. `invoke` result is `invocation_id`, `slot_id`, `started_at`, `allowed_time_ms`. It does not name the expected capture directory.

### F12. Loop Engine has no pause command

"Pause the change run" means stop driving it. The run stays `active` in `plan` until the next `append` / `invoke` / `event` / `terminate`. That is consistent with current lifecycle rules, but easy to misread as a missing control.

## Suggestions

Keep these true:

- Overlay status remains process-level.
- The driver still triages and appends; do not auto-append `review-evidence`.
- Bindings stay frozen at `start`.
- Do not make foreground `invoke` waitpid the worker by default; overrun and vanished-waiter overlay depend on the hidden waiter.
- Do not fold isolation flags into work-slot policy.

### S1. Persist the collector summary and project it on `show`

Highest leverage. Write `artifact_root/fan-out/<slot-id>/summary.json` (the existing `FanOutSummary`) and per-worker `worker.json` (`command`, `args`, `exit_code`, optional label/model). Project latest capture directory and per-worker exit codes on `show`. Return the expected capture path from `invoke`.

This would have removed most archaeology on this run.

### S2. Put `invocation_id` in the capture path

Example: `artifact_root/fan-out/<slot-id>/<invocation_id>/<index>/`. Overrun/retry must not clobber the previous capture.

### S3. Make inner worker failure visible

Either fail the collector when any worker exits nonzero, or keep collector success and put per-worker exit codes on `show` so a dead reviewer cannot hide behind overlay `succeeded`. Prefer both: collector may still reap everyone, but `show` must not look like a clean review when a worker crashed.

### S4. Name the post-invoke loop on the bound-state handoff

For a bound review slot, human `show` / `current_state_instructions` should state, in order:

1. Overlay `succeeded` is collector exit 0, not approval.
2. Captures are at this directory.
3. Triage stdout, append `review-evidence`, then request the shown event.
4. On `overrun`, invoke the same slot again. On `failed`, inspect stderr and decide.

### S5. Split invoke time from provider time

- Separate `--invoke-timeout-ms` with a multi-minute default, or stop reusing the 30s provider default.
- `show` remaining time and overlay meaning while `running`.
- Optional `invoke --wait` that polls overlay and returns the terminal status, without changing the waiter/overrun model.
- Fix help text so `--timeout-ms` is not only "Provider operation timeout."

### S6. Binding preview before `start`

A `bindings-preview` (or start preflight) should:

- Expand nested `--worker` / `--task-worker` JSON
- List every model id
- Warn on unpinned `pi --print`
- Warn on zero-worker fail-closed
- Warn when `command` is a PATH name rather than the same absolute binary that will write the catalog

Keep the frozen `{command, args}` contract. Stop asking humans and agents to author escaped nested JSON by hand.

Optional: allow `{command, args, label}` on workers so "Grok vs Sol" is a durable field.

### S7. Fail clearly on catalog/binary skew

Older CLI reading a newer catalog should say "catalog written by 0.9.0, this binary is 0.7.0," not panic on `unknown variant`. Skills should say: use the same absolute `loop-engine` for `start`, `invoke`, `show`, and `history`.

### S8. Contract bound-reviewer stdout

Bound review `instruction_body` should require one JSON object (or an array of axis records) on stdout, and should name the only files the reviewer may read. Do not auto-append that JSON. Driver triage remains required.

### S9. Be honest about empty gates

On `standard`, either unbind `plan-review` and `implementation-review`, or have `show` say this profile's gate is empty so bound workers are advisory. Do not let a successful fan-out look like a policy gate.

### S10. Driver workarounds until the above ships

For this paused run and any 0.9.0 bound run:

- Call the absolute 0.9.0 `loop-engine` for every operation.
- Pass an explicit `--timeout-ms` well above expected reviewer runtime (this run used 600000).
- After overlay `succeeded`, read `fan-out/<slot>/<index>/stdout` in `--worker` order; do not trust overlay as a verdict.
- Instruct reviewers to emit JSON only and not to read `progress.md`, repo-root `AGENTS.md`, or git history unless named.
- Do not expect bound `plan-review` / `implementation-review` to enforce axes on `standard`.

## If only three product changes ship

1. Persist fan-out summary + per-worker identity/exit code, and project it on `show`.
2. Make bound-state instructions name the post-invoke loop: triage → append → event.
3. Preview bindings and timeouts before `start`, including models, zero-worker fail-closed, and binary identity.

Those three would have made this design-review invocation a `show` plus a read of known files, instead of reconstructing worker identity, guessing timeout, pinning an absolute binary after a deserialize panic, and hand-parsing mixed stdout into evidence records.

## Code and artifact anchors

- Overlay rules: `crates/loop-core/src/invocation.rs`
- `invoke` packet and waiter spawn: `crates/loop-core/src/operations/invoke.rs`
- `show` invocation view (no capture paths): `crates/loop-core/src/operations/show.rs` (`WorkSlotInvocationView`)
- Fan-out summary (printed, not persisted in bound mode): `crates/loop-cli/src/fan_out.rs` (`FanOutSummary`, `FanOutWorkerResult`)
- Waiter discards worker stdout/stderr: `crates/loop-cli/src/lib.rs` `wait_for_worker_and_complete` (`Stdio::null()`)
- Stock bindings and empty plan/implementation policies: `crates/software-change-provider/data/configs/standard.json`
- Bound-review driver loop: `crates/software-change-provider/skills/using-software-change-provider/SKILL.md`
- Engine work-slot rules: `docs/agent-usage.md`, `skills/using-loop-engine/SKILL.md`
- This run captures: `/Users/cartwmic/.local/share/loop-engine/runs/shared-catalog-agent-default-0.9/fan-out/design-review/`
