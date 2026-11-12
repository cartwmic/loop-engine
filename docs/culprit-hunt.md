# Culprit hunt on a red main

Use this when a push run of ci.yml on main is still red after one `gh run rerun --failed <run id>`. It gives one culprit sha for each failing step. Go cheapest first and stop as soon as a step has its culprit. Run every check with kgate, not on the driver host: the host may have no cargo, and its PATH may hold an installed `bookends-check` that the gate wrapper would pick up before the source build.

## 1. List the pushes since the last green run (no compute)

```sh
R=<head sha of the red run>
gh api 'repos/cartwmic/loop-engine/activity?ref=main&per_page=100' --jq '.[] | "\(.after) \(.before)"' |
sed -n "/^$R /,\$p" |
while read -r after before; do
  run=$(gh run list --repo cartwmic/loop-engine --workflow ci.yml --branch main --event push --commit "$after" --json databaseId,status,conclusion --jq '.[0] // empty | "\(.databaseId) \(.status) \(.conclusion)"')
  echo "PUSH $after before=$before run=$run"
  [ -n "$run" ] && gh run view "${run%% *}" --repo cartwmic/loop-engine --json jobs --jq '.jobs[] | .name as $j | .steps[] | select(.conclusion == "failure" or .conclusion == "skipped") | "  \($j) | \(.name) | \(.conclusion)"'
  case $run in *success) break ;; esac
done
```

- The activity API lists every update to main, newest first, with GitHub's own `before` and `after`. That is the exact range each push run's Bookends step judged.
- `sed` starts the list at the red push. The loop stops at the first push whose run concluded `success`, which is the last green run.
- Each line shows the steps that failed or were skipped. A skipped step after a failed one never ran on that push.
- A push with an empty `run=` was not run on its own. Its commits count as part of the next push's range.
- If a run shows `in_progress`, wait with `gh run watch <run id> --exit-status`, then list again.

## 2. Tree-based steps: cargo test, journeys, `release-tests`, `checks`

These steps judge the tree at the pushed head. A failing step's culprit lies between two heads:

- **L**: the newest listed push head where that step passed. If it never ran on any listed push, use the last green head.
- **U**: the oldest push head after L where that step failed with the same failure.

Then list the candidates:

```sh
git fetch origin && git rev-list --first-parent --reverse L..U
```

- This lists every main commit after L up to and including U, oldest first. `--first-parent` keeps squash commits only.
- **One commit**: it is the culprit. Stop here.
- **More than one commit**: go to step 3 with that list.

Read the failure (no compute):

```sh
gh run view <U's run id> --repo cartwmic/loop-engine --log-failed
```

- This prints only the failed step's log. Build CHECK, the one command step 3 runs, from it:
  - **cargo test.** The log shows `test <name> ... FAILED` and cargo's hint ``error: test failed, to rerun pass `<args>` ``. CHECK is `cargo test <args> -- --exact <name> [<name>...]`. Those are cargo's own rerun arguments, so only the failing tests build and run.
  - **Journey or mutation step.** CHECK is that step's `run:` line from ci.yml, for example `python3 scripts/research-journey.py`. The bare journeys are self-tests and need no built binaries.
  - **release-tests.** A failed shard prints its unittest log. CHECK is `python3 -m unittest <id> [<id>...]` with the ids from its `FAIL:`/`ERROR:` lines. With no ids, CHECK is `python3 scripts/run-release-tests-sharded.py`.
  - **checks** (the squash result failed fmt, Clippy or build on main): CHECK is that step's `run:` line.

## 3. Test the candidates in one pod (only when the range has more than one commit)

```sh
git fetch origin
git worktree add --detach "$HUNT" U
cd "$HUNT"
run-and-wake -- kgate -- sh -c 'set -u; check=$1; shift; export PYTHONDONTWRITEBYTECODE=1; for c; do git checkout -q --detach "$c" || exit 2; echo "== $c"; sh -c "$check" || { echo "FIRST RED $c"; exit 1; }; done; echo "NO RED"' hunt "CHECK" <every candidate except U, oldest first>
```

