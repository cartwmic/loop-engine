# Loop Engine Persistence Schema

**Status:** Published SQLite schema reference for migration **`0001`** (`crates/loop-engine-integrations/migrations/0001_initial.sql`).

This document is the canonical DDL reference for bundled SQLite tables, columns, constraints, and indexes. Transactional semantics, CAS rules, and journal wire shapes remain in [persistence.md](persistence.md) and [journal-contract.md](journal-contract.md).

Related documents:

- [Persistence contract](persistence.md)
- [Journal contract](journal-contract.md)
- [Export contract](export-contract.md)
- [Graph projection](graph-projection.md)
- [System invariants](invariants.md) — I12–I16, I35–I36, I45

## Migration version

| Version | File | Scope |
|---|---|---|
| `0001` | `0001_initial.sql` | Initial MVP schema: metadata, catalog, runs, evidence, journal, sequences, associations |

Forward-only policy: newer binaries apply pending migrations; older binaries refuse newer databases ([persistence.md](persistence.md) § Schema-version compatibility).

## Authority and mutability summary

| Store | Authority | Mutability |
|---|---|---|
| `integration_metadata.integrity_key` | Integration-only HMAC secret for cursor v1 and disable-ack tokens | Insert once at migration; never rotated in MVP |
| `provider_registrations` | Machine-local provider catalog | Row never deleted; ID immutable; handle/executable/argv/CWD/timeout and `config_revision` mutate per catalog operations; tombstone clears handle |
| `runs` authoritative columns | Current workflow state, lifecycle, label, version guards, creation binding | Insert at `run.create`; state/lifecycle/label columns update per operation rules; graph/inputs/binding immutable after insert |
| `runs` JSON columns | Stored canonical graph projection and frozen inputs | Immutable after insert |
| `evidence` | Append-only run-scoped evidence inventory | Insert only; no update or delete |
| `journal_entries` | Append-only per-run activity log | Insert only; `encoded_payload_json` is wire authority for journal-contract fields |
| `evidence_associations` | Normalized attempt↔evidence links | Insert only; mirrors association facts also embedded in journal payload |
| `run_journal_sequences` | Per-run monotonic sequence allocator | Insert at run creation; `next_sequence` increments atomically inside journal-insert transactions |

Individual runs, evidence rows, journal rows, and graph snapshots **MUST NOT** be deleted in MVP (I45). Foreign keys use `ON DELETE RESTRICT`; no `DELETE` paths are defined for run-scoped tables.

## Table: `integration_metadata`

Installation-local integration facts not exposed through CLI, trace, or export.

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `key` | `TEXT` | `PRIMARY KEY`, `CHECK (key = 'integrity_key')` | Fixed metadata key | Immutable after insert | Literal `'integrity_key'` |
| `value` | `BLOB` | `NOT NULL`, `CHECK (length(value) = 32)` | 32-byte HMAC key material | Insert once at `0001`; no MVP rotation | Raw bytes from `randomblob(32)` at first migration |

**Initialization:** migration `0001` inserts exactly one row. Open fails with `persistence.failed` when the row is missing or not 32 bytes ([persistence.md](persistence.md) § Integration integrity key).

## Table: `provider_registrations`

Machine-local executable provider catalog ([persistence.md](persistence.md), [invariants.md](invariants.md) I37).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `registration_id` | `TEXT` | `PRIMARY KEY`, `CHECK (length > 0)` | Stable immutable workflow identity | Immutable | Opaque registration ID string |
| `handle` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Human-resolvable handle among enabled rows | Set on add/restore/rename; cleared (`NULL`) on disable tombstone | Lowercase handle per [operation-catalog.md](operation-catalog.md) grammar (validated before insert) |
| `enabled` | `INTEGER` | `NOT NULL`, `CHECK (enabled IN (0, 1))` | Enabled vs tombstoned | `1` enabled; `0` tombstoned | SQLite boolean (`1`/`0`) |
| `config_revision` | `INTEGER` | `NOT NULL`, `CHECK (config_revision > 0)` | Monotonic catalog revision | Increments on add (initial `1`), update, disable, restore; unchanged on rename | Positive integer |
| `executable` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Absolute executable path (`argv[0]`) | Replaced on add/update/restore | Filesystem path string |
| `argv_json` | `TEXT` | `NOT NULL`, `DEFAULT '[]'`, `CHECK (json_valid)` | Provider argv after executable | Replaced atomically with executable/CWD/timeout on update/restore | JSON array of strings (`argv[1..]`) |
| `working_directory` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Spawn working directory | Replaced on add/update/restore | Absolute directory path |
| `timeout_seconds` | `INTEGER` | `NOT NULL`, `CHECK (timeout_seconds > 0)` | Provider wall-clock timeout | Replaced on add/update/restore | Positive integer seconds |
| `created_at` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Registration creation instant | Set once at insert | RFC 3339 UTC with millisecond precision |
| `updated_at` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Last catalog mutation instant | Updated on every catalog write | RFC 3339 UTC with millisecond precision |

