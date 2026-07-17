# Loop Engine System Invariants

**Status:** Living foundation. Settled invariants are normative. Candidate invariants are explicitly unresolved and must not be treated as decisions.

Normative terms **MUST**, **MUST NOT**, **SHOULD**, and **MAY** express requirement strength.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)

## Settled invariants

### I1. Actor type cannot affect behavior

Given identical authoritative run state, stored graph, request, submitted evidence, and observed provider verdicts, the engine **MUST** make the same decision regardless of whether the performer is a human, autonomous agent, script, or external system.

External provider execution **MAY** produce different observations at different times. The engine does not claim provider determinism.

### I2. Primary work remains external

The engine **MUST NOT** invoke a coding agent or intentionally perform workflow-defined primary work. It **MAY** invoke executable workflow providers for graph discovery, gate evaluation, and explicit advisory guidance.

Provider code is arbitrary executable content and cannot be assumed side-effect free, but provider output **MUST NOT** directly set engine state.

### I3. Core remains harness-agnostic

Core workflow types and transition logic **MUST NOT** depend on harness-specific APIs, session types, hooks, or lifecycle behavior.

MVP defines no harness/model execution-hint semantics. Callers may keep such preferences outside engine; core **MUST NOT** require or interpret them to decide transitions.

### I4. Workflow authoring is code-only

Every workflow **MUST** be supplied by an executable provider implementing the engine's provider protocol. Hand-authored YAML, JSON, TOML, or another declarative file **MUST NOT** be an independent workflow-authoring or runtime-policy source.

Future SDKs, generators, templates, or wrappers **MAY** lower provider-authoring cost, but they **MUST** resolve to the same provider protocol rather than create another workflow semantics.

### I5. Providers own domain policy

Every workflow-specific state, event, transition, gate declaration, guidance rule, evidence convention, and metadata convention **MUST** originate from the workflow provider. Core **MUST NOT** contain hidden policy such as requiring a proposal, review, commit, artifact, workspace, or particular tool.

### I6. Providers emit a complete graph projection

Provider description **MUST** emit normalized complete workflow graph without receiving candidate input values. Engine **MUST** validate graph before creating run. Semantically invalid, ambiguous, or unsupported provider graph is provider operation error and prevents creation.

For each state and event ID, graph **MUST** declare at most one transition. Gates allow or deny selected transition; they never choose among targets. Runtime multi-match check remains defense in depth.

Graph projection is engine-facing data, not second manually authored source.

### I7. Every run snapshots its graph

A run **MUST** store the validated graph emitted at creation. Active-run transition resolution **MUST** use that stored graph.

Final states **MUST NOT** declare outgoing transitions. Graph **MAY** declare zero, one, or several final states; entering any final state completes run. Zero-final graph is intentionally ongoing and can become terminal only through explicit termination. Initial state that is also final creates immediately final run. Non-final state with no outgoing transitions is valid explicit terminate-only trap.

Every active/actionable state **MUST** include provider-free static guidance or explicit declaration that no additional guidance is required. Description **MUST** also declare whether live guidance is supported. Engine validates presence, not subjective quality. Stored declaration governs active run and is part of canonical projection.

Later provider graph changes **MUST NOT** silently alter active-run states, events, transitions, or gate declarations. They apply to new runs unless an explicit migration operation is designed later.

### I8. Provider implementation drift is allowed and logged

An active run **MAY** invoke a provider implementation whose identity or digest differs from earlier invocations. Engine **MUST** record provider identity and available digest/version information whenever provider outcome can be durably recorded. Abrupt process death may prevent engine from observing or recording that invocation.

Provider drift **MUST NOT** require approval. Active runs **MUST** resolve provider-dependent operations through current registration executable. Stored topology, gate IDs, guidance, and capability declarations are fixed; gate implementation/policy remains live and is not historically certified. Current provider **MUST** return incompatibility when it no longer honors stored declarations/guidance contract. Gate-evaluation result **MUST** be exactly one of: complete pass/fail verdict set, explicit stored-graph incompatibility, or evaluation error. Explicit incompatibility produces domain rejection; failed/malformed/missing result produces operation error.

