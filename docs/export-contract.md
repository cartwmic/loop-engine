# Loop Engine Export Contract

**Status:** Frozen by T015 (2026-07-17). Decision [D015](change/initial-implementation/decisions.md#d015--audit-export-scope).

This document is the canonical contract for `run.export` operation ownership, output-directory collision/permission/atomic-publication behavior, `manifest.json` / `state.json` / `journal.jsonl` schemas, deterministic ordering, file-hash rules, evidence and provider-observation inclusion, D006 additive/breaking compatibility and support-duration policy, structured CLI success shape, and the no-import guarantee. Journal entry wire shapes are owned by [journal-contract.md](journal-contract.md) (D011/T011); canonical graph bytes are owned by [graph-projection.md](graph-projection.md) (D014/T014); consistent read semantics are owned by [persistence.md](persistence.md) (D009/T009). Named numeric bounds are frozen in [cli-contract.md](cli-contract.md#resource-bounds-d008) (D008); this document references bound **names** only.

Related documents:

- [Decision D015](change/initial-implementation/decisions.md#d015--audit-export-scope)
- [Structured CLI contract](cli-contract.md) — envelope, rendering modes, D006 schema versioning
- [Application operation catalog](operation-catalog.md) — `run.export` argv, reason codes, facets
- [Journal contract](journal-contract.md)
- [Graph projection and canonical encoding](graph-projection.md)
- [Persistence contract](persistence.md)
- [Testing doctrine](testing.md)
- [Technology direction](technology.md)
- [System invariants](invariants.md) — I24, I41, I45
- [Published schema index](../schemas/index.json)

## Scope and authority

`run.export` is the **only** MVP application operation that writes audit export artifacts. Export is **read-only** with respect to authoritative SQLite state (I12–I15, C2):

| Rule | Requirement |
|---|---|
| Write authority | SQLite remains sole durable authority; export files are never registered in the database |
| Consistency | One consistent read snapshot of authoritative run state and journal rows ([persistence.md](persistence.md) § Read snapshot for export) |
| Inspection only | Export supports audit, regression artifacts, and human review — not mobility or recovery |
| No replay | Export does **not** promise journal replay, state reconstruction, or historical re-execution |
| No import | MVP provides **no** operation to import, restore, rebind, or ingest export files ([I41](invariants.md#i41-run-identity-is-not-workspace-identity)) |
| No locator dereference | Engine **MUST NOT** read, copy, or judge portability of external evidence locators during export ([I42](invariants.md#i42-evidence-retention-does-not-copy-external-content-automatically)) |
| No mutation | Export **MUST NOT** mutate, delete, or restore runs, graph snapshots, evidence, journal rows, or catalog registrations ([I45](invariants.md#i45-individual-runs-are-not-deleted)) |
| No stdout payload | Structured mode emits exactly one CLI outcome envelope on stdout; artifact bytes are written only under `--output <DIR>` ([cli-contract.md](cli-contract.md) § Rendering modes) |
| Boundary translation | Export DTOs are integration/delivery shapes only ([I24](invariants.md#i24-external-representations-translate-at-boundaries)) |

Rejected patterns:

- writing export bytes to stdout or stderr in structured mode;
- dual authority between SQLite and export files;
- treating export presence as proof of import/restore capability;
- dereferencing `file://`, relative, or provider-scheme evidence locators to embed external bytes;
- partial publication leaving final artifact names (`state.json`, `journal.jsonl`, `manifest.json`) in `<DIR>` before atomic directory rename;
- overwriting prior export contents in a non-empty directory.

Provider observations needed for inspection (`provider_observations`, `gate_verdict_facts`, drift locator/digest/version on attempts) live in **journal** entries per [journal-contract.md](journal-contract.md); `state.json` carries authoritative columns, stored graph, inputs, and evidence inventory.

## Operation ownership

| Concern | Owner |
|---|---|
| Application operation ID | `run.export` only |
| Production argv | `run export <RUN-ID> --output <DIR>` ([operation-catalog.md](operation-catalog.md)) |
| Provider subprocess | **None** — provider-free under missing provider |
| Per-run journal append | **None** — read/export only |
| Export artifact schemas | This document (`export_schema_version` `1`, `export_manifest_schema_version` `1`) |
| Implementation | `integrations` export encoder (T116); core export request shape (T060/T083); CLI adapter (T132/T166) |

No other application operation, driver function, migration hook, or `xtask` command writes `state.json`, `journal.jsonl`, or export `manifest.json` for a run.

## Output directory policy

`--output <DIR>` names the export root. `<DIR>` is bounded by `filesystem_path_utf8_bytes` ([cli-contract.md](cli-contract.md#resource-bounds-d008)).

Relative `<DIR>` values **MUST** be resolved to one absolute path before target validation and before the SQLite consistent-read snapshot. Export **MUST** reuse that same anchored absolute path for staging, `rename(2)`, parent `fsync`, and structured CLI `export.output`.

### Acceptance rules

| Condition | Result |
|---|---|
| Lexically invalid, oversize, or non-directory-creatable path | `outcome: rejected`, `reason.code: export.target.invalid` |
| Path exists and contains **any** filesystem entry (file or directory) | `outcome: rejected`, `reason.code: export.target.not_empty` |
| Path does not exist | `<DIR>` is created only by the atomic rename of a completed `0700` staging directory (D002); no pre-create |
| Path exists and is an empty directory | Publish by atomic rename replacing the empty directory |
| Parent not writable, permission denied, or media read-only | `outcome: rejected`, `reason.code: export.target.invalid` |
| Run ID not found | `outcome: rejected`, `reason.code: run.not_found` |
| Snapshot read fails unexpectedly | `outcome: error`, `reason.code: persistence.failed` |
| Payload write fails | `outcome: error`, `reason.code: resource.exhausted` |
| Private staging cleanup fails (except frozen collision loser below) | `outcome: error`, `reason.code: resource.exhausted` |
| Parent `fsync` fails after successful `rename(2)` (step 8) | `outcome: error`, `reason.code: resource.exhausted`; durability uncertain until fresh-process manifest verification |

Collision semantics are **reject**, never overwrite. A directory containing only hidden entries (names beginning with `.`) is **not** empty.

### Atomic publication

Export **MUST** publish by building the complete artifact set in a private **sibling staging directory** on the **same filesystem** as `<DIR>`, then atomically replacing `<DIR>` with that directory. Publication **MUST NOT** create final artifact names (`state.json`, `journal.jsonl`, `manifest.json`) directly under `<DIR>` before the staging directory rename completes.

#### Preconditions

Before creating a staging directory, export **MUST** verify:

| Condition | Requirement |
|---|---|
| `<DIR>` absent | Proceed; `<DIR>` is created only by rename |
| `<DIR>` exists | **MUST** be a directory with **no** filesystem entries (including hidden names beginning with `.`); otherwise `export.target.not_empty` |
| Parent of `<DIR>` | Writable and on the same filesystem as the staging directory will be created |

#### Publication steps

1. Open one consistent read snapshot of the target run and all journal rows ([persistence.md](persistence.md)).
2. Create a unique sibling staging directory in the parent of `<DIR>` (for example `<parent>/.loop-export-staging-<pid>-<nonce>`), mode `0700` on supported Unix platforms (D002). Staging **MUST** reside on the same filesystem as `<DIR>` so a single `rename(2)` publishes atomically.
3. Encode and write the **complete** artifact set inside staging: `state.json`, `journal.jsonl`, then `manifest.json` **last**.
4. Compute `sha256:` digests and byte lengths from the **on-disk bytes** of each payload file; manifest `files` **MUST** list only `journal.jsonl` and `state.json`.
5. `fsync` each payload file and `manifest.json`.
6. `fsync` the staging directory file descriptor.
7. Atomically `rename(2)` the staging directory to `<DIR>` using supported Unix semantics (D002):
   - when `<DIR>` does not exist: `rename(staging, <DIR>)` creates `<DIR>`;
   - when `<DIR>` exists as an empty directory: `rename(staging, <DIR>)` replaces the empty directory.
8. `fsync` the parent directory of `<DIR>`.

`manifest.json` is not listed in its own `files` array.

#### Failure and crash semantics

| Phase | `<DIR>` state | Staging state | Retry |
|---|---|---|---|
| Before step 2 completes | Absent or empty (unchanged) | Absent or removed | Unblocked |
| Steps 2–6 interrupted | Absent or empty (unchanged) | May exist as orphan | Unblocked; orphan removable per cleanup rules |
| After step 7, before step 8 completes | May contain a complete artifact set, or remain absent/empty if rename not yet observed | Absent if rename succeeded | See post-rename verification |
| After step 8 completes | Complete published export | Absent | `export.target.not_empty` on repeat |

On any failure before step 7: export **MUST** remove this invocation's staging directory when it exists; `<DIR>` **MUST** remain absent or empty.

On publication failure after staging creation, export **MUST** attempt to remove this invocation's private staging directory. Frozen concurrent/repeat loser semantics take precedence: when the publication error is `export.target.not_empty`, that rejection **MUST** be reported even if private staging cleanup also fails. For all other publication errors, staging cleanup failure **MUST** surface as `resource.exhausted`.

On failure during or after step 7: export **MUST NOT** delete or truncate `<DIR>` based solely on incomplete CLI exit status or abnormal process termination.

#### Post-rename ambiguity resolution

Step 8 (parent `fsync`) failure after a successful step 7 `rename(2)` leaves durability uncertain. The failing invocation **MUST** return `outcome: error`, `reason.code: resource.exhausted`. Same-process manifest/hash verification **MUST NOT** promote that invocation to success even when `<DIR>` already contains a matching manifest.

After a crash or I/O fault between steps 7 and 8, a **fresh later process** **MUST** determine export completeness by reading `<DIR>/manifest.json` and verifying every `files[*].sha256` and `bytes` against on-disk payload bytes. A matching manifest is the **only** completion boundary for that recovery path.

- Valid manifest with matching hashes: treat export as complete; **MUST NOT** roll back or delete `<DIR>` because a prior invocation exited with `resource.exhausted` or abnormal termination.
- Missing manifest, invalid manifest, or hash mismatch: treat `<DIR>` as not successfully published; cleanup **MAY** remove `<DIR>` only when it contains exclusively export artifacts from the failed attempt and no valid manifest; otherwise report `resource.exhausted` without deleting unrelated user content.

Consumers **MAY** apply the same manifest/hash verification. Payload files without a matching manifest are not a supported consumption state.

#### Collision races

| Scenario | Expected behavior |
|---|---|
| Concurrent `run.export` to the same `<DIR>` | Exactly one publication succeeds; others observe `export.target.not_empty` after the winner's rename or reject before staging when `<DIR>` is no longer empty |
| Concurrent exports to different `<DIR>` paths | Independent; unique staging directory names **MUST** prevent cross-invocation interference |
| Staging name collision in the same parent | Creation **MUST** retry with a new unique name or fail with `resource.exhausted`; **MUST NOT** reuse or delete another invocation's staging directory |
| External creation of a non-empty `<DIR>` before rename | Rename **MUST** fail safely; this invocation's staging removed; `export.target.not_empty` or `resource.exhausted` |

#### Orphan staging cleanup

Implementations **MAY** remove staging directories matching the implementation's staging prefix only when **all** of:

- the directory is a direct child of the parent used for export publication;
- the directory name matches the implementation's documented staging pattern (for example `.loop-export-staging-*`);
- the directory is owned by the current uid on supported Unix platforms (D002);
- the directory contains no valid `manifest.json` whose payload hashes verify, or the directory is older than an implementation-defined stale threshold for abandoned invocations.

Cleanup **MUST NOT** delete arbitrary user directories, unrelated hidden files, or a staging directory belonging to an in-flight export in another process.

## Artifact set and filenames

All paths are relative to `<DIR>`:

| File | Role |
|---|---|
| `manifest.json` | Completion marker; ordered payload inventory with `sha256:` and byte length |
| `state.json` | Versioned authoritative run snapshot |
| `journal.jsonl` | Versioned append-only journal snapshot |

Structured CLI success **MUST** include:

```json
"export": {
  "output": "<DIR>",
  "manifest_file": "manifest.json",
  "state_file": "state.json",
  "journal_file": "journal.jsonl"
}
```

When this document and [cli-contract.md](cli-contract.md) § Common `data` extensions differ on `run.export`, this document is authoritative until D015 is reopened.

## Schema versioning (D006 alignment)

Export uses independent integer schema fields per artifact. Rules mirror [cli-contract.md](cli-contract.md) § Schema versioning:

| Change kind | Rule |
|---|---|
| Additive | Backward-compatible only when the new field is optional and unknown fields may be ignored |
| Breaking | Removing, renaming, changing meaning or type, or making an optional field newly required requires a new schema version integer |
| MVP support | Implementations accept only `export_schema_version` `1`, `export_manifest_schema_version` `1`, and `journal_schema_version` `1` |
| Support duration | **No** support-duration promise is made for superseded export schema versions |

Journal lines reuse `journal_schema_version` from [journal-contract.md](journal-contract.md); export does not define a parallel journal version.

Published JSON Schema artifacts for export payloads are indexed at [schemas/index.json](../schemas/index.json):

| Artifact | Schema path | Version field | Value |
|---|---|---|---|
| one `journal.jsonl` line | `schemas/export/v1/journal-line.schema.json` | `journal_schema_version` | `1` |
| `manifest.json` | `schemas/export/v1/manifest.schema.json` | `export_manifest_schema_version` | `1` |
| `state.json` | `schemas/export/v1/state.schema.json` | `export_schema_version` | `1` |

Export artifact bytes are **immutable** after successful atomic publication: a published directory is never updated in place, and `run.export` rejects non-empty targets rather than overwriting prior exports.

## Canonical file encoding

Integration **MUST** encode JSON artifacts deterministically before hashing and write:

| Rule | Applies to |
|---|---|
| UTF-8 encoding | all JSON artifacts |
| Minified JSON (no insignificant whitespace) | `manifest.json`, `state.json`, each journal line object |
| Lexicographic ASCII sort of object keys at every nesting level | `manifest.json`, `state.json`, each journal line object |
| `sha256:` + 64 lowercase hex chars | manifest `files[*].sha256` |
| Digest input | exact on-disk file bytes after encoding |
| JSONL line terminator | each journal line ends with `\n` (including the final line) |

Journal line JSON **MUST** match the stored journal entry object shape from [journal-contract.md](journal-contract.md) byte-for-byte except for the required canonical key ordering and minification applied at export time.

## Deterministic ordering

| Collection | Sort key | Order |
|---|---|---|
| `journal.jsonl` lines | `sequence` | ascending |
| `state.json` → `evidence` | `(created_at, evidence_id)` | ascending `created_at`, then ascending `evidence_id` |
| `manifest.json` → `files` | `path` | ascending lexicographic ASCII |
| Manifest `files` inventory | fixed set | `journal.jsonl` then `state.json` |

Graph content inside `state.json` **MUST** use the stored canonical integration DTO object with semantic ordering already frozen in [graph-projection.md](graph-projection.md); export does not re-encode from provider wire JSON.

## `manifest.json` schema v1

| Field | Type | Required | Description |
|---|---|:---:|---|
| `export_manifest_schema_version` | integer | yes | Always `1` for this contract |
| `export_schema_version` | integer | yes | Always `1` for this contract |
| `run_id` | string | yes | Exported run identifier |
| `exported_at` | string | yes | RFC 3339 UTC timestamp with millisecond precision when export snapshot began |
| `files` | array | yes | Payload inventory; see below |

### `files[]` entry

| Field | Type | Required | Description |
|---|---|:---:|---|
| `path` | string | yes | `journal.jsonl` or `state.json` |
| `sha256` | string | yes | `sha256:` digest of file bytes |
| `bytes` | integer | yes | Length of file in bytes |

## `state.json` schema v1

| Field | Type | Required | Description |
|---|---|:---:|---|
| `export_schema_version` | integer | yes | Always `1` |
| `run_id` | string | yes | Stable run identifier |
| `exported_at` | string | yes | Same instant as `manifest.json` `exported_at` |
| `run` | object | yes | Authoritative run columns at snapshot time |
| `registration_binding` | object | yes | Immutable creation-time registration binding |
| `graph` | object | yes | Stored canonical graph snapshot |
| `inputs` | object | yes | Frozen run inputs at creation (`{}` when none) |
| `evidence` | array | yes | All evidence records for the run; `[]` when none |

### `run` object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `id` | string | yes | Run identifier |
| `label` | string or null | yes | Display label; `null` when unset |
| `lifecycle` | string | yes | `active`, `final`, or `terminated` |
| `state` | string | yes | Current workflow state identifier |
| `workflow_state_version` | integer | yes | Internal workflow-state version |
| `lifecycle_version` | integer | yes | Internal lifecycle version |
| `label_version` | integer | yes | Label freshness version |
| `created_at` | string | yes | RFC 3339 UTC timestamp with millisecond precision |

### `registration_binding` object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `registration_id` | string | yes | Stable registration ID bound at `run.create` |
| `config_revision_at_create` | integer | yes | `config_revision` observed at successful creation |

Export records creation binding only. It does **not** assert current catalog executable paths, handles, or post-create `config_revision` values.

### `graph` object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `graph_revision` | string | yes | `sha256:` digest of stored canonical bytes |
| `canonical_graph_version` | integer | yes | Always `1` ([graph-projection.md](graph-projection.md)) |
| `initial_state_id` | string | yes | Initial workflow state |
| `states` | array | yes | Canonical state objects |
| `transitions` | array | yes | Canonical transition objects |
| `input_declarations` | array | yes | Input declarations frozen at creation |
| `live_guidance_supported` | boolean | yes | Stored capability flag |

Additional canonical DTO fields present in the stored snapshot (for example transition `metadata`) **MUST** be included exactly as stored. Unknown fields from future additive graph versions **MUST** be preserved when present in the stored snapshot.

### `inputs` object

Provider-validated input values frozen at creation. Keys and value shapes follow the stored run-inputs DTO. Empty object when the run declared no inputs.

### `evidence[]` entry

| Field | Type | Required | Description |
|---|---|:---:|---|
| `evidence_id` | string | yes | Stable run-scoped evidence ID |
| `kind` | string | yes | Evidence kind supplied at append |
| `locator` | string | yes | Bounded opaque locator string; never dereferenced |
| `digest` | string or null | yes | Content digest when supplied |
| `media_type` | string or null | yes | Media type when supplied |
| `metadata` | object or null | yes | Bounded metadata object when supplied |
| `created_at` | string | yes | RFC 3339 UTC timestamp with millisecond precision |

## `journal.jsonl` schema v1

- One JSON object per line conforming to [journal-contract.md](journal-contract.md).
- Lines ordered by ascending `sequence`.
- Includes every stored journal entry for the run with nested `provider_observations`, `gate_verdict_facts`, and `evidence_associations` when present on the stored row.
- Empty runs are impossible after successful `run.create`; a run with only a creation entry produces one line.

## No-import guarantee

MVP **MUST NOT** ship:

- `run.import`, `run.restore`, or any ingest of `manifest.json` / `state.json` / `journal.jsonl`;
- catalog or run rebinding from export paths or locators;
- replay reducers that treat export as a write path;
- implicit promotion of export files into SQLite through migration, startup, or `xtask`.

Export consumers may diff, archive, or inspect offline. They **MUST NOT** assume cross-machine mobility, executable rebinding, or workspace identity preservation ([I41](invariants.md#i41-run-identity-is-not-workspace-identity)).

## Contract examples

All examples below are valid, linewise-parseable JSON/JSONL. Paths and IDs are illustrative.

### `state.json` (minimal active run)

```json
{"evidence":[],"export_schema_version":1,"exported_at":"2026-07-17T15:00:00.000Z","graph":{"canonical_graph_version":1,"graph_revision":"sha256:501a3c627bb31a7e742d8e3f5466076beeadc778f034c4be6b7c9ddd2704fde6","initial_state_id":"a","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"a","static_guidance":{"kind":"none"}},{"final":true,"id":"b","static_guidance":{"kind":"none"}}],"transitions":[{"event_id":"finish","gate_ids":[],"source_state_id":"a","target_state_id":"b"}]},"inputs":{},"registration_binding":{"config_revision_at_create":1,"registration_id":"01J9X3K2M4N5P6Q7R8S9T0ABC"},"run":{"created_at":"2026-07-17T14:00:00.123Z","id":"01J9X3K2M4N5P6Q7R8S9T0V2X","label":"checkout-redesign","lifecycle":"active","lifecycle_version":1,"label_version":1,"state":"a","workflow_state_version":1},"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X"}
```

### `journal.jsonl` (creation entry)

```json
{"entry_kind":"run.created","graph_revision":"sha256:501a3c627bb31a7e742d8e3f5466076beeadc778f034c4be6b7c9ddd2704fde6","journal_schema_version":1,"operation":"run.create","outcome":"completed","reason":null,"request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X","sequence":1,"state_after":{"lifecycle":"active","lifecycle_version":1,"state":"a","workflow_state_version":1},"state_before":{"lifecycle":"active","lifecycle_version":1,"state":"a","workflow_state_version":1},"ts":"2026-07-17T14:00:00.123Z"}
```

### `manifest.json` (illustrative digests)

Digest values are computed from the exact example `state.json` and `journal.jsonl` bytes in this section.

```json
{"export_manifest_schema_version":1,"export_schema_version":1,"exported_at":"2026-07-17T15:00:00.000Z","files":[{"bytes":528,"path":"journal.jsonl","sha256":"sha256:079ba5d73eabc24926e1d22d195bd2528e3acd96153f5745bcc2658f384f2603"},{"bytes":873,"path":"state.json","sha256":"sha256:d9c273186721835b47ef7696bdc9188eaaed20fbad92c84d3e544e41d00852b1"}],"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X"}
```

### Structured CLI success — `run.export`

```json
{
  "schema_version": 1,
  "operation": "run.export",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V5A",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V5A.jsonl",
  "outcome": "completed",
  "reason": null,
  "data": {
    "export": {
      "output": "/tmp/run-export-01J9X3K2M4N5P6Q7R8S9T0V2X",
      "manifest_file": "manifest.json",
      "state_file": "state.json",
      "journal_file": "journal.jsonl"
    }
  },
  "diagnostics": []
}
```

Exit `0`. Structured mode **MUST NOT** emit artifact bytes on stdout.

## Published export payload schemas

Machine-readable JSON Schemas for `manifest.json`, `state.json`, and one `journal.jsonl` line are listed in [schemas/index.json](../schemas/index.json). Journal line semantics remain owned by [journal-contract.md](journal-contract.md) (`journal_schema_version` `1`); export applies canonical key ordering and minification at write time without defining a parallel journal version.

| Property | Value |
|---|---|
| Exposure | **Published** — normative artifact shapes frozen by T015/T116 |
| Generation | None in repository tooling |
| Validation | Export encoder and `verify_published_export` integration checks enforce on-disk shapes; no separate schema-file validation command is defined |

Structured CLI success for `run.export` emits exactly one outcome envelope on stdout with `data.export` metadata only; artifact bytes are written only under `--output <DIR>` ([cli-contract.md](cli-contract.md) § Rendering modes).

## Verification rules (T015)

- `run.export` is the sole export writer; export never mutates SQLite or invokes providers.
- Output directory collision rejects with `export.target.not_empty` without overwriting.
- Permission and invalid path failures reject with `export.target.invalid`.
- Relative `--output <DIR>` is anchored to one absolute path before validation and SQLite snapshot, then reused through publication.
- Publication writes only inside a sibling staging directory until `rename(2)`; pre-rename failure removes staging and leaves `<DIR>` absent or empty.
- Frozen concurrent/repeat loser `export.target.not_empty` takes precedence even when private staging cleanup also fails; all other staging cleanup failures surface as `resource.exhausted`.
- Post-rename parent-`fsync` failure in the same invocation returns `resource.exhausted` without same-process manifest promotion; fresh later process manifest/hash verification remains the recovery rule without false rollback.
- `journal.jsonl` lines sort by `sequence`; `evidence` sorts by `(created_at, evidence_id)`; manifest `files` sorts by `path`.
- Manifest `sha256` values cover exact on-disk payload bytes.
- External evidence locators appear only as stored strings; export never dereferences them.
- No import/restore/replay operation exists; export is not competing authority.
- Contract examples parse as JSON / JSONL line-wise.
