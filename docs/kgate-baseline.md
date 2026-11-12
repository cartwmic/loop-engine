# Running the baseline on kgate

Load this before running `scripts/baseline.sh` on the homelab cluster: as a pre-merge option for a risky change, or when the owner asks. A culprit hunt on a red main does not run the baseline: follow [culprit-hunt.md](culprit-hunt.md). The baseline is the union of the jobs in [ci.yml](../.github/workflows/ci.yml), plus Bookends in working-tree mode. PRs do not need it. The kgate profile is [kgate.toml](../kgate.toml).

## Run

From the repository root:

```sh
run-and-wake -- kgate -- scripts/baseline.sh
```

Mac undo: to revert this switch, change the command above back to `scripts/baseline.sh` and run it from the repository root. kgate never runs its command on the Mac, so a baseline that did not run on the cluster is missing proof, not a Mac run. This applies whenever a baseline is required or asked for.

## Read the result

`run-and-wake` returns at once: it prints the job id, the pid and the output file, and its own exit status says nothing about the baseline. The baseline's real exit lands later as an `end` record for that job id in `${XDG_STATE_HOME:-~/.local/state}/run-and-wake/log.jsonl`. The wake goes to the Pi/Herdr pane or T3 thread that started the job, and it is best effort: `run-and-wake` prints `nothing will wake you` when it has no target (a session outside Pi and T3). A subagent that has no T3 thread of its own cannot be woken: ending its turn ends the subagent and the wake goes to its parent, which has none of the job context.

- If a wake will reach you, end the turn and wait for it.
- If not (no wake target, a subagent, or the wake never arrives), do not end the turn: poll that log until a line with your job id and `"event": "end"` appears, and read its `exit`. Do not read the immediate return as a pass.

Then read kgate's record for the job in `${XDG_STATE_HOME:-~/.local/state}/kgate/log.jsonl` (its `job` field is the run-and-wake job id).

- `reason` is JSON `null` when the baseline ran, and then its `exit` is the baseline's status.
- A non-empty `reason` means the job did not run to its end on the cluster, whatever the exit code (69, or 130/143 when interrupted): report missing proof.
- Exit 69 with a `null` `reason` is the baseline itself exiting 69, which is a failure.
- A job with no kgate record (kgate missing or killed) is also missing proof.

## Check the record is for your tree

The handoff cites the `tree` hash from the job's kgate record. To check that it is the tree being handed off, compute kgate's hash for the tree from the repository root and compare:

```sh
t=$(mktemp -u); git ls-files -co --exclude-standard -z | GIT_INDEX_FILE=$t git update-index --add --remove -z --stdin && GIT_INDEX_FILE=$t git write-tree; rm -f "$t"
```