**Table constraints:**

| Name | Rule |
|---|---|
| `enabled_handle_consistency` | `(enabled = 1 AND handle IS NOT NULL) OR (enabled = 0 AND handle IS NULL)` |

**Indexes:**

| Index | Columns | Condition | Purpose |
|---|---|---|---|
| `idx_provider_registrations_handle_enabled` | `(handle)` **UNIQUE** | `WHERE enabled = 1` | Handle uniqueness among enabled registrations → `catalog.handle.duplicate` / `catalog.handle.occupied` |
| `idx_provider_registrations_created_id` | `(created_at, registration_id)` | — | `provider.list` pagination ([cli-contract.md](cli-contract.md) § Collections) |
| `idx_provider_registrations_enabled_created_id` | `(created_at, registration_id)` | `WHERE enabled = 1` | Default enabled-only list pagination |

Registration rows are never deleted; disable tombstones in place.

## Table: `runs`

Authoritative per-run workflow state plus immutable creation artifacts ([persistence.md](persistence.md) § Scope and authority, I12).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `run_id` | `TEXT` | `PRIMARY KEY`, `CHECK (length > 0)` | Stable run identifier | Immutable after insert | Opaque run ID string |
| `registration_id` | `TEXT` | `NOT NULL`, `FK → provider_registrations`, `ON DELETE RESTRICT` | Creation-time registration binding | Immutable | Registration ID |
| `config_revision_at_create` | `INTEGER` | `NOT NULL`, `CHECK (> 0)` | Observed catalog revision at successful `run.create` | Immutable | Positive integer |
| `current_state` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Current workflow state ID | Updates on accepted transitions | State identifier from stored graph |
| `lifecycle` | `TEXT` | `NOT NULL`, `CHECK IN ('active','final','terminated')` | Run lifecycle | Updates on transition to final state or termination | `active`, `final`, or `terminated` |
| `workflow_state_version` | `INTEGER` | `NOT NULL`, `CHECK (> 0)` | Workflow-state CAS guard | Increments when workflow state ID changes | Positive integer; initial `1` |
| `lifecycle_version` | `INTEGER` | `NOT NULL`, `CHECK (> 0)` | Lifecycle CAS guard | Increments when lifecycle changes | Positive integer; initial `1` |
| `label_version` | `INTEGER` | `NOT NULL`, `CHECK (> 0)` | Label freshness (non-CAS for gates) | Increments on successful `run.label` while active | Positive integer; initial `1` |
| `label` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Display label | Updates on successful `run.label` while active | UTF-8 text or `NULL` |
| `graph_revision` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Canonical graph digest identity | Immutable | `sha256:` lowercase hex per [graph-projection.md](graph-projection.md) |
| `canonical_graph_version` | `INTEGER` | `NOT NULL`, `CHECK (= 1)` | Stored graph DTO major version | Immutable | Always `1` in MVP |
| `graph_canonical_projection_json` | `TEXT` | `NOT NULL`, `CHECK (json_valid)` | Stored canonical graph snapshot | Immutable | Canonical integration graph DTO object (export `state.json` → `graph`) |
| `inputs_json` | `TEXT` | `NOT NULL`, `DEFAULT '{}'`, `CHECK (json_valid)` | Frozen validated run inputs | Immutable | Provider-validated inputs object; `{}` when none |
| `created_at` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Run creation instant | Immutable | RFC 3339 UTC with millisecond precision |