### I9. Gates are authoritative

When stored graph declares gates for a transition, engine **MUST** request all required gate verdicts in one provider invocation. Provider result **MUST** contain exactly one pass/fail verdict for every requested gate and no substituted gate identity.

One or more fail verdicts produce domain rejection. Explicit incompatibility result produces domain rejection with compatibility reason. Provider inability to produce complete result—including unavailable executable, timeout, crash, explicit evaluation error, missing verdict, or malformed output—produces operation error. Neither outcome permits transition.

Transition with no declared gates **MUST NOT** invoke provider; engine decides it from stored graph and lifecycle alone.

### I10. State changes pass through one enforcement path

Ordinary callers and providers **MUST NOT** directly set current workflow state. Every MVP workflow transition **MUST** pass through engine-controlled graph resolution, validation, persistence, and journal recording. Engine-owned lifecycle, label, evidence, and note operations follow their own declared validation path. MVP exposes no administrative state override.

### I11. Rejected progress preserves current state

If a requested transition is rejected, the run **MUST** remain in its prior workflow state. The attempt, provider evaluations, and rejection reason **MUST** be recorded without implying that target state became active.

### I12. Stored current state is authoritative

Current run state and lifecycle **MUST** be persisted directly and **MUST** be authority used for inspection and future operations. Internal record versions **MAY** support atomic persistence but are not caller-managed workflow tokens.

The journal **MUST NOT** be required to reconstruct current state by replay.

### I13. Journal entries are immutable and ordered

When persistence remains available, engine **MUST** maintain append-only ordered journal of meaningful run activity, including run creation, completed/rejected/errored event and live-guidance requests after run lookup, provider gate evaluations, lifecycle and label changes, evidence attachments, notes, and observed provider drift. Abrupt process/persistence failure is limited as stated in I15/I35.

Corrections and clarifications **MUST** append linked entries rather than edit prior entries.

### I14. State and journal change atomically

Every durable run mutation that requires journal explanation—including creation, transition, termination, label change, independent evidence, and note—**MUST** commit authoritative record and corresponding journal entry atomically. Every required attempt-only journal record **MUST** commit atomically with attached submitted/provider evidence even when workflow state does not change. Engine **MUST NOT** expose or report partial commit. Persistence failure **MUST** fail operation.

### I15. Journal explains; it does not reproduce

Journal information **MUST** identify what engine durably observed: operation, timestamp, internal record version, state before and after where relevant, requested event, provider identity, gate verdicts, bounded diagnostics, evidence references, outcome, and optional actor note.

Explainability covers stored engine observations and submitted bounded records. Gate-attempt entries **MUST** surface provider identity/digest so live policy drift is observable. It **MUST NOT** imply future availability of external evidence content, prior provider code, or unchanged gate policy. Engine **MUST NOT** promise deterministic replay, exact historical re-execution, full historical-state reconstruction, or complete record of provider process started immediately before abrupt engine-process death.

### I16. Runs survive process and actor boundaries

All authoritative state, stored graph, provider-registration identity, and journal information needed to continue a run **MUST** be durably persisted. Correct continuation **MUST NOT** depend on memory held by previous CLI process, agent session, human performer, or caller working directory.

### I17. Notes and actor metadata do not grant authority

Actors **MAY** attach optional notes and opaque identity metadata. Opaque actor metadata **MUST NOT** satisfy gate or authorize transition. Separately submitted evidence **MAY** contain independently verifiable role or credential facts that provider validates as workflow policy.

Core transition semantics **MUST NOT** branch on inferred actor type.

### I18. Human and structured interfaces are equivalent

Human-readable and machine-readable CLI modes **MUST** expose same underlying state, graph, operations, provider outcomes, and journal meaning. Presentation format **MUST NOT** create privileged transition path.

