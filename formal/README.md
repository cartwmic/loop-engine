# Formal model: overlapping event attempts

`LoopRun.tla` is a TLA+ model of one promise in [docs/PRD.md](../docs/PRD.md) (lines 726-730):

> When overlapping event attempts compete against the same pre-mutation run state, at most one may commit; any competing attempt made stale by the committed mutation must fail without producing a conflicting semantic effect.
>
> A checked evaluation is stale in v0.1 when the run's state or lifecycle changes while evaluation is in flight. [...] the stale result produces no semantic history or evaluation lineage.

**What breaking it costs.** Two drivers (or a driver and `terminate`) act on one run. A stale `allow` lands after another transition or after a `terminate`: the run moves twice from one state, or an event commits on a run that was closed. History then disagrees with state.

**Run it.** `formal/check.sh`. Exit 0: every rule holds. Exit 1: a rule breaks; the report names it and prints the shortest path. Exit 2: the check could not run (tool missing, download failed). CI runs it as a step of the `checks` job.

**What passing means.** The check proves the *model*. It does not read the Rust. The link from model to code is the mapping table below, the sync check (next section) and the tests in `crates/loop-integrations/tests/concurrency.rs`. A pass covers only the bounds and the omissions below.

## Keeping the model in step with the code

`formal/COVERS` lists each source file the model covers, with the git blob hash that file had when the model was last confirmed against it. `formal/check.sh` runs `formal/check-sync.py` first, then `formal/test_check_sync.py`, both before any Java or TLC setup, so the sync answer does not depend on the model tools. The check recomputes each hash and exits 1, naming the file, when one has changed or is gone; it exits 2 when `COVERS` is missing or malformed. The test checks the failure path on a temp copy of the files.

When it fails, read the diff of the named file, update `LoopRun.tla` from the code (not from a check result), rerun `formal/check.sh`, then run `formal/check-sync.py --update` to record the new hashes. A PR that changes a covered file does all of this in the same PR. The check needs no git history, so a shallow clone works.

A sync pass says only that the covered files are unchanged since the last confirmation. It does not say the model is right.

## What is modelled

One run, a small workflow, and overlapping event attempts. The model is cut where `sqlite.rs` cuts its `BEGIN IMMEDIATE` transactions: one transaction is one action; the reads between transactions and the provider call are separate actions. Those are the interleaving points.

Workflow: states `A`, `B`, `F` (final). Events `e1` A to B (checked), `e2` B to A (check-free), `e3` B to F (checked), `e4` A to A (checked self-loop). It has a cycle, a self-loop, both edge kinds and a final edge. Each attempt requests one event, chosen freely at the start.