**Indexes:**

| Index | Columns | Condition | Purpose |
|---|---|---|---|
| `idx_runs_registration_lifecycle_id` | `(registration_id, lifecycle, run_id)` | — | Active-set digest, disable impact queries, registration-scoped listing |
| `idx_runs_registration_active_id` | `(registration_id, run_id)` | `WHERE lifecycle = 'active'` | Fast active-run enumeration per registration |
| `idx_runs_created_id` | `(created_at, run_id)` | — | `run.list` catalog pagination |

Runs are never deleted (I45).

## Table: `run_journal_sequences`

Per-run journal sequence allocator ([persistence.md](persistence.md) § Internal version and sequence fields).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `run_id` | `TEXT` | `PRIMARY KEY`, `FK → runs`, `ON DELETE RESTRICT` | Run scope | Insert with run; never deleted | Run ID |
| `next_sequence` | `INTEGER` | `NOT NULL`, `CHECK (next_sequence >= 1)` | Next sequence to allocate | Atomically incremented in the same write transaction as each journal insert | Positive integer; `2` after creation entry at sequence `1` |

**Allocation semantics:** within a `BEGIN IMMEDIATE` write transaction, integration reads `next_sequence`, inserts the journal row at that value, then increments `next_sequence`. Concurrent writers serialize on SQLite; unique `(run_id, sequence)` detects logic defects.

## Table: `evidence`

Append-only run-scoped evidence inventory ([export-contract.md](export-contract.md) § `evidence[]`, I35).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `run_id` | `TEXT` | `NOT NULL`, `FK → runs`, `ON DELETE RESTRICT` | Run scope | Immutable | Run ID |
| `evidence_id` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Stable run-scoped evidence ID | Immutable | Opaque evidence ID |
| `kind` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Evidence kind | Immutable | Provider/caller kind string |
| `locator` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Opaque external locator | Immutable | Bounded opaque locator string; never dereferenced by engine |
| `digest` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Optional content digest | Immutable | Digest string when supplied |
| `media_type` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Optional media type | Immutable | Media type string when supplied |
| `metadata_json` | `TEXT` | nullable, `CHECK (NULL OR json_valid)` | Bounded metadata object | Immutable | JSON object when supplied |
| `source` | `TEXT` | `NOT NULL`, `CHECK IN ('caller','provider')` | Submitter category | Immutable | `caller` or `provider` |
| `created_at` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Append instant | Immutable | RFC 3339 UTC with millisecond precision |

**Primary key:** `(run_id, evidence_id)` — duplicate ID → `evidence.invalid` ([persistence.md](persistence.md) § SQLite constraint mapping).

**Indexes:**

| Index | Columns | Purpose |
|---|---|---|
| `idx_evidence_run_created_id` | `(run_id, created_at, evidence_id)` | `run.evidence list` and export ordering |

Evidence rows are append-only; no update or delete path.

## Table: `journal_entries`

Append-only per-run activity journal ([journal-contract.md](journal-contract.md), I13–I14).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `run_id` | `TEXT` | `NOT NULL`, `FK → runs`, `ON DELETE RESTRICT` | Run scope | Immutable | Run ID |
| `sequence` | `INTEGER` | `NOT NULL`, `CHECK (sequence > 0)` | Per-run monotonic order | Immutable after insert | Positive integer allocated from `run_journal_sequences` |
| `outcome` | `TEXT` | `NOT NULL`, `CHECK IN ('completed','rejected','error')` | Entry outcome class | Immutable | Denormalized from wire payload for constraint mapping and queries |
| `encoded_payload_json` | `TEXT` | `NOT NULL`, `CHECK (json_valid)` | Full journal entry wire object | Immutable | UTF-8 JSON object per journal-contract v1, including `state_before`/`state_after`, attempt nests, provider observations, stale-evaluation facts, and correction links |

**Primary key:** `(run_id, sequence)` — duplicate insert → `persistence.failed` (unexpected race or logic defect). Primary-key order supports history/export ascending `(run_id, sequence)` scans.

Journal rows are never updated or deleted. Stale-evaluation attempts, terminal label rejections, and evidence-bearing rejections are represented as insert-only rows whose payload records provider observations and associations without advancing authoritative `runs` columns ([persistence.md](persistence.md) § Stale evaluation branch, § Attempt journaling).

