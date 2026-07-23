# Loop Engine CLI Contract

**Status:** Frozen by T006 (2026-07-17); resource bounds, collection pagination, and cursor v1 frozen by T008 (2026-07-17). Decisions [D006](change/initial-implementation/decisions.md#d006--structured-cli-contract) and [D008](change/initial-implementation/decisions.md#d008--resource-bounds-and-timeout-defaults).

This document is the canonical contract for production `loop-engine` CLI rendering, global flags, structured outcome envelope schema v1, human/structured parity, stdout/stderr/trace boundaries, process exit codes, **resource bounds**, **collection pagination**, and **cursor v1**. Application subcommand argv for the frozen 21-operation target catalog is defined in [operation-catalog.md](operation-catalog.md) (D004); current alpha runtime exposure is the nine-operation subset named below. This document owns global flags, outcome rendering, bounds, and pagination only.

Related documents:

- [Decision D006](change/initial-implementation/decisions.md#d006--structured-cli-contract)
- [Decision D008](change/initial-implementation/decisions.md#d008--resource-bounds-and-timeout-defaults)
- [Application operation catalog](operation-catalog.md)
- [System invariants](invariants.md) — I18, I27, I34, I46
- [Interaction storyboards](ux-storyboards.md)
- [Testing doctrine](testing.md)
- [Published schema index](../schemas/index.json)

## Executable and namespaces

Production binary name: `loop-engine`.

MVP's final catalog contains exactly **21** application operations in two namespaces (`provider.*`, `run.*`). During the 2026-07-22 staged implementation, `--list-operations` reports only checkpoint-closed runtime routes; Checkpoints A through D expose `provider.add`, `provider.list`, `provider.check`, `run.create`, `run.list`, `run.terminate`, `run.show`, `run.request`, and `run.history`. Final closure requires all 21 IDs. No additional application operation, alias, or hidden route is permitted without reopening D004. CLI `--help`, `--version`, pre-dispatch usage display, and `--list-operations` are driver functions, not application operations ([operation-catalog.md](operation-catalog.md) § Explicit non-operations).

## Schema versioning

Structured CLI outcome envelope uses integer field `schema_version` with current value `1`.

| Change kind | Rule |
|---|---|
| Additive | Backward-compatible only when the new field is optional and unknown fields may be ignored by readers |
| Breaking | Removing, renaming, changing meaning or type, or making an optional field newly required requires a new `schema_version` |
| Support | MVP accepts only `schema_version` `1`. No support-duration promise is made for superseded versions |

The same additive/breaking/support rule applies to audit export schemas (D015).

Within major version `1`, same-major evolution is additive: new optional fields may appear and readers ignore unknown fields. Wire JSON consumed by integrations **MUST** reject duplicate object keys and trailing values after the first complete document ([provider-protocol-v1.md](provider-protocol-v1.md) § Byte framing; [graph-projection.md](graph-projection.md) § Wire JSON parse requirements).

## Rendering modes

| Mode | Selection | Application dispatch stdout | Driver metadata stdout | Pre-dispatch failure |
|---|---|---|---|---|
| Human | default; `--format human` | Human-readable lines derived from the same outcome DTO | Human-readable driver output (see [Driver metadata outputs](#driver-metadata-outputs)) | Rich human-readable stderr |
| Structured | `--format json` | Exactly one UTF-8 JSON outcome envelope | One UTF-8 JSON driver-metadata object (not an outcome envelope; see [Driver metadata outputs](#driver-metadata-outputs)) | Rich stderr; when trace exists or failure is parseable, stderr carries one JSON pre-dispatch failure object (see below) |

Human and structured modes invoke the same operations, observe the same underlying state, and map to the same semantic outcome class, reason code, and exit code (I18). Presentation format must not create a privileged transition path.

## Global flags

Global flags apply before application subcommands. Configuration path and TOML precedence are frozen in `configuration.md` (T007); only rendering and driver-metadata globals are owned here.

| Flag | Purpose |
|---|---|
| `--format <human\|json>` | Output rendering mode. Default: `human`. |
| `--help`, `-h` | Emit usage help on stdout. Initializes trace per I46/D010. See [Driver metadata outputs](#driver-metadata-outputs). |
| `--version` | Emit build/version metadata on stdout. Initializes trace per I46/D010. See [Driver metadata outputs](#driver-metadata-outputs). |
| `--list-operations` | Emit currently exposed application operation IDs and argv templates on stdout (nine in the alpha). Driver metadata, not an application operation. See [Driver metadata outputs](#driver-metadata-outputs). |

Environment variable `LOOP_ENGINE_HOME` overrides machine-local roots for tests and portable use (D007). It is not a CLI flag.

Unsupported host targets fail pre-dispatch with exit `64` and rich stderr naming the detected target and supported triples (D002).

## Application argv surface

Application subcommand argv is frozen in [operation-catalog.md](operation-catalog.md) § Production CLI surface. The tables below are an exact copy for contract closure; if text diverges, `operation-catalog.md` is authoritative until D004 is reopened.

**Alpha availability:** only `provider.add`, `provider.check`, `provider.list`, `run.create`, `run.history`, `run.list`, `run.request`, `run.show`, and `run.terminate` are callable. Remaining rows document deferred WP6 target syntax and are not hidden routes. `--list-operations` is authoritative for installed-binary availability.

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

Paged operations additionally expose `--cursor` and `--limit` as defined in the catalog. Cursor v1 schema, numeric bounds, and pagination rules are frozen in [Resource bounds (D008)](#resource-bounds-d008) and [Collection pagination and cursor v1](#collection-pagination-and-cursor-v1) below.

## Resource bounds (D008)

**Canonical source of truth.** Every scalar, payload, path, argv, configuration, diagnostic, trace, and timeout bound is named once in this table. Other foundation documents reference these names; they **MUST NOT** restate numeric limits independently.

| Name | Bound | Applies to |
|---|---:|---|
| `identifier_utf8_bytes` | 128 | Registration IDs, run IDs, evidence IDs, invocation IDs, and other stable identifiers |
| `provider_handle_utf8_bytes` | 128 | Provider handles (D004 grammar) |
| `run_label_utf8_bytes` | 256 | Optional run display labels |
| `note_text_utf8_bytes` | 65,536 (64 KiB) | Annotation and attempt note text |
| `actor_metadata_encoded_bytes` | 16,384 (16 KiB) | Opaque actor metadata on journal attempts |
| `evidence_locator_utf8_bytes` | 8,192 (8 KiB) | Evidence locator strings (see [Evidence locator policy](#evidence-locator-policy)) |
| `filesystem_path_utf8_bytes` | 4,096 (4 KiB) | One filesystem path in argv, configuration, or export paths |
| `provider_argv_element_count` | 128 | Elements in `registration.argv` (`argv[1..]`) |
| `provider_argv_element_utf8_bytes` | 16,384 (16 KiB) | One provider argument string |
| `provider_argv_encoded_total_bytes` | 262,144 (256 KiB) | Sum of UTF-8 bytes across all `registration.argv` elements |
| `toml_config_file_bytes` | 1,048,576 (1 MiB) | One global or project TOML configuration file |
| `provider_request_json_bytes` | 4,194,304 (4 MiB) | One provider protocol request envelope on stdin |
| `provider_result_stdout_bytes` | 1,048,576 (1 MiB) | One provider protocol result envelope on stdout |
| `provider_stderr_trace_bytes` | 1,048,576 (1 MiB) | Provider stderr retained in operational trace per invocation |
| `graph_projection_canonical_bytes` | 524,288 (512 KiB) | Canonical graph projection emitted by `describe` |
| `run_inputs_encoded_total_bytes` | 1,048,576 (1 MiB) | Accepted run input values encoded total |
| `evidence_record_encoded_bytes` | 65,536 (64 KiB) | One evidence record as stored or submitted |
| `inline_evidence_context_total_bytes` | 1,048,576 (1 MiB) | Inline evidence in one gate/guidance/request attempt |
| `selected_evidence_context_total_bytes` | 1,048,576 (1 MiB) | Caller-selected existing evidence in one gate/guidance/request attempt |
| `provider_snapshot_envelope_bytes` | 524,288 (512 KiB) | Non-evidence provider snapshot/envelope in one gate request |
| `guidance_text_bytes` | 262,144 (256 KiB) | Static or live guidance text in one result |
| `compatibility_finding_encoded_bytes` | 65,536 (64 KiB) | One compatibility or impact finding row |
| `journal_evidence_associations_encoded_bytes` | 262,144 (256 KiB) | Evidence associations in one journal entry |
| `journal_provider_facts_encoded_bytes` | 262,144 (256 KiB) | Provider observation facts in one journal entry |
| `journal_gate_verdict_facts_encoded_bytes` | 524,288 (512 KiB) | Gate/verdict facts in one journal entry |
| `journal_entry_encoded_bytes` | 2,621,440 (2.5 MiB) | One aggregate journal entry encoded |
| `diagnostic_encoded_bytes` | 8,192 (8 KiB) | One diagnostic object in an outcome envelope |
| `diagnostics_per_result_count` | 100 | Diagnostics array length per outcome envelope |
| `metadata_nesting_depth` | 16 | Maximum JSON/TOML metadata nesting depth |
| `provider_timeout_seconds_default` | 60 | Default `timeout_seconds` when unset ([configuration.md](configuration.md)) |
| `sqlite_busy_timeout_ms` | 5,000 | SQLite connection busy wait on `SQLITE_BUSY` ([persistence.md](persistence.md)) |
| `collection_page_default_count` | 100 | Default `--limit` for paged operations |
| `collection_page_max_count` | 1,000 | Maximum `--limit` for paged operations |
| `opaque_integrity_wire_utf8_bytes` | 768 | Complete URL-safe base64 opaque wire values for cursor v1, `--warning-cursor`, `data.next_cursor`, and disable `ack_token` |
| `collection_page_data_budget_bytes` | 3,145,728 (3 MiB) | Encoded `data` payload budget per page (items only) |
| `structured_cli_envelope_bytes` | 4,194,304 (4 MiB) | One structured CLI outcome envelope on stdout |
| `trace_init_reservation_bytes` | 16,777,216 (16 MiB) | Per-invocation trace file reservation at initialization |
| `trace_provider_call_reservation_bytes` | 10,485,760 (10 MiB) | Additional reservation before each provider subprocess call |
| `provider_calls_per_paged_invocation_max` | 10 | Maximum provider subprocess calls in one paged CLI invocation |
| `trace_file_max_bytes` | 125,829,120 (120 MiB) | Maximum encoded bytes for one active trace file |
| `trace_retained_files_max` | 100 | Maximum closed trace files retained in the trace directory |
| `trace_directory_budget_bytes` | 134,217,728 (128 MiB) | Cross-process trace directory budget including open files and unused reservations |

### Opaque integrity wire bound arithmetic

Complete opaque wire values (`--cursor`, `--warning-cursor`, `--allow-active-runs`, `data.next_cursor`, and `data.ack_token`) are measured **after** URL-safe base64 encoding (RFC 4648 §5, no padding) as UTF-8 bytes of the wire string callers pass and parsers read. Inner canonical JSON (`{"mac":"…","payload":{…}}`) is measured **before** base64 encoding when computing worst-case size.

Symbolic components (see [Resource bounds (D008)](#resource-bounds-d008)):

| Symbol | Meaning |
|---|---|
| `I` | `identifier_utf8_bytes` (128) |
| `D` | SHA-256 lowercase hex digest length (64) |
| `M` | HMAC-SHA-256 tag as URL-safe base64 without padding (43) |
| `L` | Longest cursor `collection` name length (32; `provider.registration_active_runs`) |
| `R` | Maximum `config_revision` decimal digit count (19) |
| `T` | Maximum RFC 3339 UTC timestamp length in `last_key.created_at` (24) |

Disable-ack inner payload worst case (two SHA-256 digests, registration ID, revision metadata):

`P_ack = 148 + 2×D + I + R` (148 B = canonical JSON key/punctuation overhead including `token_kind` and `token_version`)

Cursor inner payload worst case (filter fingerprint, optional traversal digest, timestamp + stable ID `last_key`):

`P_cur = 135 + L + 2×D + T + I` (135 B = canonical JSON key/punctuation overhead including `last_key` framing)

HMAC wrapper and pre-base64 wire object:

`W_wrapper = 21 + M` (64 B outer `{"mac":"…","payload":…}` shell excluding inner `payload` object bytes only)

`W_pre = W_wrapper + max(P_ack, P_cur)` (worst case 511 B)

Post-base64 wire string:

`W_post = ⌈W_pre × 4/3⌉` (worst case 682 B)

Framing headroom:

`H_wire = opaque_integrity_wire_utf8_bytes − W_post` (86 B)

Therefore minted and accepted opaque wire values satisfy `len_utf8(wire) ≤ opaque_integrity_wire_utf8_bytes`. Inner payload field values remain subject to their component bounds (for example `registration_id` and `run_id` at most `I`).

### Aggregate envelope arithmetic

Component budgets are subordinate to aggregate envelopes:

- **Gate request** — at most `selected_evidence_context_total_bytes` + `inline_evidence_context_total_bytes` + `run_inputs_encoded_total_bytes` + `provider_snapshot_envelope_bytes`, leaving 512 KiB framing headroom inside `provider_request_json_bytes` (4 MiB).
- **Describe result** — at most `graph_projection_canonical_bytes` (512 KiB) leaves result-envelope and diagnostic headroom inside `provider_result_stdout_bytes` (1 MiB).
- **Journal entry** — diagnostic aggregate at most 800 KiB; together with `journal_evidence_associations_encoded_bytes`, `journal_provider_facts_encoded_bytes`, `journal_gate_verdict_facts_encoded_bytes`, `note_text_utf8_bytes`, `actor_metadata_encoded_bytes`, and framing, one encoded entry remains below `journal_entry_encoded_bytes` (2.5 MiB) and therefore fits one `collection_page_data_budget_bytes` (3 MiB) history page without record truncation.
- **Structured CLI envelope** — `structured_cli_envelope_bytes` (4 MiB) retains at least 1 MiB framing/diagnostic headroom below the cap; each `next_cursor` or `ack_token` wire value is at most `opaque_integrity_wire_utf8_bytes` (post-base64 UTF-8 bytes).

### Overflow and rejection policy

- Caller-owned overflow (configuration, argv, selected evidence, pre-spawn request assembly) **rejects** before provider invocation (`resource.exhausted` or domain rejection as applicable).
- Caller-supplied opaque wire values (`--cursor`, `--warning-cursor`, `--allow-active-runs`) exceeding `opaque_integrity_wire_utf8_bytes` UTF-8 bytes (post-base64) **reject** before dispatch (`cursor.invalid` or `catalog.ack_token.invalid` as applicable).
- Provider-owned malformed or oversized authoritative results map to provider protocol errors.
- **Selected evidence is never truncated** (I35, I42). Selected context exceeding `selected_evidence_context_total_bytes` rejects before provider invocation.
- Captured provider stdout in operational trace retains in-bound bytes exactly; when retention exceeds `provider_result_stdout_bytes`, integration stores a prefix and explicit truncation metadata (original byte length and truncated flag); oversized remainder is drained but not stored ([provider-protocol-v1.md](provider-protocol-v1.md) § Stdout).
- Captured provider stderr **may** use explicit truncation markers in operational trace only when the protocol stdout result remains independently complete ([provider-protocol-v1.md](provider-protocol-v1.md) § Stderr).

### Evidence locator policy

Evidence locators are bounded opaque non-empty UTF-8 strings with no NUL or C0/C1 control characters, at most `evidence_locator_utf8_bytes`. The engine does not parse locators as URIs or paths, dereference them, resolve them against caller CWD, or judge portability. Self-contained versus provider-documented input-relative meaning is a caller/provider convention; the engine rejects only syntax and bounds violations and preserves exact locator bytes.

### Contract owners

| Concern | Canonical document | Freezing task |
|---|---|---|
| Named bounds table (this section) | `cli-contract.md` | T008 |
| Cursor v1 schema and pagination | `cli-contract.md` | T008 |
| Provider byte framing and timeout termination | `provider-protocol-v1.md` | T005 (bounds referenced here) |
| Configuration file size and built-in `timeout_seconds` | `configuration.md` | T007 (bounds referenced here) |
| SQLite connection pragmas and busy/locking policy | `persistence.md` | T009 (bounds referenced here) |
| Trace reservation behavior and rotation | `technology.md` (summary); `operational-trace.md` (T010) | T008/T010 |
| Provider JSON wire schemas | `provider-protocol-v1.md` + `schemas/provider/v1/*` | T084 |
| Structured CLI outcome JSON Schema | `schemas/cli/v1/outcome.schema.json` + [schema index](../schemas/index.json) | T125/T134 |
| Journal entry wire shapes | `journal-contract.md` | T011 |

End-to-end proof owners for pagination and trace reservations: T147 (registration list and zero-row `--active-runs-for`), T150/T152 (run list, history, and `--active-runs` compatibility pages), T157/T175 (nonempty impact/disable warning pages), T160 (evidence list), T101/T152/T182 (trace directory budget and per-invocation reservation limits).

## Collection pagination and cursor v1

Every growing collection or report is **count- and byte-paged**. Paged operations expose `--cursor` and `--limit`.

### Page encoding rules

| Rule | Value |
|---|---|
| Default `--limit` | `collection_page_default_count` (100) |
| Maximum `--limit` | `collection_page_max_count` (1,000) |
| Page data budget | `collection_page_data_budget_bytes` (3 MiB) encoded bytes in returned `data.items` (and operation-specific row payloads counted toward the page) |
| Count semantics | `--limit` is a record **count ceiling**, never a size promise |
| Byte stop | Encoder stops before the page data budget; emits `next_cursor` to the first unreturned record; **never truncates a record** |
| Provider-call stop | Provider-invoking pages additionally stop before the eleventh provider subprocess call (`provider_calls_per_paged_invocation_max`) |
| Structured response | Paged reads return `data.items` and optional `data.next_cursor` per [Common `data` extensions](#common-data-extensions) |

### Paged surfaces and sort keys

| Collection name | CLI surface | Sort key (ascending) | Filter fingerprint inputs |
|---|---|---|---|
| `provider.registrations` | `provider.list` | `(created_at, stable_id)` | `enabled`, `tombstoned` flags |
| `provider.registration_active_runs` | `provider.list --active-runs-for <REGISTRATION-ID>` | `run_id` | `registration_id` |
| `provider.check_active_runs` | `provider.check <TARGET> --active-runs` | `run_id` | resolved `registration_id` |
| `provider.disable_warnings` | `provider.disable <TARGET>` (warning pagination) | `run_id` | resolved `registration_id`, `config_revision`, active-set digest |
| `run.catalog` | `run.list` | `(created_at, stable_id)` | `terminal`, `all` flags |
| `run.history` | `run.history <RUN-ID>` | `sequence` (immutable per-run journal sequence) | `run_id` |
| `run.evidence` | `run evidence list <RUN-ID>` | `(created_at, evidence_id)` | `run_id` |

`provider.update` and other catalog mutations that return impact links use the `provider.registration_active_runs` collection and cursor form.

### Cursor v1 wire form

Cursors are opaque to callers: **URL-safe base64** (RFC 4648 §5, no padding) over a UTF-8 JSON object with an integration-owned integrity MAC ([persistence.md](persistence.md) § Integration integrity key). Callers **MUST NOT** construct, edit, or decode cursor payloads; they pass the opaque wire string only. The complete wire string **MUST** be at most `opaque_integrity_wire_utf8_bytes` UTF-8 bytes (post-base64); decoded inner JSON **MUST** satisfy [Opaque integrity wire bound arithmetic](#opaque-integrity-wire-bound-arithmetic) pre-base64 limits.

#### Wire object

| Field | Type | Required | Description |
|---|---|:---:|---|
| `payload` | object | yes | Canonical cursor payload (schema below); MAC input |
| `mac` | string | yes | URL-safe base64 (no padding) of the 32-byte HMAC-SHA-256 tag |

#### Payload schema

| Field | Type | Required | Description |
|---|---|:---:|---|
| `cursor_version` | integer | yes | Always `1` for cursor v1 |
| `collection` | string | yes | Collection name from the table above |
| `filter_fingerprint` | string | yes | Lowercase hex SHA-256 of the canonical filter JSON for this page (stable key order, no whitespace) |
| `last_key` | object | yes | Exclusive start key; shape depends on `collection` |
| `warning_traversal_digest` | string | `provider.disable_warnings` only | Lowercase hex SHA-256 over UTF-8 canonical JSON array of every `run_id` returned on all **prior** warning pages in traversal order (each page in sort-key order, pages concatenated). Omitted on the first warning page. |

`last_key` shapes:

| Collection | `last_key` fields |
|---|---|
| `provider.registrations`, `run.catalog` | `created_at` (RFC 3339 UTC), `stable_id` (registration ID or run ID) |
| `provider.registration_active_runs`, `provider.check_active_runs`, `provider.disable_warnings` | `run_id` |
| `run.history` | `sequence` (positive integer) |
| `run.evidence` | `created_at` (RFC 3339 UTC), `evidence_id` |

#### Integrity MAC (cursor v1)

Integration computes HMAC-SHA-256 over the canonical cursor payload using the machine-local 32-byte integrity key ([persistence.md](persistence.md) § Integration integrity key). Implementation **MUST** use the approved `sha2` SHA-256 primitive (standard HMAC construction); no additional digest dependency is introduced.

| Rule | Requirement |
|---|---|
| Canonical payload bytes | UTF-8 JSON of `payload` with keys sorted lexicographically, no insignificant whitespace, and `warning_traversal_digest` omitted when absent |
| Domain separation | HMAC input is `loop-engine.integrations.cursor-v1` (UTF-8) + `0x00` + canonical payload bytes |
| Verification order | Constant-time MAC compare **before** reading or acting on `last_key` or `warning_traversal_digest` |
| MAC failure | Reject with `cursor.invalid`; **MUST NOT** use tampered `last_key` |
| Key unavailable | Pre-dispatch persistence failure (`phase` `persistence`, exit `64`); no cursor minting or verification |
| Exposure | Integrity key and raw MAC tags **MUST NOT** appear in CLI stdout/stderr, operational trace, or export artifacts |

Records are not deleted in MVP, so cursors carry **no server-side session or lease state**; integrity is local keyed authentication of the opaque payload, not a network cursor ticket. There is no ordinary stale-cursor condition beyond filter, version, MAC, or traversal-chain mismatch.

#### Disable acknowledgement token wire form

Final-page `ack_token` values for `provider.disable` use the same URL-safe base64 JSON wire shape as cursors (`payload` + `mac`). Payload schema:

| Field | Type | Required | Description |
|---|---|:---:|---|
| `token_version` | integer | yes | Always `1` |
| `token_kind` | string | yes | Always `provider.disable_ack` |
| `registration_id` | string | yes | Target registration stable ID |
| `config_revision` | integer | yes | `config_revision` at final warning page issuance |
| `active_set_digest` | string | yes | Lowercase hex SHA-256 over UTF-8 canonical JSON array of sorted active `run_id` values at issuance |
| `warning_traversal_digest` | string | yes | Lowercase hex SHA-256 over UTF-8 canonical JSON array of every `run_id` returned across the completed warning traversal in presentation order; on a completed traversal the multiset **MUST** equal the active set and the digest **MUST** be derivable from the same sorted ID list used for `active_set_digest` |

MAC domain label: `loop-engine.integrations.disable-ack-v1` + `0x00` + canonical payload bytes (same canonical JSON rules as cursors). Verification **MUST** constant-time compare MAC before using bound fields. The complete wire string **MUST** be at most `opaque_integrity_wire_utf8_bytes` UTF-8 bytes (post-base64); inner payload field values remain subject to their component bounds (for example `registration_id` at most `identifier_utf8_bytes`).

#### Empty, valid, and invalid cursors

| Input | Behavior |
|---|---|
| `--cursor` omitted or empty string | Start first page of the requested collection with current filters |
| Malformed base64, invalid UTF-8, invalid JSON, missing required field, `last_key` shape mismatch, MAC/tag failure, or opaque wire string exceeding `opaque_integrity_wire_utf8_bytes` UTF-8 bytes (post-base64) | Reject with `cursor.invalid` |
| `cursor_version` not `1` | Reject with `cursor.invalid` (unsupported cursor version) |
| `collection` does not match the invoked operation/page | Reject with `cursor.invalid` |
| `filter_fingerprint` does not match normalized filters for this request | Reject with `cursor.invalid` |
| `provider.disable_warnings` cursor with `warning_traversal_digest` inconsistent with `last_key` position | Reject with `cursor.invalid` |
| Valid cursor for exhausted collection | Return empty `data.items` and omit `next_cursor` |

`provider.disable` warning pagination uses `--warning-cursor` with the same cursor v1 wire form and `provider.disable_warnings` collection. Only the **final** warning page (no `next_cursor`) emits an `ack_token`; intermediate or edited cursors **MUST NOT** authorize disable.

### Cursor v1 examples

Examples show `payload` before MAC and outer base64 encoding. `mac` values are illustrative; conformance tests **MUST** mint and verify real tags with the installation integrity key.

**Registration list** — payload JSON before MAC:

```json
{
  "cursor_version": 1,
  "collection": "provider.registrations",
  "filter_fingerprint": "3b78e8d8bafe537cc9ed0e4970b784ea7f4c2e6c0f4b86e3a40e5f0c9a1b2d3",
  "last_key": {
    "created_at": "2026-07-17T12:00:00.000Z",
    "stable_id": "019f6e88-b403-73a6-89f9-ebfe668b417e"
  }
}
```

Wire object (before URL-safe base64):

```json
{
  "payload": { "cursor_version": 1, "collection": "provider.registrations", "filter_fingerprint": "3b78e8d8bafe537cc9ed0e4970b784ea7f4c2e6c0f4b86e3a40e5f0c9a1b2d3", "last_key": { "created_at": "2026-07-17T12:00:00.000Z", "stable_id": "019f6e88-b403-73a6-89f9-ebfe668b417e" } },
  "mac": "<base64url-32-byte-tag>"
}
```

**Run history** — payload JSON before MAC:

```json
{
  "cursor_version": 1,
  "collection": "run.history",
  "filter_fingerprint": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
  "last_key": {
    "sequence": 42
  }
}
```

**Active-run compatibility page** — payload JSON before MAC:

```json
{
  "cursor_version": 1,
  "collection": "provider.check_active_runs",
  "filter_fingerprint": "2c26b46b68ffc68ff949b17e33663f0c4c97bdb784abc92e0b4538adfae8a6f0",
  "last_key": {
    "run_id": "019f6e88-b403-73a6-89f9-ebfe668b417f"
  }
}
```

### Operational trace budgets (cross-process)

Trace encoding budgets are **on-disk encoded bytes**, not raw in-memory sizes.

| Phase | Reservation rule |
|---|---|
| Trace initialization | After evicting eligible closed files, reserve `trace_init_reservation_bytes` (16 MiB) against `trace_directory_budget_bytes` |
| Before each provider call | Add `trace_provider_call_reservation_bytes` (10 MiB) unused capacity before launch |
| After each write | Atomically convert reserved capacity into actual bytes; bytes are never double-counted |
| Provider call close | Release unused reservation remainder |
| Rotation coordinator | Counts actual bytes plus only unused reservation remainder against `trace_directory_budget_bytes` (128 MiB) and `trace_retained_files_max` (100) |

**Dispatcher embedding** — request and outcome envelopes are embedded once as JSON values (never JSON strings). `trace_init_reservation_bytes` covers two `structured_cli_envelope_bytes` envelopes, persistence/decision events, JSONL framing, and worst-case escaping.

**Provider-call embedding** — provider request is embedded once as a JSON value; stdout retains a prefix up to `provider_result_stdout_bytes` stored once as base64 (4/3 expansion), plus original byte length and a truncated flag when the stream exceeds the bound (in-bound stdout is exact; oversized remainder is drained but not stored; [provider-protocol-v1.md](provider-protocol-v1.md) § Stdout); stderr retains a prefix up to `provider_stderr_trace_bytes` stored once as base64, plus original byte length and a truncated flag when the stream exceeds the bound (oversized remainder is drained but not stored; [provider-protocol-v1.md](provider-protocol-v1.md) § Stderr); parsed result is not duplicated (digest/size metadata only). Worst case per call: `provider_request_json_bytes` (4 MiB) + base64-expanded `provider_result_stdout_bytes` retained prefix (~1.33 MiB) + base64-expanded `provider_stderr_trace_bytes` retained prefix (~1.33 MiB) + 512 KiB facts/framing stays below `trace_provider_call_reservation_bytes` (10 MiB).

**Per-file cap** — one active trace file remains below `trace_file_max_bytes` (120 MiB): `trace_init_reservation_bytes` + `provider_calls_per_paged_invocation_max` × `trace_provider_call_reservation_bytes` = 16 MiB + 10 × 10 MiB = 116 MiB.

**Failure mapping** — insufficient base reservation at initialization is a trace-initialization failure (pre-dispatch). After page progress, insufficient next-call reservation ends the page with `next_cursor`. Before any row is returned, insufficient reservation yields `resource.exhausted` with unchanged cursor.

Schema tests **MUST** calculate encoded bytes including control-heavy JSON and binary streams.

## Dispatch boundary

Every CLI invocation initializes one current-user-only operational trace before application dispatch when possible (I46). Driver metadata requests (`--help`, `--version`, `--list-operations`) and pre-dispatch failures are also invocations and follow the same trace rule unless trace initialization itself fails.

| Phase | stdout (human) | stdout (structured) | stderr | Exit |
|---|---|---|---|---:|
| Pre-dispatch failure | empty | empty | rich failure (human text or one pre-dispatch JSON object) | `64` |
| Successful driver metadata (`--help`, `--version`, `--list-operations`) | driver output per [Driver metadata outputs](#driver-metadata-outputs) | one driver-metadata JSON object per [Driver metadata outputs](#driver-metadata-outputs) | empty | `0` |
| After application operation dispatch | human outcome lines | exactly one outcome envelope | empty in normal success/failure paths; late envelope-construction failure only | `0`, `2`, or `1` per outcome class |

Pre-dispatch failures include trace-initialization failure, unsupported platform, malformed global flags, unknown subcommand, argv/flag syntax errors, configuration load errors, and persistence open/migration/schema/integrity failures that occur before an application operation is dispatched ([persistence.md](persistence.md) § Database identity and open lifecycle). They never emit an outcome envelope or driver-metadata payload on stdout.

**Persistence boundary** — failures while opening `state.db`, applying startup migrations, rejecting newer unsupported schema versions, or validating integration metadata (for example missing or wrong-length `integrity_key`) are **pre-dispatch** (`phase`: `persistence`, exit `64`, empty stdout, no outcome envelope). When trace initialization succeeded, operational trace records `invocation.error` with `phase` `persistence` before `invocation.finish` ([operational-trace.md](operational-trace.md) § Driver, help, version, and parse behavior). Post-dispatch persistence operation errors (for example unexpected `SQLITE_CORRUPT` during a mutation, commit I/O failure without authoritative verification, or `SQLITE_BUSY` exhaustion after `sqlite_busy_timeout_ms`) are **not** pre-dispatch: they emit a full outcome envelope with `outcome` `error` and `reason.code` `persistence.failed`, exit `1`, and empty stderr in structured mode.

Successful driver metadata requests never emit an application outcome envelope. They exit `0` after writing driver output to stdout.

Post-dispatch domain validation denials (for example `run.not_found`, `gate.failed`) are **not** pre-dispatch; they emit a full outcome envelope and exit `2`.

## Driver metadata outputs

`--help`, `--version`, and `--list-operations` are driver functions ([operation-catalog.md](operation-catalog.md) § Explicit non-operations). They are not application operations, do not populate `operation`/`outcome`/`reason` outcome-envelope fields, and must not be described as application outcome envelopes.

All successful driver-metadata invocations initialize trace per I46, leave stderr empty, and exit `0`.

### `--help` / `-h`

**Human (default):** UTF-8 usage text on stdout. Includes global flags, alpha availability summary, and pointer to `--list-operations` for currently exposed argv templates.

**Structured (`--format json`):** one JSON object on stdout:

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Always `1` for this contract |
| `kind` | string | yes | Always `help` |
| `usage` | string | yes | Full usage text (same semantic content as human mode) |
| `request_id` | string | yes | Correlates invocation and trace |
| `trace` | string | yes | Absolute path to per-invocation JSONL trace file |

### `--version`

**Human (default):** one line on stdout: `loop-engine <VERSION>` where `<VERSION>` is the release version (MVP: `0.1.0` per [technology.md](technology.md)).

**Structured (`--format json`):** one JSON object on stdout:

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Always `1` for this contract |
| `kind` | string | yes | Always `version` |
| `name` | string | yes | Always `loop-engine` |
| `version` | string | yes | Release version string |
| `request_id` | string | yes | Correlates invocation and trace |
| `trace` | string | yes | Absolute path to per-invocation JSONL trace file |

### `--list-operations`

**Human (default):** one line per currently exposed application operation: `<OPERATION-ID><TAB><argv template>`, in stable frozen-catalog order. Alpha output contains exactly nine IDs. Final closure covers exactly all 21 IDs.

**Structured (`--format json`):** one JSON object on stdout:

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Always `1` for this contract |
| `kind` | string | yes | Always `operation_list` |
| `operations` | array | yes | Currently exposed checkpoint-closed objects in stable frozen-catalog order; nine in the alpha and exactly 21 at final closure |
| `operations[].id` | string | yes | Stable operation ID |
| `operations[].argv` | string | yes | argv template copied from this document |
| `request_id` | string | yes | Correlates invocation and trace |
| `trace` | string | yes | Absolute path to per-invocation JSONL trace file |

## Process exit codes

| Exit | Semantic class | When |
|---:|---|---|
| `0` | completed | Operation achieved its purpose |
| `2` | rejected | Request understood and evaluated but denied by domain rules |
| `1` | error | Operation could not be reliably evaluated or committed |
| `64` | pre-dispatch | Usage, configuration, platform, persistence open/migration/schema/integrity, or parse failure before dispatch |

Every reason code in [operation-catalog.md](operation-catalog.md) § Outcome and reason taxonomy maps to exactly one primary outcome class and therefore exactly one of exits `0`, `2`, or `1`. Compatibility and graph checks that successfully obtain findings **complete** (exit `0`) even when findings report invalidity or incompatibility.

## Stdout, stderr, and trace boundaries

1. **Structured post-dispatch stdout** — exactly one UTF-8 JSON outcome envelope for application operations; no prefix, suffix, or second JSON value (I18, I27).
2. **Driver metadata stdout** — successful `--help`, `--version`, and `--list-operations` write driver output only (see [Driver metadata outputs](#driver-metadata-outputs)); never an outcome envelope.
3. **Provider stdout/stderr** — never written to CLI stdout or stderr. Retained only through bounded operational trace capture (I18, I46).
4. **Operational trace** — one current-user-only JSONL file per invocation, initialized before dispatch when possible. Path is exposed in outcome `trace`, driver-metadata `trace`, and pre-dispatch failure `trace` when available. Trace never contaminates stdout.
5. **Human stdout** — human-readable rendering of the same outcome DTO or driver-metadata output; no provider stream passthrough.
6. **Stderr** — pre-dispatch rich failures and inability to construct a post-dispatch envelope only. Successful post-dispatch and driver-metadata paths leave stderr empty.

## Structured outcome envelope v1

### Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Always `1` for this contract |
| `operation` | string | yes | Stable operation ID actually executed (I27) |
| `request_id` | string | yes | Correlates invocation, envelope, and trace |
| `trace` | string | yes | Absolute path to per-invocation JSONL trace file |
| `outcome` | string | yes | One of `completed`, `rejected`, `error` (I34) |
| `reason` | object or null | yes | `null` when `outcome` is `completed`; otherwise stable `code` plus `message` |
| `data` | object | yes | Operation-specific payload; `{}` when empty |
| `diagnostics` | array | yes | Ordered ancillary diagnostics; `[]` when none |

### Reason object

| Field | Type | Required | Description |
|---|---|---|---|
| `code` | string | yes when `reason` is non-null | Stable reason code from operation-catalog taxonomy |
| `message` | string | yes when `reason` is non-null | Short human-readable summary; not a substitute for diagnostics |

Completed operations with no denial or failure use `"reason": null`. Omission of `reason` is not permitted in structured mode.

### Run summary (`data.run`)

Present for run-scoped operations when a run was resolved or created.

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable run ID |
| `label` | string or null | Display label if set |
| `lifecycle` | string | `active`, `final`, or `terminated` |
| `state` | string | Current state identifier |
| `state_changed` | boolean | Whether workflow state identifier changed in this operation |

### Requestable events (`data.requestable_events`)

String array of event names permitted from current stored graph when the run remains readable and lifecycle is `active`. “Requestable” does not imply gates will pass.

| Condition | `data.requestable_events` |
|---|---|
| Run resolved and `data.run.lifecycle` is `active` | required; non-empty or `[]` as graph permits |
| Run resolved and `data.run.lifecycle` is `final` or `terminated` | required; exactly `[]` |
| Run not resolved (for example `run.not_found`, `run.create` error before run exists) | omitted |
| Operation is not run-scoped (for example `provider.list`, `provider.add`) | omitted |

### Current-work projection (`run.show`)

Successful `run.show` adds these provider-free fields beside `data.run` and `data.requestable_events`:

| Field | Type | Description |
|---|---|---|
| `graph_revision` | string | Immutable stored graph revision digest |
| `inputs` | object | Immutable accepted non-secret run-input values reconstructed from persistence |
| `static_guidance` | object | `{ "kind": "text", "text": "..." }` or `{ "kind": "none_required" }` for current state |
| `live_guidance` | string | `supported` or `unsupported` capability declared by stored graph |
| `selected_evidence` | array of strings | Caller-owned current selection; `run.show` has no selection input and returns `[]` |
| `requestable_event_details` | array of objects | Stored event, target state, and `required_gates` array for each requestable event |

`requestable_event_details[*].event` corresponds one-for-one and in same order with `data.requestable_events`. Final, terminated, and active sink projections return both arrays empty. This read never resolves or invokes current provider configuration.

### Evidence-recorded status (`data.evidence_recorded`)

Present on applicable mutation attempts after run lookup.

| Field | Type | Description |
|---|---|---|
| `inline` | boolean | Caller inline evidence retained for this attempt |
| `selected_associations` | boolean | Caller-selected evidence associations retained |
| `provider` | boolean | Provider-returned evidence retained |

### Export result (`data.export`)

Present on successful `run.export` only. Normative artifact schemas, atomic publication, and completion-marker semantics are in [export-contract.md](export-contract.md) (D015). Structured CLI success **MUST NOT** emit artifact bytes on stdout; only metadata paths below. Artifact bytes are written only under `--output <DIR>`.

| Field | Type | Required | Description |
|---|---|---|---|
| `output` | string | yes | Export root directory (`--output <DIR>`) |
| `manifest_file` | string | yes | Completion-marker filename relative to `output`; always `manifest.json` |
| `state_file` | string | yes | Payload filename relative to `output`; always `state.json` |
| `journal_file` | string | yes | Payload filename relative to `output`; always `journal.jsonl` |

### Diagnostics entry

| Field | Type | Required | Description |
|---|---|---|---|
| `code` | string | yes | Stable diagnostic code |
| `message` | string | yes | Actionable detail |
| `context` | object | no | Bounded structured context |

### Common `data` extensions

Operation-specific fields live under `data` without breaking top-level shape:

- **Paged reads** — `items` array plus optional `next_cursor` string.
- **Provider catalog** — `registration`, `registrations`, `conformance`, `impact`, or `ack_token` objects as applicable.
- **Provider invocation flag** — `provider_executed` boolean when [ux-storyboards.md](ux-storyboards.md) shows `Provider executed:` (provider-invoking operations only).
- **Conformance summary** — `conformance` object for `provider.check` (default and `--active-runs` pages that include emitted-graph conformance): `protocol_major` (integer), `graph_status` (`valid` or `invalid`), `graph_revision` (string digest when available).
- **Compatibility findings** — `findings` array for `provider.check --active-runs` active-graph rows and `run.compatibility` per-capability results; each entry names a capability or event key, `status` (`compatible`, `incompatible`, or `unknown`), and optional nested diagnostic lines.
- **Graph/history/evidence** — `graph`, `entries`, `evidence`, `guidance`, `export` objects as applicable.
- **Export** — `export: { "output": "<DIR>", "manifest_file": "manifest.json", "state_file": "state.json", "journal_file": "journal.jsonl" }` on success (see [Export result (`data.export`)](#export-result-dataexport)).

Exact inner shapes for each operation are defined by implementation DTOs constrained by this envelope and operation-catalog semantics. For `run.export`, [export-contract.md](export-contract.md) is authoritative for `data.export` when this document and that contract differ.

## Pre-dispatch failure object (stderr, structured mode)

When `--format json` and failure occurs before dispatch, stderr contains one JSON object (never stdout):

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Always `1` |
| `phase` | string | yes | `trace_init`, `platform`, `config`, `persistence`, `parse`, or `usage` |
| `message` | string | yes | Human-readable summary |
| `request_id` | string or null | no | Present when trace allocation succeeded |
| `trace` | string or null | no | Present when trace file was created |
| `source_chain` | array of strings | no | Nested failure context, root cause last |

Pre-dispatch failures always exit `64` regardless of rendering mode.

## Human parity rules

Human rendering presents the same semantic fields as structured `data`, `outcome`, `reason`, and `diagnostics`. The table below is normative for fields shown in [ux-storyboards.md](ux-storyboards.md).

| Human line / section | Structured source | Operations (when shown) | Exit |
|---|---|---|---|
| `Outcome: completed` | `"outcome": "completed"`, `"reason": null` | all dispatched application operations | `0` |
| `Outcome: rejected` | `"outcome": "rejected"`, `reason.code` set | all dispatched application operations | `2` |
| `Outcome: error` | `"outcome": "error"`, `reason.code` set | all dispatched application operations | `1` |
| `Run:` / `Lifecycle:` / `State:` | `data.run.id`, `data.run.lifecycle`, `data.run.state` | run-scoped operations with resolved run | per outcome |
| `State changed: yes\|no` or `(unchanged)` suffix | `data.run.state_changed` | run-scoped mutations and reads that report change | per outcome |
| `Requestable events:` list | `data.requestable_events` (required `[]` when lifecycle is `final` or `terminated`) | run-scoped operations with resolved run | per outcome |
| `Graph revision:` / `Inputs:` | `data.graph_revision`, `data.inputs` | `run.show` | `0` |
| `Guidance:` / `Live guidance:` | `data.static_guidance`, `data.live_guidance` | `run.show` | `0` |
| `Selected evidence:` / `Requestable event details:` | `data.selected_evidence`, `data.requestable_event_details` | `run.show` | `0` |
| `Registration ID:` / `Handle:` | `data.registration.id`, `data.registration.handle` | provider-catalog mutations and reads | per outcome |
| `Protocol major:` | `data.conformance.protocol_major` | `provider.check` | per outcome |
| `Graph: valid\|invalid` | `data.conformance.graph_status` | `provider.check` (default conformance) | per outcome |
| `Graph revision:` | `data.conformance.graph_revision` | `provider.check` (default conformance) | per outcome |
| `Provider executed: yes\|no` | `data.provider_executed` | `run.guidance`, `run.compatibility`, and other provider-invoking operations that report invocation | per outcome |
| `Guidance:` prose | `data.guidance` | `run.guidance` | per outcome |
| `Findings:` / per-capability lines | `data.findings[*]` | `run.compatibility`, `provider.check --active-runs` active-graph rows | per outcome |
| `Active graphs:` section | `data.items[*]` or embedded active-graph rows under conformance/findings paging | `provider.check --active-runs` | per outcome |
| Tabular list rows | `data.items[*]` | paged list operations | per outcome |
| `Output:` / `Manifest:` / `State file:` / `Journal file:` | `data.export.output`, `data.export.manifest_file`, `data.export.state_file`, `data.export.journal_file` | `run.export` | per outcome |
| Reason prose | `reason.message` plus `diagnostics` | all dispatched application operations | per outcome |

Human mode must not show additional policy, hide rejections, or report a different outcome class than structured mode for the same invocation.

## Contract examples

All examples below are valid JSON conforming to this contract. Paths and IDs are illustrative.

### Completed — `run.show`

```json
{
  "schema_version": 1,
  "operation": "run.show",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl",
  "outcome": "completed",
  "reason": null,
  "data": {
    "run": {
      "id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
      "label": "checkout-redesign",
      "lifecycle": "active",
      "state": "explore",
      "state_changed": false
    },
    "requestable_events": ["intent-ready"]
  },
  "diagnostics": []
}
```

Exit `0`.

### Completed — terminal `run.show`

When lifecycle is `final` or `terminated`, `data.requestable_events` is required and exactly `[]`.

```json
{
  "schema_version": 1,
  "operation": "run.show",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V6B",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V6B.jsonl",
  "outcome": "completed",
  "reason": null,
  "data": {
    "run": {
      "id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
      "label": "checkout-redesign",
      "lifecycle": "final",
      "state": "shipped",
      "state_changed": false
    },
    "requestable_events": []
  },
  "diagnostics": []
}
```

Exit `0`.

### Rejected — `run.request` (`gate.failed`)

```json
{
  "schema_version": 1,
  "operation": "run.request",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V3Y",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V3Y.jsonl",
  "outcome": "rejected",
  "reason": {
    "code": "gate.failed",
    "message": "One or more required gates failed"
  },
  "data": {
    "run": {
      "id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
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

Exit `2`. Reason code `gate.failed` is rejected class per operation-catalog.

### Error — `run.create` (`provider.timeout`)

```json
{
  "schema_version": 1,
  "operation": "run.create",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V4Z",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V4Z.jsonl",
  "outcome": "error",
  "reason": {
    "code": "provider.timeout",
    "message": "Provider process exceeded configured timeout"
  },
  "data": {},
  "diagnostics": [
    {
      "code": "provider.invocation",
      "message": "Role describe timed out after 60 seconds",
      "context": {
        "role": "describe",
        "timeout_seconds": 60
      }
    }
  ]
}
```

Exit `1`. Rejected/error creation emits no run and no run journal (operation-catalog verification rules).

### Pre-dispatch — parse failure (stderr, structured mode)

Not written to stdout. Example stderr payload:

```json
{
  "schema_version": 1,
  "phase": "parse",
  "message": "unknown flag --limt",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V5A",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V5A.jsonl",
  "source_chain": [
    "run list: unrecognized flag --limt"
  ]
}
```

Exit `64`. No outcome envelope on stdout.

### Pre-dispatch — persistence open failure (stderr, structured mode)

Not written to stdout. Example stderr payload when startup migration fails after trace initialization:

```json
{
  "schema_version": 1,
  "phase": "persistence",
  "message": "Database migration failed",
  "request_id": "01J9X3K2M4N5P6Q7R8S9T0V7C",
  "trace": "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V7C.jsonl",
  "source_chain": [
    "migration 0002_add_indexes: UNIQUE constraint failed"
  ]
}
```

Exit `64`. No outcome envelope on stdout. Operational trace records `invocation.error` with matching `phase` and `message` when the trace file was created.

Post-dispatch `persistence.failed` (for example snapshot read failure during `run.export`) is **not** this shape: it uses the [structured outcome envelope](#structured-outcome-envelope-v1) on stdout with exit `1`.

## Published structured outcome schema

The machine-readable JSON Schema for the [structured outcome envelope](#structured-outcome-envelope-v1) is indexed at [schemas/index.json](../schemas/index.json) as `schemas/cli/v1/outcome.schema.json` (`schema_version` `1`, title `StructuredCliOutcomeEnvelopeV1`).

| Property | Value |
|---|---|
| Exposure | **Planned** — WP1 publishes the schema and private renderer only; no production application operation route is exposed yet |
| Generation | None — the published file is maintained alongside the private CLI renderer |
| Validation | `cargo test -p loop-engine-cli published_schema_enums_match_core_catalog_and_taxonomy`; `cargo test -p loop-engine-cli contract_examples_render_with_eight_required_top_level_fields` |

Structured application dispatch emits **exactly one** UTF-8 JSON outcome envelope on stdout after dispatch ([Stdout, stderr, and trace boundaries](#stdout-stderr-and-trace-boundaries)). Operational trace remains a separate JSONL file initialized before dispatch when possible (I46, D010); provider streams never appear on CLI stdout or stderr.

## Verification rules (T006)

- Every D004 operation argv row appears exactly once in this document and matches `operation-catalog.md`.
- No extra application operation appears in argv tables.
- Each contract example is valid JSON with consistent `outcome`, `reason.code`, and exit mapping.
- Dispatch table distinguishes pre-dispatch failure (empty stdout, exit `64`), successful driver metadata (stdout per [Driver metadata outputs](#driver-metadata-outputs), exit `0`), and application dispatch (one outcome envelope, exit `0`/`2`/`1`).
- Terminal resolved runs require `data.requestable_events` exactly `[]`; omission only when run is unresolved or operation is non-run-scoped.
- Structured post-dispatch mode emits exactly one stdout outcome envelope for application operations; driver metadata emits one driver-metadata object; provider streams never appear on stdout/stderr.
- Human and structured modes preserve semantic parity per table above.
- Reason codes are drawn only from [operation-catalog.md](operation-catalog.md) § Outcome and reason taxonomy.

## Verification rules (T008)

- Every bound in [Resource bounds (D008)](#resource-bounds-d008) appears exactly once in the named table; no other foundation document restates numeric limits.
- Aggregate envelope arithmetic (gate request, describe result, journal entry, structured CLI envelope) matches component bound names.
- Trace reservation arithmetic: per-call worst case stays below `trace_provider_call_reservation_bytes`; per-invocation reservation sum stays below `trace_file_max_bytes`; directory budget uses actual bytes plus unused reservation remainder only.
- Opaque integrity wire bound arithmetic: `W_post + H_wire = opaque_integrity_wire_utf8_bytes`; sample disable `ack_token` and cursor v1 examples satisfy `len_utf8(wire) ≤ opaque_integrity_wire_utf8_bytes` (post-base64).
- Cursor v1 examples decode from URL-safe base64, parse as JSON, verify MAC with installation integrity key, and satisfy the cursor payload schema.
- Disable `ack_token` examples verify MAC under the `loop-engine.integrations.disable-ack-v1` domain and bind registration ID, `config_revision`, `active_set_digest`, and completed `warning_traversal_digest`.
- Pagination covers `provider.registrations`, `provider.registration_active_runs`, `provider.check_active_runs`, `provider.disable_warnings`, `run.catalog`, `run.history`, and `run.evidence` with the documented sort keys and filter fingerprints.
- Selected evidence overflow rejects before provider invocation; selected evidence is never truncated.
- E2E proof owners: T147 (registration list and zero-row `--active-runs-for`), T150/T152 (run list, history, `--active-runs` compatibility pages), T157/T175 (nonempty impact/disable warning pages), T160 (evidence list), T101/T152/T182 (trace directory budget and per-invocation reservation limits).
- Provider timeout termination grace period remains owned solely by [provider-protocol-v1.md](provider-protocol-v1.md) (5-second `SIGTERM` grace; not restated here).
