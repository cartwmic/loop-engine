# Loop Engine Configuration

**Status:** Machine-local layout frozen by T007 (2026-07-17); project-default discovery frozen by T016 (2026-07-17). Decisions [D007](change/initial-implementation/decisions.md#d007-configuration-and-filesystem-layout) and [D016](change/initial-implementation/decisions.md#d016-project-default-discovery).

This document is the canonical contract for machine-local filesystem layout, `LOOP_ENGINE_HOME`, global/project TOML shape, precedence, unknown-key policy, path normalization, registration executable/working-directory rules, malformed-config behavior, caller-working-directory independence, and project-default ancestor discovery. Global CLI rendering flags and numeric resource bounds are in [cli-contract.md](cli-contract.md) (D006, D008).

Related documents:

- [Decision D007](change/initial-implementation/decisions.md#d007-configuration-and-filesystem-layout)
- [Decision D008](change/initial-implementation/decisions.md#d008-resource-bounds-and-timeout-defaults)
- [Decision D016](change/initial-implementation/decisions.md#d016-project-default-discovery)
- [CLI contract](cli-contract.md)
- [Application operation catalog](operation-catalog.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [System invariants](invariants.md) — I16, I40–I41

## Scope

Machine-local configuration covers:

- one global defaults file under the user config root;
- one optional project defaults file discovered per D016;
- one SQLite state database under the machine state root;
- one operational-trace directory under the machine state root;
- provider registrations stored in SQLite (not in TOML).

Project and global TOML files supply **defaults and registration references only**. They **MUST NOT** define provider executables, argument vectors, working directories, alternate state stores, trace roots, or database paths. Provider registration remains explicit machine-local catalog mutation (`provider.add`, `provider.update`, `provider.restore`) per I40.

`provider.add`, `provider.update`, `provider.rename`, `provider.disable`, and `provider.restore` are exposed registration mutations. Use `loop-engine --list-operations` as runtime authority.

## Machine-local roots

All supported platforms expose exactly two machine-local roots when `LOOP_ENGINE_HOME` is unset or empty:

| Root | Purpose |
|---|---|
| **Config root** | Global `config.toml` only |
| **State root** | `state.db`, `traces/`, and all SQLite-backed catalog/run persistence |

There is exactly **one** SQLite database file and **one** trace directory per machine-local installation. Multiple state stores, shard paths, or project-selected database identities are forbidden.

### `LOOP_ENGINE_HOME`

#### Effective value

| Condition | Behavior |
|---|---|
| Environment variable unset | Use [OS default layout](#os-default-layout) |
| Environment variable set to empty string (`""`) | Treat as unset; use [OS default layout](#os-default-layout) |
| Environment variable set to a non-empty value | Resolve **machine home root** below and use [override layout](#loop_engine_home-override-layout) |

Empty `LOOP_ENGINE_HOME` is **not** a configuration error; it selects OS defaults like an unset variable.

#### Resolution

When `LOOP_ENGINE_HOME` is set to a non-empty value:

1. Expand a leading `~` to `$HOME`.
2. Lexically normalize to an absolute path without querying the filesystem (see [Path normalization](#path-normalization)). Relative values are invalid.
3. Determine **machine home root**:
   - If the lexically normalized path exists, resolve symlinks on that path component only (not parent traversal) to obtain the machine home root.
   - If the lexically normalized path does not exist, the machine home root is the lexically normalized absolute path. Symlink canonicalization is not claimed for nonexistent roots; [first-use creation](#directory-creation-and-permissions) uses this lexical identity.
4. Derive **all** machine-local paths from the machine home root using the [override layout](#loop_engine_home-override-layout) below.

Machine home root identity depends only on `LOOP_ENGINE_HOME` and `$HOME` (for `~` expansion). Caller CWD **MUST NOT** influence it (I16, I41).

`LOOP_ENGINE_HOME` overrides OS user config/state directories entirely. It is intended for tests, portable installs, and isolation harnesses ([testing.md](testing.md)). It is not a CLI flag ([cli-contract.md](cli-contract.md)).

### `LOOP_ENGINE_HOME` override layout

| Path | Location |
|---|---|
| Config root | `{machine_home_root}/` |
| Global config file | `{machine_home_root}/config.toml` |
| State root | `{machine_home_root}/` |
| SQLite database | `{machine_home_root}/state.db` |
| Trace directory | `{machine_home_root}/traces/` |
| Per-invocation trace file | `{machine_home_root}/traces/{request_id}.jsonl` |

`request_id` is the invocation UUID v7 string from the CLI contract.

### OS default layout

#### Linux (glibc)

Respect XDG base-directory variables when `LOOP_ENGINE_HOME` is unset or empty:

| Path | Location |
|---|---|
| Config root | `${XDG_CONFIG_HOME:-$HOME/.config}/loop-engine/` |
| Global config file | `${XDG_CONFIG_HOME:-$HOME/.config}/loop-engine/config.toml` |
| State root | `${XDG_STATE_HOME:-$HOME/.local/state}/loop-engine/` |
| SQLite database | `${XDG_STATE_HOME:-$HOME/.local/state}/loop-engine/state.db` |
| Trace directory | `${XDG_STATE_HOME:-$HOME/.local/state}/loop-engine/traces/` |
| Per-invocation trace file | `…/traces/{request_id}.jsonl` |

#### macOS

When `LOOP_ENGINE_HOME` is unset or empty:

| Path | Location |
|---|---|
| Config root | `$HOME/Library/Application Support/loop-engine/` |
| Global config file | `$HOME/Library/Application Support/loop-engine/config.toml` |
| State root | `$HOME/Library/Application Support/loop-engine/` |
| SQLite database | `$HOME/Library/Application Support/loop-engine/state.db` |
| Trace directory | `$HOME/Library/Application Support/loop-engine/traces/` |
| Per-invocation trace file | `…/traces/{request_id}.jsonl` |

### Directory creation and permissions

On first use the engine **MAY** create missing machine home root, config-root, state-root, and `traces/` parent directories. This includes a nonexistent `LOOP_ENGINE_HOME` target: creation uses the lexical machine home root identity from [resolution](#resolution) step 3 without requiring prior symlink canonicalization.

Sensitive directories **MUST** be created with mode `0700`; sensitive files (`state.db`, trace files, `config.toml` when written) **MUST** use mode `0600`. Permission failure before dispatch stops the invocation ([technology.md](technology.md) § Supported platforms).

### Caller-working-directory independence

Machine-local roots, `state.db`, trace directory, and provider-registration catalog resolution **MUST NOT** depend on the caller's current working directory (I16, I41). Two invocations from different directories with the same `LOOP_ENGINE_HOME` (or the same OS user account and unset or empty `LOOP_ENGINE_HOME`) **MUST** observe the same catalog and persistence store.

Caller CWD affects only:

- [project-default discovery](#project-default-discovery) of `.loop-engine.toml`;
- lexical resolution of **relative** paths supplied on provider-registration mutation argv (`--exec`, `--working-directory`) at the moment of that mutation.

Caller CWD **MUST NOT** alter provider subprocess working directory for stored registrations (I40).

## Project-default discovery

Normative owner: [D016](change/initial-implementation/decisions.md#d016-project-default-discovery). Frozen by T016 (2026-07-17).

### What this is and is not

| Kind | Behavior |
|---|---|
| **CLI-default discovery (this section)** | Locate at most one optional `.loop-engine.toml` to merge `defaults.*` into CLI configuration |
| **Provider registration** | Explicit catalog-mutation argv stored in SQLite; all provider lifecycle operations are exposed |
| **Executable discovery** | Forbidden — no PATH search, package scan, or inference from repository layout |
| **Workflow discovery** | Forbidden — no scanning for workflow sources, manifests, or provider packages in project trees |

Project-default discovery **MUST NOT** register providers, define executables, override argv/working-directory bindings, select alternate persistence roots, or rebind an existing run's stored registration ID (I40, I41, [architecture.md](architecture.md)).

### Filename and content

| Property | Value |
|---|---|
| Filename | `.loop-engine.toml` (exact spelling; case-sensitive where the host filesystem is case-sensitive) |
| Allowed content | Same [TOML schema](#toml-schema-version-1) as global `config.toml` — `schema_version` and `[defaults]` with `format`, `provider`, `timeout_seconds` only |
| Forbidden content | Provider executable definitions, argv, working-directory overrides, database/trace/path overrides, or any second state store |

`defaults.provider` is a **reference** to an already-registered enabled handle or registration ID in the machine-local catalog. It is **not** an executable locator and **MUST NOT** create, mutate, or restore catalog rows.

### Ancestor-search algorithm

Configuration load obtains **caller CWD** once per invocation as an absolute path from the operating system (`getcwd` or platform equivalent) **before** application dispatch.

Given caller CWD absolute path `dir`:

1. **Check:** if `{dir}/.loop-engine.toml` exists as a regular file, or as a symlink whose final target is a regular file readable by the current user, **select** that path as the project configuration file and **stop**.
2. **Stop at root:** if `dir` is the filesystem root (`/` on supported Unix platforms), **stop** with an empty project layer.
3. **Ascend:** set `dir` to the lexical parent of `dir` (POSIX `dirname`; `/foo/bar` → `/foo`, `/foo` → `/`).
4. **Repeat** from step 1.

Properties:

- At most **one** project file participates in precedence — the **nearest** ancestor match to caller CWD.
- Search is independent of `LOOP_ENGINE_HOME`, machine-local roots, repository boundaries, and VCS metadata (for example `.git` is not a stop boundary).
- A project file above caller CWD is visible only when the invocation starts from a descendant directory; it does not retroactively affect runs created elsewhere.

### Symlink, permission, and error behavior

| Condition | Behavior |
|---|---|
| Caller CWD unavailable (`getcwd` failure: deleted directory, permission denied) | Pre-dispatch failure: exit `64`, `phase = "config"`, actionable stderr |
| Intermediate ancestor not traversable (search permission denied) | Same pre-dispatch failure |
| `.loop-engine.toml` present but unreadable | Same pre-dispatch failure |
| `.loop-engine.toml` is a symlink to a readable regular file | **Select** the symlink path; load follows the link |
| `.loop-engine.toml` is a broken symlink or symlink loop | Treat as **absent**; continue ancestor search |
| No `.loop-engine.toml` on the chain to root | Project layer **empty** (not an error) |
| Selected file fails TOML/schema/unknown/forbidden-key validation | Pre-dispatch failure per [Malformed configuration behavior](#malformed-configuration-behavior) |

Ancestor traversal uses the absolute caller-CWD spelling and lexical parents. It **MUST NOT** require canonicalization of each directory component during the walk.

### Precedence and non-rebinding

Effective defaults merge per [Precedence](#precedence). The discovered project file is the **project** layer in:

```text
CLI flags  >  project `.loop-engine.toml`  >  global `config.toml`  >  built-in defaults
```

Project defaults **MUST NOT**:

- change `LOOP_ENGINE_HOME` resolution, config/state roots, `state.db`, or trace directory;
- supply or alter `executable`, `argv`, or `working_directory` for any registration;
- change the registration ID stored on an existing run;
- cause provider-catalog mutations without explicit `provider.*` operation argv.

Provider-catalog mutations always take executable configuration from explicit argv, never from discovered TOML ([Precedence](#precedence) § Existing-run and registration identity).

## TOML schema (version 1)

Parser: workspace `toml` crate `1.1.3` ([technology.md](technology.md) approved dependency contract).

Both `config.toml` and `.loop-engine.toml` share one schema.

### Top level

| Key | Type | Required | Semantics |
|---|---|:---:|---|
| `schema_version` | integer | yes when file is non-empty | Must be exactly `1` |
| `defaults` | table | no | CLI defaults; see below |

No other top-level keys are permitted.

### `[defaults]` table

| Key | Type | Required | Semantics |
|---|---|:---:|---|
| `format` | string | no | `human` or `json`; mirrors `--format` |
| `provider` | string | no | Default provider **reference** (enabled handle or registration ID) for a positional `<TARGET>` only when the operation's frozen argv in [operation-catalog.md](operation-catalog.md) permits omitting that positional; **MUST NOT** invent alternate argv (for example a `--provider` flag) |
| `timeout_seconds` | integer | no | Default provider timeout when an operation does not override it; positive; bound name `provider_timeout_seconds_default` in [cli-contract.md](cli-contract.md#resource-bounds-d008) |

No other `defaults` keys are permitted.

### Missing, empty, and built-in layers

| Condition | Behavior |
|---|---|
| Global `config.toml` missing | Treat global layer as empty |
| Project `.loop-engine.toml` missing | Treat project layer as empty |
| File exists but contains only `schema_version = 1` and/or an empty `[defaults]` | Layer contributes no overrides |
| All layers empty | Use [built-in defaults](#precedence) |

### File size

Each TOML configuration file is bounded to **`toml_config_file_bytes`** (1 MiB; [cli-contract.md](cli-contract.md#resource-bounds-d008)). Exceeding the bound is a pre-dispatch configuration error.

## Precedence

Effective defaults merge in strict order; higher layers override lower layers only for keys they explicitly set:

```text
CLI flags  >  project `.loop-engine.toml`  >  global `config.toml`  >  built-in defaults
```

Built-in defaults:

| Key | Value |
|---|---|
| `format` | `human` |
| `provider` | unset |
| `timeout_seconds` | `provider_timeout_seconds_default` (60; [cli-contract.md](cli-contract.md#resource-bounds-d008)) |

`LOOP_ENGINE_HOME` is not part of this merge; it selects filesystem roots only.

### Existing-run and registration identity

- Active runs store stable provider **registration ID**; project/global defaults **MUST NOT** redefine or rebind the registration selected by an existing run (I40, I41).
- `defaults.provider` may supply a positional `<TARGET>` only when the operation's frozen argv permits omitting that positional and argv does not include it; it **MUST NOT** invent alternate argv (for example a `--provider` flag) and **MUST NOT** retroactively change stored registration IDs on existing runs.
- `run.create` frozen argv is `run create <TARGET> [--label <LABEL>] [--inputs <PATH>]` ([operation-catalog.md](operation-catalog.md)); `<TARGET>` is mandatory on argv as a positional token. `defaults.provider` **MUST NOT** substitute for an omitted `<TARGET>` or add a `--provider` flag.
- Provider-catalog mutations always take executable configuration from explicit argv, never from TOML. This applies to `provider.add`, `provider.update`, and `provider.restore`.

## Unknown and forbidden keys

### Unknown keys

Any unknown key at any depth **MUST** be rejected at configuration load time with an actionable diagnostic naming the key path. Unknown keys **MUST NOT** be ignored.

### Forbidden provider-definition keys

The following are forbidden in **both** global and project TOML (non-exhaustive illustrative list; any key that embeds provider executable configuration or alternate persistence roots is forbidden):

- `executable`, `exec`, `argv`, `arg`, `working_directory`, `working-directory`
- `database`, `state`, `trace`, `traces`, `paths`, `data_dir`, `catalog`
- `[[providers]]`, `[providers]`, `[providers.*]`, or any table that names a provider executable

Presence of forbidden keys is a pre-dispatch configuration error.

## Path normalization

Path normalization applies to filesystem paths in configuration roots, `LOOP_ENGINE_HOME`, and provider-registration mutation argv.

### Lexical normalization (always)

Given an input path string:

1. Reject empty paths and paths exceeding **`filesystem_path_utf8_bytes`** ([cli-contract.md](cli-contract.md#resource-bounds-d008)).
2. Expand a leading `~` to `$HOME`.
3. If the path is relative, resolve it against the **caller CWD at the moment the path is supplied** (registration mutation argv only). Relative `LOOP_ENGINE_HOME` values are invalid and **MUST NOT** be resolved against caller CWD.
4. Apply POSIX lexical simplification of `.` and `..` components.
5. Produce an absolute path string with `/` separators (no trailing slash unless the path is `/`).

Lexical normalization **MUST NOT** query the filesystem and **MUST NOT** follow symlinks.

### Symlink policy

| Context | Policy |
|---|---|
| `LOOP_ENGINE_HOME` value | When the lexically normalized home path exists, resolve symlinks on that path component only once when computing machine-local roots; when it does not exist, use the lexical absolute path as machine home root identity (no symlink canonicalization claimed) |
| Registration `executable` and `working_directory` | Store lexical-normalized absolute paths; do **not** require symlink resolution at registration time |
| Provider invocation | Open the stored path; OS symlink semantics apply at open time |

### Configured-spelling identity

Provider-registration records store the **lexical-normalized absolute** path produced from the caller's mutation argv. The engine **MUST NOT** rewrite stored paths to symlink-canonical targets during registration or update. Registration **ID** identity is independent of later handle renames, project defaults, and caller CWD (I41).

Equivalent paths supplied through different symlink spellings are **not** deduplicated automatically; stored text reflects each mutation's normalized absolute spelling.

### Absolute registration paths

At `provider.add`, `provider.update`, and `provider.restore` completion, persisted `executable` and `working_directory` **MUST** be absolute lexical-normalized paths. Relative paths **MUST NOT** remain stored. Provider protocol handoff uses these stored absolutes; caller CWD is not consulted again at invocation (I40, [provider-protocol-v1.md](provider-protocol-v1.md)).

## Nonexistent executable and working directory

| Phase | Executable path | Working directory |
|---|---|---|
| Registration mutation (`provider.add`, `provider.update`, `provider.restore`) | **MAY** be nonexistent; mutation succeeds when paths are syntactically valid and within bounds | **MAY** be nonexistent or not yet a directory; mutation succeeds under the same rules |
| Provider-dependent operation | **MUST** exist and be executable at invocation time or the operation errors | **MUST** exist and be a directory at invocation time or the operation errors |
| Provider-free operation | Ignored | Ignored |

Missing or non-executable provider at invocation time is an operation **error**, not a configuration parse failure. Safe inspection, annotation, listing, and termination **MUST** remain available without a live provider where the operation catalog already permits provider-free behavior (I40).

## Malformed configuration behavior

All conditions below are **pre-dispatch** failures:

| Condition | Exit code | `phase` | Stderr |
|---|---:|---|---|
| TOML syntax error | `64` | `config` | Rich diagnostic with parse location |
| Unsupported `schema_version` | `64` | `config` | Rich diagnostic naming supported version |
| Unknown key | `64` | `config` | Rich diagnostic naming key path |
| Forbidden provider/persistence key | `64` | `config` | Rich diagnostic naming forbidden key |
| File size overflow | `64` | `config` | Rich diagnostic naming bound |
| Invalid `defaults.format` value | `64` | `config` | Rich diagnostic naming allowed values |
| Invalid `LOOP_ENGINE_HOME` (relative, bound overflow, or other syntactic violation; empty string is **not** invalid — it is treated as unset) | `64` | `config` | Rich diagnostic |
| Caller CWD unavailable (`getcwd` failure) | `64` | `config` | Rich diagnostic |
| Ancestor traversal permission denied during project-default discovery | `64` | `config` | Rich diagnostic |
| Discovered `.loop-engine.toml` unreadable | `64` | `config` | Rich diagnostic |

Structured-mode pre-dispatch failures follow [cli-contract.md](cli-contract.md) § Pre-dispatch failures. Configuration load **MUST** complete before application operation dispatch.

## Contract examples

Paths below are illustrative. Every TOML example is valid and parseable.

### Normal user home — Linux

Environment: `LOOP_ENGINE_HOME` unset, `HOME=/home/alice`, default XDG paths.

```text
Global config:  /home/alice/.config/loop-engine/config.toml
SQLite database: /home/alice/.local/state/loop-engine/state.db
Trace directory: /home/alice/.local/state/loop-engine/traces/
```

Example global file:

```toml
schema_version = 1

[defaults]
format = "json"
provider = "reference-workflow"
timeout_seconds = 120
```

### Normal user home — macOS

Environment: `LOOP_ENGINE_HOME` unset, `HOME=/Users/alice`.

```text
Global config:  /Users/alice/Library/Application Support/loop-engine/config.toml
SQLite database: /Users/alice/Library/Application Support/loop-engine/state.db
Trace directory: /Users/alice/Library/Application Support/loop-engine/traces/
```

Structured outcome examples in [cli-contract.md](cli-contract.md) use illustrative trace paths and IDs; normative macOS machine-local layout is defined in this document.

### Isolated test home

Environment:

```bash
export LOOP_ENGINE_HOME=/tmp/loop-engine-test-001
```

Resolved layout:

```text
Machine home root: /tmp/loop-engine-test-001
Global config:     /tmp/loop-engine-test-001/config.toml
SQLite database:   /tmp/loop-engine-test-001/state.db
Trace directory:   /tmp/loop-engine-test-001/traces/
```

Example minimal global file:

```toml
schema_version = 1
```

### `LOOP_ENGINE_HOME` symlink and first-use creation

Existing symlink root:

```bash
ln -s /tmp/loop-engine-real /tmp/loop-engine-link
export LOOP_ENGINE_HOME=/tmp/loop-engine-link
```

Resolved machine home root: `/tmp/loop-engine-real` (symlink resolved because the path exists).

Nonexistent root (first use):

```bash
export LOOP_ENGINE_HOME=/tmp/loop-engine-test-002
```

Resolved machine home root: `/tmp/loop-engine-test-002` (lexical absolute; symlink canonicalization not claimed). First catalog write **MAY** create `/tmp/loop-engine-test-002/`, `state.db`, and `traces/` with current-user-only permissions.

Empty value (treated as unset):

```bash
export LOOP_ENGINE_HOME=
```

Uses the [OS default layout](#os-default-layout); not a configuration error.

### Symlink and nonexistent registration paths

Registration mutation from caller CWD `/workspace/app`:

```bash
loop-engine provider add demo \
  --exec ./vendor/bin/workflow-provider \
  --working-directory ./fixtures/demo
```

If `./vendor/bin/workflow-provider` is a symlink to `/opt/providers/v1/workflow-provider`, the stored executable **MUST** be `/workspace/app/vendor/bin/workflow-provider` (lexical absolute, symlink segment preserved). If `/opt/providers/v1/workflow-provider` does not exist at registration time, the mutation still succeeds; `provider.check demo` errors at invocation until the executable exists.

If `--working-directory` names a not-yet-created directory, registration still succeeds; provider invocation errors until the directory exists.

### Unknown and forbidden keys

Invalid — unknown key:

```toml
schema_version = 1
unknown_key = true
```

Invalid — forbidden provider definition in project file:

```toml
schema_version = 1

[[providers]]
handle = "inline"
exec = "/bin/sh"
```

Both are pre-dispatch `config` failures with exit `64`.

### Caller-CWD independence

Given `LOOP_ENGINE_HOME=/tmp/shared-home` and a populated `state.db`:

```bash
cd /tmp/project-a && loop-engine run list
cd /tmp/project-b && loop-engine run list
```

Both commands **MUST** return the same run catalog. A `.loop-engine.toml` present only under `/tmp/project-a` **MAY** change defaults for invocations started there, but **MUST NOT** change database location, trace root, or stored registration IDs on existing runs.

### Nearest-ancestor selection

Directory layout:

```text
/tmp/monorepo/.loop-engine.toml                 # defaults.provider = "team-default"
/tmp/monorepo/services/api/.loop-engine.toml    # defaults.provider = "api-local"
```

Invocation from `/tmp/monorepo/services/api/src`:

- Selected file: `/tmp/monorepo/services/api/.loop-engine.toml` (nearest ancestor).
- `defaults.provider = "api-local"` may supply a positional `<TARGET>` only when the relevant operation's frozen argv permits omitting it; it does not apply to `run.create`, whose frozen argv requires positional `<TARGET>`.

Invocation from `/tmp/monorepo/tools` (no local file):

- Selected file: `/tmp/monorepo/.loop-engine.toml`.

### No registration or executable rebinding

Setup:

- `LOOP_ENGINE_HOME=/tmp/shared-home` with one populated catalog.
- Registration `demo` has ID `01HXYABCDEF` with stored executable `/opt/providers/v1/workflow` (from prior `provider.add`).
- `/tmp/project-a/.loop-engine.toml`:

```toml
schema_version = 1

[defaults]
provider = "other-handle"
```

`other-handle` may name a different enabled registration in the same catalog. Existing run `R1` created with `run create demo` (ID `01HXYABCDEF`) **MUST** continue resolving `01HXYABCDEF` for provider-dependent operations regardless of caller CWD or project defaults.

From `/tmp/project-a`:

```bash
loop-engine run show R1
```

- Run `R1` registration ID unchanged.
- Provider subprocess uses stored executable `/opt/providers/v1/workflow` for `R1`, not any executable associated with `other-handle`, unless the operation explicitly targets a different run or registration.

From `/tmp/project-b` (no project file):

```bash
loop-engine run list
```

- Same catalog and `state.db` as `/tmp/project-a` invocations with the same `LOOP_ENGINE_HOME`.
- Project defaults under `/tmp/project-a` **MUST NOT** affect this invocation.

New `run.create` invocations from `/tmp/project-a` use frozen argv `run create <TARGET> [--label <LABEL>] [--inputs <PATH>]`; `<TARGET>` is a mandatory positional token, not a `--provider` flag:

```bash
loop-engine run create other-handle
```

That selects the `other-handle` catalog reference for the new run only; it does **not** modify `R1` or inline executable configuration.

### Broken symlink skipped during discovery

`/tmp/app/.loop-engine.toml` is a broken symlink; `/tmp/.loop-engine.toml` exists and is valid.

From `/tmp/app/sub`:

- Broken `/tmp/app/.loop-engine.toml` is skipped.
- Selected file: `/tmp/.loop-engine.toml`.

## Stop conditions

Implementation **MUST** stop and escalate if a design requires:

- more than one SQLite state store or competing persistence authority in project/global configuration;
- provider executable definitions or argv/working-directory bindings in project/global TOML;
- provider discovery from project files beyond defaults/reference keys defined here.
