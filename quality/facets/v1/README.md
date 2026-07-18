# Operation facet inventories (v1)

**Status:** Frozen by T004 (2026-07-17).

Each exposed application operation owns one machine-readable facet manifest validated by [schema.json](schema.json). Manifests record behavioral closure required by [testing.md](../../../docs/testing.md) and [operation-catalog.md](../../../docs/operation-catalog.md).

## Path convention

```text
quality/facets/v1/<operation-id>.json
```

Examples:

- `quality/facets/v1/provider.add.json`
- `quality/facets/v1/run.evidence.add.json`

The `operation_id` field inside the manifest **must** equal the filename stem exactly (including dots).

## Status rules

| Status | Meaning |
|---|---|
| `open` | Applicable facet not yet closed by production CLI evidence |
| `closed` | Passing production CLI E2E and/or trace proof recorded in `evidence` |

Exposure tasks start with all applicable rows `open`. A row becomes `closed` only when named evidence is recorded. Aggregate tasks (for example T174, T182) may repeat already-closed rows but cannot supply first proof for an operation's mandatory facet.

`closed` rows **must** include at least one `evidence` string naming exact proof, for example:

- `e2e:provider_add`
- `trace:/tmp/loop-home/logs/01J....jsonl`
- `request_id:01J...`
- `acceptance:I40`

## Applicable facet set

Derive applicable facet names from [operation-catalog.md](../../../docs/operation-catalog.md) § Behavioral facet flags. Every manifest includes the universal row:

**Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope**

No manifest may list a facet name outside [schema.json](schema.json) `facet_name` enum. Names must match [testing.md](../../../docs/testing.md) § Facet matrix exactly. Each manifest must include the universal valid-path row (enforced by schema `contains`). Closure validation (T062/T167) must reject duplicate facet `name` values in one manifest even when row bodies differ.

## Provider and persistence trace rows

- Operations that invoke provider code **must** include **Trace provider boundary** when applicable.
- Every MVP application operation touches persistence and **must** include **Trace persistence boundary**, closed using that operation's production CLI trace (attempted read/transaction, applicable version check, commit/rollback/read outcome).

## Validation

```bash
# JSON Schema parse (requires ajv or equivalent in CI)
python3 -c "import json; json.load(open('quality/facets/v1/schema.json'))"

# Per-manifest validation is enforced by xtask operation-coverage (T062/T167)
```

Exposure commits are blocked when any applicable row remains `open` unless an authorized candidate closure explicitly permits it (T146–T151, T159 intermediate trees only).

## Example (illustrative)

```json
{
  "schema_version": 1,
  "operation_id": "provider.add",
  "facets": [
    {
      "name": "Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope",
      "status": "open"
    },
    {
      "name": "Provider-catalog mutation",
      "status": "open"
    },
    {
      "name": "Rejectable provider-catalog mutation",
      "status": "open"
    },
    {
      "name": "Trace persistence boundary",
      "status": "open"
    }
  ]
}
```
