# Initial Implementation Coverage Map

**Status:** Planned

This map prevents requirements from disappearing between foundation, implementation tasks, and runtime evidence. Task completion alone is not acceptance. Final evidence columns must name passing command, invocation/request ID, trace file/artifact, and exact commit.

The implementation task inventory is exactly T001–T200. Junction validation/fix governance tasks `V001`–`V013`/`F001`–`F013` (defined in `tasks.md`) sit outside T numbering: they execute accumulated deterministic verification at checkpoint junctions and batch resulting repairs, add no operations, own no coverage rows, and supply no first proof for any facet. Every evidence column in this map continues to cite T-task-owned suites and reports; junction evidence directories are untracked and never satisfy a row.

## Final operation catalog

Decision D004 must confirm this catalog before implementation. `Private task` builds internal operation. `Exposure task` adds production CLI route, catalogs, required E2Es, traces, and documentation atomically.

> **Amendment (2026-07-22):** exposure is staged per [tasks.md § Amended execution plan](tasks.md#amended-execution-plan-2026-07-22). Nine alpha operations (`provider.add`, `provider.list`, `provider.check`, `run.create`, `run.list`, `run.show`, `run.terminate`, `run.request`, `run.history`) expose in WP3; the remaining twelve expose in WP6. D004's 21-ID catalog is unchanged and `final` closure still requires all 21 at change close. `Exposure task` and cross-coverage T-number columns below identify contracts of record now executed inside work packages; aggregate rows T170/T182 follow the shared provider-failure family rule in [testing.md § Facet matrix](../../testing.md#facet-matrix), and T183 is owner-optional deferred.
>
> **Checkpoint A evidence:** `provider_add` production E2Es prove explicit registration, stable identity, duplicate rejection without mutation, fresh-process persistence, human/JSON parity, no provider invocation, no run journal, and correlated persistence traces. `provider_list` production E2Es prove enabled/tombstoned/all and zero-impact reads, invalid/filter-mismatched cursors, exact default/max count ceilings, byte-budget stops, cursor progress, complete records, no provider invocation, no run journal, and correlated read traces. Closed manifests are `quality/facets/v1/provider.add.json` and `quality/facets/v1/provider.list.json`.

| Operation ID | CLI intent | Characteristics | Private task | Exposure task | Main cross-coverage |
|---|---|---|---|---|---|
| `provider.add` | register explicit executable | Provider-catalog mutation; Rejectable provider-catalog mutation | T063 | T147 | T167, T177 |
| `provider.list` | inspect registrations or paged active-run impact | Read | T064 | T147 | T167 |
| `provider.check` | protocol/graph and registration-wide active-run conformance | Provider invoking; Read; Compatibility sensitive | T065 | T152 | T167, T170–T171, T175–T176 |
| `provider.update` | change current config under same ID | Provider-catalog mutation; Rejectable provider-catalog mutation | T066 | T155 | T167, T175, T177 |
| `provider.rename` | change enabled handle | Provider-catalog mutation; Rejectable provider-catalog mutation | T067 | T156 | T167, T175 |
| `provider.disable` | tombstone config and release handle | Provider-catalog mutation; Rejectable provider-catalog mutation | T068 | T157 | T167, T175–T176 |
| `provider.restore` | restore tombstoned ID/config | Provider-catalog mutation; Rejectable provider-catalog mutation | T069 | T158 | T167, T175 |
| `run.create` | describe/validate/snapshot new run | Successful creation; Rejected/error creation; Provider invoking | T070 | T152 | T168, T170–T171, T173 |
| `run.list` | list active/default or terminal/all | Read; Lifecycle family | T071 | T152 | T167, T174, T177 |
| `run.terminate` | explicitly close active run | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required | T082 | T152 | T167, T173–T174 |
| `run.show` | inspect current work | Read; Lifecycle family | T072 | T153 | T167–T168, T174–T176 |
| `run.graph` | inspect full stored projection | Read | T073 | T154 | T167–T168, T171, T175 |
| `run.history` | inspect ordered journal | Read | T074 | T152 | T167, T172–T175 |
| `run.evidence.add` | append independent evidence | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required | T075 | T160 | T167, T172–T174 |
| `run.evidence.list` | inspect evidence inventory | Read | T076 | T160 | T167, T172 |
| `run.annotate` | append note/actor/correction | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required | T077 | T161 | T167, T172, T174, T176 |
| `run.label` | change active display label | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Lifecycle family; Journal required | T078 | T162 | T167, T173–T175, T178 |
| `run.request` | request stored-graph event | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Provider invoking; Gate driven; Lifecycle family; Compatibility sensitive; Journal required | T079 | T163 | T167–T178, T182–T190 |
| `run.guidance` | explicitly request live advisory | Run-state or run-journal mutation; Rejectable run mutation after run lookup; Provider invoking; Lifecycle family; Compatibility sensitive; Journal required | T080 | T164 | T167, T170, T174–T176, T182 |
| `run.compatibility` | inspect current support for stored run | Provider invoking; Read; Lifecycle family; Compatibility sensitive; Journal required | T081 | T165 | T167, T170, T174–T176, T182 |
| `run.export` | emit read-only audit snapshot | Read | T083 | T166 | T167, T181 |

### Non-application functions

These must not enter operation-catalog equality:

- CLI help/version/schema display;
- driver operation-list metadata;
- provider protocol roles (`describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, `check_compatibility`);
- migrations and startup initialization;
- quality, hook, judge, architecture, report, and schema-generation commands;
- reference/scenario provider self-tests.

## Facet applicability

Every operation gets universal valid-path, operation-ID, request-ID, structured outcome, human parity, exit, trace start/result, and isolated fresh-process proof. Additional facets:

| Facet | Operations | Required tasks/evidence |
|---|---|---|
| Provider-catalog mutation | add/update/rename/disable/restore | exposure task; fresh-process `provider.list`; no run journal per I40 |
| Rejectable provider-catalog mutation | add/update/rename/disable/restore | exposure task; unchanged fresh catalog read; trace outcome; no run journal per I40 |
| Successful creation | create | T149/T152 fresh state and creation history; T173 aggregate atomicity |
| Rejected/error creation | create | T149/T152 prove no run and no run journal; T171 aggregate repetition |
| Run-state or run-journal mutation | evidence add/annotate/label/request/guidance/terminate | exposure task; T173 aggregate atomicity; fresh-process state/history |
| Rejectable run mutation after run lookup | evidence add/annotate/label/request/guidance/terminate | exposure task; T169 aggregate taxonomy; T173 aggregate unchanged/rollback |
| Provider invoking | check/create/gated request/guidance/compatibility | T148–T149/T152 and T163–T165 exposure facets; T170/T182 aggregate repetition |
| Gate driven | gated `run.request` | T163 exposure facets; T170/T172–T173/T178 aggregate repetition |
| Read | `provider.list`, `provider.check`, `run.list`, `run.show`, `run.graph`, `run.history`, `run.evidence.list`, `run.compatibility`, `run.export` | exposure task plus invalid/not-found; T169 |
| Lifecycle family | show/list/evidence add/annotate/label/request/guidance/compatibility/terminate | command-owned slice per `testing.md`; T174 repeats complete family |
| Compatibility sensitive | registration-wide check/gated request/guidance/per-run compatibility | T152 and T163–T165 exposure facets; T176 aggregate repetition |
| Provider-free under missing provider | list/show/graph/history/evidence add/list/annotate/label/terminate/gate-free request/export | each exposure task; T170/T175–T176 aggregate repetition |
| Journal required | creation; event/guidance/per-run compatibility attempts; evidence/annotation/label/termination | T108, T111–T115, T149–T151, T159, T161–T165, T173, T176 |
| Trace provider boundary | check/create/gated request/guidance/compatibility | T102, T148–T149, T152, T163–T165, T182 |
| Trace persistence boundary | every persistence access per contract | T118, exposure tasks, T182 |

Lifecycle row is family inventory, not demand that unrelated command repeat every scenario. Ownership is exact: C5 list/show/terminate own active/final/terminated visibility, neutral/initial/zero-final/sink shapes, termination note/repeat denial, terminal empty events, and absence of reopen; evidence add and annotate own allowed terminal append; label/request/guidance/compatibility own their terminal rejections. Each owner closes its facets before exposure; T174 only repeats integrated family.

## Settled invariant traceability

Before candidate commit, final evidence replaces `Planned` with stable report keys such as `acceptance:I1`; post-commit local/CI artifact maps each key to immutable `<sha>/<report-key>`. Tracked file never embeds its own SHA or mutable artifact URL.

| Invariant | Implementation tasks | Behavioral/deterministic proof | Final evidence |
|---|---|---|---|
| I1 Actor type cannot affect behavior | T035, T048–T049, T077, T079 | T051, T161, T188 | Planned |
| I2 Primary work remains external | T005, T043–T044, T056, T079–T081 | T137, T184, T185–T190 | Planned |
| I3 Core remains harness-agnostic | T017–T022, T031–T061 | architecture gate T021–T022, T184 | Planned |
| I4 Workflow authoring is code-only | T005, T084–T095 | T135–T142, T148, T152, T184 | Planned |
| I5 Providers own domain policy | T038–T044, T140–T142 | T184, reference suite | Planned |
| I6 Providers emit complete graph | T038–T041, T065, T070, T086, T091 | T148–T152, T168, T171 | Planned |
| I7 Every run snapshots graph | T038–T041, T045, T070, T073, T108–T109 | T149–T150, T152–T154, T168, T175 | Planned |
| I8 Provider drift allowed/logged | T042–T046, T066, T079–T081, T090–T102 | T155, T163–T165, T175–T176, T187 | Planned |
| I9 Gates authoritative | T039, T043, T049, T079, T093 | T137, T163, T170, T172–T173 | Planned |
| I10 One enforcement path | T048–T059, T079, T112–T113, T124 | T163, T173, T184 | Planned |
| I11 Rejected progress preserves state | T047–T049, T079, T112 | T163, T168–T173, T185–T189 | Planned |
| I12 Stored state authoritative | T045–T046, T057–T059, T105–T115 | T149–T165, T173, T179–T180 | Planned |
| I13 Journal immutable/ordered | T011, T046, T058–T059, T105, T114–T115 | T152, T165, T173, T176, T179–T180 | Planned |
| I14 State/journal atomic | T058–T059, T108, T111–T114 | T149–T152, T159, T161–T165, T173 | Planned |
| I15 Journal explains, not reproduces | T011, T042–T046, T115 | T152, T170, T175, T184, T194 | Planned |
| I16 Runs survive boundaries | T045, T057, T105–T115 | all exposure restarts; T149–T154, T177, T187 | Planned |
| I17 Notes/actor no authority | T035, T046, T077, T079 | T161, T188 | Planned |
| I18 Human/structured equivalence | T006, T047, T125–T128 | every exposure; T169 | Planned |
| I19 Ambiguity cannot advance | T040, T048, T079 | T149–T150, T163, T168 | Planned |
| I20 Provider execution explicit | T056, T065, T070–T081, T091–T095 | T148–T150, T152–T154, T159–T161, T163–T165, T170 | Planned |
| I21 Clean-room workflow | all tasks; README rules | T184 and source/artifact scan | Planned |
| I22 Three product crates | T017–T022, T031 | T021–T022, T184 | Planned |
| I23 Core dependencies inward | T021–T022, T031, T053–T061 | architecture canaries | Planned |
| I24 DTOs at boundaries | T031, T084–T086, T097, T106, T122, T125 | architecture/schema tests T022, T184 | Planned |
| I25 Every operation driver-covered | T062, T133, T146–T167 | T167 | Planned |
| I26 Catalogs mechanically closed | T062, T133, T145, T167 | exact-set report T167 | Planned |
| I27 Structured outcomes identify op | T047, T124–T125 | every exposure, T167, T169, T182 | Planned |
| I28 E2E behavioral authority | T143–T145, T146–T190 | acceptance reports T167, T190–T191 | Planned |
| I29 No mock behavioral tests | T013, T135–T145 | dependency/source audit T184, T191 | Planned |
| I30 Facet depth | exposure tasks, T168–T183 | facet report T191 | Planned |
| I31 Defect fixes get driver regression | T195 quality policy/docs | future gate check and contributor docs | Planned |
| I32 Inputs immutable/evidence append-only | T036–T037, T070, T075–T076, T079, T092 | T149–T150, T159–T160, T163, T171–T172 | Planned |
| I33 Minimal lifecycle | T045, T048, T072, T077–T082 | T151–T165, T168, T174 | Planned |
| I34 Three outcomes | T006, T047–T049, all operations | T169 plus every exposure | Planned |
| I35 Evidence independent/inline | T037, T059, T075–T076, T079, T110–T113 | T159–T160, T163, T172–T173, T189 | Planned |
| I36 One-current-caller overlap safety | T034, T045, T059, T079, T113, T119, T139 | T178 | Planned |
| I37 Stable identity/major protocol | T005, T033, T042, T055, T063–T070, T084 | T146–T158, T175 | Planned |
| I38 Narrow roles/conformance | T005, T043–T044, T055–T056, T084–T095 | T135–T148, T152, T163–T165 | Planned |
| I39 Compatibility/safe inspection | T044, T055–T057, T072–T081, T095 | T151–T165, T170, T175–T176, T187 | Planned |
| I40 Provider config authorizes execution | T007, T042, T055, T063–T070, T087–T098 | T146–T158, T170, T175, T177 | Planned |
| I41 Run identity not workspace identity | T033, T037, T045, T071–T078, T096, T116 | T149–T162, T172, T175, T177, T181 | Planned |
| I42 Evidence retention/no auto-copy | T008, T037, T099–T102, T110, T116 | T159–T160, T163, T172, T181–T182 | Planned |
| I43 Active runs current registration | T042, T055–T056, T066, T079–T081, T090 | T155, T163–T165, T175–T176 | Planned |
| I44 No retry keys/automatic retry | T005, T079, T087–T095, T121 | T163, T170, T178, T184 | Planned |
| I45 No individual deletion | T004, T045–T046, T083, T105, T121 | T166, T174, T181, T184 | Planned |
| I46 Every invocation traceable | T010, T054, T099–T102, T118, T120, T124 | every exposure; T167, T182, T189 | Planned |
| I47 Every publication checkpoint documentation-coherent | T012, T023–T029, R001–R005, T195, T198–T200 | aggregate judge/hook/CI tests and base-to-head publication reports | Planned |

## Reference workflow behavior traceability

| # | Required behavior | Provider tasks | Product/cross tasks | Dedicated proof | Final evidence |
|---:|---|---|---|---|---|
| 1 | Creation and safe inspection | T140 | T150, T152–T154 | T185 | Planned |
| 2 | Happy path to `end` | T140–T141 | T163, T174 | T185 | Planned |
| 3 | Missing output rejection | T141 | T163, T172 | T185 | Planned |
| 4 | Invalid output rejection | T141 | T163, T172 | T185 | Planned |
| 5 | Design revision cycle | T140–T141 | T163, T168 | T186 | Planned |
| 6 | Plan revision cycle | T140–T141 | T163, T168 | T186 | Planned |
| 7 | Implementation revision cycle | T140–T141 | T163, T168 | T186 | Planned |
| 8 | Validation revision cycle | T140–T141 | T163, T168 | T186 | Planned |
| 9 | Verdict consistency | T141 | T163, T170 | T186 | Planned |
| 10 | Append-only same-path evidence | T141 | T159–T160, T172 | T187 | Planned |
| 11 | Restart and handoff | T140–T142 | T149–T163 | T187 | Planned |
| 12 | Provider drift | T142 | T155, T163, T175 | T187 | Planned |
| 13 | Provider incompatibility | T142 | T165, T176 | T187 | Planned |
| 14 | Guidance/cold handoff | T142 | T152–T153, T160, T164, T172 | T188 | Planned |
| 15 | Actor neutrality | T141 | T161, T163 | T188 | Planned |
| 16 | Journal/state consistency | T141 | T112–T115, T163, T173 | T188 | Planned |
| 17 | Interaction contract | T140 | T150–T162, T174, T177 | T188 | Planned |
| 18 | Attempt evidence | T141 | T112–T113, T163, T172–T173 | T189 | Planned |
| 19 | Provider resolution | T142 | T150, T155–T158, T175, T177 | T189 | Planned |
| 20 | Automation envelope | T140–T142 | T125, T128, T169 | T189 | Planned |
| 21 | Operational visibility | T140–T142 | T099–T102, T118, T120, T124, T182 | T189 | Planned |

T190 mechanically verifies exactly 21 evidence-backed rows.

## Foundation-section ownership

| Foundation area | Primary tasks |
|---|---|
| Actor/harness neutrality | T031–T061, T077, T079, T161, T184, T188 |
| Provider authoring/protocol | T005, T084–T095, T135–T142, T148, T152 |
| Graph snapshot/validation | T038–T041, T070, T086, T091, T149–T150, T154, T168, T171 |
| Inputs/evidence | T036–T037, T070, T075–T076, T079, T110–T113, T149–T150, T159–T160, T163, T172 |
| State/lifecycle | T034, T045, T048–T050, T079, T082, T108–T113, T149–T165, T174, T178 |
| Journal/audit | T011, T046, T058–T059, T105–T116, T149–T152, T159, T161–T164, T173, T181 |
| Provider registration/drift | T042, T055, T063–T069, T090, T107, T146–T148, T152–T158, T175–T177 |
| CLI/outcomes | T004, T006, T047, T120–T134, T146–T169 |
| Operational trace | T010, T054, T099–T102, T118, T120, T124, T145–T182 |
| Testing authority | T013, T135–T145, T146–T191 |
| Documentation/judgment | T012, T023–T029, R001–R005, T192–T200 |
| Reference workflow | T140–T142, T185–T190 |

## Explicit exclusion audit

T184 must prove absence or deliberate non-exposure of each item:

| Excluded behavior | Evidence source |
|---|---|
| Declarative workflow DSL | command/schema/source audit |
| Agent invocation/primary work | dependency/source/provider-contract audit |
| Daemon/HTTP/server/async runtime | binaries/dependencies/help audit |
| Hierarchy/parallel/timers/child/compensation | graph schema and invalid-fixture E2Es |
| Mutable variables/expression language | model/schema/catalog audit |
| Event-sourced replay/reconstruction | persistence API/history/export docs audit |
| Provider sandbox/trust DB/discovery/registry/installer | command/config/dependency audit |
| Claims/leases/revision tokens/idempotency keys/retries | CLI schema/help and overlap behavior |
| Pause/resume/terminal reopen | lifecycle CLI/schema/E2Es |
| Individual deletion/silent compaction | catalog/help/persistence audit |
| Import/restore/cross-machine mobility | catalog/help/export schema audit |
| Active-run graph migration/gate bypass | catalog/help/compatibility E2Es |
| Official SDK requirement | distribution/docs audit |
| Pervasive tracing/compiler logging enforcement | architecture/dependency/source audit |
| Software-change vocabulary in core | core symbol/source audit |
| Generic `util`/`common`/repository/catch-all service | architecture source check |

## Final evidence record format

Generated T191 report is an untracked local/CI artifact emitted after candidate commit and keyed by actual SHA; tracked coverage contains requirement mapping and schema only, avoiding self-referential commits. Each row materializes as:

```json
{
  "requirement": "I46",
  "commit": "<sha>",
  "commands": ["cargo test ..."],
  "operation_ids": ["run.request"],
  "request_ids": ["..."],
  "trace_artifacts": ["..."],
  "other_artifacts": ["..."],
  "status": "pass"
}
```

No manually asserted `pass`, task checkbox, source-code presence, or lower-level test alone may satisfy behavioral rows.
