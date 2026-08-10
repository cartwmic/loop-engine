# Loop Engine Interaction Storyboards

**Status:** Normative product interaction semantics. Stable operation IDs and CLI argv are defined in [operation-catalog.md](operation-catalog.md). Structured envelope field names, human parity mapping, and exit codes are defined in [cli-contract.md](cli-contract.md).

Related documents:

- [Product intent](intent.md)
- [System invariants](invariants.md)
- [Code architecture](architecture.md)
- [Journal entry contract](journal-contract.md)
- [Testing doctrine](testing.md)
- [Reference workflow](reference-workflow.md)

## Interaction model

Humans, agents, scripts, and external systems use same operations and outcomes.

MVP assumes one current caller operates a run and refreshes stored state while work progresses. CLI does not expose optimistic revision tokens, claims, leases, idempotency keys, or automatic retries. Engine serializes durable writes; stale provider evaluation cannot overwrite newer state if processes accidentally overlap.

Primary work occurs outside engine. Engine shows current work, accepts notes/evidence, evaluates requested events, persists authoritative state, and explains result.

## Outcome vocabulary

Every dispatched operation has one semantic outcome:

- **completed**: requested operation achieved its purpose;
- **rejected**: request was understood and evaluated, but domain validation denied it;
- **error**: operation could not be reliably evaluated or committed.

Rejections include unavailable stored-graph events, lifecycle denial, failed gates, invalid caller input/evidence selection, unsupported guidance, and explicit provider-declared capability incompatibility. Errors include invalid provider graph, detected creation-time provider drift, and—for provider-dependent requests—tombstoned/missing registration or executable, unsupported protocol major, crashed/timed-out/erroring provider, malformed protocol/provider-evidence result, stale evaluated state, and persistence failure. Successful explicit compatibility check completes even when report contains incompatibility findings.

Operation completion is distinct from run completion. A `completed` inspection or event request may leave run lifecycle `active`.

Detailed reason code explains exact denial/failure. Every dispatched result exposes request ID and operational-trace location. Run-related result reports current lifecycle/state, whether state identifier changed, and next **requestable** events when run remains readable. Completed self-loop reports unchanged state while history records transition. Requestable means graph permits event request from current state; it does not promise gates will pass.

## Operational trace contract

Normative JSONL v1 schema, permissions, budget, and late sink-failure rules: [operational-trace.md](operational-trace.md).

Every CLI invocation initializes one current-user-only structured JSONL trace before operation dispatch. Failure to initialize secure trace stops invocation before provider execution, persistence mutation, or dispatch and emits rich stderr failure. When trace exists, request ID correlates human/structured result with exact file.

Trace retains all bounded engine/provider payloads without redaction, including accepted inputs/evidence context, configured provider arguments, protocol requests/results, and captured stdout/stderr. Inherited process environment is excluded. Callers/provider authors must treat trace directory as sensitive; rotation limits retention but trace is not encrypted, evidence authority, or run journal.

Normal output stays focused. Trace captures operation start/result, provider execution, persistence outcome, and consequential transition/gate/lifecycle decisions. Abrupt death may leave final marker absent; last flushed pre-effect marker identifies latest engine-observed phase.

## Storyboard 1: Register and check provider

Provider author distributes executable plus explicit registration command. No project scanning, import manifest, package registry, installer, or automatic discovery is involved.

Illustrative interaction:

```console
$ loop-engine provider add software-change \
    --exec ./software-change-provider \
    --working-directory /work/provider

Outcome: completed
Registration ID: 01J-WORKFLOW...
Handle: software-change

$ loop-engine provider check software-change

Outcome: completed
Protocol major: 1
Graph: valid
Graph revision: sha256:91ab...
```

