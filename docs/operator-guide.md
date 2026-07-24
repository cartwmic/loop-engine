# Operator Guide

This guide covers one local, offline, single-user `loop-engine` installation. Engine has no daemon, server, remote control plane, multi-user authorization, or automatic retry service.

## Install and isolate state

Build with repository toolchain and lockfile:

```bash
env -u RUSTUP_TOOLCHAIN cargo build --locked -p loop-engine-cli
```

Set `LOOP_ENGINE_HOME` to installation-owned directory before first invocation. Engine creates private state, trace, and configuration paths beneath it. Do not share one home between users or copy a live home between machines. See [configuration.md](configuration.md) for path precedence and permissions.

## Discover callable operations

```bash
loop-engine --list-operations
loop-engine --format json --list-operations
```

This runtime catalog is authoritative. All 21 operation IDs are documented in [operation-catalog.md](operation-catalog.md); argv, outcome, exit, and output rules are in [cli-contract.md](cli-contract.md).

## Routine workflow

1. Register an explicit provider executable with `provider add`.
2. Run `provider check` before creating runs.
3. Create a run with immutable provider inputs.
4. Use `run show`, `run graph`, and `run guidance` to inspect current work.
5. Perform primary work outside engine, then append evidence or annotations.
6. Request only events listed by `run show`.
7. Inspect `run history`; export a completed audit snapshot when needed.
8. Terminate abandoned active runs explicitly. Runs are never deleted.

Provider update, rename, disable, and restore preserve stable registration identity. Active runs use current registration configuration for provider execution while retaining their immutable stored graph and creation identity. Use `run compatibility` before provider-dependent work after an update.

## Structured automation

Use `--format json`. One invocation emits exactly one bounded JSON object to stdout. Treat `operation`, `outcome`, `reason`, `data`, `request_id`, and `trace` as one correlated result. Exit `0` means completed, `2` rejected, `1` application error, and `64` pre-dispatch failure. Never infer semantic outcome from diagnostics alone.

Every invocation reserves its trace before argument parsing or application work. Preserve `request_id` and trace path when reporting defects. Trace files are diagnostic evidence, not durable state authority.

## Backup and recovery

SQLite file `{state_root}/state.db` is sole authority. No backup, restore, import, repair, replay, or delete command exists.

For a consistent filesystem backup:

1. Stop launching CLI invocations and wait for all existing processes to exit.
2. Copy entire `LOOP_ENGINE_HOME`, including `state.db`, possible `state.db-wal` / `state.db-shm`, configuration, and integration metadata.
3. Preserve ownership and restrictive permissions.
4. Test copied home with same or newer compatible binary before relying on it.

To restore, keep original untouched, restore complete home into a private replacement directory, set `LOOP_ENGINE_HOME` to that directory, and run a provider-free read such as `run list --all`. Do not combine a database from one home with metadata from another. Export directories cannot restore engine authority.

## Migration policy

Database migrations run automatically, forward-only, and transactionally during startup. Back up complete quiescent home before upgrading binary. Older binary rejects newer database versions; failed migration or schema verification dispatches no application operation and starts no provider. Never edit `PRAGMA user_version`, migration tables, DDL, or `integration_metadata` manually. Full contract: [persistence.md](persistence.md#migration-policy).

## Troubleshooting

| Symptom | Action |
|---|---|
| `provider.*` error | Run `provider check`; verify executable path, working directory, timeout, protocol major, one-object stdin/stdout, and stderr diagnostics. |
| `provider.registration.stale` | Re-read `provider list`; another catalog mutation changed configuration revision. Rebuild request from current record. |
| `state.stale_version` | Re-read `run show` and requestable events; another invocation committed first. Do not retry stale command blindly. |
| `run.lifecycle.terminal` | Run is final or terminated. Inspect `run history`; no reopen operation exists. |
| `cursor.invalid` | Cursor belongs to different query/filter/store or was altered. Restart pagination without cursor. |
| `persistence.failed` at startup | Stop. Preserve complete home unchanged. Check free space, permissions, filesystem health, and exact stderr/trace. Do not repair SQLite manually. |
| Future schema version | Use compatible newer binary or restore pre-upgrade complete backup. Never decrement version metadata. |
| Trace sink failure | Application outcome remains authoritative if dispatch completed. Preserve stdout object and stderr; correct trace-directory permissions or capacity before next invocation. |
| Export target exists | Choose new absent directory. Export never overwrites or merges. |

For corruption, migration, transaction, WAL, and commit-outcome details, see [persistence.md](persistence.md). For trace rotation and late-sink behavior, see [operational-trace.md](operational-trace.md).

## Defect report checklist

Include binary revision/version, platform, full argv with secrets removed, structured outcome, exit code, request ID, trace file, relevant provider stderr, and whether state was copied or restored. Do not attach secrets or private evidence content. Keep failed database unchanged until diagnosis completes.
