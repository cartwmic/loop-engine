# Bookends v1 PRD grammar (frozen)

This file is the sole v1 token and record contract for a living git markdown
PRD. A later checker parses this grammar and must not widen tokens, classes,
or citation forms. The living git PRD is the sole authority that declares
requirement IDs. Proof surfaces and software-change artifacts may only cite
IDs already declared live here; they do not allocate or mint IDs.

The PRD never names proof files by path. Coverage names classes, not files.

README.md and AGENTS.md are not coverage classes.

## Frozen tokens (do not widen)

| Kind             | Form                                                                 |
| ---------------- | -------------------------------------------------------------------- |
| ID token         | `LE-<n>` with `<n>` = `[1-9][0-9]*`                                  |
| Citation token   | exact substring `bookends:LE-<n>`                                    |
| Live status      | `- Status: live`                                                     |
| Tombstone status | `- Status: tombstone`                                                |
| Live coverage    | `- Coverage: e2e/journey` or `- Coverage: e2e/journey, contract`     |

Not tokens: Compass `@spec:` spelling, Compass `PREFIX-N` headings.

## ID token

A requirement identity is the ID token `LE-<n>` where `<n>` is `[1-9][0-9]*`
(no leading zeros; `LE-0` is not an ID).

The continuity identity is the exact pair of ID token and heading title.
Changing the title under an existing ID is a reassignment and is red when a
preceding committed PRD exists. Statement and coverage prose may evolve while
that exact pair remains live.

## Immediate-parent continuity

Continuity is deliberately a one-step check, not a history database. The
current PRD is compared only with the immediately preceding committed PRD
when that file exists. A live record may remain live or become a tombstone
with the exact same ID and title. A previously retained tombstone must remain
the exact same tombstone; it cannot disappear or become live again. New IDs
may be adopted. If no parent PRD exists, the current document is first
adoption and has no continuity baseline. A malformed immediate parent is not
skipped in favor of an older commit.

The parser-only candidate command validates this grammar without reading
`bookends.toml`, proof files, workflows, or git continuity:

```text
bookends-check candidate PRD.md
```

The candidate command applies the grammar only, but a candidate must contain
at least one live record or retained tombstone; it does not require repository
configuration, proof coverage, CI eligibility, or continuity.

## Citation token

The citation token is the exact substring `bookends:LE-<n>` where `<n>` is
`[1-9][0-9]*` and the following character is not a digit. That last clause
keeps `bookends:LE-1` from matching inside `bookends:LE-10`.

Not a citation token:

- Compass `@spec:` spelling (including `@spec:LE-<n>` and `@spec:PREFIX-N`)
- Compass `PREFIX-N` headings or tokens (`UPL-1`, `OPS-1`, and the like)
- `bookends:LE-0`, `bookends:LE-01`, `bookends:LE-`, or `bookends:LE-<n>`
  with a leading zero in `<n>`
- any other prefix, punctuation, or wrapping around `LE-<n>`

A citation that is not that exact token is malformed.

## Skip marker

A tracked file that contains the exact substring `bookends:skip` is
ineligible. That marker is not a citation token and does not name a
requirement.

## Heading classification

A heading in this grammar begins with exactly `### ` (three `#` characters
and one space). The remainder is the rest of that line after `### `. A line
that begins with `###` but not that exact prefix (for example, `###LE-1` or
`###\tLE-1`) is outside this grammar: it is not a record and is not malformed.
A line such as `###  Prose` has the prefix; its remainder begins with a space
and does not match `^LE-`, so it is human prose and not malformed.

- If the remainder matches `^LE-`, the heading is an attempted ID record.
  It must be exactly `### LE-<n>: <title>` with `<n>` = `[1-9][0-9]*` and a
  non-empty title, or the PRD is malformed. Malformed attempted ID-record
  headings include `### LE-0:`, `### LE-01:`, `### LE-`, and `### LE-<n>`
  missing `: <title>`.
- If the remainder does not match `^LE-`, the heading is human prose, is
  not a record, and is not malformed.

A `###` heading whose remainder after `### ` matches `^LE-` is an attempted ID record that must be `### LE-<n>: <title>` or malformed, and that any other `###` heading is human prose and not malformed.

