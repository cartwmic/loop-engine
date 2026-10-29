---
name: using-lean-change-provider
description: Use when driving a lean-change run in Loop Engine: explore, owner-approved intent, implement, verify with digest-bound checks, and (profile b) one fresh-context intent check and one fresh-context diff review.
---

# Using the lean-change provider

lean-change checks; it does not work. You do the work and append records. The provider refuses
a transition until the records for it are complete. Load the `using-loop-engine` skill for
the engine commands (`start`, `show`, `append`, `event`). Use explicit `--database` and `--config`.

Two profiles, one provider: **b** has an intent check and a review state, **c** has neither. The profile is frozen at start.
The old-vs-new check is on by default in both (see its section below).

## Start (owner confirms first)

Starting a run is an owner decision. Before `start`, show the owner: the profile file path and its
sha256, the repository path and base, the provider binary version, and what you will build.
Wait for their go. Never swap the profile.

```sh
lean-change setup --profile b --repo /abs/repo --principles /abs/design-principles.md > /abs/profile.json
lean-change setup --profile c --repo /abs/repo > /abs/profile.json     # no review
lean-change setup --profile c --repo /abs/repo --proof-path tests --proof-path ':(glob)**/*_test.go' > /abs/profile.json
shasum -a 256 /abs/profile.json
loop-engine --database D --config C --json start --id RUN lean-change @/abs/profile.json "label"
```

`setup` freezes the Git worktree root, even when `--repo` names a subdirectory; a profile that names
a subdirectory is refused at verify and review. `--proof-path P` (repeatable) names each directory or file that
holds the tests or journeys: a git pathspec relative to the repository root, such as `tests` or `:(glob)**/*_test.go`.
With none, the old tree is the bare base; see the old-vs-new check below.
`providers.toml` names the provider `lean-change` with the `lean-change` binary as its command.
Keep every file the run produces (profile, database, config, verification, packet) in a work directory
`$WORK` OUTSIDE the repository: a new file inside it is an edit and makes the verification stale.
Run `show --view full RUN` after every transition: it gives the state's instructions and the
denial feedback. A denial names what is missing; fix that, do not argue with it.

## The loop

1. **explore.** Read the repo and the request. Append the intent, then request `intent-ready`:

   ```sh
   loop-engine ... append --record-id intent-1 RUN intent '{
     "author": {"name": "you", "kind": "agent"}, "goal": "...", "constraints": ["..."], "hard": false,
     "criteria": [{"id": "c1", "statement": "...", "check": {"argv": ["sh","-c","pytest tests/x.py"], "timeout_ms": 120000},
                   "old": {"expect": "fails"}}]}'
   ```

   Every criterion also needs `old`, the old-vs-new expectation described in its section below.

   Every criterion needs a command that fails when the criterion is not met and passes when it is.
   Cover edge cases with extra criteria, not prose. Set `hard` true only when the work needs a plan.
   Each new intent gets a new record id (`intent-2`, ...); the latest one counts.
   **Intent check (profile b).** Before `intent-ready` have the draft checked once by a reviewer who has not seen this work
   (a fresh session or a different model). Give them the packet unchanged:

   ```sh
   loop-engine ... --json show --view full RUN | lean-change intent-packet > "$WORK/intent-packet.md"
   ```

   The reviewer returns one JSON object; append it as kind `intent-check`. Each question they could not settle from the
   intent fails with a finding whose text is a concrete question for the owner. The first well-formed check is the
   check; a second cannot replace it. For each finding, either change the intent (`fixed`, and append a new intent record
   that carries the fix) or reject it with a reason you would say to the owner. A `criteria-gap` finding names a delivery
   that passes every check yet misses the goal; fix it with a criterion whose check fails for that delivery. Append one
   `finding-disposition` per finding, with `review` set to the `intent-check` record id. The check runs once per run; a later revision is not re-checked.
2. **intent-approval.** Show the owner the intent verbatim, with the check's findings and how each was disposed (profile b), and ask: "What do you expect that is not listed?" Their answer may change the intent. Append their exact words (do not edit them):

   ```sh
   loop-engine ... append RUN owner-decision '{"checkpoint": "intent", "subject": "intent",
     "subject_revision": "intent-1", "decision": "approve", "text": "<their words>",
     "actor": {"name": "owner", "kind": "human"}}'
   ```

   Request `approved` (hard false) or `approved-hard` (hard true). If they want changes, request `revise`,
   write `intent-2`, and ask again. A rejection is the same record with `"decision": "reject"`, a `target` of `intent` and a `reason`.