Immutable machine-local registration ID is logical workflow identity. Mutable handle is unique among enabled registrations and resolves to ID for convenient commands. Registration explicitly configures executable, argument vector, and working directory. Active run stores ID; caller CWD or project defaults cannot rebind it. Handle rename preserves ID. Disabling executable configuration tombstones ID and releases handle; restore targets ID and assigns any free handle. Reusing handle never captures runs because they store ID. Same executable may back several registrations.

Engine computes graph revision from full canonical validated projection, including topology, gates, input declarations, static guidance, and live-guidance capability. Provider path, observed executable digest, and self-reported build version are best-effort audit facts, not workflow/graph identity and not proof of interpreted dependencies or environment.

Boundary conformance checks:

- protocol handshake/framing;
- result shapes for each provider operation;
- emitted graph validity;
- compatibility with selected active graph snapshots.

Explicit graph check completes with valid/invalid finding when provider result is obtainable; run creation from invalid graph errors. Conformance does not certify semantic correctness of provider gate/guidance code. Provider author owns those tests.

## Storyboard 2: Create, discover, and inspect run

```console
$ loop-engine run create software-change \
    --label checkout-redesign \
    --inputs inputs.json

Outcome: completed
Run: 01J...
Label: checkout-redesign
Lifecycle: active
State: explore

$ cd /another/directory
$ loop-engine run list

RUN     LABEL                WORKFLOW          LIFECYCLE  STATE
01J...  checkout-redesign    software-change   active     explore

$ loop-engine run show 01J...

Run: 01J...
Label: checkout-redesign
Lifecycle: active
State: explore

Guidance:
  Clarify intent and unresolved constraints.

Requestable events:
  intent-ready
```

Machine-local catalog resolves stable run ID independently of caller working directory. Default listing shows active runs; explicit terminal/all filter reveals final and terminated runs.

Optional label is non-unique display/search metadata, never run identity or ambiguous command target. Label may change while active and each change is journaled. Terminal label remains fixed.

`run show` is provider-free and current-work focused. It displays lifecycle, current state, inspectable immutable non-secret inputs, stored static guidance (or explicit no-additional-guidance marker), required gates per requestable event, live-guidance capability, caller-owned evidence-selection default (`none`), and requestable events. Separate provider-free graph inspection exposes full stored projection; history remains separate.

Run inputs are optional and declared by input-free description. Separate value-only operation validates candidate values during creation and cannot return projection; it is skipped when no declarations or values exist. When both calls run, they use same resolved registration and observed executable digest when available. Invalid/missing input rejects; digest drift errors creation. Accepted values are immutable, provider-free inspectable, and unsuitable for secrets. MVP graph does not vary with input values; alternate topology uses another registration. Exact descriptor/wire representation remains protocol-design detail.

### Provider-free history and evidence inventory

```console
$ loop-engine run history 01J...
$ loop-engine run evidence list 01J...

EVIDENCE          KIND    LOCATOR                              DIGEST
01J-EVIDENCE...   design  file:///work/checkout/docs/design.md sha256:...
```

History and evidence inventory execute no provider code. Each history row is one immutable aggregate journal entry per [journal-contract.md](journal-contract.md) (sequence-ordered, append-only, non-replay). Stable evidence IDs, kinds, locators, digests/metadata, and prior event associations support cold-session handoff. Selection defaults empty and remains caller-owned; static/live guidance may recommend IDs but engine never auto-selects.

### Explicit live guidance

```console
$ loop-engine run guidance 01J...

Outcome: completed
Provider executed: yes
Guidance: Address unresolved rollback risks before review.
```

Live guidance is explicit and advisory. Every guidance request after run lookup is journaled when persistence remains available, including unsupported/lifecycle rejection and provider error; completed invocation records provider digest. Stored projection declares support. If unsupported, request rejects with `guidance.unsupported` without executing provider. Live guidance targets active runs only and cannot append evidence.

## Storyboard 3: Append evidence and request event

Caller performs primary work externally, then may append evidence independently:

