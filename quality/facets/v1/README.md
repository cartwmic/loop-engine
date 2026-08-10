# Operation facet inventories (v1)

**Status:** Operation facet inventory schema v1 is published.

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

Coverage starts with all applicable rows `open`. A row becomes `closed` only when named evidence is recorded. Aggregate coverage may repeat already-closed rows but cannot supply first proof for an operation's mandatory facet.

`closed` rows **must** include at least one `evidence` string naming exact proof, for example:

- `e2e:provider_add`
- `trace:/tmp/loop-home/logs/01J....jsonl`
- `request_id:01J...`
- `acceptance:I40`

## Applicable facet set

Derive applicable facet names from [operation-catalog.md](../../../docs/operation-catalog.md) § Behavioral facet flags. Every manifest includes the universal row:

**Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope**

No manifest may list a facet name outside [schema.json](schema.json) `facet_name` enum. Names must match [testing.md](../../../docs/testing.md) § Facet matrix exactly. Each manifest must include the universal valid-path row (enforced by schema `contains`). Manifest validation must reject duplicate facet `name` values in one manifest even when row bodies differ.

## Provider and persistence trace rows

- Operations that invoke provider code **must** include **Trace provider boundary** when applicable.
- Every MVP application operation touches persistence and **must** include **Trace persistence boundary**, closed using that operation's production CLI trace (attempted read/transaction, applicable version check, commit/rollback/read outcome).
- Trace-facet evidence uses the `trace:e2e:<module>::<test>` namespace. Closure rejects untyped trace evidence and trace-prefixed evidence attached to non-trace facets.

## Validation

```bash
# JSON syntax check
python3 -c "import json; json.load(open('quality/facets/v1/schema.json'))"

# Per-manifest validation is enforced by `cargo test -p loop-engine-cli --test driver_catalog`
```

Coverage review should account for every applicable row and its recorded evidence.

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
