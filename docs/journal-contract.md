# Loop Engine Journal Contract

**Status:** Frozen by T011 (2026-07-17). Decision [D011](change/initial-implementation/decisions.md#d011--journal-granularity).

This document is the canonical contract for immutable per-run activity journal entry schema v1, entry kinds, attempt shape, monotonic sequence allocation, correction links, provider/gate/evidence nesting, `state_changed` alignment with the structured CLI envelope, aggregate and component encoded-size bounds, oversize rejection, and operation journal obligations. Named numeric bounds are frozen in [cli-contract.md](cli-contract.md#resource-bounds-d008) (D008); this document references bound **names** only. Transactional insert semantics and version/sequence allocation are frozen in [persistence.md](persistence.md) (D009); table DDL is owned by T105 (`0001_initial.sql`).

Related documents:

- [Decision D011](change/initial-implementation/decisions.md#d011--journal-granularity)
- [Resource bounds (D008)](cli-contract.md#resource-bounds-d008)
- [Structured CLI contract](cli-contract.md) — `data.run.state_changed`, `data.evidence_recorded`, reason taxonomy
- [Persistence contract](persistence.md)
- [Application operation catalog](operation-catalog.md)
- [Provider protocol v1](provider-protocol-v1.md)
- [Reference workflow journal expectations](reference-workflow.md#journal-expectations)
- [Code architecture](architecture.md)
- [Interaction storyboards](ux-storyboards.md)
- [System invariants](invariants.md) — I8, I11–I15, I35

## Scope and authority

The activity journal is **explanatory storage only** (I12–I15, C2). It is **not**:

- authoritative current workflow state or lifecycle (stored in SQLite columns);
- a replay or reconstruction source for current state;
- operational trace (diagnostic JSONL per invocation);
- evidence authority beyond bounded references and associations recorded with an attempt;
- a second write path for provider-catalog mutations.

Core **MUST NOT** derive current state by replaying journal entries ([architecture.md](architecture.md)). Export `journal.jsonl` (D015) is a read-only snapshot of stored entries; it never becomes write authority and does not promise replay.

One **aggregate immutable** journal entry is appended per meaningful operation or post-lookup attempt (D011). Entries are never edited or deleted. Corrections and clarifications append new entries linked to prior `sequence` values (I13).

Rejected patterns:

- event-sourced micro-events that must be replayed to infer state;
- in-place mutation of prior journal rows;
- journal truncation to fit a history page;
- treating missing journal rows as proof that a provider process never started (I15).

## Wire encoding

Each stored journal entry is one UTF-8 JSON object. Export writes the same object as one JSONL line terminated by `\n`. Within major version `1`, same-major evolution is additive: new optional fields may appear; readers ignore unknown fields.

### Schema version field

| Field | Type | Required | Description |
|---|---|:---:|---|
| `journal_schema_version` | integer | yes | Always `1` for this contract |

## Immutable entry kinds

`entry_kind` is a closed string enum. Implementations **MUST NOT** invent additional kinds without reopening D011.

| `entry_kind` | Producing operation(s) | Typical `outcome` values |
|---|---|---|
| `run.created` | `run.create` (success only) | `completed` |
| `evidence.added` | `run.evidence.add` | `completed`, `rejected` |
| `annotation` | `run.annotate` | `completed`, `rejected` |
| `label.changed` | `run.label` | `completed`, `rejected` |
| `transition.attempt` | `run.request` | `completed`, `rejected`, `error` |
| `guidance.attempt` | `run.guidance` | `completed`, `rejected`, `error` |
| `compatibility.attempt` | `run.compatibility` | `completed`, `rejected`, `error` |
| `run.terminated` | `run.terminate` | `completed`, `rejected` |

Provider-catalog operations (`provider.add`, `provider.update`, `provider.rename`, `provider.disable`, `provider.restore`, `provider.list`, `provider.check`) **MUST NOT** append per-run journal entries (I40). Rejected or errored `run.create` produces **no** run row and **no** journal entry.

## Required fields (every entry)

| Field | Type | Required | Description |
|---|---|:---:|---|
| `journal_schema_version` | integer | yes | Always `1` |
| `sequence` | integer | yes | Per-run monotonic positive integer; allocated atomically with insert ([persistence.md](persistence.md)) |
| `run_id` | string | yes | Stable run identifier |
| `ts` | string | yes | RFC 3339 UTC timestamp with millisecond precision |
| `operation` | string | yes | Stable operation ID that produced this entry (I27) |
| `request_id` | string | yes | Invocation identifier correlating CLI envelope and operational trace |
| `entry_kind` | string | yes | One of the immutable kinds above |
| `outcome` | string | yes | `completed`, `rejected`, or `error` — same semantic class as structured CLI `outcome` (I34) |
| `reason` | object or null | yes | `null` when `outcome` is `completed`; otherwise `{ "code", "message" }` from [operation-catalog.md](operation-catalog.md) taxonomy |
| `state_before` | object | yes | Authoritative observation immediately before this entry's durable effect |
| `state_after` | object | yes | Authoritative observation immediately after commit; equals `state_before` when workflow state and lifecycle are unchanged |

### `state_before` / `state_after` object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `state` | string | yes | Workflow state identifier |
| `lifecycle` | string | yes | `active`, `final`, or `terminated` |
| `workflow_state_version` | integer | yes | Internal workflow-state version ([persistence.md](persistence.md)) |
| `lifecycle_version` | integer | yes | Internal lifecycle version |

`label` is **not** duplicated in `state_before`/`state_after`; label mutations use `label` / `label_before` / `label_after` on `label.changed` entries only.

## Attempt shape

Post-lookup mutations that explain a caller or provider attempt share a common **attempt** envelope in addition to required fields:

| Field | Type | When present | Description |
|---|---|---|---|
| `transition` | object | `transition.attempt` | Requested event and whether a stored transition applied |
| `provider_observations` | array | Provider invoked or observation recorded | Ordered bounded provider invocation audit ([Provider observations](#provider-observations-nesting)) |
| `gate_verdict_facts` | object | Gate evaluation occurred | Bounded gate verdict set ([Gate verdict facts](#gate-verdict-facts-nesting)) |
| `evidence_associations` | object | Evidence submitted or returned | Bounded inline/selected/provider associations ([Evidence associations](#evidence-associations-nesting)) |
| `evidence_recorded` | object | After run lookup when evidence categories may apply | Mirrors CLI `data.evidence_recorded` |
| `note` | string | Caller supplied `--note` on `run.annotate`, `run.request`, or `run.terminate` | Bounded by `note_text_utf8_bytes` |
| `actor` | object | Caller supplied `--actor` on `run.annotate` | Opaque metadata bounded by `actor_metadata_encoded_bytes` |
| `corrects_sequence` | integer | `run.annotate --corrects <SEQUENCE>` | Links clarification to prior entry; prior entry is never modified |
| `diagnostics` | array | Optional ancillary detail | Bounded diagnostic objects; counted toward journal diagnostic aggregate |

`annotation` entries without a transition use the attempt envelope for `note`, `actor`, and `corrects_sequence` only.

### Entry-kind extensions

Optional fields beyond the attempt envelope, allowed only on the kinds listed:

| Field | Allowed on | Description |
|---|---|---|
| `graph_revision` | `run.created` | Canonical graph projection digest at creation |
| `label_before`, `label_after` | `label.changed` | Display label change; `null` when cleared |
| `evidence_id`, `kind`, `locator`, `digest` | `evidence.added` | Newly persisted evidence record summary |
| `guidance_text` | `guidance.attempt` | Bounded live guidance text when completed |
| `findings` | `compatibility.attempt` | Per-capability compatibility rows when findings exist |

## Sequence and correction links

- `sequence` starts at `1` for the creation entry and increments by `1` for each subsequent insert on that run.
- Allocation occurs inside the same SQLite write transaction as the insert ([persistence.md](persistence.md)).
- `corrects_sequence`, when present, **MUST** reference an earlier `sequence` on the same `run_id`.
- The referenced entry **MUST NOT** be altered; the new entry explains the correction/clarification (I13).
- `run.history` sorts by ascending `sequence` ([cli-contract.md](cli-contract.md#paged-surfaces-and-sort-keys)).

## `state_changed` and self-loop alignment

Structured CLI `data.run.state_changed` answers one question only: **did the workflow state identifier change in this operation?** ([cli-contract.md](cli-contract.md#run-summary-datarun)). It does **not** mean “no durable effect” and does **not** track lifecycle or label changes alone.

| Scenario | `state_before.state` vs `state_after.state` | `workflow_state_version` bump | CLI `state_changed` | Journal `transition.applied` (when applicable) |
|---|---|---|---|---|
| Completed transition to new state | differ | yes (on accepted transition) | `true` | `true` |
| Completed self-loop (same source and target state) | equal | **no** | `false` | `true` |
| Domain rejection (gate, unknown event, lifecycle denial, incompatibility) | equal | no | `false` | `false` or `transition` omitted |
| Stale evaluation (`state.stale_version`) | equal | no | `false` | `false` or `transition` omitted |
| `run.label`, `run.annotate`, `run.evidence.add` | equal | no | `false` | n/a |
| `run.terminate` (lifecycle only) | equal unless terminate also changes state | lifecycle bump only | `false` unless state ID changes | n/a |
| `run.guidance`, `run.compatibility` | equal | no | `false` | n/a |

**Self-loop rule (normative):** When a completed `run.request` applies a stored transition whose `source_state` and `target_state` are the same state identifier, the operation is **completed**, CLI reports `state_changed: false`, and the journal **MUST** still record `transition.applied: true` with the event name and matching `source_state`/`target_state`. History therefore shows the applied transition even though inspection reports an unchanged state identifier (I invariants, [ux-storyboards.md](ux-storyboards.md) outcome vocabulary).

### `transition` object (`transition.attempt` only)

| Field | Type | Required | Description |
|---|---|:---:|---|
| `event` | string | yes | Caller-requested event name |
| `source_state` | string | yes | State before evaluation |
| `target_state` | string | yes when `applied` is true | Resolved target from stored graph |
| `applied` | boolean | yes | `true` when engine committed an accepted transition; `false` on rejection/error paths that evaluated an event |

## Provider observations nesting

Encoded size of the `provider_observations` array **MUST NOT** exceed `journal_provider_facts_encoded_bytes` ([cli-contract.md](cli-contract.md#resource-bounds-d008)). The array **MUST** list every attempted provider subprocess invocation for the producing operation in deterministic invocation order. Each element is one bounded observation; the array **MUST NOT** duplicate operational-trace payloads (request/result JSON, stdout/stderr, or stream metadata). Correlation uses `invocation_id` only; full protocol exchange detail remains in operational trace ([operational-trace.md](operational-trace.md)).

### Per-observation object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `registration_id` | string | yes | Stable registration ID observed at invocation |
| `config_revision` | integer | yes | Observed `config_revision` at invocation |
| `role` | string | yes | `describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, or `check_compatibility` |
| `invocation_id` | string | yes | Provider protocol correlation ID; matches request/result and operational trace |
| `executable` | string | yes | Lexical absolute executable path used at spawn |
| `outcome` | string | yes | Per-invocation outcome: `completed`, `rejected`, or `error` |
| `executable_digest` | string | no | Observed digest when available (I8, I43) |
| `provider_version` | string | no | Provider self-report when available; audit only |
| `protocol_major` | integer | no | Negotiated protocol major |

Per-invocation `outcome` semantics:

| Outcome | Meaning |
|---|---|
| `completed` | Role-valid protocol result received and consumed for this invocation step |
| `rejected` | Role-valid domain rejection (only roles that permit rejection, e.g. `validate_inputs`) |
| `error` | Process termination, protocol malformation, timeout, `evaluation_error`, or other failure before a consumable role-valid result |

Invocation order **MUST** match provider subprocess order within the operation. For `run.create`, observations **MUST** appear as `describe` first, then `validate_inputs` when that role is invoked ([operation-catalog.md](operation-catalog.md)). Provider drift is allowed; gate and compatibility attempts **MUST** record observed locator/digest/version when available so live policy drift is observable without mutating stored graph (I8, I15).

## Gate verdict facts nesting

Encoded size of the `gate_verdict_facts` object **MUST NOT** exceed `journal_gate_verdict_facts_encoded_bytes`.

| Field | Type | Required | Description |
|---|---|:---:|---|
| `event` | string | yes | Event being evaluated |
| `gate_ids` | array of strings | yes | Required gate IDs from stored graph for this transition |
| `verdicts` | array | when result is verdict set | One object per required gate: `{ "gate_id", "status": "pass" \| "fail", "message"?: string }` |
| `incompatibility` | object | when provider declared incompatibility | Bounded diagnostic; no verdict substitution |

Failed gates produce `outcome: rejected` with `reason.code` such as `gate.failed`; verdict details live in `gate_verdict_facts`.

## Evidence associations nesting

Encoded size of the `evidence_associations` object **MUST NOT** exceed `journal_evidence_associations_encoded_bytes`.

| Field | Type | Required | Description |
|---|---|:---:|---|
| `inline` | array | no | Inline evidence records retained on this attempt (bounded ids/kinds/locators) |
| `selected_ids` | array of strings | no | Caller-selected existing evidence IDs associated with this attempt |
| `provider_recorded_ids` | array of strings | no | New evidence IDs persisted from valid provider gate evidence on this attempt |

`evidence_recorded` on the entry **MUST** mirror which categories actually committed:

```text
"evidence_recorded": {
  "inline": true,
  "selected_associations": true,
  "provider": false
}
```

Selected evidence is never truncated (I35). Associations and inline/provider evidence **MUST** commit in the same transaction as the attempt ([persistence.md](persistence.md)).

## Note and actor vocabulary

| User-facing surface | Journal field | Entry kind |
|---|---|---|
| `run annotate … [--note <TEXT>]` | `note` | `annotation` |
| `run annotate … [--actor <PATH>]` | `actor` (opaque object; path is caller convention only) | `annotation` |
| `run annotate … [--corrects <SEQUENCE>]` | `corrects_sequence` | `annotation` |
| `run request … [--note <TEXT>]` | `note` | `transition.attempt` |
| `run terminate … [--note <TEXT>]` | `note` | `run.terminated` |

The term **annotation** names the `annotation` entry kind produced by `run.annotate`. The term **note** names the optional `note` field that may appear on annotation, transition, and termination entries. Implementations **MUST NOT** use alternate field names (`annotate_text`, `comment`, etc.) on the wire.

## Component and aggregate bounds

All encoded-size limits use canonical bound **names** from [cli-contract.md](cli-contract.md#resource-bounds-d008). Integrations measure UTF-8 encoded JSON bytes of each nested object independently before assembly. Numeric limits **MUST** be resolved from that table and [aggregate envelope arithmetic — Journal entry](cli-contract.md#aggregate-envelope-arithmetic) at verification time; this document does not restate them.

| Bound name | Applies to |
|---|---|
| `journal_entry_encoded_bytes` | Full encoded journal entry object |
| `journal_evidence_associations_encoded_bytes` | `evidence_associations` object |
| `journal_provider_facts_encoded_bytes` | `provider_observations` array |
| `journal_gate_verdict_facts_encoded_bytes` | `gate_verdict_facts` object |
| `note_text_utf8_bytes` | `note` string |
| `actor_metadata_encoded_bytes` | `actor` object |
| Journal diagnostic aggregate | Sum of encoded `diagnostics` entries plus entry-level diagnostic fields not nested elsewhere (budget per [cli-contract.md aggregate envelope arithmetic — Journal entry](cli-contract.md#aggregate-envelope-arithmetic)) |

Component budgets are subordinate to `journal_entry_encoded_bytes`. No field may be truncated to satisfy the aggregate cap.

### Maximum encoded-size arithmetic

Worst-case component sum uses each component at its bound from [cli-contract.md](cli-contract.md#resource-bounds-d008):

| Component | Symbol |
|---|---|
| `journal_evidence_associations_encoded_bytes` | `E` |
| `journal_provider_facts_encoded_bytes` | `P` |
| `journal_gate_verdict_facts_encoded_bytes` | `G` |
| `note_text_utf8_bytes` | `N` |
| `actor_metadata_encoded_bytes` | `A` |
| Journal diagnostic aggregate | `D` |

**Component subtotal:** `S = E + P + G + N + A + D`

**Framing headroom:** `H = journal_entry_encoded_bytes − S` (required fields, keys, `state_before`/`state_after`, `transition`, timestamps, IDs; see [cli-contract.md aggregate envelope arithmetic — Journal entry](cli-contract.md#aggregate-envelope-arithmetic))

**Aggregate cap:** `encoded_len(entry) ≤ journal_entry_encoded_bytes` when `S + H ≤ journal_entry_encoded_bytes`

Therefore one encoded entry fits one `collection_page_data_budget_bytes` history page without record truncation ([cli-contract.md aggregate envelope arithmetic — Journal entry](cli-contract.md#aggregate-envelope-arithmetic), D008).

### One-byte-over rejection

Before insert, integration **MUST** compute `encoded_len(entry)` as UTF-8 byte length of the canonical JSON representation used for storage.

| Condition | Behavior |
|---|---|
| `encoded_len(entry) ≤ journal_entry_encoded_bytes` | Insert allowed (subject to other validation) |
| `encoded_len(entry) > journal_entry_encoded_bytes` | **Reject** the operation; **no** partial journal row; **no** field truncation |

Caller-owned overflow (note, actor, inline/selected evidence associations assembled before provider call) **MUST** reject with `resource.exhausted` when a component bound or the aggregate cap would be exceeded. Provider-owned oversize gate or provider fact payloads **MUST** map to provider protocol or operation errors per [operation-catalog.md](operation-catalog.md); persistence **MUST NOT** silently drop gate verdicts or provider observations.

**Illustrative semantics:** When `encoded_len(entry) = journal_entry_encoded_bytes`, insert succeeds. When `encoded_len(entry) > journal_entry_encoded_bytes` (for example a one-byte `note` overflow past remaining headroom), the operation **MUST** reject.

## Operation journal obligations

| Operation | Journal obligation |
|---|---|
| `provider.add`, `provider.update`, `provider.rename`, `provider.disable`, `provider.restore` | **None** — verify via fresh `provider.list` |
| `provider.list`, `provider.check` | **None** (including `--active-runs`) |
| `run.create` | `run.created` on success only; **none** on rejection/error. When provider invocations occur, `provider_observations` **MUST** list every attempted invocation in deterministic order (`describe` first; `validate_inputs` second when declarations and/or candidate values exist) |
| `run.evidence.add` | `evidence.added` on success; rejection after lookup when persistence available |
| `run.annotate` | `annotation` on success; rejection after lookup when persistence available |
| `run.label` | `label.changed` on success; rejection after lookup when persistence available |
| `run.request` | `transition.attempt` for every post-lookup attempt including unknown event, lifecycle denial, gate pass/fail, incompatibility, stale error, and completed self-loop; `provider_observations` when `evaluate_gates` is invoked |
| `run.guidance` | `guidance.attempt` for every post-lookup attempt including unsupported, terminal denial, provider error, and completed guidance; `provider_observations` when `live_guidance` is invoked |
| `run.compatibility` | `compatibility.attempt` for every post-lookup attempt; no state/version mutation; `provider_observations` when `check_compatibility` is invoked |
| `run.terminate` | `run.terminated` on success; rejection when already terminal |
| `run.list`, `run.show`, `run.graph`, `run.history`, `run.evidence.list`, `run.export` | **None** (read/export only) |

Post-lookup rejections and errors **MUST** still append when persistence is available ([persistence.md](persistence.md) § Attempt journaling). Failures before run lookup produce no journal entry.

## Contract examples

All examples below are valid JSON conforming to this contract. IDs and paths are illustrative.

### Creation — `run.created`

```json
{
  "journal_schema_version": 1,
  "sequence": 1,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T14:00:00.123Z",
  "operation": "run.create",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W",
  "entry_kind": "run.created",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "explore",
    "lifecycle": "active",
    "workflow_state_version": 1,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "explore",
    "lifecycle": "active",
    "workflow_state_version": 1,
    "lifecycle_version": 1
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 3,
      "role": "describe",
      "invocation_id": "pv-describe-001",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:91ab…",
      "provider_version": "0.3.1",
      "protocol_major": 1
    },
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 3,
      "role": "validate_inputs",
      "invocation_id": "pv-validate-002",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:91ab…",
      "provider_version": "0.3.1",
      "protocol_major": 1
    }
  ],
  "graph_revision": "sha256:canonical-projection…"
}
```

### Mutation — completed transition (`transition.attempt`)

```json
{
  "journal_schema_version": 1,
  "sequence": 4,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T15:10:00.456Z",
  "operation": "run.request",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V3Y",
  "entry_kind": "transition.attempt",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "design-review",
    "lifecycle": "active",
    "workflow_state_version": 4,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "plan",
    "lifecycle": "active",
    "workflow_state_version": 5,
    "lifecycle_version": 1
  },
  "transition": {
    "event": "approved",
    "source_state": "design-review",
    "target_state": "plan",
    "applied": true
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 3,
      "role": "evaluate_gates",
      "invocation_id": "pv-gates-003",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:91ab…",
      "protocol_major": 1
    }
  ],
  "gate_verdict_facts": {
    "event": "approved",
    "gate_ids": ["design-is-complete", "risks-are-addressed"],
    "verdicts": [
      {"gate_id": "design-is-complete", "status": "pass"},
      {"gate_id": "risks-are-addressed", "status": "pass"}
    ]
  },
  "evidence_associations": {
    "selected_ids": ["01J-EVIDENCE-001"],
    "provider_recorded_ids": []
  },
  "evidence_recorded": {
    "inline": false,
    "selected_associations": true,
    "provider": false
  },
  "note": "Design review completed"
}
```

CLI envelope for the same invocation reports `"state_changed": true`.

### Mutation — completed self-loop (`transition.attempt`)

```json
{
  "journal_schema_version": 1,
  "sequence": 7,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T16:00:00.000Z",
  "operation": "run.request",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V7C",
  "entry_kind": "transition.attempt",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "transition": {
    "event": "checkpoint",
    "source_state": "implement",
    "target_state": "implement",
    "applied": true
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 4,
      "role": "evaluate_gates",
      "invocation_id": "pv-gates-007",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:cd00…",
      "protocol_major": 1
    }
  ],
  "gate_verdict_facts": {
    "event": "checkpoint",
    "gate_ids": ["tests-green"],
    "verdicts": [
      {"gate_id": "tests-green", "status": "pass"}
    ]
  },
  "evidence_recorded": {
    "inline": false,
    "selected_associations": false,
    "provider": false
  }
}
```

CLI envelope for the same invocation reports `"state_changed": false` while history shows `transition.applied: true`.

### Rejection — failed gate (`transition.attempt`)

```json
{
  "journal_schema_version": 1,
  "sequence": 5,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T15:20:00.789Z",
  "operation": "run.request",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V4Z",
  "entry_kind": "transition.attempt",
  "outcome": "rejected",
  "reason": {
    "code": "gate.failed",
    "message": "One or more required gates failed"
  },
  "state_before": {
    "state": "design-review",
    "lifecycle": "active",
    "workflow_state_version": 4,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "design-review",
    "lifecycle": "active",
    "workflow_state_version": 4,
    "lifecycle_version": 1
  },
  "transition": {
    "event": "approved",
    "source_state": "design-review",
    "target_state": "plan",
    "applied": false
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 3,
      "role": "evaluate_gates",
      "invocation_id": "pv-gates-004",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:91ab…",
      "protocol_major": 1
    }
  ],
  "gate_verdict_facts": {
    "event": "approved",
    "gate_ids": ["design-is-complete", "risks-are-addressed"],
    "verdicts": [
      {"gate_id": "design-is-complete", "status": "pass"},
      {"gate_id": "risks-are-addressed", "status": "fail", "message": "Missing rollback strategy."}
    ]
  },
  "evidence_recorded": {
    "inline": true,
    "selected_associations": true,
    "provider": true
  }
}
```

### Provider observation — compatibility attempt (`compatibility.attempt`)

```json
{
  "journal_schema_version": 1,
  "sequence": 10,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T16:15:00.400Z",
  "operation": "run.compatibility",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0VAF",
  "entry_kind": "compatibility.attempt",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 5,
      "role": "check_compatibility",
      "invocation_id": "pv-compat-010",
      "executable": "/work/provider/software-change-provider-v2",
      "outcome": "completed",
      "executable_digest": "sha256:ef12…",
      "provider_version": "0.4.0",
      "protocol_major": 1
    }
  ],
  "findings": [
    {"capability": "evaluate_gates", "status": "compatible"},
    {"capability": "live_guidance", "status": "incompatible", "message": "Stored graph requires live guidance v2"}
  ]
}
```

Gate-attempt entries use the same `provider_observations` element shape on `transition.attempt`; observed digest may differ from earlier sequences on the active run (I8).

### Stale — post-provider version mismatch (`transition.attempt`)

```json
{
  "journal_schema_version": 1,
  "sequence": 6,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T15:30:00.100Z",
  "operation": "run.request",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V5A",
  "entry_kind": "transition.attempt",
  "outcome": "error",
  "reason": {
    "code": "state.stale_version",
    "message": "Workflow state changed during provider evaluation"
  },
  "state_before": {
    "state": "design-review",
    "lifecycle": "active",
    "workflow_state_version": 4,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "design-review",
    "lifecycle": "active",
    "workflow_state_version": 4,
    "lifecycle_version": 1
  },
  "transition": {
    "event": "approved",
    "source_state": "design-review",
    "target_state": "plan",
    "applied": false
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 3,
      "role": "evaluate_gates",
      "invocation_id": "pv-gates-005",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:91ab…",
      "protocol_major": 1
    }
  ],
  "gate_verdict_facts": {
    "event": "approved",
    "gate_ids": ["design-is-complete", "risks-are-addressed"],
    "verdicts": [
      {"gate_id": "design-is-complete", "status": "pass"},
      {"gate_id": "risks-are-addressed", "status": "pass"}
    ]
  }
}
```

Authoritative state remains at `design-review`; CLI reports `"state_changed": false` and outcome `error`.

### Guidance — `guidance.attempt`

```json
{
  "journal_schema_version": 1,
  "sequence": 8,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T16:05:00.200Z",
  "operation": "run.guidance",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V8D",
  "entry_kind": "guidance.attempt",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "provider_observations": [
    {
      "registration_id": "01J-REG-SOFTWARE-CHANGE",
      "config_revision": 4,
      "role": "live_guidance",
      "invocation_id": "pv-guidance-008",
      "executable": "/work/provider/software-change-provider",
      "outcome": "completed",
      "executable_digest": "sha256:cd00…",
      "protocol_major": 1
    }
  ],
  "guidance_text": "Address unresolved rollback risks before review."
}
```

### Correction — `annotation` with `corrects_sequence`

```json
{
  "journal_schema_version": 1,
  "sequence": 9,
  "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
  "ts": "2026-07-17T16:10:00.300Z",
  "operation": "run.annotate",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V9E",
  "entry_kind": "annotation",
  "outcome": "completed",
  "reason": null,
  "state_before": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "state_after": {
    "state": "implement",
    "lifecycle": "active",
    "workflow_state_version": 8,
    "lifecycle_version": 1
  },
  "note": "Clarification: rollback strategy documented in EV-002, not EV-001.",
  "corrects_sequence": 5
}
```

Sequence `5` (gate rejection example) remains immutable; this entry appends linked clarification only.

## Verification rules (T011)

- Every immutable `entry_kind` appears in [Immutable entry kinds](#immutable-entry-kinds) and maps to exactly one producing operation class.
- [Operation journal obligations](#operation-journal-obligations) covers all 21 application operations and matches [operation-catalog.md](operation-catalog.md) mutation classification.
- Component bound names match [cli-contract.md](cli-contract.md#resource-bounds-d008) without independent numeric literals.
- Maximum encoded-size arithmetic: `S + H = journal_entry_encoded_bytes` per [aggregate envelope arithmetic — Journal entry](cli-contract.md#aggregate-envelope-arithmetic); numeric resolution **MUST** use [cli-contract.md](cli-contract.md#resource-bounds-d008) only; single entry fits `collection_page_data_budget_bytes` per the same section.
- One-byte-over rule rejects when `encoded_len(entry) > journal_entry_encoded_bytes` (minimum excess: `journal_entry_encoded_bytes + 1` in encoded-length terms) with no truncation.
- Self-loop example shows `transition.applied: true` with equal `state_before.state` and `state_after.state` and documents CLI `state_changed: false`.
- `note` / `annotation` / `corrects_sequence` vocabulary matches `run.annotate`, `run.request`, and `run.terminate` argv in [operation-catalog.md](operation-catalog.md).
- All [Contract examples](#contract-examples) parse as JSON and include creation, mutation, rejection, provider/compatibility, stale, guidance, and correction cases.
- `provider_observations` is an ordered array bounded by `journal_provider_facts_encoded_bytes`; `run.created` lists `describe` then `validate_inputs` when both invoked; observations carry `invocation_id`, `executable`, and per-invocation `outcome` without duplicating operational-trace payloads.
- Non-replay authority statement matches I12 and [persistence.md](persistence.md) scope.