After operation dispatch, structured mode **MUST** write exactly one authoritative outcome envelope to stdout for completed, rejected, and error results. Always-on operational trace **MUST** remain in separate file and **MUST NOT** contaminate stdout. Stderr is reserved for rich failures before dispatch or inability to construct envelope.

### I19. Ambiguity cannot advance a run

If a requested event selects no transition or more than one transition in the stored graph, the engine **MUST NOT** guess. It **MUST** reject the request with an actionable diagnostic.

### I20. Provider execution is explicit

Validation that only checks stored engine data **MUST NOT** execute provider code. Creating a run, evaluating gates, explicitly checking compatibility, or explicitly requesting live provider guidance **MAY** execute provider code and **MUST** make that effect visible to the caller.

Ordinary inspection of an existing run **MUST** use stored state and graph without provider execution. Live guidance **MUST NOT** be hidden inside passive inspection.

### I21. Clean-room project workflow

Project work **MUST NOT** import prior loop-specific implementations or artifacts. OpenSpec-related skills, commands, and artifacts **MUST NOT** be used for this project.

### I22. Product code uses three inward-pointing crates

Product code **MUST** be divided into `loop-engine-core`, `loop-engine-integrations`, and `loop-engine-cli`. Core **MUST NOT** depend on integrations or CLI. Integrations **MAY** depend on core. CLI **MAY** depend on both and **MUST** be the composition root.

Non-shipping build tooling does not count as a product architecture crate.

### I23. Core internal dependencies point toward the model

Within core, operations **MAY** depend on capabilities and model; capabilities **MAY** depend on model; model **MUST NOT** depend on operations or capabilities.

### I24. External representations translate at boundaries

Provider protocol JSON, engine configuration, persistence rows, journal exports, CLI arguments, and CLI responses **MUST** translate through integration or delivery DTOs. Core model types **MUST NOT** carry Clap, Rusqlite, protocol, parser, or persistence-format annotations.

### I25. Every operation has production-driver coverage

Every application operation **MUST** have a stable operation identifier, **MUST** be reachable through at least one production driver, and **MUST** be observed executing with a correlated operational-trace envelope in at least one passing black-box end-to-end scenario. CLI is the only current production driver.

### I26. Operation completeness is mechanically closed

The core operation catalog, union of driver-supported operations, operations observed in passing required end-to-end scenarios, and operation envelopes observed in their trace files **MUST** agree. A test declaration **MUST NOT** count without runtime evidence from the production driver and trace.

### I27. Structured outcomes identify the operation

Every structured CLI outcome produced after operation dispatch, including completed, domain-rejected, and operation-error outcomes, **MUST** contain stable identifier of operation actually executed.

### I28. End-to-end tests are behavioral authority

Only black-box tests through a production driver **MAY** satisfy behavioral acceptance and regression requirements. Lower-level tests **MUST NOT** substitute for missing production-driver coverage.

Pure core property tests **MAY** supplement end-to-end coverage when they explore useful combinatorial invariants.

### I29. Mock-based behavioral tests are prohibited

Required behavioral tests **MUST** use production CLI, persistence, and provider-process integrations. Mock frameworks and mock-based behavioral tests **MUST NOT** be used.

Controlled executable providers, temporary filesystems, legacy data fixtures, and deliberately corrupted fixtures are permitted test inputs.

### I30. End-to-end depth follows operation facets

Every operation **MUST** have a primary valid-path CLI scenario. State-mutating, rejectable, provider-invoking, lifecycle, read, and compatibility-sensitive operations **MUST** additionally cover applicable facets defined in [testing.md](testing.md).

### I31. Defect fixes carry driver-level regression proof

Every corrected behavioral defect **MUST** add or identify a production-driver scenario that reproduces the defect against faulty behavior and passes after correction.

### I32. Run inputs are immutable and evidence is append-only

Input-free provider description **MAY** declare zero or more run inputs and **MUST** return graph/static guidance/live-guidance capability. Separate value-only input-validation operation receives candidate values and **MUST NOT** return or alter projection; it is skipped when no input declarations or candidate values exist. Description and validation use same resolved registration configuration and observed executable digest when available; detected change errors without creating run. Digest is best-effort audit fact and does not identify interpreted dependencies or ambient environment. Invalid or missing required input produces domain rejection. Engine validates protocol-level shape and bounds.

