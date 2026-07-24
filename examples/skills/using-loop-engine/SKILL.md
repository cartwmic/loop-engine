---
name: using-loop-engine
description: Use when an agent needs to create, resume, advance, inspect, audit, or terminate a durable Loop Engine workflow supplied by an executable provider.
---

# Using Loop Engine

## Principle

Loop Engine coordinates work; it does not perform it. Provider owns workflow graph and gate policy. Caller performs external work. Engine owns durable run state, lifecycle, evidence, and journal. Never edit engine SQLite state.

## Start or Resume

Use configured `LOOP_ENGINE_HOME`; changing it selects a different catalog and database. Prefer structured output and treat each result's `outcome`, `reason`, `data`, `request_id`, and `trace` as one unit.

```bash
loop-engine --format json --list-operations
loop-engine --format json provider list
loop-engine --format json run list --all
```

Resume known run with `run show`. For new run, use existing provider registration. Register only when given explicit executable and working-directory paths:

```bash
loop-engine --format json provider add <HANDLE> \
  --exec <ABSOLUTE-PATH> --working-directory <ABSOLUTE-DIR>
loop-engine --format json provider check <HANDLE>
loop-engine --format json run create <HANDLE> \
  --label <LABEL> [--inputs <JSON-FILE>]
```

Record returned run ID. Provider documentation defines input object.

## Drive Workflow

Repeat until lifecycle is `final` or `terminated`:

1. Inspect current state and only requestable events returned by engine.
2. Read stored graph and provider guidance when needed.
3. Perform requested work outside engine; produce artifacts expected by provider.
4. Add or select evidence when workflow requires it.
5. Request one listed event, then inspect outcome and journal.

```bash
loop-engine --format json run show <RUN-ID>
loop-engine --format json run graph <RUN-ID>
loop-engine --format json run guidance <RUN-ID>
loop-engine --format json run evidence add <RUN-ID> \
  --kind <KIND> --ref <OPAQUE-LOCATOR> [--digest <DIGEST>]
loop-engine --format json run request <RUN-ID> <EVENT> \
  [--evidence-id <ID> ...] [--evidence <JSON-FILE>] [--note <TEXT>]
loop-engine --format json run history <RUN-ID> --limit 100
```

`completed` means operation committed. `rejected` means understood but denied; inspect `reason`, fix work or choose valid event. `error` means evaluation or commit did not complete reliably. After `state.stale_version`, re-read `run show`; never retry stale request blindly.

## Inspect and Close

```bash
loop-engine --format json run evidence list <RUN-ID> --limit 100
loop-engine --format json run compatibility <RUN-ID>
loop-engine --format json run export <RUN-ID> --output <NEW-DIRECTORY>
loop-engine --format json run terminate <RUN-ID> --note <REASON>
```

Follow returned cursors to read complete history or evidence. `run history` is ordered activity journal; `run show` is current authority. Export creates `manifest.json`, `state.json`, and `journal.jsonl`; it is not backup or restore input. Use compatibility after provider updates. Terminate abandoned active runs; runs cannot reopen or be deleted.

## Do Not

- Invent events; request only events from `run show`.
- Assume `run request` performs primary work.
- Parse human output for automation.
- Treat exit `2` as infrastructure failure; it represents rejection.
- Dereference evidence through engine; locators are opaque.
- Assume provider process retains memory between calls.

Provider authoring example: `examples/providers/reference-go/README.md`. Full operation guidance: `docs/operator-guide.md`.