3. **plan** (hard only). Append `{"intent": "intent-1", "text": "..."}` as kind `plan`, request `planned`.
4. **implement.** Change the code. Stay inside the approved goal and constraints. Request `ready`.
5. **verify.** Run the checks on the current tree and append what they print:

   ```sh
   loop-engine ... --json show --view full RUN | lean-change verify > "$WORK/v.json"
   loop-engine ... append RUN verification "$(cat "$WORK/v.json")"
   ```

   Request `verified`. It is refused if any criterion failed, was not run, its old runs do not meet its `old`
   expectation, or the tree changed after the run (any edit, even a comment, to a tracked or not-ignored file;
   git-ignored files are not covered, so a check must not depend on them). Checks run in their own process group and are killed at their timeout.
   If the checks rewrite the repository between them, verify refuses; make them read-only. If a check fails, fix the code and verify again. Do not edit the intent
   to make a check pass; that needs a new intent and the owner's approval.
6. **review** (profile b only). Produce the packet and give it to a reviewer who has not seen this work
   (a fresh session or a different model), unchanged:

   ```sh
   loop-engine ... --json show --view full RUN | lean-change review-packet > "$WORK/packet.md"
   ```

   The reviewer returns one JSON object. Append it as kind `review`. The first well-formed review is the
   review; a second one cannot replace it. Then for each finding, either fix it
   or reject it with a reason, and append one `finding-disposition` per finding:

   ```sh
   loop-engine ... append RUN finding-disposition '{"review": "<review record id>", "finding": "f1",
     "disposition": "fixed", "reason": "..."}'
   ```

   If you changed code, run verify again and append the new verification. Request `reviewed`. There is one
   review; do not ask for a second.
7. **end.** Report the criteria, their results, and each finding with its disposition.

`revise` from implement, verify and review goes back (to explore, implement and implement). Going back is free; the
gates check the records again when you return.

## The old-vs-new check (on by default)

Every profile this setup writes checks each criterion on the old code as well as the current code. The old tree is the
intent's base commit in a temporary checkout outside the repository, with the current proof files laid over it:
tracked and not-ignored proof files are copied and proof files the change deleted are removed; git-ignored files are not
copied. `verify` runs the check there and records the runs; `verified` judges them against the `old` expectation:

- `"fails"`: a bug fix or a feature. The check must fail on the old tree, twice, with the same exit code (not 126 or 127,
  not a timeout).
- `"same"`: a refactor. The check must exit 0 on the old tree with exactly the output the current tree gives.
- `"absent"`: brand-new behavior. Any result but a pass; a command that cannot start there is accepted.
- `"none"`: work the check cannot apply to, with a required `reason`. Its old runs are not made. This is the escape hatch,
  for example a guard the old tree cannot inform, such as a full mutation proof.

`--proof-path` tells the old tree where the proof lives. With no `--proof-path` the old tree is the bare base: a test
file the change adds is missing there, so a check that runs it exits non-zero cheaply. For `"fails"` that counts as a real
failure but proves nothing, so add `--proof-path` for test files. A check whose proof sits inside product files (Rust
unit tests in source files, for example) cannot be separated by proof paths; use `"absent"` or `"none"` with a reason.
The old tree has no build cache, so a check that builds pays a cold build. A check whose old binary dies by a signal has no
exit code there, which is no real failure for `"fails"`; wrap such a command (for example in `sh -c`) so the death becomes an
exit code. Each criterion that is not `none` adds one run
(`same`, `absent`) or two (`fails`) to every verify.

## Rules

- The reviewer must not be you and must be in a fresh context. `fresh_context: true` is a claim; make it true.
- The intent check has five fixed questions and the review has six, including whether the diff breaks the owner's design principles.
  Do not shorten or reword them. Do not pre-answer them for the reviewer.
- Findings are not nits: each has an obligation, evidence and a consequence. Reject a finding only with a reason you would say to the owner.
- Never write a `verification` record by hand. Never copy an owner decision from an earlier intent.
- The engine never runs your checks or agents. Everything outside the records is yours.
