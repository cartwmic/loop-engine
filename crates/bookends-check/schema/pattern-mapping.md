# Bookends v1 three-corpus pattern mapping (frozen)

This file maps retained, changed, and rejected source patterns from the three
named corpora onto the grammar in `prd.md`, `repo-config.md`, and
`runner-grammar.md`. It is the reviewable acceptance proof for schema-declared
tokens. The checker must not re-open these decisions.

Fenced excerpts below parse under `prd.md`. Original corpus excerpts that are
not `### LE-<n>: <title>` records parse as human prose (valid, not a record).
Each corpus also has a v1 transcription that parses as a live or tombstone
record.

## Corpora

1. **loop-engine docs/PRD.md** — living git markdown PRD, numbered human-prose
   headings, no requirement IDs, no citation tokens.
2. **AAR Forge v1.4** — living PRD at Google Doc
   `1hpn2uOnxHb-sM-TpKr-khGIzHRxnyzsrhfRJUMZPuDk`, accepted 17-Aug-2026.
   `aar-forge docs/prd.md` is only a pointer to that document. Requirement
   identifiers in the document are `R1`–`R20`, `G1`–`G8`, `J1`–`J7`.
3. **Compass specs/*.md PREFIX-N blocks** plus
   `scripts/check-spec-bookends.py` — split `specs/` capability files with
   `### PREFIX-N: <title>` records, `@spec:` citations, tombstones, coverage
   classes, reverse-ratchet check (c), and Phoenix `fw:` / `regen/map.tsv`
   durability.

## Frozen mapping decisions

### Retained

- **Never-reuse.** IDs are never reused. Identity is the ID token.
- **Tombstones.** Retirement keeps the same ID heading, stays parseable, and
  is exempt from coverage.
- **Dangling-tag failure.** A citation whose ID is missing or tombstoned
  fails (Compass check (a)).
- **Mandatory live coverage.** Every live requirement declares coverage and
  must have at least one eligible citation of each declared class (Compass
  check (b), narrowed to `e2e/journey` plus optional `contract`).
- **Optional contract class.** `contract` exists and may be omitted at
  repo-config level. Once declared it is enforced the same way as
  `e2e/journey`.
- **Surface-liveness of discovery pathspecs.** Every declared class pathspec
  must match at least one tracked file, or the check is red.
- **Fail-closed malformed input.** Missing Status, live without Coverage,
  unknown coverage class names, duplicate Status or Coverage, a bad attempted
  ID-record heading, a citation that is not `bookends:LE-<n>`, duplicate IDs,
  and discovery or configuration errors fail. Nothing-to-validate is not
  green when the layer is enabled.

### Changed

- **One living git markdown PRD.** The left bookend is one committed markdown
  file named by `bookends.toml` `prd`, not a `specs/` split and not a
  Drive-only document. AAR Forge's Google Doc remains a source pattern, not
  the v1 left bookend.
- **Schema-declared tokens.** IDs are `LE-<n>`. Citations are
  `bookends:LE-<n>`. Not Compass `PREFIX-N` and not `@spec:`.
- **Fail-closed eligibility from named-job command collection.** A citation
  is eligible only when a named required job's `run:` command matches
  `runner-grammar.md` and that command's collection set includes the file.
  Job existence plus pathspec is not enough.
- **Immediate-parent continuity.** The current PRD is compared only with the
  immediately preceding committed PRD, when that PRD exists. A requirement's
  exact ID plus title is its identity. A live record may stay live or retire
  only as a same-ID, same-title tombstone; retained tombstones cannot
  disappear or become live again. New IDs are allowed. No older commit is
  promoted into authority, and a malformed immediate parent is not skipped.
  When no parent PRD exists, the current document is first adoption and has no
  continuity baseline.
- **One Rust library+CLI.** `bookends-check` is one deterministic library plus
  CLI. Its `candidate` command validates PRD grammar only; repository config,
  coverage, CI eligibility, and continuity remain checker concerns. Compass
  `check-spec-bookends.py` is a source pattern, not the implementation.

### Rejected

- **`specs/` as left bookend.** Compass capability files and front-matter
  prefixes are not the PRD.
- **Compass PREFIX-N / `@spec:` spelling.** Not required, not accepted as v1
  tokens.
- **Reverse-ratchet check (c) and its allowlist.** Durable e2e/journey files
  may exist without a live-ID citation. `spec-bookends-allowlist.txt` is not
  imported. Untagged durable files may be advisory and never fail the check.
- **Phoenix `fw:` / `regen/map.tsv` durability.** Eligibility does not read
  grain maps, firewall-cited files, or regen ledgers.
- **NLP.** The checker does not infer untagged English claims.
- **nothing-to-validate-as-green when enabled.** Compass exits 0 when
  `specs/` is absent or empty. v1 enabled with a missing or malformed PRD,
  or a PRD that parses to zero records, is red.

## Representative excerpts that parse under `prd.md`

Heading classification is mechanical: a `###` heading whose remainder after
`### ` matches `^LE-` is an attempted ID record; any other `###` heading is
human prose and not malformed. The original excerpts below therefore parse
as valid under `prd.md` (human prose, or no attempted ID record). Each
corpus also has a v1 transcription that parses as a live record.

### From loop-engine docs/PRD.md

This excerpt is labeled from loop-engine docs/PRD.md.

Living-prose `###` headings. Remainder `3.1 Goals` does not match `^LE-`, so
the heading is human prose, not a record, and not malformed.

```markdown
### 3.1 Goals

Loop Engine v2 must:

- preserve workflow control state across process, session, and actor boundaries;
- mechanically enforce permitted transitions rather than trusting callers to follow the workflow;
- remain neutral to actor type, agent harness, model, and workflow domain;
```

v1 transcription of that claim as a live record (parses under `prd.md`):

```markdown
### LE-1: Preserve control state across actor boundaries
- Status: live
- Coverage: e2e/journey

Loop Engine v2 must preserve workflow control state across process, session,
and actor boundaries.
```

A later heading `### 3.2 Non-goals for v0.1` ends the record. Statement prose may evolve while the exact `LE-1` title stays live; changing
that title against an immediate parent is reassignment and is red.

### From AAR Forge v1.4

This excerpt is labeled from AAR Forge v1.4.

`aar-forge docs/prd.md` is only a pointer. Remainder of its headings does not
match `^LE-`. The living PRD is Google Doc
`1hpn2uOnxHb-sM-TpKr-khGIzHRxnyzsrhfRJUMZPuDk` (accepted 17-Aug-2026).

```markdown
# AAR Forge — Product Requirements Document

**The PRD lives in Google Docs, not in this repository.**

→ **[AAR Forge — Product Requirements Document](https://docs.google.com/document/d/1hpn2uOnxHb-sM-TpKr-khGIzHRxnyzsrhfRJUMZPuDk/edit)**

That document is the **source of truth for product requirements**. Version 1.4 was accepted by the owner on 17-Aug-2026.
```

Plain-text requirement `R1` from that Google Doc (v1.4, accepted 17-Aug-2026).
No `### LE-` heading, so the excerpt is human prose and not malformed.

```text
R1 A person or agent invokes each run through a harness. Iteration one has no scheduler, daemon, or service identity, and AAR Forge's repository stores no run artifacts. Certified evaluation cases are not run artifacts. They are curated data a human commits, and R20 governs them.
```

v1 transcription (parses under `prd.md`):

```markdown
### LE-1: A person or agent invokes each run through a harness
- Status: live
- Coverage: e2e/journey

Iteration one has no scheduler, daemon, or service identity, and AAR Forge's
repository stores no run artifacts.
```

A retired AAR Forge identifier would keep the same heading and become:

```markdown
### LE-1: A person or agent invokes each run through a harness
- Status: tombstone
```

### From a Compass specs/*.md PREFIX-N block

This excerpt is labeled from a Compass specs/*.md PREFIX-N block.

`specs/uploads.md` PREFIX-N record `UPL-1`. Remainder `UPL-1: Large or
geocoding-heavy uploads ingest asynchronously` does not match `^LE-`, so the
heading is human prose, is not a v1 record, and is not malformed. Compass
`@spec:UPL-1` is not a v1 citation token.

```markdown
### UPL-1: Large or geocoding-heavy uploads ingest asynchronously

`POST /api/v1/uploads` SHALL respond `202 Accepted` with the batch in status
`processing` — ingest continuing in the background — whenever the file's
valid-row count exceeds the async routing threshold or its projected geocoding
pass is heavy, and otherwise complete ingest before responding `201 Created`
with the batch already terminal.

Coverage: contract, go
```

v1 transcription. Compass `contract, go` does not transfer: `go` is an
unknown coverage class name and would be invalid. v1 keeps optional
`contract` only when `[classes.contract]` is declared; otherwise
`e2e/journey` alone:

```markdown
### LE-1: Large or geocoding-heavy uploads ingest asynchronously
- Status: live
- Coverage: e2e/journey, contract

POST /api/v1/uploads responds 202 Accepted with the batch in status
processing whenever the file's valid-row count exceeds the async routing
threshold or its projected geocoding pass is heavy.
```

Compass tombstone pattern from `specs/CLAUDE.md` (retained as a tombstone,
rejected as title-encoded retirement). Remainder `UPL-15: (retired 2026-07-11
— superseded by UPL-4)` does not match `^LE-`, so the original heading is
human prose and not malformed.

```markdown
### UPL-15: (retired 2026-07-11 — superseded by UPL-4)
```

v1 tombstone for that same ID identity (parses under `prd.md`). Retirement is
`- Status: tombstone`, not a title suffix. The title is not identity; `LE-15`
is:

```markdown
### LE-15: Uploads all-or-nothing ingest
- Status: tombstone
```

The citation in proof code is `bookends:LE-1`, not `@spec:UPL-1`.

Rejected Compass check (c) and Phoenix durability, quoted from
`scripts/check-spec-bookends.py` (not a PRD record; remainder never matches
`^LE-`):

```text
  (c) every durable e2e spec file (e2e/tests/** or e2e/invariants/**) carries
      >=1 `@spec:` tag, unless listed in scripts/spec-bookends-allowlist.txt
      — a shrink-only ratchet: an allowlisted file that ALREADY carries a
      tag is itself an error (the entry should have been removed once the
      file was tagged).

Durability (`go` and `ts` coverage classes only): a `*_test.go` file counts
toward `go` only when it is durable — it carries `//go:build integration`,
OR it is cited by an `fw:` token in regen/map.tsv (the firewall registry;
see load_fw_paths).

Exit codes: 0 = clean, or nothing to validate (specs/ absent or empty — a
no-op during the pre-P0 transition window before specs/ exists); 1 = one or
more findings; 2 = CLI usage error (argparse default).
```