Other actors: `Observe` (the driver's `show`), `AppendContext` (context-only write), `Terminate` (the operator).

### Bounds

| Bound | Value | In |
|---|---|---|
| Overlapping attempts | 3 | `LoopRun.cfg` `Attempts` |
| Context appends | 1 | `LoopRun.cfg` `MaxAppends` |
| Attempts per event | one pass each (an attempt ends with a commit, a denial or a refusal) | `LoopRun.tla` |
| Workflow | the 3-state, 4-event workflow above | `LoopRun.tla` |

At these bounds TLC explores 150,125 states. If a warm run goes over 2 minutes, lower `Attempts` or the workflow.

### Left out

Provider evaluation content; the owned-work barrier; advice closure (LE-166); context-record ordering (PRD.md:724); override and advice-exception commits; provider failure and `unsupported` results (they end an attempt with no write, the same as a refusal); the `expected_last_sequence` check on append; run creation. Time and fairness: the model checks safety only.

## The rules

| Rule (INVARIANT in `LoopRun.cfg`) | Says | PRD |
|---|---|---|
| `TypeOK` | Variables stay in their types. | |
| `AtMostOneCommitPerRevision` | No two history entries that advance `control_revision` (transition or termination) assert the same revision. | 726-728 |
| `StaleWritesNothing` | Every transition, denial and termination on record asserted exactly the revision and source state the run had when it was written. | 728, 730 |
| `NoWriteAfterTerminal` | Nothing follows a termination or a transition into a final state; a run is terminated at most once. | 500, 504 |
| `StoredStateMatchesHistory` | Stored state, revision and lifecycle equal what the history says. A denial, an append or a refused request moves none of them; an append does not stale an in-flight evaluation (it does not change the revision). | 334, 340, 341, 730 |
| `WritesNeedObservation` | Every history entry was written while the run was observed: an unobserved attempt, append or `terminate` is refused and writes nothing. | 421, 457, 498 |
| `NoDeadlock` (statechart's own) | Every state has a way out; the end state is the `Done` action. | |

## Mapping: action to code

Each action is one indivisible step in the code. Lines are at the commit that last confirmed the model.

| Action | Code | Note |
|---|---|---|
| `Observe` | `sqlite.rs:1012-1030` (`load_show_data`, the `UPDATE` at `:1022-1027`); `sqlite_observation.rs:25-31` (the `arm` write) | Two writers set `observed_control_revision = control_revision`. Works on a run in any lifecycle. |
| `AppendContext` | `sqlite.rs:448-512` (guards at `:459-460`) | `require_active` then `require_observed`. No revision change. |
| `Terminate` | `sqlite.rs:797-830` (guards at `:804-805`); `terminate.rs:15-22` | One transaction: lifecycle, revision, one history entry. |
| `LoadOk`, `LoadRejected` | `event.rs:90-143` (`load_authoritative_run`, active check, event resolution, `require_current_observation`) | Two reads in the code (`load_authoritative_run`, then `observation_is_current`, `sqlite.rs:832-851`) are one action here. Sound: revision only grows, and `observation_is_current` matches on `control_revision`, so a write between the two reads fails it. The extra refusal is an attempt ending with no write, which the model already has. |
| `CommitFree` | `event.rs:285-314`; `sqlite.rs:597-743` | The commit transaction. Guards: active `:656-662`, revision and source `:664-668`, observed `:670`. |
| `Snapshot`, `SnapshotRejected` | `event.rs:382-398`; `sqlite.rs:960-998` | A read (deferred transaction). Takes the run's revision at that moment. `:970` active, `:971-981` source and edge still present. |
| `ProviderAllows`, `ProviderDenies` | `event.rs:402-416` | Outside every persistence transaction: either answer can arrive after the run moved. |
| `CommitAllowed` | `event.rs:454-487`; `sqlite.rs:597-743` | The same commit transaction, asserting the snapshot's revision (`event.rs:473-476`). |
| `RecordDenial` | `event.rs:489-510`; `sqlite.rs:745-795` | Same guards. Writes one history entry; state and revision unchanged. |
| `WriteRefused` | `sqlite.rs:656-670`, `:755-768` (the guard fails) | The stale attempt ends, nothing written (PRD.md:730). |
| `Done` | | End state only; no code. |

PRD.md:155 (an event ID identifies at most one transition) and 157 (workflow validation) hold by the shape of the workflow in this model, so no rule checks them.

Guards in the model are separate operators (`ActiveGuard`, `ObservedGuard`, `RevisionGuard`, `SourceGuard`) so a scratch break can delete exactly one.

## Seeing it fail

A check never seen failing proves nothing. Edit `LoopRun.tla` in a scratch copy, run `formal/check.sh`, then drop the edit. Results seen on 2026-11-23 (main at b011838, with `WritesNeedObservation` added):

| Edit | Exit | Rule that breaks | Shortest path |
|---|---|---|---|
| `RevisionGuard(a) == TRUE` | 1 | `AtMostOneCommitPerRevision`, `StaleWritesNothing` | 10 steps: two attempts snapshot revision 0; one commits; after `show` the other commits on revision 1 |
| `ActiveGuard == TRUE` | 1 | `NoWriteAfterTerminal`, `StoredStateMatchesHistory` | 4 steps: show, terminate, show, append |
| `ObservedGuard == TRUE` | 1 | `WritesNeedObservation` | 1 step: terminate with no `show` first |
| Remove only the `ActiveGuard` line inside `CommitGuard` | 0 | none | Defence in depth, see below |

**Finding, not a bug.** The lifecycle-active check in the commit transaction (`sqlite.rs:656-662`) is redundant in this protocol. Every way out of `Active` (terminate, a commit into a final state) also advances `control_revision`, and an attempt can only read its revision while the run is active, so the revision guard already refuses every late commit. The same lifecycle guard in `AppendContext` is not redundant: appends assert no revision, and `show` works on a terminated run. Deleting the append guard is the break that shows the lifecycle rule.

After a break statechart reruns TLC with a 100,000-state cap, so the report lists some rules under "Rules not checked". The exit code is still 1.

## Reading a result

`check.sh` prints statechart's `report.md`. A broken rule has a numbered path; each line is one step, in words from `view.mjs`, followed by the state it reaches: lifecycle, workflow state, revision, whether the run is observed, and each attempt's stage (`new`, `read`, `snapped`, `evaluated`, `done`). Replay the path against the code one step at a time. If it does not happen in the code, suspect the model first. Above 500 states statechart draws no chart for a pass; for a break it draws the path.

## Tools

statechart: [cursor/plugins](https://github.com/cursor/plugins) `pstack/skills/statechart` at the commit pinned in `check.sh` (MIT). TLC: `tla2tools.jar` 1.7.4, pinned by sha256 in `check.sh`. Both are fetched into `~/.cache/loop-engine-formal` on first use. Java and Node are used as installed; when there is no Java, `mise` installs `java@temurin-25`.
