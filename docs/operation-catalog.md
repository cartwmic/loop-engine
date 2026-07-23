# Application Operation Catalog

**Status:** Frozen by T004 (2026-07-17). Decision [D004](change/initial-implementation/decisions.md#d004--application-operation-catalog).

This document is the canonical closed catalog of MVP application operations. It fixes stable operation IDs, production CLI argv/flag ownership, behavioral facet applicability, provider-role invocation rows, reason-code taxonomy, mutation classification, lifecycle-facet ownership, and explicit non-operations. Generic semantics from [architecture.md](architecture.md), [invariants.md](invariants.md), [testing.md](testing.md), and [ux-storyboards.md](ux-storyboards.md) remain authoritative; this catalog names the operations that implement them.

Related documents:

- [Decision D004](change/initial-implementation/decisions.md#d004--application-operation-catalog)
- [Decision D005](change/initial-implementation/decisions.md#d005--provider-protocol-v1)
- [Coverage map](change/initial-implementation/coverage.md)
- [Export contract](export-contract.md) (D015)
- [Facet inventory schema](../quality/facets/v1/schema.json)

## Closed catalog (21 operations)

| # | Operation ID | Namespace |
|---:|---|---|
| 1 | `provider.add` | provider |
| 2 | `provider.list` | provider |
| 3 | `provider.check` | provider |
| 4 | `provider.update` | provider |
| 5 | `provider.rename` | provider |
| 6 | `provider.disable` | provider |
| 7 | `provider.restore` | provider |
| 8 | `run.create` | run |
| 9 | `run.list` | run |
| 10 | `run.show` | run |
| 11 | `run.graph` | run |
| 12 | `run.history` | run |
| 13 | `run.evidence.add` | run |
| 14 | `run.evidence.list` | run |
| 15 | `run.annotate` | run |
| 16 | `run.label` | run |
| 17 | `run.request` | run |
| 18 | `run.guidance` | run |
| 19 | `run.compatibility` | run |
| 20 | `run.terminate` | run |
| 21 | `run.export` | run |

No operation split, merge, or rename is permitted without reopening D004.

### Staged runtime exposure

Per the 2026-07-22 execution-plan amendment, runtime exposure advances atomically by checkpoint while this change remains open. Checkpoint A exposes `provider.add` and `provider.list`; all other catalog IDs remain private until their named WP3 or WP6 checkpoint closes. `--list-operations` reports only currently exposed IDs during this staged implementation. Final closure still requires exact equality with all 21 IDs above.

## Provider handle grammar

Provider handles are lowercase ASCII conveniences, never identity. Case-sensitive. No normalization.

```abnf
handle      = alnum / (alnum *126(handle-mid) alnum)
handle-mid  = alnum / "." / "_" / "-"
alnum       = %x30-39 / %x61-7A
```

One-character handles are valid. CLI `<TARGET>` for provider-catalog commands accepts either a handle (among enabled registrations) or a registration ID. `run.create` `<TARGET>` accepts the same provider handle or registration ID. All other run commands accept `<RUN-ID>` only.

## Production CLI surface

Global rendering, structured-mode, trace, configuration, and help/version flags are owned by [cli-contract.md](cli-contract.md) (frozen by T006, [D006](change/initial-implementation/decisions.md#d006--structured-cli-contract)). This section owns **application** subcommand argv only.

Notation:

- `<TARGET>` — provider handle or registration ID (provider-catalog commands and `run.create`).
- `<RUN-ID>` — stable run ID (all run commands except `run.create`).
- `<PATH>` — bounded filesystem path per [cli-contract.md](cli-contract.md) [Resource bounds (D008)](cli-contract.md#resource-bounds-d008) (`filesystem_path_utf8_bytes`).
- `<CURSOR>` — opaque cursor v1 per [cli-contract.md](cli-contract.md) [Collection pagination and cursor v1](cli-contract.md#collection-pagination-and-cursor-v1).
- `<COUNT>` — page count ceiling per [cli-contract.md](cli-contract.md) [Resource bounds (D008)](cli-contract.md#resource-bounds-d008) (`collection_page_default_count`, `collection_page_max_count`).
- Repeatable flags may appear multiple times in stable argv order.

### Provider commands

| Operation ID | argv |
|---|---|
| `provider.add` | `provider add <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]` |
| `provider.list` | `provider list [--enabled] [--tombstoned] [--active-runs-for <REGISTRATION-ID>] [--cursor <CURSOR>] [--limit <COUNT>]` |
| `provider.check` | `provider check <TARGET> [--active-runs] [--cursor <CURSOR>] [--limit <COUNT>]` |
| `provider.update` | `provider update <TARGET> --exec <PATH> [--arg <VALUE> ...] [--working-directory <PATH>] [--timeout <SECONDS>]` |
| `provider.rename` | `provider rename <TARGET> <NEW-HANDLE>` |
| `provider.disable` | `provider disable <TARGET> [--warning-cursor <CURSOR>] [--limit <COUNT>] [--allow-active-runs <ACK-TOKEN>]` |
| `provider.restore` | `provider restore <REGISTRATION-ID> --handle <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]` |

`provider.list` defaults to enabled registrations when neither `--enabled` nor `--tombstoned` is supplied. `--active-runs-for` returns byte/count-paged active-run impact IDs for one registration.

`provider.check` without `--active-runs` performs protocol conformance and current emitted-graph validation for one registration (one `describe` invocation). With `--active-runs`, each page resolves registration once, spends one call slot on current `describe`/conformance, then processes at most nine active-run `check_compatibility` rows (ten provider calls total per page). Zero rows completes. `describe` process/protocol failure errors the invocation; on `--active-runs`, any `check_compatibility` process/protocol/evaluation failure errors the whole page. Cursor retrieves the next page.

`provider.update` completes without approval flag. Response includes affected active-run count and a `provider.list --active-runs-for` cursor/link; never returns unbounded run IDs.

`provider.update` mutation semantics (frozen, no additional flags):

- `--exec <PATH>` is **required** and replaces the stored executable path.
- Each supplied `--arg <VALUE>` appends to the new argv in stable order; the supplied `--arg` list **replaces** the full stored argv. Omitting all `--arg` flags clears argv to empty.
- Omitted `--working-directory` and `--timeout` preserve existing stored values.

`provider.disable` without `--allow-active-runs` is non-mutating warning pagination: initial call and `--warning-cursor` pages name affected runs; only the final page emits opaque `ack_token` bound to registration ID, config revision, and full active-set digest. `--allow-active-runs <ACK-TOKEN>` authorizes tombstone in one atomic mutation that rechecks token binding and current active set. First/intermediate cursors or independently supplied digest reject.

### Run commands

| Operation ID | argv |
|---|---|
| `run.create` | `run create <TARGET> [--label <LABEL>] [--inputs <PATH>]` |
| `run.list` | `run list [--terminal] [--all] [--cursor <CURSOR>] [--limit <COUNT>]` |
| `run.show` | `run show <RUN-ID>` |
| `run.graph` | `run graph <RUN-ID>` |
| `run.history` | `run history <RUN-ID> [--cursor <CURSOR>] [--limit <COUNT>]` |
| `run.evidence.add` | `run evidence add <RUN-ID> --kind <KIND> --ref <LOCATOR> [--digest <DIGEST>] [--media-type <TYPE>] [--metadata <PATH>]` |
| `run.evidence.list` | `run evidence list <RUN-ID> [--cursor <CURSOR>] [--limit <COUNT>]` |
| `run.annotate` | `run annotate <RUN-ID> [--note <TEXT>] [--actor <PATH>] [--corrects <SEQUENCE>]` |
| `run.label` | `run label <RUN-ID> [--set <LABEL> \| --clear]` |
| `run.request` | `run request <RUN-ID> <EVENT> [--evidence-id <ID> ...] [--evidence <PATH>] [--note <TEXT>]` |
| `run.guidance` | `run guidance <RUN-ID> [--evidence-id <ID> ...]` |
| `run.compatibility` | `run compatibility <RUN-ID>` |
| `run.terminate` | `run terminate <RUN-ID> [--note <TEXT>]` |
| `run.export` | `run export <RUN-ID> --output <DIR>` |

`run.list` defaults to active runs. `--terminal` includes final and terminated runs. `--all` is shorthand for terminal-inclusive listing with full lifecycle filters.

`run.export` writes versioned `manifest.json`, `state.json`, and ordered `journal.jsonl` to `<DIR>` per [export-contract.md](export-contract.md). `<DIR>` must be new and empty. Export is read-only; no import, restore, replay, or locator dereference.

## Mutation classification

| Class | Operations | Authoritative effect | Per-run journal |
|---|---|---|---|
| Provider-catalog mutation | `provider.add`, `provider.update`, `provider.rename`, `provider.disable`, `provider.restore` | Machine-local registration catalog | **None** (I40) |
| Run creation | `run.create` | New run + creation journal atomically | Creation entry only |
| Run-state or run-journal mutation | `run.evidence.add`, `run.annotate`, `run.label`, `run.request`, `run.guidance`, `run.terminate` | Authoritative run state and/or journal | Yes |
| Compatibility-attempt journal only | `run.compatibility` | No state/version/latch change | Compatibility-attempt facts only |
| Read / report | `provider.list`, `provider.check`, `run.list`, `run.show`, `run.graph`, `run.history`, `run.evidence.list`, `run.export` | None (export reads snapshot) | **None**, including `provider.check --active-runs` |

Provider-catalog mutation success or rejection is verified by fresh-process `provider.list` with **no** per-run journal entry. Invocation trace remains diagnostic proof (I40).

Rejected or errored `run.create` produces **no** run and **no** run journal.

Rejectable run mutation after successful run lookup records rejection in journal when persistence remains available; fresh-process `run.history` (or state query where applicable) verifies unchanged authoritative state.

## Registration-wide vs per-run compatibility

| Concern | Owner operation | Journal behavior | Latching |
|---|---|---|---|
| Registration-wide active-graph conformance report | `provider.check --active-runs` | No per-run journal fan-out | Non-latching report only |
| Per-run explicit compatibility inspection | `run.compatibility` | Atomically appends compatibility-attempt / provider-observation facts for that run, including drift | Non-latching; no state/version mutation |

`provider.check` conformance on latest emitted graph completes with valid/invalid finding; invalid graph during `run.create` is operation error, not domain rejection.

`run.compatibility` on terminal lifecycle rejects by lifecycle. `run.request` / `run.guidance` reject only selected unsupported capabilities; supported and gate-free paths remain usable.

## Update without approval vs disable acknowledgement

| Operation | Active runs present | Caller acknowledgement | Mutation |
|---|---|---|---|
| `provider.update` | Allowed | **Not required** | Immediate atomic config replacement under same registration ID; config revision increments; affected count + paged impact link returned |
| `provider.disable` | Allowed with warning | **Required** via final-page `ack_token` and `--allow-active-runs` | Tombstone only after token validates full active-set digest + config revision |

Drift of executable/policy for active runs requires no approval; stored graph remains fixed; gate attempts journal actual locator/digest (storyboard 6, I8).

## Behavioral facet flags

Facet names match [testing.md](testing.md) exactly. Universal row applies to every operation. Additional rows list only non-universal facets from [coverage.md](change/initial-implementation/coverage.md).

| Operation ID | Applicable facets (beyond universal) |
|---|---|
| `provider.add` | Provider-catalog mutation; Rejectable provider-catalog mutation; Trace persistence boundary |
| `provider.list` | Read; Trace persistence boundary |
| `provider.check` | Provider invoking; Read; Compatibility sensitive; Trace provider boundary; Trace persistence boundary |
| `provider.update` | Provider-catalog mutation; Rejectable provider-catalog mutation; Trace persistence boundary |
| `provider.rename` | Provider-catalog mutation; Rejectable provider-catalog mutation; Trace persistence boundary |
| `provider.disable` | Provider-catalog mutation; Rejectable provider-catalog mutation; Trace persistence boundary |
| `provider.restore` | Provider-catalog mutation; Rejectable provider-catalog mutation; Trace persistence boundary |
| `run.create` | Successful creation; Rejected/error creation; Provider invoking; Trace provider boundary; Trace persistence boundary |
| `run.list` | Read; Lifecycle family; Provider-free under missing provider; Trace persistence boundary |
| `run.show` | Read; Lifecycle family; Provider-free under missing provider; Trace persistence boundary |
| `run.graph` | Read; Provider-free under missing provider; Trace persistence boundary |
| `run.history` | Read; Provider-free under missing provider; Trace persistence boundary |
| `run.evidence.add` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required; Provider-free under missing provider; Trace persistence boundary |
| `run.evidence.list` | Read; Provider-free under missing provider; Trace persistence boundary |
| `run.annotate` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required; Provider-free under missing provider; Trace persistence boundary |
| `run.label` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required; Provider-free under missing provider; Trace persistence boundary |
| `run.request` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Provider invoking; Gate driven; Lifecycle family; Compatibility sensitive; Provider-free under missing provider; Journal required; Trace provider boundary; Trace persistence boundary |
| `run.guidance` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Provider invoking; Lifecycle family; Compatibility sensitive; Journal required; Trace provider boundary; Trace persistence boundary |
| `run.compatibility` | Provider invoking; Read; Lifecycle family; Compatibility sensitive; Journal required; Trace provider boundary; Trace persistence boundary |
| `run.terminate` | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required; Provider-free under missing provider; Trace persistence boundary |
| `run.export` | Read; Provider-free under missing provider; Trace persistence boundary |

Universal facet (every operation): **Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope**.

Exposure tasks close applicable facets in `quality/facets/v1/<operation-id>.json` before commit (see [quality/facets/v1/README.md](../quality/facets/v1/README.md)).

## Lifecycle-family ownership

Distributed ownership matches [testing.md](testing.md). Each row is closed by named owner before that operation's exposure.

| Lifecycle-family member | Owner operation(s) |
|---|---|
| Active run visibility | `run.list`, `run.show`, `run.terminate` |
| Neutral final with domain meaning in state ID | `run.list`, `run.show`, `run.terminate` |
| Intentional zero-final ongoing run | `run.list`, `run.show`, `run.terminate` |
| Non-final terminate-only sink | `run.list`, `run.show`, `run.terminate` |
| Explicit termination with optional note | `run.terminate` |
| Repeated termination rejection | `run.terminate` |
| Empty terminal requestable events | `run.list`, `run.show`, `run.terminate` |
| No reopen | `run.terminate` |
| Terminal evidence append allowance | `run.evidence.add` |
| Terminal annotation allowance | `run.annotate` |
| Terminal label change rejection | `run.label` |
| Terminal event request rejection | `run.request` |
| Terminal live guidance rejection | `run.guidance` |
| Terminal compatibility check rejection | `run.compatibility` |

## Provider role and result rows (D005)

Transport, process-group spawn setup, protocol-major, framing, timeout, crash, malformed, invalid-UTF-8, pre-spawn request overflow, and oversized authoritative result failures are **operation errors** for every invoked role. Engine never retries. One fresh provider process per provider call.

### Transport bounds

Named limits and overflow policy are canonical in [cli-contract.md](cli-contract.md) [Resource bounds (D008)](cli-contract.md#resource-bounds-d008) (frozen by T008, [D008](change/initial-implementation/decisions.md#d008--resource-bounds-and-timeout-defaults)) and [Overflow and rejection policy](cli-contract.md#overflow-and-rejection-policy). Catalog operation-error mapping:

| Condition | Reason code | Notes |
|---|---|---|
| Encoded request exceeds bound before spawn | `resource.exhausted` | Provider never receives the request |
| Oversized stdout or authoritative result envelope | `provider.protocol.oversized` | Distinct from stderr retention |
| Stderr exceeds trace retention budget | — (not a semantic outcome) | Drain/truncate with explicit trace truncation marker; does not override a complete independent protocol result |

| Operation ID | Invoked role(s) | Role-valid completed results | Domain rejection (from role) | Operation error (from role or engine) |
|---|---|---|---|---|
| `provider.check` (default) | `describe` | Completed description; consumer maps semantically invalid graph to completed invalid conformance finding | — (`describe` has no denial variant) | Process/protocol failure only (`describe` has no role-valid `evaluation_error`) |
| `provider.check` (`--active-runs`) | `describe`, then `check_compatibility` per row | Conformance summary + per-run completed findings (including incompatible) | — (incompatibility is finding, not rejection) | `describe` process/protocol failure; any `check_compatibility` process/protocol/evaluation failure errors whole page |
| `run.create` | `describe`; `validate_inputs` when declarations/values exist | Accepted graph snapshot; accepted input values | `validate_inputs` rejected values | Invalid graph (`provider.graph.invalid`); describe/validate evaluation errors; observed digest drift; tombstoned/missing/stale registration; spawn setup failure |
| `run.request` (gated) | `evaluate_gates` | Complete pass/fail verdict set | Failed verdicts; explicit stored-graph incompatibility | Evaluation error; malformed provider evidence; missing/incomplete verdict set |
| `run.request` (gate-free) | — | Engine decision from stored graph | Unknown event; lifecycle denial | Persistence failure |
| `run.guidance` | `live_guidance` | Advisory guidance text | Stored-guidance incompatibility; `guidance.unsupported` when stored capability absent | Evaluation error |
| `run.compatibility` | `check_compatibility` | Completed capability findings (including incompatible) | — (incompatibility is finding) | Evaluation error |

Provider roles `describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, and `check_compatibility` are **not** application operations.

## Outcome and reason taxonomy

Three top-level outcome classes (I34): `completed`, `rejected`, `error`. Structured envelope carries stable `reason.code` from the table below. Each code maps to exactly one primary outcome class. Compatibility/graph checks that successfully obtain findings **complete** even when findings report invalidity or incompatibility.

### Catalog and registration

| Reason code | Class | Typical operations |
|---|---|---|
| `catalog.handle.duplicate` | rejected | `provider.add`, `provider.rename`, `provider.restore` |
| `catalog.handle.invalid` | rejected | provider-catalog mutations |
| `catalog.handle.occupied` | rejected | `provider.restore` |
| `catalog.registration.not_found` | rejected | provider-catalog commands targeting missing ID/handle at dispatch lookup |
| `catalog.config.invalid` | rejected | `provider.add`, `provider.update`, `provider.restore` |
| `catalog.ack_token.invalid` | rejected | `provider.disable` |
| `catalog.ack_token.stale` | rejected | `provider.disable` |
| `catalog.active_runs.changed` | rejected | `provider.disable` |

### Provider execution and protocol

| Reason code | Class | Typical operations |
|---|---|---|
| `provider.tombstoned` | error | provider-dependent paths when registration is tombstoned |
| `provider.registration.missing` | error | provider-dependent paths when registration is unavailable for invocation (distinct from catalog-target rejection) |
| `provider.registration.stale` | error | provider-dependent paths when bound registration config revision is stale |
| `provider.spawn.failed` | error | process-group or spawn setup failure before protocol exchange |
| `provider.executable.not_found` | error | provider-dependent paths |
| `provider.protocol.unsupported_major` | error | provider invoking operations |
| `provider.protocol.malformed` | error | provider invoking operations |
| `provider.protocol.oversized` | error | oversized stdout or authoritative result envelope on provider invoking operations |
| `provider.protocol.invalid_utf8` | error | provider invoking operations |
| `provider.timeout` | error | provider invoking operations |
| `provider.crash` | error | provider invoking operations |
| `provider.nonzero_exit` | error | provider invoking operations |
| `provider.signal` | error | provider invoking operations |
| `provider.evaluation_error` | error | roles that permit `evaluation_error` (`validate_inputs`, `evaluate_gates`, `live_guidance`, `check_compatibility`); not `describe` |
| `provider.graph.invalid` | error | `run.create` only; semantically invalid graph on `provider.check` completes as invalid conformance finding |
| `provider.drift.detected` | error | `run.create` (between describe and validate) |
| `provider.evidence.malformed` | error | `run.request` |

### Run, event, and input

| Reason code | Class | Typical operations |
|---|---|---|
| `run.not_found` | rejected | run commands |
| `run.lifecycle.denied` | rejected | mutating run commands |
| `run.lifecycle.terminal` | rejected | `run.label`, `run.request`, `run.guidance`, `run.compatibility` |
| `event.unknown` | rejected | `run.request` |
| `gate.failed` | rejected | `run.request` |
| `compatibility.unsupported` | rejected | `run.request`, `run.guidance` |
| `guidance.unsupported` | rejected | `run.guidance` |
| `input.rejected` | rejected | `run.create` |
| `input.invalid` | rejected | `run.create`, evidence/input-bearing commands |
| `evidence.invalid` | rejected | `run.evidence.add`, `run.request` |
| `evidence.selection.invalid` | rejected | `run.request`, `run.guidance` |
| `label.invalid` | rejected | `run.label`, `run.create` |
| `note.invalid` | rejected | `run.annotate`, `run.terminate`, `run.request` |
| `actor.invalid` | rejected | `run.annotate` |

### Engine, pagination, and export

| Reason code | Class | Typical operations |
|---|---|---|
| `state.stale_version` | error | `run.request` and other versioned mutations |
| `persistence.failed` | error | any mutating operation |
| `cursor.invalid` | rejected | paged operations |
| `resource.exhausted` | error | pre-spawn request overflow, provider pages, trace budget |
| `export.target.invalid` | rejected | `run.export` |
| `export.target.not_empty` | rejected | `run.export` |

Completed operations with no denial or failure carry `"reason": null` per [cli-contract.md](cli-contract.md) (D006). Omission of `reason` is not permitted in structured mode.

## Caller-action mapping

Each desired caller action from [intent.md](intent.md) and [ux-storyboards.md](ux-storyboards.md) maps to exactly one operation.

| Caller action | Operation ID |
|---|---|
| Register explicit provider executable | `provider.add` |
| Inspect registrations or active-run impact | `provider.list` |
| Check provider protocol, emitted graph, or registration-wide active-run compatibility | `provider.check` |
| Change provider executable/argv/CWD/timeout under same registration ID | `provider.update` |
| Rename enabled provider handle | `provider.rename` |
| Tombstone provider registration (with acknowledgement when active runs exist) | `provider.disable` |
| Restore tombstoned registration ID | `provider.restore` |
| Create durable run from provider graph | `run.create` |
| Discover/list runs from any working directory | `run.list` |
| Inspect current work without provider execution | `run.show` |
| Inspect full stored graph projection | `run.graph` |
| Inspect ordered activity journal | `run.history` |
| Append evidence independently | `run.evidence.add` |
| List evidence inventory | `run.evidence.list` |
| Append note, actor metadata, or correction | `run.annotate` |
| Change active display label | `run.label` |
| Request named stored-graph event | `run.request` |
| Explicitly request live advisory guidance | `run.guidance` |
| Explicitly check per-run provider compatibility | `run.compatibility` |
| Explicitly close active run | `run.terminate` |
| Emit read-only audit export | `run.export` |

## Foundation outcome mapping

Each foundation outcome class and special completion rule maps to exactly one catalog rule.

| Foundation outcome | Catalog rule |
|---|---|
| Completed operation | Operation achieved its purpose; three-class `completed` |
| Domain rejection | Request understood and denied; `rejected` with stable reason code |
| Operation error | Evaluation or commit could not complete reliably; `error` with stable reason code |
| Successful explicit compatibility report | `completed` even when findings include incompatibility (`provider.check`, `run.compatibility`) |
| Invalid provider graph at creation | `error` (`provider.graph.invalid`); no run, no journal |
| Invalid provider graph on conformance check | `completed` with invalid conformance finding (`provider.check` default); not `provider.graph.invalid` |
| Rejected/error creation | No run, no run journal |
| Provider-catalog mutation success | Fresh `provider.list` proves new catalog state; no per-run journal |
| Provider-catalog mutation rejection | Fresh `provider.list` proves unchanged catalog; no per-run journal; trace records outcome |
| Rejectable run mutation after lookup | Fresh `run.history` proves rejection fact; state unchanged on rejection |
| Completed run transition | Fresh state + journal prove atomic advancement |

## Rationale for selected operations

### `provider.update`

Required by provider-drift storyboards ([ux-storyboards.md](ux-storyboards.md) storyboard 6) and [I8](invariants.md). Executable, arguments, working directory, and timeout may change outside caller control; engine must mutate registration atomically without rebinding run IDs or stored graphs. Frozen argv: `provider update <TARGET> --exec <PATH> [--arg <VALUE> ...] [--working-directory <PATH>] [--timeout <SECONDS>]`. `--exec` is required and replaces executable; supplied `--arg` list replaces argv (absence clears argv); omitted `--working-directory` and `--timeout` preserve existing values. Completion returns affected-run count and paged impact link so callers assess blast radius without unbounded ID lists. No approval gate: drift is allowed; incompatibility is reported at use time.

### `run.compatibility`

Satisfies [I39](invariants.md) per-run explicit check distinct from registration-wide `provider.check --active-runs` ([I13](invariants.md) journal facts for drift observations). Produces non-latching per-capability findings while preserving inspect/annotate/terminate and supported/gate-free `run.request` paths. Appends compatibility-attempt journal with provider locator/digest/version without mutating workflow state or latching compatibility.

### `run.export`

Resolves [D015](change/initial-implementation/decisions.md#d015--audit-export-scope): read-only `manifest.json` + `state.json` + `journal.jsonl` for inspection and regression artifacts without import, restore, replay, or locator dereference. Normative schemas, ordering, manifest hashes, atomic publication, and CLI `data.export` shape are frozen in [export-contract.md](export-contract.md). Supports testing doctrine export scenarios and reference-workflow audit needs without becoming write authority.

## Explicit non-operations

The following must **not** appear in core/driver/route/E2E/trace catalog equality (I25/I26, [coverage.md](change/initial-implementation/coverage.md)):

- CLI `--help`, `--version`, and pre-dispatch usage display
- `--list-operations` and driver operation-list metadata
- Provider protocol roles: `describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, `check_compatibility`
- Database migration, startup initialization, and schema generation invoked outside dispatched application operations
- `xtask` quality, hook, judge, architecture, report, and schema-generation commands
- Reference/scenario provider self-tests

## Verification rules (T004)

- Provider-catalog mutation: fresh-process `provider.list` is authoritative proof; **no** per-run journal at check time.
- Rejected or errored `run.create`: **no** run and **no** run journal.
- Rejectable run mutation after run lookup: fresh `run.history` (and state where applicable) proves rejection journaling and unchanged state on denial.
- Facet inventories validate against `quality/facets/v1/schema.json`; facet names match [testing.md](testing.md) exactly.
- Closure stages in [tasks.md](change/initial-implementation/tasks.md) compare runtime catalog sets; final stage requires exact D004 21-ID equality.