Accepted input values **MUST** be fixed at run creation, provider-free inspectable, and unsuitable for secrets. Credentials belong in environment or external secret systems. Inputs **MUST NOT** become general mutable workflow-variable bag. Input-independent topology is deliberate MVP limit; separate registration represents alternate graph. Evidence **MAY** be submitted or produced as run progresses and **MUST** accumulate through append-only records. Corrections **MUST** add new evidence or journal entries rather than silently replace historical evidence.

Core **MUST NOT** require artifacts, files, repositories, workspaces, or any other workflow-specific input/evidence concept.

### I33. Run lifecycle is minimal

MVP run lifecycle **MUST** expose only three concepts:

- active run that may receive event requests;
- final run when stored graph enters final state;
- explicit termination by caller.

Final is lifecycle-neutral. Domain meaning such as success, decline, cancellation, or failure comes from final state ID and provider-defined metadata, not lifecycle class.

Final and terminated runs **MUST NOT** reopen or accept events and **MUST** report empty requestable-event set. They **MUST** remain inspectable and **MAY** accept append-only notes and evidence for audit correction. Termination **MAY** include caller note.

MVP **MUST NOT** add pause/resume status, background activity status, work claims, or leases. Process or actor absence does not change active-run lifecycle.

### I34. Public outcomes use three semantic classes

Every dispatched operation **MUST** produce one of three semantic outcome classes:

- completed operation;
- domain rejection;
- operation error.

Domain rejection means request was understood and evaluated but denied by stored graph, lifecycle, failed gate verdict, invalid caller input/evidence selection, unsupported guidance, or explicit provider-declared capability incompatibility. Operation error means provider-dependent request could not be reliably evaluated or any request could not be committed, including tombstoned/missing registration or executable, unsupported protocol major, invalid provider graph, detected creation-time provider drift, provider crash/timeout/evaluation error, malformed protocol/evidence output, stale workflow-state version, or persistence failure. Gate-free events and other provider-free operations do not require usable registration/executable.

Explicit provider graph or compatibility check that successfully obtains findings is completed operation even when findings report invalidity/incompatibility. Run creation with invalid provider graph is operation error; provider-dependent event blocked by incompatibility is domain rejection.

Detailed reason codes **MUST** preserve actionable recovery information. Run-operation output **MUST** report current state, whether state identifier changed, and next requestable events whenever run remains readable. Completed self-loop reports state unchanged while journal records applied transition. “Requestable” **MUST NOT** imply required gates will pass.

### I35. Evidence supports independent and inline submission

Every accepted evidence record **MUST** receive stable run-scoped ID and remain provider-free listable with kind, locator, digest/metadata, and prior event associations. Caller owns selection: default is empty, and caller **MUST** be able to append evidence independently, select existing IDs for later event request, and include new evidence inline. Static guidance describes required evidence kinds/conventions but cannot name future IDs; live guidance **MAY** recommend existing IDs. Engine **MUST NOT** auto-select, and empty selection remains valid input to evaluation. Only selected existing evidence plus inline evidence enters gate context; unrelated history does not. Missing ID or selected context exceeding configured bound rejects before provider invocation; selected evidence **MUST NOT** be truncated.

Syntactically valid event request becomes journalable after engine loads run, including unknown event and terminal-lifecycle denial. When persistence remains available, completed transitions, domain rejections, and later operation errors **MUST** durably record attempt with submitted inline evidence, selected-evidence associations, and any valid provider evidence. Outcome **MUST** distinguish which of those categories committed; recorded evidence never implies provider side effects.

Failures before run lookup need not create run journal entry. Rejected creation creates no run and therefore no run journal. Persistence failure or abrupt process death may prevent record; outcome, when one can be emitted, **MUST NOT** claim evidence was recorded. Attempt record and attached evidence **MUST** commit atomically.