```console
$ loop-engine run evidence add 01J... \
    --kind design \
    --ref file:///work/checkout/docs/design.md \
    --digest sha256:...

Outcome: completed
Evidence recorded: yes
```

Later event request may select previously appended evidence records and/or include new evidence inline:

```console
$ loop-engine run request 01J... approved \
    --evidence-id 01J-EVIDENCE... \
    --evidence evidence.json \
    --note "Design review completed"
```

Engine flow:

1. Load authoritative state/lifecycle, workflow-state/lifecycle version, stored graph, immutable inputs, caller-selected existing evidence, and inline evidence.
2. Resolve named event against stored graph.
3. If transition has no gates, skip provider and decide from stored graph/lifecycle.
4. Otherwise resolve executable/arguments/working directory from run's stable registration ID and invoke provider once with complete gate set and bounded loaded snapshot. Oversized selected evidence rejects before invocation; selected evidence is never truncated.
5. Require exactly one result: complete pass/fail verdicts with valid optional evidence, explicit incompatibility, or evaluation error.
6. Complete transition, reject request, or report operation error. Valid provider evidence on pass/fail commits with attempt; malformed provider evidence errors entire provider result.
7. Commit only if internal version/lifecycle still match evaluated snapshot; atomically persist authoritative mutation and explanatory journal.

Caller working directory never changes provider invocation. Workflow-specific paths belong in immutable run inputs, self-contained/provider-defined evidence locators, or explicit provider configuration. If external resource moves, provider may support append-only remap evidence; otherwise restore location or create new run.

## Storyboard 4: Rejection and revision cycle

```text
run show → external work → request event → gate rejection
→ state unchanged → diagnostics explain failed gates
→ external revision → run show → request event again
```

Illustrative result:

```text
Outcome: rejected
Reason: gate.failed
Lifecycle: active
State: design-review (unchanged)
Evidence recorded: yes

Gate verdicts:
  design-is-complete: pass
  risks-are-addressed: fail
    Missing rollback strategy.

Requestable events:
  approved
  changes-requested
```

After successful run lookup, every syntactically valid event request engine can durably record retains inline evidence and selected-evidence associations, including unknown-event/lifecycle rejection and later operation error. Valid provider evidence accompanies pass/fail attempts. Response states whether evidence was recorded. Attempt/evidence commit is all-or-nothing.

No automatic retry or exactly-once provider guarantee exists. After uncertain process interruption, caller inspects current state/history before issuing another request. Missing history record does not prove provider process never started; provider roles must not be relied on for primary work or exactly-once external side effects.

## Storyboard 5: Provider failure and incompatibility

Provider failure:

```text
Outcome: error
Reason: provider.timeout
State: design-review (unchanged)
Submitted inline evidence recorded: yes
Selected evidence associations recorded: yes
Provider evidence recorded: no
```

Compatibility report:

```console
$ loop-engine provider check software-change --active-runs

Outcome: completed

Active graphs:
  01J... sha256:44cd... incompatible
    unsupported gate: implementation.has-review
```

For provider-dependent request, tombstoned/missing registration or executable, provider crash/timeout/evaluation error, and missing/malformed result are errors. Executable-present explicit unsupported capability is incompatibility rejection. Compatibility check is non-latching per-capability report; unsupported selected request rejects while supported and gate-free events remain usable. Provider-emitted invalid graph is error during creation; rejected creation has no run journal.

Missing/incompatible provider does not block provider-free show/graph/history, evidence/annotation, termination, or gate-free events.

### Per-run compatibility check

Registration-wide `provider check … --active-runs` reports across active graphs without per-run journal fan-out. Explicit per-run inspection is a separate operation:

```console
$ loop-engine run compatibility 01J...

Outcome: completed
Provider executed: yes

Findings:
  event.approved: compatible
  guidance.live: incompatible
    stored guidance contract no longer supported

State: design-review (unchanged)
Lifecycle: active
```