Human-prose `###` headings anywhere in the document, including living-prose
sections, must parse as valid. `#`, `##`, and `####` (or deeper) lines are
not `### ` headings and are not records.

## Record extent

A record ends at the next `###` heading (attempted ID record or human-prose) or EOF.
The record starts at an attempted ID-record heading.

## Canonical record shape

The parser ignores non-list lines inside a record. Extra human prose inside
a record is allowed and ignored. Recognized list items use exact spelling:
an unindented hyphen, one space, capitalised key, colon, one space. A line
that begins `- Status:` is a Status list item even when the value is not
`live` or `tombstone`; those values are malformed, not ignored. A line that
begins `- Coverage:` is a Coverage list item even when the value is not one of
the two canonical live lines; those values are malformed, not ignored. Other
lines, including indented list items and other list keys, are ignored.

Status appears once. Live Coverage appears once. Duplicate Status or
Coverage is malformed. Coverage, when present, follows Status in document
order.

### Live record

Exactly:

```markdown
### LE-<n>: <title>
- Status: live
- Coverage: e2e/journey
```

or, when the contract class is declared for this requirement:

```markdown
### LE-<n>: <title>
- Status: live
- Coverage: e2e/journey, contract
```

A live record has `- Status: live` and `- Coverage:` listing `e2e/journey`
and optionally `contract`. `e2e/journey` is mandatory on every live record.
`contract` is additional and may appear only when a live contract surface
exists to hold tags. The Coverage line is exactly `- Coverage: e2e/journey`
or `- Coverage: e2e/journey, contract` (that order, comma-space).

Human prose may sit between the heading, Status, and Coverage. Status still
appears once; Coverage still appears once; Coverage still follows Status in
document order.

### Tombstone record

Exactly:

```markdown
### LE-<n>: <title>
- Status: tombstone
```

A tombstone record has `- Status: tombstone`, retains the same ID heading,
is exempt from coverage, and stays parseable. A tombstone has no Coverage
line. A Coverage line on a tombstone is malformed.

## Coverage classes

v1 class names are only:

| PRD token     | Meaning                                      |
| ------------- | -------------------------------------------- |
| `e2e/journey` | mandatory on every live record               |
| `contract`    | optional; additional on top of `e2e/journey` |

`contract` exists in this schema and may be omitted at repo-config level.
Omitting `[classes.contract]` in `bookends.toml` means the class is
undeclared and unenforced; a live record must not list `contract` unless
that class is declared.

Unknown coverage class names are invalid.

## Parse vs enabled check

Parsing a document of only human-prose `###` headings succeeds: those
headings are not records and are not malformed. When bookends are enabled,
a PRD that parses to zero live and tombstone records is red.
Nothing-to-validate is allowed only when the layer is off.

## Worked mixed document

Living-prose headings and ID records may share one file. The prose heading
is not a record. Identity is `LE-1` / `LE-2`, not the title text.

```markdown
### 3.1 Goals

Loop Engine v2 must preserve workflow control state across process, session,
and actor boundaries.

### LE-1: Preserve control state across actor boundaries
- Status: live
- Coverage: e2e/journey

Changing the statement while the exact ID/title pair stays live is allowed;
changing the title is an identity reassignment and is red against the
immediate parent.

### LE-2: Retired experimental path
- Status: tombstone
```

## Malformed inputs that must fail

The PRD is malformed when any of the following hold:

- an attempted ID-record heading that is not `### LE-<n>: <title>`
- a record with missing Status
- a Status value other than `live` or `tombstone`
- a live record without Coverage
- a tombstone record with a Coverage line
- Coverage that appears before Status
- unknown coverage class name
- Coverage that omits `e2e/journey` on a live record
- Coverage that lists `contract` before `e2e/journey`, or with spelling
  other than the two canonical lines above
- duplicate Status or Coverage inside a record
- duplicate ID tokens across live records and retained tombstones in the
  current document
- a citation that is not the exact token `bookends:LE-<n>` as defined above

Missing or malformed enabled PRDs fail closed. Nothing-to-validate is not
green when bookends are enabled.