### I36. Same-run interaction assumes one current caller

MVP **MUST** target one local user operating one or several independent runs. Public interaction **MUST** assume one current caller operates each run and refreshes stored state as work progresses. Callers **MUST NOT** provide or manage optimistic revision tokens.

Engine **MUST** serialize durable mutations and preserve state/journal atomicity if processes overlap. Provider verdict may commit transition only when workflow state/lifecycle version still matches snapshot evaluated by provider; stale evaluation produces operation error and never advances run. Label, note, and append-only evidence mutations do not invalidate in-flight gate evaluation. Intentional concurrent same-run collaboration is not MVP workflow semantics. MVP **MUST NOT** introduce persistent work claims, leases, or actor locks.

### I37. Workflow identity is stable and protocol compatibility is major-versioned

Machine-local provider registration ID **MUST** be immutable logical workflow identity stored by runs. Enabled registration has mutable human handle unique among enabled registrations; commands **MAY** resolve handle to ID. Disabling executable configuration tombstones ID and releases handle without rebinding existing runs. Tombstoned registration is addressed/restored by ID and restore requires currently free handle. New registration **MUST NOT** reuse old ID; handle reuse never captures runs because they store ID. Executable locator and observed digest **MUST NOT** define identity. Same executable **MAY** back several registrations.

Engine **MUST** compute graph-revision identity from canonical validated graph projection. Provider **MAY** supply human-readable version/build metadata, but it **MUST NOT** define authoritative graph identity.

Provider protocol additions within a major version **MUST** remain compatible. Breaking protocol semantics **MUST** require new major version. Engine **MUST NOT** require exact provider/engine release-version equality.

### I38. Provider roles are narrow and conformance is supported

Provider protocol **MUST** expose narrow operations for:

- input-free graph/input/static-guidance/live-guidance-capability description;
- value-only create-time input validation;
- required-gate evaluation and verdict/evidence reporting;
- explicitly requested live advisory guidance;
- compatibility checking against active stored graph.

Gate evaluation **MUST** receive bounded engine-supplied snapshot containing workflow/graph identity, lifecycle/current state, selected event/transition/gates, immutable inputs, inline evidence, and caller-selected prior evidence records/references. Live guidance receives same context without transition authority and may likewise name relevant evidence. Compatibility receives stored graph identity and declarations needed to report support. Gate evaluation may return bounded evidence with complete verdicts; valid provider evidence persists atomically for both pass and fail attempts. Incompatibility/evaluation-error results carry diagnostics, not evidence. Malformed provider evidence makes provider result operation error. Live guidance does not append evidence. None may directly mutate engine state. Engine distribution **MUST** provide protocol/schema examples, actionable validation diagnostics, and provider conformance checks for handshake/framing, operation result shapes, graph validity, and active-run compatibility. Conformance **MUST NOT** claim provider gate logic is semantically correct. MVP **MUST NOT** require official language SDK.

### I39. Provider compatibility checking preserves safe inspection

Provider compatibility **MUST** be checked only by explicit compatibility operation or as part of provider-dependent use. Ordinary inspection **MUST NOT** execute compatibility check.

When current provider cannot support requested capability from active stored graph, run **MUST** remain inspectable, annotatable, and terminable. Explicit compatibility check is non-latching report with structured per-run/per-capability findings, including mixed results. Provider-dependent request rejects only when selected event/guidance needs unsupported capability; supported and gate-free events remain usable. Per-run live guidance and compatibility execution target active runs only; terminal-run request rejects by lifecycle. Registration-wide compatibility report does not append each run journal and creates no latched compatibility state. MVP **MUST NOT** offer gate bypass or active-run graph migration; caller restores compatible provider or creates new run.

### I40. Provider configuration is execution authorization

Provider registrations **MUST** be machine-local catalog records, not ambient project definitions. Executable locator, arguments, and working directory **MUST** be explicitly configured on registration. Project configuration **MAY** provide CLI defaults or registration references but **MUST NOT** redefine registration selected by existing run. Configuring locator authorizes provider execution with caller's operating-system permissions. Caller working directory **MUST NOT** alter provider invocation.