- `HUNT` is a path in a driver-owned scratch folder outside the checkout, for a new worktree.
- kgate ships this worktree and the shared Git folder to one pod, so every candidate commit is there. `git checkout` changes only the pod's copy.
- kgate takes argv, so the loop is one `sh -c` script in single quotes. CHECK goes in as `$1`, as one argument in double quotes, and the inner `sh -c "$check"` runs it. The candidate shas follow as the loop's `$@`.
- One pod tests the candidates in order and keeps one `target/` directory, so later builds can reuse the first one's output. The central tests may still rebuild their own binaries, which is slower, not wrong. The loop stops at the first commit that fails.
- U is left out on purpose: its own push run already failed this step. `NO RED` therefore means U is the culprit by elimination.
- `PYTHONDONTWRITEBYTECODE=1` matches the `tests` and `release-tests` jobs.
- Exit 2 means a checkout failed. That is a setup failure, not a result.
- Add `--cpu 2 --mem 2Gi` before `--` when CHECK is Python only. The profile's 12 CPUs can wait a long time to be scheduled on a busy cluster.

Read the result as in [Read the result](kgate-baseline.md#read-the-result) once the job has ended, then:

- A non-empty kgate `reason` means the check did not run to its end. That is no result. Run the job again.
- With `reason` null, read the pod log.
  - `FIRST RED <sha>`: that is the culprit, but only if the pod log shows the same failure as U's run. If it fails because the test, the target or the script does not exist yet at that commit, it is not the culprit. Run step 3 again with only the commits after it.
  - `NO RED`: U is the culprit.

Remove the worktree when you are done: `git worktree remove "$HUNT"`.

## 4. Bookends pushed-range gate

Bookends judges a push's `before..after` range, not a tree. It evaluates every commit the push introduced and prints each finding with that commit's sha. On RED it prints the blocking findings first, then the intermediate ones.

**The step failed** (no compute). Run `gh run view <run id> --repo cartwmic/loop-engine --log-failed` and read the lines after `RED`:

- `<sha> (parent <p>): …`: a continuity finding. `<sha>` is the culprit.
- `<after>: …`: a finding on the pushed head's tree. If the push carried one commit, that commit is the culprit. Otherwise the culprit is the earliest commit in `git rev-list --first-parent --reverse <before>..<after>` whose own `<sha>: …` line has the same text after the sha. The checker prints findings in sha order, not history order, so match them against the rev-list.
- `history acquisition incomplete`, `publication history is incomplete`, `historical … unavailable` or `local Git state …`: these are about acquiring history, not about a commit. Nothing is reverted. Report it to the lead.

**The step was skipped** (an earlier `tests` step failed): no later push will check this push's range. Run the gate on that range yourself, with that push's own `before` and `after` from step 1. Do it from a worktree at that `after`, so the checker is the one CI would have built:

```sh
git fetch origin
git worktree add --detach "$HUNT" <after>
cd "$HUNT"
run-and-wake -- kgate --cpu 4 --mem 3Gi -- sh -c 'set -u; before=$1; shift; cargo build -q --locked -p bookends-check || exit 2; for c; do echo "== $c"; printf "refs/heads/main %s refs/heads/main %s\n" "$c" "$before" | cargo run -q --locked -p bookends-check -- --updates-stdin --remote origin --receipt-root /tmp/bookends-receipts || { echo "FIRST RED $c"; exit 1; }; done; echo "NO RED"' hunt <before> <after>
```

- The pod's checker is built from the worktree's source with cargo. That avoids the gate wrapper, which takes the first `bookends-check` on PATH, and any inherited `BOOKENDS_*` variables, so no bypass applies.
- The arguments are the ones the wrapper passes in CI: `--updates-stdin --remote origin`. The `printf` line is the same update line the CI step pipes.
- `--receipt-root /tmp/bookends-receipts` keeps the receipt in the pod, outside the repository, as the checker requires.
- `before` is passed as `$1`. Each further argument becomes the pushed head, so `<before> <after>` checks exactly that push's range.
- Exit 2 means the build failed. That is no result.
- If the replay prints `RED`, read its findings as in "The step failed" above.
- Remove the worktree when you are done: `git worktree remove "$HUNT"`.