Nested journal fields (`provider_observations`, `gate_verdict_facts`, `evidence_associations`, `transition`, `note`, `actor`, etc.) live inside `encoded_payload_json`; integration enforces journal-contract encoded-size bounds before insert.

## Table: `evidence_associations`

Normalized links from journal attempts to evidence records ([journal-contract.md](journal-contract.md) § Evidence associations nesting, I14/I35).

| Column | Type | Constraints | Authority | Mutability | Representation |
|---|---|---|---|---|---|
| `run_id` | `TEXT` | `NOT NULL` | Run scope | Immutable | Run ID |
| `journal_sequence` | `INTEGER` | `NOT NULL`, `CHECK (> 0)` | Journal entry for this attempt | Immutable | Sequence referencing `journal_entries` |
| `evidence_id` | `TEXT` | `NOT NULL`, `CHECK (length > 0)` | Associated evidence | Immutable | Evidence ID scoped to `run_id` |
| `event_id` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Transition event context | Immutable | Event ID when association is event-scoped |
| `gate_id` | `TEXT` | nullable, `CHECK (NULL OR length > 0)` | Gate context | Immutable | Gate ID when association is gate-scoped |

**Primary key:** `(run_id, journal_sequence, evidence_id)`

**Foreign keys:**

| FK | References | On delete |
|---|---|---|
| `(run_id, journal_sequence)` | `journal_entries (run_id, sequence)` | `RESTRICT` |
| `(run_id, evidence_id)` | `evidence (run_id, evidence_id)` | `RESTRICT` |

**Indexes:**

| Index | Columns | Purpose |
|---|---|---|
| `idx_evidence_associations_run_journal` | `(run_id, journal_sequence)` | Load associations for one journal entry / attempt |
| `idx_evidence_associations_run_evidence` | `(run_id, evidence_id)` | Reverse lookup: which attempts referenced an evidence ID |

Association rows insert in the same transaction as their journal entry and any new evidence rows. The journal payload remains authoritative for wire export; this table supports indexed lookup and atomic commit of association facts.

## Constraint-to-reason mapping

Expected `SQLITE_CONSTRAINT` mappings from [persistence.md](persistence.md) § SQLite constraint mapping:

| Enforcement | Context | Reason code |
|---|---|---|
| `idx_provider_registrations_handle_enabled` | `provider.add`, `provider.rename` | `catalog.handle.duplicate` |
| `idx_provider_registrations_handle_enabled` | `provider.restore` | `catalog.handle.occupied` |
| `evidence` PK `(run_id, evidence_id)` | `run.evidence.add`, attempt paths inserting evidence | `evidence.invalid` |
| `journal_entries` PK `(run_id, sequence)` | any journal insert | `persistence.failed` |

All other constraint or integrity failures map to `persistence.failed`.

## Deliberate exclusions from DDL

- Event-sourced projection tables rebuilding current state from journal.
- Run-delete or cascade-delete paths (I45).
- ORM-managed shadow tables or dual-write stores.
- Per-field decomposition of journal attempt nests (stored in `encoded_payload_json`).
- Provider-catalog journal rows (I40).
- SQL enforcement of encoded-size bounds, handle grammar, graph semantic validation, or RFC 3339 timestamp syntax (integration validates before write).

## Verification rules

- `integration_metadata` holds exactly one `integrity_key` row of 32 bytes initialized via `randomblob(32)`.
- Enabled handle uniqueness is enforced only among enabled rows via partial unique index.
- Runs bind `(registration_id, config_revision_at_create)` with authoritative lifecycle/state/version columns separate from journal payload.
- Graph snapshot and inputs are immutable JSON columns; authoritative current state lives in typed columns.
- Journal is append-only with unique `(run_id, sequence)` and atomic allocation via `run_journal_sequences`.
- Evidence and evidence associations are append-only with run-scoped IDs.
- All run-scoped foreign keys use `ON DELETE RESTRICT`; no cascade run delete exists.
- Stale-evaluation and evidence-association attempts are representable as journal inserts (optionally with association rows) without mutating workflow/lifecycle columns.