Disabling registration used by active runs **MAY** proceed only after warning names affected runs. Stable registration identity **MUST** remain restorable by ID while referenced. Registration changes persist as catalog state but need no immutable per-run audit. Safe inspection and annotation **MUST** remain available without executable configuration. Provider timeout **MUST** be configurable per registration or invocation.

Engine **MUST NOT** claim sandboxing and MVP **MUST NOT** add separate trust database, provider scanning, automatic project discovery, package registry, provider installer, or import manifest.

### I41. Run identity is not workspace identity

Every run **MUST** have stable engine identity independent of repository, workspace, artifact path, or provider executable path. Moving external work location **MUST NOT** change run identity, though immutable references may become unusable. Provider may document append-only evidence remap convention; otherwise caller restores location or creates new run. Durable handoff assumes referenced external resources remain valid and does not promise automatic rebinding.

MVP **MUST** maintain machine-local run catalog addressable independent of caller working directory. Default listing **MUST** show active runs; explicit option **MUST** expose final and terminated runs.

Run **MAY** have optional non-unique display label. Label **MUST NOT** be identity or valid ambiguous command target. Label **MAY** change while run is active, with journal record, and **MUST NOT** change after terminal lifecycle.

MVP **MUST NOT** promise run import, restore-from-export, or cross-machine mobility. Read-only state/journal export for inspection **MAY** be provided and is never competing authority.

### I42. Evidence retention does not copy external content automatically

Engine **MUST** durably store bounded evidence record and structured provider diagnostics exactly as accepted from caller or provider. Provider authors and callers remain responsible for secrets intentionally submitted in those records.

Always-on rotating operational traces **MUST** retain all bounded engine/provider payloads without redaction, including accepted inputs/evidence context, configured invocation arguments, protocol requests/results, and captured provider stdout/stderr. Inherited process environment is not engine payload and **MUST NOT** be copied into trace. Trace retention does not convert payload into evidence or journal authority, and rotation may remove old trace files.

When evidence contains external locator, engine **MUST NOT** automatically dereference, copy, or retain external content. Locator used across handoff **MUST** be self-contained (for example absolute URI) or use provider-documented scheme resolved from immutable run input; caller-CWD-relative meaning is not portable. Journal **MUST** record submitted evidence reference and available digest/metadata. External content availability remains caller/provider responsibility.

### I43. Active runs use current registration

Provider-dependent operation on active run **MUST** resolve current executable, arguments, and working directory from stable workflow registration. Creation-time locator **MUST NOT** pin active run. Each durably observed invocation **MUST** record actual locator and available executable digest/version.

New provider graph affects new runs only. Active run continues using stored graph. Canonical graph digest covers full stored projection, including topology, gate declarations, input declarations, static guidance, and live-guidance capability.

### I44. MVP has no retry-key or automatic-retry contract

MVP **MUST NOT** require or accept caller idempotency keys for event requests and **MUST NOT** automatically retry provider-dependent operations. Provider roles, especially gates, **MUST** evaluate/report rather than serve as supported primary-work executor and **MUST NOT** be relied on to provide exactly-once external side effects. After uncertain interruption, caller uses stored state and history before deciding whether to issue another request; absent record does not prove provider process never started.

### I45. Individual runs are not deleted

MVP **MUST NOT** provide operation that deletes individual run, graph snapshot, evidence history, or journal. Terminal lifecycle plus catalog filtering is retention behavior. MVP accepts unbounded local growth, performs no silent compaction, and sets no large-scale performance promise. Future destructive retention policy requires separate explicit design.

### I46. Every CLI invocation is operationally traceable

CLI **MUST** create one current-user-only structured JSONL trace file per invocation before operation dispatch. Failure to initialize secure trace **MUST** fail invocation before provider execution, persistence mutation, or operation dispatch. Public outcome/error **MUST** expose request ID and trace location when trace exists; trace-initialization error **MUST** remain rich on stderr.

