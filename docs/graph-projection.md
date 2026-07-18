# Loop Engine Graph Projection and Canonical Encoding

**Status:** Frozen by T014 (2026-07-17). Decision [D014](change/initial-implementation/decisions.md#d014--canonical-graph-encoding).

This document is the canonical contract for provider-emitted workflow graph projection field semantics, semantic validation expectations, canonical integration DTO v1, deterministic byte encoding, `graph_revision` computation, metadata treatment (including RFC 8785 / JCS metadata number encoding), golden vectors, and the field-change matrix governing when graph identity changes. Named numeric bounds are frozen in [cli-contract.md](cli-contract.md#resource-bounds-d008) (D008); this document references bound **names** only. Provider subprocess transport remains in [provider-protocol-v1.md](provider-protocol-v1.md) (T005). JSON Schema files are published in T084; this document is normative for semantics and canonical bytes.

Related documents:

- [Decision D014](change/initial-implementation/decisions.md#d014--canonical-graph-encoding)
- [Provider protocol v1](provider-protocol-v1.md)
- [Resource bounds (D008)](cli-contract.md#resource-bounds-d008)
- [Persistence contract](persistence.md)
- [Journal contract](journal-contract.md)
- [Technology direction](technology.md) — graph validation direction
- [Code architecture](architecture.md)
- [System invariants](invariants.md) — I6–I8, I32, I37, I43

## Scope and authority

This contract governs:

- the `graph` object returned by provider `describe` (protocol wire projection);
- semantic validation before run creation;
- the integration-owned canonical DTO and UTF-8 byte encoding hashed as `graph_revision`;
- immutable per-run stored graph snapshots and journal `graph_revision` facts.

This contract does **not** govern:

- provider subprocess transport or envelope framing ([provider-protocol-v1.md](provider-protocol-v1.md));
- executable file digest observation (audit-only `executable_digest`);
- evidence wire shapes (T084);
- run input **values** accepted at creation (immutable after creation per I32).

Rejected patterns:

- using raw provider `describe` JSON bytes or stdout formatting as graph identity;
- using `provider_version`, registration locator, or `executable_digest` as `graph_revision`;
- hash-map iteration order or provider key order affecting canonical bytes;
- Serde or other JSON libraries inside `core` for canonical encoding (integration-only).

## Identity distinctions

Three digests are often observed together but serve different purposes:

| Artifact | Role | Input | Wire form | Affects `graph_revision` |
|---|---|---|---|:---:|
| **`graph_revision`** | Semantic workflow-graph identity for a run snapshot (I37, I43) | Canonical integration DTO bytes (this document) | `sha256:` + 64 lowercase hex chars | — |
| **`executable_digest`** | Best-effort audit fact for which provider binary ran (I8, I32, I43) | Raw executable file bytes at observation time | `sha256:` + 64 lowercase hex chars | no |
| **Raw describe JSON** | Provider protocol transport only | Provider `result.graph` as emitted on stdout | not used for identity | no |

Rules:

1. Integration **MUST** map provider wire DTO → validated core semantic graph → canonical integration DTO → canonical bytes → `graph_revision`.
2. `graph_revision` **MUST** change iff canonical bytes change.
3. Re-serializing provider wire JSON with different whitespace, key order, or duplicate-prone array order **MUST NOT** change `graph_revision` when semantic content is unchanged.
4. `provider_version` in the result envelope is audit-only and is excluded from canonical bytes (I37).
5. Stored per-run graph snapshot bytes **MUST** equal canonical bytes computed at creation. Persistence stores the canonical UTF-8 JSON document; `graph_revision` is `sha256:` over those exact bytes.

## Processing pipeline

```
provider describe result.graph (wire DTO)
        │  deserialize + structural bounds (integrations)
        ▼
validated core semantic graph (core — no Serde/canonical bytes)
        │  project digest-relevant fields (core)
        ▼
canonical integration DTO v1 (integrations)
        │  apply semantic ordering + canonical JSON encode (integrations)
        ▼
canonical bytes (UTF-8) ──SHA-256──► graph_revision
        │
        └──► immutable stored graph snapshot (persistence)
```

Core exposes a semantic projection of all digest-relevant fields (T041). Integrations own wire DTO types, canonical DTO types, byte encoding, and hashing (T086). Core **MUST NOT** import Serde, JSON libraries, or hashing crates for this pipeline.

## Provider wire graph projection (protocol v1)

The `describe` result `graph` object is the provider wire projection (I6). Same-major unknown fields are ignored per [provider-protocol-v1.md](provider-protocol-v1.md); they are stripped before canonicalization and never affect `graph_revision`.

### Top-level wire object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `initial_state` | string | yes | State ID of the run's initial workflow state |
| `states` | array | yes | State declarations (may be empty only when graph is intentionally invalid — validation rejects empty) |
| `transitions` | array | yes | Transition declarations (may be `[]`) |
| `input_declarations` | array | yes | Run-input declarations (may be `[]`) |
| `live_guidance_supported` | boolean | yes | Whether provider supports explicitly requested live guidance for runs created from this graph |
| `metadata` | object | no | Provider-defined graph metadata retained in the stored snapshot |

### Wire state object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `id` | string | yes | Stable state identifier |
| `final` | boolean | yes | `true` when entering this state completes the run (final sink) |
| `static_guidance` | string **or** object | yes | Static guidance declaration (see below) |
| `metadata` | object | no | Provider-defined state metadata retained in snapshot |

`static_guidance` on the wire is exactly one of:

- a **non-empty** UTF-8 string — shorthand for text guidance;
- an object `{ "kind": "text", "text": "<non-empty string>" }`; or
- an object `{ "kind": "none" }` — explicit declaration that no additional static guidance is required (I7).

Integrations **MUST** normalize string shorthand to canonical `{ "kind": "text", "text": "<string>" }`. The explicit `kind:text` object form maps to the same canonical object unchanged.

### Wire transition object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `source_state` | string | yes | Source state ID |
| `event` | string | yes | Event ID triggering the transition |
| `target_state` | string | yes | Target state ID |
| `gate_ids` | array of strings | yes | Required gate IDs (may be `[]` for gate-free transitions) |
| `metadata` | object | no | Provider-defined transition metadata retained in snapshot |

At most one transition per `(source_state, event)` pair (I6). Gate IDs within a transition **MUST** be unique; duplicates are validation errors.

### Wire input declaration object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `id` | string | yes | Input identifier |
| `kind` | string | yes | Provider-defined input kind token interpreted by `validate_inputs` |
| `required` | boolean | yes | Whether run creation requires a value |
| `metadata` | object | no | Provider-defined declaration metadata retained in snapshot |

Input declarations are input-topology only. Candidate values never appear in `describe` (I6, I32).

### Identifier and size bounds

| Element | Bound |
|---|---|
| `initial_state`, state `id`, `source_state`, `event`, `target_state`, gate IDs, input `id` | `identifier_utf8_bytes` each |
| `static_guidance` text (string or `kind:text`) | `guidance_text_bytes` |
| `kind` on input declarations | `identifier_utf8_bytes` |
| Metadata nesting depth at any `metadata` object | `metadata_nesting_depth` |
| Encoded canonical graph document | `graph_projection_canonical_bytes` |

Empty identifiers are invalid. Integrations **MUST** reject graphs whose canonical encoding exceeds `graph_projection_canonical_bytes` before run creation.

## Wire JSON parse requirements

Integrations **MUST** enforce these rules when parsing provider `describe` result `graph` JSON **before** semantic projection. They apply to the root graph object and every nested JSON value reachable from it (including all `metadata` trees).

### Duplicate object keys

1. Every JSON object in the wire graph **MUST** have unique member names at every nesting depth.
2. If duplicate keys appear in any object (for example `{"a":1,"a":2}`), parsing **MUST** fail with `provider.graph.invalid` before core semantic validation or canonicalization.
3. Rationale: RFC 8259 discourages duplicate names and leaves behavior implementation-defined; `graph_revision` **MUST NOT** depend on which duplicate survives a permissive parser.

Integrations using `serde_json` (approved in [technology.md](technology.md)) **MUST NOT** rely on its default last-key-wins behavior; a strict parse or explicit duplicate-key detection pass is required.

### JSON number domain (pre-projection)

1. Each JSON number token **MUST** parse to a **finite** IEEE 754 binary64 (`f64`) value per RFC 8259.
2. `NaN`, `+Infinity`, and `-Infinity` **MUST** be rejected with `provider.graph.invalid` at parse time (before semantic projection). They have no canonical JSON number encoding.
3. Negative zero (`-0`) **MAY** appear on the wire as a distinct IEEE 754 value at parse time; it **MUST** canonicalize to `0` at encode time (see [Metadata number encoding](#metadata-number-encoding)).
4. Wire lexical form (`1`, `1.0`, `1e0`, `-0`) **MUST NOT** affect identity except through the parsed IEEE 754 value.

## Semantic validation (summary)

Validation runs on the core semantic graph after wire mapping and before canonicalization. Invalid graphs are `provider.graph.invalid` and prevent run creation (I6, I34).

Required checks (non-exhaustive; T040 owns the full matrix):

| Rule | Invariant |
|---|---|
| `initial_state` references an existing state | I6 |
| At most one transition per `(source_state, event)` | I6 |
| Every transition `source_state`, `event`, `target_state`, and gate ID references valid declarations | I6 |
| Final states declare no outgoing transitions | I7 |
| Every state declares `static_guidance` (text or explicit none) | I7 |
| `live_guidance_supported` is present | I7 |
| Gate IDs unique within each transition | I6 |
| Input declaration IDs unique | I32 |
| Duplicate object keys anywhere in wire graph JSON | I6 |
| Non-finite JSON numbers (`NaN`, `±Infinity`) | I6 |
| Metadata nesting depth exceeds `metadata_nesting_depth` | I6 |

Cycles, zero-final graphs, multiple finals, initial-final states, and non-final traps with no outgoing transitions are **valid** when declarations satisfy the rules above (I7).

## Canonical integration DTO v1

The canonical DTO is the sole input to byte encoding and hashing. Field names differ from wire names where noted.

### Top-level canonical object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `canonical_graph_version` | integer | yes | Always `1` for this contract |
| `initial_state_id` | string | yes | Same value as wire `initial_state` |
| `live_guidance_supported` | boolean | yes | Same value as wire |
| `states` | array | yes | Canonically ordered state objects |
| `transitions` | array | yes | Canonically ordered transition objects |
| `input_declarations` | array | yes | Canonically ordered input declaration objects |
| `metadata` | object | no | Present only when wire graph `metadata` is non-empty after normalization |

### Canonical state object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `id` | string | yes | State identifier |
| `final` | boolean | yes | Final-sink flag |
| `static_guidance` | object | yes | `{ "kind": "text", "text": "..." }` or `{ "kind": "none" }` |
| `metadata` | object | no | Present only when non-empty |

### Canonical transition object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `source_state_id` | string | yes | Wire `source_state` |
| `event_id` | string | yes | Wire `event` |
| `target_state_id` | string | yes | Wire `target_state` |
| `gate_ids` | array of strings | yes | Sorted ascending by UTF-8 byte order; `[]` when gate-free |
| `metadata` | object | no | Present only when non-empty |

### Canonical input declaration object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `id` | string | yes | Input identifier |
| `kind` | string | yes | Provider kind token |
| `required` | boolean | yes | Required flag |
| `metadata` | object | no | Present only when non-empty |

## Semantic ordering rules

Before canonical JSON encoding, integrations **MUST** sort semantically unordered collections. Sorting uses **UTF-8 byte lexicographic order** (Rust `str` / `memcmp` order), not locale-aware collation.

| Collection | Sort key |
|---|---|
| `states` | `id` ascending |
| `transitions` | `(source_state_id, event_id)` ascending |
| `gate_ids` within each transition | ascending |
| `input_declarations` | `id` ascending |
| Object keys within every `metadata` object (and nested metadata objects) | ascending |
| Top-level canonical object keys | ascending (enforced again at encode time) |

Arrays whose order is semantically meaningful (`gate_ids` after sorting, metadata array values) preserve the order established by normalization above. Provider wire array order **MUST NOT** affect canonical bytes when the multiset of elements is unchanged.

## Canonical byte encoding

Canonical bytes are UTF-8 encoding of a single JSON object with these rules:

1. **Minified** — no insignificant whitespace (no spaces outside strings).
2. **Sorted object keys** — at every JSON object, keys sorted by UTF-8 byte lexicographic order (including the root).
3. **Strings** — JSON string escaping per RFC 8259; Unicode in UTF-8 without `\u` escapes unless required for control characters.
4. **Booleans** — lowercase `true` / `false`.
5. **Numbers** — all JSON numbers (including `canonical_graph_version` and every metadata number) per [Metadata number encoding](#metadata-number-encoding) (RFC 8785 / JCS IEEE 754 binary64 rules).
6. **Empty arrays** — encode as `[]`.
7. **Omitted optional fields** — `metadata` keys are omitted when the normalized metadata object is empty `{}`.
8. **No trailing newline** — canonical bytes end with the closing `}` of the root object.
9. **No BOM**.

Implementations **MAY** use any JSON writer that produces this exact byte sequence. Golden vectors below are authoritative for cross-checking.

### `graph_revision` computation

```
canonical_bytes = UTF-8(minified_sorted_json(canonical_dto_v1))
graph_revision  = "sha256:" + lowercase_hex(SHA-256(canonical_bytes))
```

Example: `sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77`.

## Metadata treatment

Provider-defined `metadata` objects at graph, state, transition, and input-declaration levels are part of the stored snapshot and canonical projection when non-empty (D014, I37).

Rules:

1. Metadata root **MUST** be a JSON object (not array, string, or scalar).
2. Keys are strings; values are any JSON value permitted by RFC 8259 after [wire parse requirements](#wire-json-parse-requirements) (finite numbers only; duplicate keys rejected).
3. Nested objects sort keys recursively. Array elements preserve semantic order.
4. Nesting depth **MUST NOT** exceed `metadata_nesting_depth`.
5. Empty `{}` metadata **MUST** be omitted from the canonical object (not encoded as `"metadata":{}`).
6. Audit-only provider envelope fields (`provider_version`, `executable_digest`, registration facts) **MUST NOT** appear in graph metadata unless the provider explicitly places them in wire `metadata`.

### Metadata number encoding

All JSON numbers in canonical bytes — `canonical_graph_version` and every numeric metadata value at any depth — **MUST** use one deterministic serialization algorithm so bytes are unambiguous across integrators.

**Accepted domain:** finite IEEE 754 binary64 values only. Non-finite values are rejected at [wire parse](#json-number-domain-pre-projection) and never reach canonical encoding.

**Normative serialization (RFC 8785 / JCS §3.2.2.3):** Serialize each number as ECMAScript (ES2019+) `Number.prototype.toString()` would output for the IEEE 754 binary64 value:

1. **Negative zero** — the `-0` bit pattern **MUST** encode as `0` (no `-` prefix).
2. **Safe integers** — values in `[-9007199254740991, 9007199254740991]` that are mathematically integers **MUST** encode without `.` or `e`/`E` exponent (examples: `42`, `-1`, `0`).
3. **Exponent notation** — values that ECMAScript represents with scientific notation **MUST** use lowercase `e` and a signed exponent (examples: `5e-324`, `1e+21`).
4. **No trailing zeros** — `1.50` **MUST** encode as `1.5`; `1.0` **MUST** encode as `1`.
5. **No leading plus** — positive numbers **MUST NOT** use a `+` sign prefix.
6. **Shortest round-trip** — output **MUST** be the shortest decimal string that round-trips to the same IEEE 754 binary64 (JCS uniqueness guarantee).
7. **Range and precision** — integrators **MUST NOT** assume decimal wire literals beyond ±2^53 preserve integer precision; only the parsed binary64 value is semantically relevant.

`canonical_graph_version` is always integer `1` and encodes as `1`.

Implementations **MAY** use any serializer that produces byte-identical output to JCS number encoding (for example V8 `Number::toString`, Ryu with ECMAScript-compatible exponent selection). Golden vectors below are authoritative for cross-checking.

## Field-change matrix

Whether `graph_revision` **MUST** change when only the listed aspect differs:

| Field or aspect | Revision changes? | Notes |
|---|---|---|
| `canonical_graph_version` | yes | New major canonical version defines new identity space |
| `initial_state` / `initial_state_id` | yes | |
| State `id` set | yes | |
| State `final` flag | yes | |
| `static_guidance` kind or text | yes | Text vs none changes capability contract |
| State `metadata` content | yes | Omitted vs present `{}` does not apply — empty omits |
| Transition `source_state` / `event` / `target_state` | yes | |
| `gate_ids` set membership | yes | Order ignored after canonical sort |
| Transition `metadata` content | yes | |
| Input `id` / `kind` / `required` | yes | |
| Input declaration `metadata` content | yes | |
| `live_guidance_supported` | yes | |
| Graph-level `metadata` content | yes | |
| Metadata number IEEE 754 value | yes | Sign, magnitude, and fractional part |
| Metadata number wire lexical form (same IEEE 754 value) | no | e.g. `1.0`, `1`, `1e0` all canonicalize to `1` |
| Metadata number `-0` vs `+0` | no | Both canonicalize to `0` |
| Non-finite metadata number on wire | reject | `provider.graph.invalid` before projection; no revision |
| Duplicate wire object keys | reject | `provider.graph.invalid` before projection; no revision |
| Provider wire key order | no | Stripped before canonicalization |
| Provider wire array order (states, transitions, inputs, gate_ids) | no | Canonical sort normalizes |
| Insignificant JSON whitespace in provider output | no | |
| Same-major unknown wire fields | no | Stripped before canonicalization |
| `provider_version` | no | Audit-only envelope field |
| `executable_digest` | no | Separate audit digest |
| `registration_id` / `config_revision` | no | Workflow registration identity, not graph |
| Run input **values** | no | Values are not in graph projection (I32) |
| Provider source code / gate implementation | no | Unless describe output changes |

## Golden vectors

Each vector lists exact canonical bytes and verified `graph_revision`. Wire excerpts appear where mapping illustration is useful. Bytes and digests were computed by the reference procedure below on 2026-07-17.

### Reference procedure

Integration-owned reference steps (core performs semantic validation and field projection only; no Serde or JSON encoding in core):

1. Parse provider wire `graph` JSON with [duplicate-key rejection](#duplicate-object-keys) and [finite binary64 number domain](#json-number-domain-pre-projection) (integrations).
2. Strip same-major unknown fields; map wire names to core semantics; validate per [Semantic validation](#semantic-validation-summary) (core + integrations).
3. Project digest-relevant fields into the canonical integration DTO v1 (integrations).
4. Apply [semantic ordering](#semantic-ordering-rules) to arrays and sort `metadata` object keys recursively.
5. Encode with [canonical byte rules](#canonical-byte-encoding), applying [metadata number encoding](#metadata-number-encoding) to every numeric value (minified JSON, UTF-8 byte lexicographic key order at every object, no trailing newline).
6. Compute `graph_revision` = `sha256:` + lowercase hex(SHA-256(canonical bytes)).

Steps 4–5 **MUST** depend only on semantic content. Provider wire key order, insignificant whitespace, hash-map iteration order, and provider array order before sorting **MUST NOT** affect output bytes.

### GV-01 — Minimal single-state graph

Maps the [provider-protocol-v1.md `describe` example](provider-protocol-v1.md#example-describe-result) (`static_guidance` string shorthand).

**Wire `graph` (excerpt):**

```json
{
  "initial_state": "draft",
  "states": [{"id": "draft", "static_guidance": "Prepare the change.", "final": false}],
  "transitions": [],
  "input_declarations": [],
  "live_guidance_supported": false
}
```

**Canonical bytes (232 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77`

### GV-02 — Two states with gated transition

**Canonical bytes (400 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":true,"id":"done","static_guidance":{"kind":"none"}},{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare."}}],"transitions":[{"event_id":"submit","gate_ids":["review-approved","tests-passed"],"source_state_id":"draft","target_state_id":"done"}]}
```

**`graph_revision`:** `sha256:4584a384f85d331737718124c7b201a57e12472fb4921f066cfa49f17d7d28a7`

Note: `states` sort with `done` before `draft` (`"done" < "draft"` in UTF-8 byte order).

### GV-03 — Reordering equivalence

Provider wire that permutes `states`, `gate_ids`, and `input_declarations` order but matches GV-02 semantics **MUST** produce identical canonical bytes and `graph_revision` as GV-02.

**Wire `graph` (permuted order; same semantics as GV-02):**

```json
{
  "initial_state": "draft",
  "states": [
    {"id": "done", "static_guidance": {"kind": "none"}, "final": true},
    {"id": "draft", "static_guidance": "Prepare.", "final": false}
  ],
  "transitions": [
    {
      "source_state": "draft",
      "event": "submit",
      "target_state": "done",
      "gate_ids": ["tests-passed", "review-approved"]
    }
  ],
  "input_declarations": [],
  "live_guidance_supported": false
}
```

**Canonical bytes:** identical to GV-02 (400 UTF-8 bytes).

**`graph_revision`:** `sha256:4584a384f85d331737718124c7b201a57e12472fb4921f066cfa49f17d7d28a7` (same as GV-02)

### GV-04 — Input declarations and live guidance

**Canonical bytes (296 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"start","input_declarations":[{"id":"repo","kind":"string","required":true},{"id":"ticket","kind":"string","required":false}],"live_guidance_supported":true,"states":[{"final":false,"id":"start","static_guidance":{"kind":"none"}}],"transitions":[]}
```

**`graph_revision`:** `sha256:a544edf97670e7fac5fe135d37eabd3a15b47d2cd54b7f7cff741da4bfd34ee2`

### GV-05 — Graph and state metadata

**Canonical bytes (294 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"metadata":{"version":"1","workflow":"example"},"states":[{"final":false,"id":"draft","metadata":{"owner":"team-a"},"static_guidance":{"kind":"text","text":"Go."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:93866bf1b909c1511970036da77f6509da4848b9663f464f009fef6585929060`

### GV-06 — Initial-final state

**Canonical bytes (206 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"instant","input_declarations":[],"live_guidance_supported":false,"states":[{"final":true,"id":"instant","static_guidance":{"kind":"none"}}],"transitions":[]}
```

**`graph_revision`:** `sha256:9906465342e0971b14f51b543de7558125938485a36db033a111051274702316`

### GV-07 — Gate-free transition

**Canonical bytes (332 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"a","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"a","static_guidance":{"kind":"none"}},{"final":true,"id":"b","static_guidance":{"kind":"none"}}],"transitions":[{"event_id":"finish","gate_ids":[],"source_state_id":"a","target_state_id":"b"}]}
```

**`graph_revision`:** `sha256:501a3c627bb31a7e742d8e3f5466076beeadc778f034c4be6b7c9ddd2704fde6`

### GV-08 — Transition metadata (revision differs from GV-07)

**Canonical bytes (357 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"a","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"a","static_guidance":{"kind":"none"}},{"final":true,"id":"b","static_guidance":{"kind":"none"}}],"transitions":[{"event_id":"finish","gate_ids":[],"metadata":{"auto":true},"source_state_id":"a","target_state_id":"b"}]}
```

**`graph_revision`:** `sha256:e0fdd1d618d511f2ebbcb6e6626c04cab4aa92c740cb0e5b3d68e17213c77f77`

### GV-09 — Graph metadata numbers (integer, fraction, exponent)

**Wire `graph` (excerpt):**

```json
{
  "initial_state": "draft",
  "states": [{"id": "draft", "static_guidance": "Go.", "final": false}],
  "transitions": [],
  "input_declarations": [],
  "live_guidance_supported": false,
  "metadata": {"count": 42, "large": 1e+21, "ratio": 1.5, "tiny": 5e-324}
}
```

**Canonical bytes (280 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"metadata":{"count":42,"large":1e+21,"ratio":1.5,"tiny":5e-324},"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Go."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:d6586d19813d7238e60b389e85ac7c293885c95445ed6db324d61279ce85a54f`

### GV-10 — Negative-zero metadata normalization

Wire `metadata.offset` is IEEE 754 `-0`. Canonical bytes **MUST** encode the value as `0`.

**Wire `graph` (excerpt):**

```json
{
  "initial_state": "draft",
  "states": [{"id": "draft", "static_guidance": "Go.", "final": false}],
  "transitions": [],
  "input_declarations": [],
  "live_guidance_supported": false,
  "metadata": {"offset": -0}
}
```

**Canonical bytes (240 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"metadata":{"offset":0},"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Go."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:d10f0a20022ee783a2532adca5c8251a861a8537d493107e51c1c1ac9b3703d9`

### GV-11 — Metadata number value change (revision differs from GV-09)

Same semantics as GV-09 except `metadata.count` is `43` instead of `42`.

**Canonical bytes (280 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"metadata":{"count":43,"large":1e+21,"ratio":1.5,"tiny":5e-324},"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Go."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:adfcff9ecec3c70358988762bdc593c738f30bbf596d453dfd1ed6f23c2734e7`

### GV-12 — Nested state metadata safe-integer boundaries

**Canonical bytes (272 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","metadata":{"limits":{"max":9007199254740991,"min":-1}},"static_guidance":{"kind":"text","text":"Go."}}],"transitions":[]}
```

**`graph_revision`:** `sha256:3354c9b751f17b7b4d8d34c108566959d121d50df0dc42f192f2f82e79c0bd0d`

### GV-13 — Transition metadata fractional negative number

**Canonical bytes (361 UTF-8 bytes):**

```text
{"canonical_graph_version":1,"initial_state_id":"a","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"a","static_guidance":{"kind":"none"}},{"final":true,"id":"b","static_guidance":{"kind":"none"}}],"transitions":[{"event_id":"finish","gate_ids":[],"metadata":{"weight":-0.125},"source_state_id":"a","target_state_id":"b"}]}
```

**`graph_revision`:** `sha256:46bf149868353f4633814edbb0366f856b26de504e21829054f040c0be32c1ef`

### GV-14 — Metadata number wire lexical equivalence

Provider wire that uses alternate JSON number spellings for the same IEEE 754 values as GV-09 **MUST** produce identical canonical bytes and `graph_revision` as GV-09.

**Wire `graph` (excerpt; alternate number spellings):**

```json
{
  "initial_state": "draft",
  "states": [{"id": "draft", "static_guidance": "Go.", "final": false}],
  "transitions": [],
  "input_declarations": [],
  "live_guidance_supported": false,
  "metadata": {"count": 42.0, "large": 1.0e+21, "ratio": 1.50, "tiny": 5e-324}
}
```

**Canonical bytes:** identical to GV-09 (280 UTF-8 bytes).

**`graph_revision`:** `sha256:d6586d19813d7238e60b389e85ac7c293885c95445ed6db324d61279ce85a54f` (same as GV-09)

## Persistence and journal usage

| Consumer | Field | Rule |
|---|---|---|
| Run row | stored graph snapshot | Canonical bytes at creation; immutable thereafter ([persistence.md](persistence.md)) |
| Run row | `graph_revision` | `sha256:` digest of stored canonical bytes |
| `run.created` journal entry | `graph_revision` | Same digest at creation ([journal-contract.md](journal-contract.md)) |
| `provider.check` conformance | `graph_revision` | Digest of emitted graph when valid ([cli-contract.md](cli-contract.md)) |
| `run.graph` / `run.show` | stored projection | Provider-free read of canonical snapshot |

Provider implementation drift does not recompute stored `graph_revision` on active runs (I7, I8, I43). New describe output affects new runs only.

## Schema implementation boundary

T084 publishes `schemas/provider/v1/graph.json` from the wire shapes in this document. T086 implements the mapping and canonical encoder in `integrations`. T041 implements the core semantic projection without serialization technology.

Breaking changes to canonical field semantics or ordering require a new `canonical_graph_version` and a new major provider protocol or explicit migration design; same-major provider protocol additions cannot redefine existing canonical fields.