`run compatibility` targets one active stored run. It completes with non-latching per-capability findings even when findings include incompatibility. After run lookup it atomically appends compatibility-attempt and provider-observation journal facts (including drift) without changing workflow state or version. Terminal lifecycle rejects the request. Unsupported selected capabilities on `run request` or `run guidance` reject while supported and gate-free events remain usable.

## Storyboard 6: Provider update and removal

```text
registration changes executable
→ new runs use newly emitted graph digest
→ active runs retain creation-time graph
→ active provider operations use current registration executable
→ invocation journals actual locator/digest/version
```

Provider drift requires no approval. Topology/declarations/guidance are fixed; gate policy is live. Gate-attempt history shows provider digest. Current provider must honor stored contract or report selected capability incompatible.

Disabling registration used by active runs is allowed because configuration/executable may disappear outside engine control. CLI warns and identifies affected runs. Stable ID remains inspectable/restorable; restore assigns currently free handle. Former handle may be reused without rebinding runs because runs store ID. Stranded active runs remain inspectable, annotatable, terminable, and able to take gate-free events; restoring compatible executable to same ID resumes provider-dependent use.

## Storyboard 7: Finalization and termination

Run becomes neutrally final when transition enters any stored final state. Final state ID/provider metadata conveys domain meaning such as success, decline, cancellation, or failure. Graph may have zero, one, or several finals; each final is sink. Zero-final run is intentionally ongoing and can close only through explicit termination. Non-final sink is valid terminate-only trap. Initial final state creates immediately final run. Caller may terminate active run without provider execution and may include note. Termination against terminal run rejects.

Final and terminated runs:

- never reopen;
- reject further events and report empty requestable-event set;
- remain inspectable;
- remain visible through terminal/all catalog filter;
- accept append-only notes/evidence for audit correction;
- retain fixed display label;
- cannot be individually deleted or silently compacted in MVP.

Provider-dependent live guidance/compatibility execution on terminal run rejects by lifecycle. Starting new work requires new run. New run may reference old run through provider-defined input, evidence, or note; core adds no lineage semantics.

## Storyboard 8: Automation

Automation invokes same operations as human caller.

Structured mode emits exactly one authoritative JSON outcome envelope on stdout for every dispatched completed, rejected, or error outcome. Always-on trace is separate file and never contaminates stdout. Stderr is reserved for rich failures before dispatch or inability to construct envelope. Provider stdout/stderr never bypass engine envelope and is retained only through bounded operational trace capture.

Frozen envelope shape ([cli-contract.md](cli-contract.md)):

```json
{
  "schema_version": 1,
  "operation": "run.request",
  "request_id": "01J...",
  "trace": "/machine-local/state/loop-engine/traces/01J....jsonl",
  "outcome": "rejected",
  "reason": {
    "code": "gate.failed",
    "message": "One or more required gates failed"
  },
  "data": {
    "run": {
      "id": "01J...",
      "label": "checkout-redesign",
      "lifecycle": "active",
      "state": "design-review",
      "state_changed": false
    },
    "evidence_recorded": {
      "inline": true,
      "selected_associations": true,
      "provider": true
    },
    "requestable_events": ["approved", "changes-requested"]
  },
  "diagnostics": []
}
```

Human mode presents the same semantics: `Outcome: rejected` corresponds to `"outcome": "rejected"` and exit `2`; `Outcome: completed` to exit `0`; `Outcome: error` to exit `1`. Pre-dispatch failures exit `64` with rich stderr and no stdout envelope.

## MVP exclusions exposed by storyboards

MVP interaction does not include:

- concurrent same-run collaboration protocol;
- caller-managed revisions;
- claims or leases;
- idempotency keys or automatic retries;
- pause/resume lifecycle;
- terminal reopening;
- individual run deletion;
- run import, restore-from-export, or cross-machine mobility (read-only audit export remains allowed);
- provider discovery/registry/manifest installation;
- active-run graph migration or gate bypass;
- official language SDK requirement.