Operation dispatcher **MUST** record request/outcome envelope for every operation. Provider integration **MUST** record invocation identity, bounded request/result/stdout/stderr, and execution outcome. Persistence integration **MUST** record attempted transaction, version check when applicable, and commit/rollback outcome. Consequential transition/gate/lifecycle decisions not explained by boundary payloads **MUST** emit targeted trace events. Trace sink rotates by configurable bounded file count and total bytes, never removes open trace, and is operational diagnostics rather than authoritative state/journal.

Abrupt process/machine failure, storage failure, and rotation can limit trace completeness; engine **MUST NOT** claim impossible complete observation. Instrument stable choke points rather than require every function to log.

### I47. Every commit is documentation-coherent

Every commit **MUST** independently leave relevant documentation coherent with behavior, architecture, contracts, testing policy, and development policy it introduces. Versioned semantic judge **MUST** evaluate exact parent-to-commit diff and resulting documentation through generic replaceable executable contract. Candidate commit is judged by parent revision's rubric; accepted rubric change applies to following commit. Bootstrap rule: initial foundation commit and first publication **MAY** proceed through explicit owner approval without parent rubric or judge executable; that commit becomes parent rubric for every following commit.

Determinate local judge failure **MUST** block commit. Local unavailable/indeterminate judge **MAY** warn and allow commit. Before publication, every commit **MUST** receive determinate pass; fail, unavailable, or indeterminate result blocks pre-push/authoritative gate. Later commit **MUST NOT** substitute for required same-commit documentation. Deterministic formatting/link/schema checks remain separate and cannot replace semantic judgment.

## Candidate invariants pending decision

These are current recommendations, not settled requirements.

### C1. Control plane ships as one native executable

Candidate: `loop-engine` requires no daemon or external database. Workflow providers may explicitly depend on shells, language runtimes, binaries, services, or network resources.

### C2. State and journal use SQLite with JSON/JSONL export

Candidate: SQLite supplies local transactions, locking, and querying while stable JSON and JSONL exports keep state and journal inspectable.

### C3. Exact revisions are gated before publication

Candidate: versioned local hooks run canonical checks, and an eventual protected remote requires the same gate before merge or release.

### C4. Build tooling uses a Rust `xtask`

Candidate: non-shipping `xtask` installs/version-checks hooks, runs the canonical quality gate, and coordinates exact-revision checks.

## Explicit non-invariants

The following are not requirements:

- workflow authoring uses YAML, JSON, TOML, or another declarative DSL;
- journal is source of truth for current state;
- current state can be reconstructed by replay;
- provider execution is deterministic;
- prior provider code remains available forever;
- provider implementation is pinned for active runs;
- provider graph changes apply automatically to active runs;
- MVP migrates active-run graphs or bypasses incompatible gates;
- engine sandboxes arbitrary provider code;
- engine maintains a separate provider-trust database;
- providers are discovered by scanning projects or consulting a registry;
- an official language SDK is required;
- exact provider and engine release versions must match;
- old provider-protocol majors are supported forever;
- runs have pause/resume status, work claims, or leases;
- callers manage optimistic revision tokens;
- intentional concurrent callers on same run are coordinated by engine;
- provider invocation inherits caller working directory;
- creation-time executable locator pins active run;
- event requests carry idempotency keys or retry automatically;
- terminal runs reopen or accept events;
- individual runs are deleted in MVP;
- repository or workspace path defines run identity;
- MVP guarantees cross-machine run mobility;
- all implementation, providers, and dependencies must be Rust;
- an actor must identify as human or agent;
- a workflow must use a particular coding harness;
- workflows must use artifacts, files, repositories, or workspaces;
- workflows must be acyclic;
- every CLI read creates a journal entry;
- every behavioral rule also has a unit test;
- lower-level tests compensate for missing CLI coverage;
- declared test-coverage labels prove execution;
- candidate recommendations are already decided.
