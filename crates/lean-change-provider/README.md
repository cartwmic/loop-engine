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

Binary: `lean-change` (not released). Commands: `setup`, `verify`, `intent-packet`, `review-packet`, `outcome-map`,
and describe/evaluate on stdin for the engine. Requirements: LE-202 to LE-212 in docs/PRD.md at the checkout root.
Driving procedure: [skill](skills/using-lean-change-provider/SKILL.md).

`outcome-map` makes one page for the owner from a finished run. It reads `show --view full` on stdin and prints HTML
on stdout. It writes no file. The section "What was approved and what ran" shows the goal, each criterion with one
label, and each issue with how it was settled. With `--into AR_PAGE` the section goes into Accountable Review's page at
its two anchors and every other byte is kept; a page without the anchors exits 1. Exit 0 on success. Exit 1 on any
error, with the message on stderr and nothing on stdout.

The labels are:

- `changed as intended: failed on the old code, passes now`
- `kept: same output on the old and new code`
- `new: the old code did not have this`
- `old code did not behave as expected: expected it to fail`
- `old code did not behave as expected: expected the same output`
- `old code did not behave as expected: expected the feature to be missing`
- `not run on the old code: <reason>`
- `not run on the old code: this run has no old-code check`
- `fails on the new code`
- `not run yet`

The section's first line is one of:

- `These results are current: the last run of the checks (<verification id>) used the code as it is now.`
- `These results are out of date: the code changed after the last run of the checks (<verification id>). Code fingerprint now <tree>; when checked <tree>.` Each row with a recorded result then starts with `out of date: `.
- `The checks have not run yet for the latest goal (<intent id>).`
- `The last run of the checks (<verification id>) could not be read: <error>`
- `It is unknown whether these results are current: <reason>`

## Build and test

```sh
cargo test -p lean-change-provider
python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change
python3 scripts/lean-change-mutations.py --engine "$BIN_DIR"/loop-engine
```

(run the scripts from the checkout root). The mutation script builds each mutant from a disposable
copy into a disposable target directory (`--work-root DIR` to place it), so the checkout and its
`target/debug/lean-change` are never altered.
