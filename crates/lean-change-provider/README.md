# lean-change-provider

A small Loop Engine provider for software changes. It checks; the driver works.

```text
explore -> intent-approval -> (plan if hard) -> implement -> verify -> review -> end
```

- The intent names an executable check for each criterion. The owner approves it in their own words.
- `verify` runs the checks and binds the result to the repository tree. Any later edit makes it stale.
- The old-vs-new check is on by default: `verify` also runs each check on the base commit with the proof files laid over
  it, and each criterion says what it must do there (`fails`, `same`, `absent`, or `none` with a reason). `--proof-path`
  at setup names the directories or files that hold the tests and journeys.
- Profile `b` adds one fresh-context check of the draft intent before the owner sees it (six fixed questions: use,
  edge, control, terms, criteria-gap, principles-gap; the packet carries the owner's design principles) and one fresh-context diff review with six fixed questions (intent, scope, edge cases,
  checks, docs, the owner's design principles). Every finding gets a disposition.
- Profile `c` is the same code with the intent check and the review state absent.

Binary: `lean-change` (not released). Commands: `setup`, `verify`, `intent-packet`, `review-packet`, and
describe/evaluate on stdin for the engine. Requirements: LE-202 to LE-210 in docs/PRD.md at the checkout root.
Driving procedure: [skill](skills/using-lean-change-provider/SKILL.md).

## Build and test

```sh
cargo test -p lean-change-provider
python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change
python3 scripts/lean-change-mutations.py --engine "$BIN_DIR"/loop-engine
```

(run the scripts from the checkout root). The mutation script builds each mutant from a disposable
copy into a disposable target directory (`--work-root DIR` to place it), so the checkout and its
`target/debug/lean-change` are never altered.
