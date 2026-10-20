# Arm comparison: options, arms and commands

This page documents the Stage 0 additions to the software-change harness. They let four arms run on the same
cases: native Pi (`native`), Pi with Superpowers (`superpowers`), Pi with OpenSpec (`openspec`) and Pi with
Loop Engine (`loop-engine`). The design and the gap list (G1 to G10) are in the
[Stage 0 spec](../specs/stage0-native-vs-workflow-vs-loop-engine.md), sections 5 to 8.

Everything here is opt-in. With no `arm` option set, prompts, scoring, captures and reports are the ones in the
[software-change README](README.md) and the [evals README](../README.md). Those two files gained only link lines.

Read [section 15](#15-what-scripted-proofs-do-not-show) before using any number from these commands. The additions
are covered by scripted and fake-backend tests only. No paid model call, smoke trial or Stage 1 trial is part of
this change.

## 1. What is opt-in

| Piece | Turned on by | Off by default |
|---|---|---|
| Arm descriptor: driver argv, prompt, harness baseline, completion rule, image | `target_options.arm` | Existing preamble, top-level `driver_command` and `image`, existing success rule |
| Final message, owner word totals, driver-only time | `arm` | Not recorded |
| Independent grader (`outcome-grade-v1`) | `grade = true` with an arm | No grader call |
| Visible-test receipt (a harness receipt for the grader, not a success rule) | `visible_test_command` with `grade = true` | Not run; the grader's claim call sees `null` |
| Driver margin | `driver_margin_seconds` | 300 seconds, the same as before |
| Workspace snapshots and replay | `snapshot_interval_seconds` | No snapshots |
| Pi session record | `SC_PI_SESSION_DIR` in the image | Pi runs with `--no-session`; arm trials set it to `/workspace/trial/pi-sessions` (section 9) |
| Block labels for `compare` | `block` | Unlabelled campaigns compare by case and combination |
| `evalctl compare`, `grader-calibrate`, `stage-ledger`, `findings-check` | Running the subcommand | Nothing runs |

The subcommands read or write only their own files. None launches a trial. `grader-calibrate` is the one that
calls a grader model, and a real grader model call is paid, so it needs owner approval.

## 2. Target options

Set these in the campaign's `[target_options]` table. Other target options are unchanged.

| Option | Type and default | Needs | Effect |
|---|---|---|---|
| `arm` | table; absent | nothing | Selects one arm (section 3) and turns on the arm path: rendered prompt, harness baseline commit, the one completion rule, final-message capture, owner word totals and driver-only time. |
| `driver_margin_seconds` | positive number; 300 | `hard_limit_seconds` for any effect | Seconds kept between the driver's stop and the hard limit, for capture and scoring. The driver budget is `hard_limit_seconds - driver_margin_seconds`, never below 60 seconds. Works with or without an arm. |
| `snapshot_interval_seconds` | positive number; absent (off) | `idle_limit_seconds` (the progress poll takes the snapshots) | Takes a private workspace snapshot about this often during the driver run (section 7). Works with or without an arm. |
| `grade` | boolean; `false` | an `arm`, and a judge model from a different provider family than the driver | After scoring, runs the independent grader (section 8) and stores its verdict fields and the grade detail (section 8). |
| `visible_test_command` | argv array of nonempty strings; absent (off) | `grade = true`, and so an `arm` | Runs the repository's visible tests once, after the hidden tests, in the restored replay container at `/workspace/public/repo`, with the case test timeout. Exit 0 gives `passed`; any other exit, or a timeout, gives `failed`; exit 126 or 127 (the command cannot run) gives `error`. The status is a harness receipt for the grader's claim call. Only the status is stored, and success does not change. |
| `block` | table; absent | nothing | Labels a single-trial campaign for `evalctl compare` (section 11). The target records it and does nothing else with it. |

Refused before any trial starts: a margin or snapshot interval that is not a positive number (booleans
included), a snapshot interval with no idle limit, `grade` that is not a boolean or is set without an arm, `grade`
when the judge's provider family is the driver's or cannot be determined, and a `visible_test_command` that is set
without `grade = true` or is not a nonempty argv array of nonempty strings.

With an `arm` set, top-level `driver_command` and `image` are refused. Put them inside the arm table.

The campaign settings `idle_limit_seconds`, `hard_limit_seconds` and `judge_model` are not target options. The
snapshot, margin and grader options read them.

Example for a native-arm trial. Keep `[target_options.arm]` last, so later keys are not read as arm keys:

```toml
[target_options]
driver_margin_seconds = 1500
snapshot_interval_seconds = 300
grade = true
block = { task = "label-keys", repetition = 1, arm = "native", order = 1, attempt = 1 }

[target_options.arm]
id = "native"
driver_command = ["arm-native"]
prompt_template = "native"
workflow_owned_paths = []
completion_rule = "driver-exit"
image = "loop-engine-sc-evals:arms-v1"
```

With `grade = true`, the campaign's `judge_model` must come from another provider family than the driver model.
Use one campaign per trial, as the Tier 0 campaigns do, so each campaign has one driver.

## 3. Arms

The arm table has these keys: `id`, `driver_command`, `prompt_template`, `workflow_owned_paths`, `completion_rule`
and optional `image`. Validation enforces:

- `id` is one of `native`, `superpowers`, `openspec`, `loop-engine`.
- `driver_command` is a nonempty list of nonempty strings. The harness appends the prompt.
- `prompt_template` equals `id`. No other template exists.
- `workflow_owned_paths` is a list of repo-relative glob strings. Absolute paths, `..` and backslashes are refused.
  It must be nonempty for `superpowers` and `openspec`, and empty for `native` and `loop-engine`.
- `completion_rule` is `driver-exit`, the only rule.
- `image` is an optional nonempty tag. Without it, the default trial image is used.

| Arm | `driver_command` | Prompt addition | `workflow_owned_paths` | `image` |
|---|---|---|---|---|
| `native` | `["arm-native"]` | none | `[]` | the arms image (section 10) |
| `superpowers` | `["arm-superpowers"]` | "Use the Superpowers workflow installed in this Pi session for this request." | required, for example the four globs below | the arms image |
| `openspec` | `["arm-openspec"]` | "Use the OpenSpec workflow initialised in this repository for this request." | required, for example the four globs below | the arms image |
| `loop-engine` | `["sc-driver"]` | Loop Engine addition: profile, the two installed skills, `sc-implement` and `sc-review`, run ID and artifact paths | `[]` | omit: the default image |

The design's four workflow-owned globs are `openspec/**`, `.pi/**`, `docs/superpowers/**` and `.superpowers/**`.
Any declared workflow-owned path leaves the grader's file inputs and the discipline scope, and is counted as artifact volume.

Omitting `image` for `native`, `superpowers` or `openspec` passes validation, but the default image has no
`arm-*` launchers. The driver command then fails inside the container, and that exit is scored like any other
driver exit. Check the image before a paid run. The launcher must exist in the image named by the arm, for
example with `docker run --rm TAG sh -c 'command -v arm-native'`. That command was not run while writing this page.

Superpowers arm:

```toml
[target_options.arm]
id = "superpowers"
driver_command = ["arm-superpowers"]
prompt_template = "superpowers"
workflow_owned_paths = ["openspec/**", ".pi/**", "docs/superpowers/**", ".superpowers/**"]
completion_rule = "driver-exit"
image = "loop-engine-sc-evals:arms-v1"
```

Loop Engine arm, on the default image:

```toml
[target_options.arm]
id = "loop-engine"
driver_command = ["sc-driver"]
prompt_template = "loop-engine"
workflow_owned_paths = []
completion_rule = "driver-exit"
```

## 4. Prompt and starting state

With no arm, the prompt is the existing `PREAMBLE` in `scevals/target.py`, byte for byte. A golden test pins it.

With an arm, the prompt is the neutral base prompt, then the arm's addition (section 3), then the scripted-driver
line when one is set, then `Owner request:` and the request. The base prompt says that the harness has already
created the starting baseline commit and an empty bare remote, that delivered changes must not be committed or
pushed, that `ask-owner` is the only way to reach the owner, and that the launch is preapproved. This is a
different text from `PREAMBLE`, so an arm prompt does not match a pilot prompt, even for `loop-engine`. The
baseline wording moved from the agent to the harness on purpose.

Every trial gets the bare remote `/workspace/local-remote.git`, with a post-receive hook that counts successful
pushes. That is existing behaviour. For an arm trial that is not a resume, the harness also runs, before the
driver starts:

- `git init` in `/workspace/public/repo`,
- a commit `Starting baseline` (author `eval-harness`, allowed empty),
- `git remote add origin /workspace/local-remote.git`.

So the diff and the push count mean the same thing for every arm. With no arm, the driver makes that setup itself,
as before.

## 5. Completion and scoring

With an arm set, one rule applies to every arm, `loop-engine` included:

- **Complete**: the driver process exited by itself before its driver budget. A nonzero exit code does not make it
  incomplete. A harness stop (idle, hard time or campaign limit) does.
- **Success**: complete, and every tagged hidden test ran exactly once, with no collection error, no skip and no
  failure. This is `hidden_tests_passed` in `scevals/scoring_common.py`.
- Delivery recall and discipline are scored from the final workspace, whatever the outcome. Elicitation recall comes
  from the owner receipt. Workflow-owned paths are removed from the discipline scope and counted as artifact volume.
- Engine lifecycle, `intent.json` validity, the exit code and workflow adherence are diagnostics. They never decide
  success. They are `null` for an arm that has no such thing.
- A trusted provider route failure (`provider policy block` or `provider rejected bridged request`) is infrastructure
  whatever the driver exit, so the trial is excluded from the success rate rather than scored as a failure.

| Driver ends | Complete | Success, if every hidden test passes | Outcome |
|---|---|---|---|
| Exits by itself, exit 0 | yes | yes | success or failure by tests |
| Exits by itself, nonzero exit | yes | yes | exit code is a diagnostic |
| Loop Engine run `terminated` or `active`, driver exited by itself | yes | yes | engine state is a diagnostic |
| Idle stop (no progress for `idle_limit_seconds`) | no | no | failure |
| Hard time stop or campaign limit | no | no | `limit`; delivery is still scored |
| No exit code recorded | no | no | incomplete, reason `no_driver_exit` |

With no arm, the existing rule is unchanged. A trial succeeds only when the engine lifecycle is `final`, the
`intent.json` is valid and the driver did not time out. A terminated engine run is a failure, as before. That
difference is intended, and it is the Tier 0 Sol case the spec describes.

Metrics written for an arm trial: `complete`, `incomplete_reason`, `hidden_test_outcome`, `delivery_recall`,
`elicitation_recall`, `discipline` and `diagnostics`. `diagnostics` holds `driver_exit_code`, `engine_lifecycle`,
`intent_valid` and `workflow_adherence`. Workflow adherence is `null` for every arm in this version. A
`loop-engine` arm also gets the existing engine diagnostics: `run_health`, `honesty`, `all_reviews_passed`,
`review_escapes`, `needed_role_aliases`, `profile_config_version` and `intent_judge`. `intent_judge` is the older
`intent-judge-v1` diagnostic. It is separate from the grader. It calls the judge model when the intent artifact is valid.
A failed judge call (a model error, an infrastructure failure or a timeout) is recorded as `judge_error` and never fails
the trial.

## 6. Driver margin

The driver is stopped at `hard_limit_seconds - driver_margin_seconds`, never below 60 seconds. The margin is what
capture and scoring get. Without the option it is 300 seconds, as before.

Example: a hard limit of 4500 seconds with a 1500-second margin stops the driver at 3000 seconds. The spec's Stage 1
classes use 1500 seconds for T1 and T2 and 1800 seconds for T3, because T3's hidden-test timeout is 1200 seconds
and scoring runs inside the hard limit. Those values are in the spec's section 7 table. No campaign file in the
repository sets them.

## 7. Snapshots and delivery trajectory

With `snapshot_interval_seconds` set, the progress poll takes a snapshot of `public/repo` (including its `.git`,
with the same cache exclusions as the final capture) at most about every `snapshot_interval_seconds`. The poll
runs every `min(60, idle_limit_seconds / 2)` seconds. The first snapshot is due one interval after the driver
starts. A snapshot is taken at the first poll that is at least one interval after the previous one, so spacing
rounds up to whole polls. Snapshots are taken while the driver runs, so a file may change during the copy. A
tar exit of 1 is tolerated, as in the capture.

Layout, beside the trial's capture and never inside it:

```text
captures/CAMPAIGN/RUN_TIMESTAMP/SAMPLE_ID-snapshots/
  snapshots.json            index: sequence, file, elapsed_seconds
  snapshot-000001.tar
  snapshot-000002.tar
```

`captures/` is git-ignored. Snapshots and replay results are private evidence and are never published.

Limits:

- Each snapshot is read in one sandbox read. Inspect limits that read to 100 MiB unless
  `INSPECT_SANDBOX_MAX_READ_FILE_SIZE` changes it. The final capture avoids the limit by splitting at
  `capture_chunk_bytes` (64 MiB by default). Snapshots are not split, so an oversize snapshot raises from the
  progress poll.
- A failed snapshot command raises `InfrastructureError("workspace snapshot failed")`.

Replay, which needs Docker: `scevals.snapshots.replay_snapshots(directory, case, image)` runs the hidden tests on
each snapshot in a fresh mount-free container and returns one point per snapshot, oldest first:
`sequence`, `elapsed_seconds`, `delivery_recall` and `delivery_by_requirement`. Pass the arm's image explicitly.
The default is the default image. No `evalctl` command calls replay, and campaign reports do not include the
trajectory yet.

With the option absent, no snapshot directory is made and the capture is unchanged.

## 8. Independent grader (`outcome-grade-v1`)

The grader is `scevals.grader.grade`. A campaign reaches it through `grade = true` and the campaign's `judge` role,
which is `judge_model`. There is no `evalctl grade` command.

**Inputs.** The owner brief, the requirement IDs, the baseline and final contents of each changed file, the
unified diff, and the final message. The final message is the driver's stdout, kept privately as
`final-message.txt` in the trial's capture. Workflow-owned paths are removed from the baseline, the final and the
diff alike. The owner Q&A and workflow artifacts are never inputs.

**Two calls, two samples each.**

- *Fidelity call.* It sees no test results. It returns, per requirement, a verdict (`satisfied`, `partial`,
  `violated` or `not_addressed`) and a probability. It also lists unrequested changes with a `material` flag, and
  it gives `intent_faithful` with a probability.
- *Claim call.* It runs only when the final message has sentences. It sees numbered sentences and trusted receipts:
  the hidden-test status per requirement. Each claim is `supported`, `contradicted` or `not_checkable`. An honest
  statement about checks the grader cannot see is `not_checkable`. The visible-test receipt is `null` unless
  `visible_test_command` is set (section 2). When it is set, its status is `passed`, `failed` or `error`.

**Evidence pointers.** The host checks each pointer. The quoted text must appear in the cited lines of the final
file, or of the diff, after whitespace is normalised. A pointer may be `null` only with a stated reason.

**Unstable verdicts.** If the two fidelity samples disagree on `intent_faithful` or on any requirement, that verdict
is `unstable`. An unstable intent is counted as not faithful. A claim that the samples disagree on is also
`unstable`.

**Redaction and leakage.** In the final message, these terms become `[redacted]`, case-insensitively:
`Loop Engine`, `loop-engine`, `OpenSpec`, `Superpowers`, `intent.json`, `opsx`, `sc-driver`, `sc-implement` and
`sc-review`. The brief, file contents and diff are not redacted. They are scanned, and the counts are reported
in the `leakage` field of the grade result.

**Independence.** The provider family comes from the leaf of the model name: `claude` is `anthropic`; `gpt`, `o1`,
`o3` and `o4` are `openai`; `gemini` is `google`. Any other name is unverifiable. The grader is refused with
`refused-same-family` when its family matches the driver's, and with `refused-family-unverified` when either
family cannot be determined. There is no override. The grader's calls are judge-role usage, so they are evaluation
overhead, apart from workflow cost.

**What a campaign stores.** Three target metrics from the grade result: `outcome_grade`, with `intent_faithful`
(`true`, `false` or `null`) and `unstable`; `outcome_grade_status`; and `outcome_grade_detail`. The status is one of
`graded`, `refused-same-family`, `refused-family-unverified`, `grader_error` or `limit`. `grader_error` covers an invalid
grader reply and a failed grader call (an infrastructure failure or a timeout). The hidden tests have already scored
when the grader runs, so a grader error or a budget stop never changes the trial's success.

`outcome_grade_detail` is the `grade()` export without its `overhead` entry, because usage is reported as
evaluation overhead through the judge role. It holds per-requirement verdicts and probabilities, the unrequested
change flags, pointer tallies, claim counts, leakage counts and the status. It holds no prose, diff text or quote
text. A grade that is not `graded` stores the same keys with nulls and empty requirements. The detail sits under
each trial's `target_metrics` in `summary.json` and in `report.md`.

**Calibration: `evalctl grader-calibrate`.** This runs the predeclared controls and, optionally, the owner-label
acceptance rule, and writes one receipt.

```sh
uv run --project evals evalctl grader-calibrate evals/software-change/example-case \
  --grader-model codex/gpt-6.1-sol \
  --driver-model pi/claude-compat/claude-sonnet-5-5 \
  --case-check-receipt ~/eval-private/case-check.txt \
  --near-miss-workspace ~/eval-private/near-miss \
  --near-miss-requirement REQ-2 \
  --owner-labels ~/eval-private/owner-labels.json \
  --output ~/eval-private/grader-receipt.json
```

- `--case-check-receipt` is a file holding the output of `evalctl case-check` for this case. It must contain one
  `CASE_CHECK` line, and the case must be `VALID` with the same ID and version. `case-check` runs Docker, so it
  was not run while writing this page. The failing no-op requirements come from this receipt.
- `--near-miss-workspace` is a directory of delivered files that must not satisfy `--near-miss-requirement`.
- `--output` is a new file. An existing file is never overwritten.

The controls are:

1. *Reference overlay.* The public repository plus the case's reference overlay must be counted faithful, and every
   requirement `satisfied`.
2. *No-op repository.* The untouched repository must not be counted faithful, and each requirement that fails on it
   must not be `satisfied`.
3. *Near-miss workspace.* The named requirement must not be `satisfied`.

The owner-label file has `schema_version` 1 and an `entries` list. Each entry has an `id`, `owner_label` (`good` or
`deficient`), `scope_violating` (a boolean) and `workspace` (a directory). An entry may also have `final_message`
text and an `arm` table, which removes that arm's workflow paths. Acceptance needs exactly 12 entries, at least 10
of them agreeing with the owner, no owner-`deficient` diff counted faithful, and every scope-violating diff
graded with a material unrequested change. False alarms and confident-wrong verdicts (wrong, at probability 0.9
or more) are reported. They are not part of acceptance.

The receipt's `mode` is `normal` only when all three controls pass and the owner labels are accepted. Otherwise it is
`diagnostic-only`, and `reasons` names each failure, including `owner labels not supplied`. The receipt holds
verdict fields, counts and those short reason strings. It holds no file contents and no final messages.

Exit codes: 0 when a receipt is written, in either mode; read the `Calibration mode` line. 2 when refused, for
example a same-family grader, an invalid or mismatched case-check receipt, or an invalid owner-label file.

The controls make six fidelity calls and no claim calls, because they have no final message. Each owner label makes two
fidelity calls, and two claim calls when it has a final message.
Those calls go to a real grader model when run, so they need owner approval. The spec estimates the controls at
under $1.

`evalctl compare --calibration FILE` reads the receipt's mode and labels every intent verdict `diagnostic-only` unless
the mode is `normal` and the receipt's `grader_model` is the campaign's own grader, its `settings.judge_model`
(section 11). Campaign reports do not read the receipt. Their `outcome_grade` values are raw, so the owner must still
apply a `diagnostic-only` result to them by hand, as the spec says.

## 9. Phase attribution and Pi session records

**The hook.** `image/pi-role.py` has `session_arguments()`. When `SC_PI_SESSION_DIR` is set and non-empty, Pi is
started with `--session-dir DIR` and keeps its JSONL record there. When it is unset or empty, Pi gets
`--no-session`, as before. The arm launchers use the same function.

**Wired into arm trials.** When an arm is set, `scevals/target.py` sets `SC_PI_SESSION_DIR` to
`/workspace/trial/pi-sessions` for the driver and everything it starts. That path is inside `/workspace/trial`, so the
private capture tar keeps the records beside `workspace.tar`'s other trial files. Without an arm nothing changes: Pi
keeps `--no-session`. Run `python -m scevals.phases` over the extracted directory. The wiring is tested at the
target seam and the launchers are proved in the arms image; no paid trial has produced a real record.

**The parser.** `scevals/phases.py` splits tokens and wall time by phase and role. A phase is a `read` of a
`SKILL.md` file (`skill:` followed by the directory name), or a user message starting with `/NAME` (`command:NAME`;
`/skill:NAME` maps to `skill:NAME`). Work before any marker is `unmarked`. Role is the model alias of the assistant message.

```sh
uv run --project evals python -m scevals.phases evals/software-change/tests/fixtures/pi-session
```

The argument is a `.jsonl` record or a directory of them. The output is JSON with `available`, `reason` and
`phases`. Each phase row has `phase`, `role`, `turns`, the five token fields and `seconds`. The result is
unavailable, with `phases` set to `null`, when the record is missing, unreadable or unparseable, or has a message
without a timestamp, a timestamp that goes backwards, or usage without token counts. Unavailable is never zero.
The command exits 0 in both cases, so read `available`.

OpenSpec limit: Pi strips the `/opsx-*` name from stored user messages. Template-driven OpenSpec work therefore
shows as `unmarked`.

## 10. Arms image and launchers

`image/Dockerfile.arms` layers on the pinned trial image. It does not change `image/Dockerfile`, and the default
image and its identity are unchanged.

- Base: build argument `BASE_IMAGE`, default `loop-engine-sc-evals:v0.26.0`. Build that base first with
  `image-check`.
- Pi `@earendil-works/pi-coding-agent@1.2.0`. The base image has 1.0.0. The frozen pilot used 1.0.0.
- Superpowers `v7.0.0`, cloned at that tag. The build fails unless the checkout is commit
  `bb92a77741419a4ab5f06e711a283343f1ada0c3`.
- OpenSpec `@fission-ai/openspec@1.14.3`.
- `/opt/sc-evals/arms/versions.txt` records these pins. The build also checks `pi --version` and
  `openspec --version`.
- Copies `pi-role.py` and `pi-arm.py` into `/opt/sc-evals/`. Symlinks `arm-native`, `arm-superpowers` and
  `arm-openspec` to `pi-arm.py`.

Build, from the checkout root. It needs network access for npm and git, and no model credentials:

```sh
docker build -f evals/software-change/image/Dockerfile.arms -t loop-engine-sc-evals:arms-v1 evals/software-change/image
```

Any tag works, as long as the arm's `image` names it. The trial's image check still compares the revision label with
the configured refs. The arms image inherits the base image's labels, so that check still applies.
`evalctl image-check` does not check the arms image.

Launchers, all with the role-driver model alias through the bridge and prompt arguments only:

- `arm-native`: Pi print mode with `--no-extensions --no-skills --no-prompt-templates`. Loads nothing else.
- `arm-superpowers`: as native, plus `--extension /opt/sc-evals/arms/superpowers`.
- `arm-openspec`: runs `openspec init --tools pi` in the case repository, with its output on stderr. Then loads
  `.pi/skills` and `.pi/prompts` from the repository.

Flags are refused. Use `arm-native PROMPT` or `arm-native -- PROMPT`, as with `sc-driver`.

## 11. `evalctl compare`

```sh
uv run --project evals evalctl compare SUMMARY [SUMMARY ...] --out DIR [--calibration RECEIPT]
```

It reads each campaign's `summary.json` (schema version 1) and never writes to it. It writes `summary.json` and
`report.md` into a new timestamped directory under `DIR`. It refuses an output directory inside an input campaign
directory. Keep `DIR` outside the checkout unless you are selecting a report to commit.

Labelled campaigns carry `target_options.block` with `task`, `repetition`, `arm`, `order` and `attempt`. `task` and
`arm` are nonempty strings. `repetition`, `order` and `attempt` are integers of at least 1. `attempt_reason`
is required when `attempt` is above 1.
A labelled campaign holds exactly one trial. The same task, repetition, arm and attempt may appear once only.
Unlabelled campaigns, such as the Tier 0 summaries, are grouped under their case and combination. Their order is
shown as not recorded.

Each block shows its realised arm order, then one row per arm attempt. A rerun is a separate row beside the
original. The columns are: outcome and reason, hidden tests, delivery, elicitation, intent, workflow cost and
tokens, overhead cost and tokens, driver wall seconds, trial wall seconds, owner questions, owner words (questions
and answers), and notes. Notes flag limit, infrastructure, resumed and carried trials, and non-terminal trials.

Field sources:

- Driver wall seconds: `target_metrics.driver_elapsed_seconds`. This is set only for arm trials. Older summaries
  show `unavailable`.
- Owner words: `target_metrics.owner_interaction`, numbers only. Older summaries show `unavailable`.
- Intent: `target_metrics.outcome_grade`. An unstable verdict shows as `unstable`. A campaign without a grader
  verdict shows `unavailable`.
- Calibration: the `mode` of `--calibration` and its `grader_model`. An intent verdict is labelled `(diagnostic-only)`,
  as in `faithful (diagnostic-only)`, unless the mode is `normal` and the `grader_model` equals the campaign's own
  `settings.judge_model`. The report states the receipt once, above the tables: `normal`, the `diagnostic-only`
  reasons, or `not supplied`. Below that it states, for each campaign, which receipt applied, for example `receipt
  normal for grader X; this campaign's grader is Y: diagnostic-only`.

`--calibration` takes a `grader-calibrate` receipt. A missing or unreadable receipt, or one whose mode is neither
`normal` nor `diagnostic-only`, is diagnostic-only, never an error and never normal. With no receipt the mode is
`not supplied`, which is also diagnostic-only. A `normal` receipt without a `grader_model` is diagnostic-only, with the
reason `receipt has no grader model`. `summary.json` records the same `calibration` state; a normal one includes its
`grader_model`.

Missing usage, price or metric shows as `unavailable`, never zero. Workflow cost is the driver, implementer and
reviewer roles. Overhead is the simulator and judge roles, shown apart.

A resumed or rescored trial has no exact per-call dollars, so its role dollars are an upper bound from its recorded
usage (with input and output counts) at each model's highest-tier price, shown as `<=$`. `summary.json` labels each
figure with `workflow_cost_basis` or `overhead_cost_basis` (`exact`, `upper-bound` or `unavailable`), and an upper-bound
delta reads `(upper-bound)`.

**Paired deltas vs native.** A block with a native trial shows, after its table, a table titled "Paired deltas vs
native (arm minus native)". Each non-native attempt appears there as its arm minus the block's native reference, the
original (lowest) native attempt. The four deltas are workflow cost (USD), workflow tokens, trial wall seconds and
driver-only seconds. A delta is `unavailable` when either side is unavailable, and null in `summary.json`, never
zero. The workflow-dollar delta is also unavailable when the native side's dollars are only an upper bound, because
subtracting an upper bound gives neither a bound nor an exact value. Each block's `paired_deltas` in `summary.json` holds the same values. A block without a native trial shows no
deltas and says so. These are descriptive differences on one block. They make no significance claim. Three
repetitions are a screen (spec section 7).

Run on the two shipped Tier 0 full summaries, writing to a directory outside the checkout:

```sh
uv run --project evals evalctl compare \
  evals/software-change/results/tier-0-full-sol/20261018T135956.603628Z/summary.json \
  evals/software-change/results/tier-0-full-sonnet/20261018T132346.405605Z/summary.json \
  --out /tmp/arm-compare
```

The inputs are not modified. The Sol trial shows `failure (target_failure)` although its hidden tests pass. That run
used the existing engine-based rule, which scores a terminated engine run as a failure (spec section 7).

## 12. `evalctl stage-ledger`

The ledger is one JSON file for one stage. It records every command of the stage, including smoke trials,
controls, grading, failed attempts and approved reruns. It launches nothing. Run the command only after `admit`
succeeds.

```sh
uv run --project evals evalctl stage-ledger init LEDGER --stage-dollars 400 --stage-hours 40
uv run --project evals evalctl stage-ledger admit LEDGER --command "T1 native trial 1" --class T1
uv run --project evals evalctl stage-ledger record LEDGER 1 --outcome success --hours 0.5 --dollars 3.5
uv run --project evals evalctl stage-ledger admit LEDGER --command "T1 native trial 1" --class T1 --reason "approved rerun"
uv run --project evals evalctl stage-ledger status LEDGER
```

- `init` creates a ledger with the stage dollar and hour caps. It refuses an existing file. The spec's stage cap
  is $400 and 40 hours. The code does not hard-code it.
- `admit` takes `--command` (the exact command text), `--class` (`T1`, `T2` or `T3`) and `--reason`. It prints
  the new entry, including `handed_cap`. Repeating the same command text makes attempt 2, 3 and so on. Attempt 2 or
  later needs `--reason`. A command whose previous entry is still open cannot be admitted again.
- `record` takes the entry ID, `--outcome`, `--hours` and optional `--dollars`. Omit `--dollars` only when the
  cost is unavailable.
- `status` prints spent, reserved, remaining, overshoot, `unpriced_commands` and `open_entries`.

Class caps, from the spec's per-trial hard limits (4500, 13500 and 23400 seconds):

| Class | Dollars | Hours |
|---|---|---|
| T1 | 12.00 | 1.25 |
| T2 | 45.00 | 3.75 |
| T3 | 120.00 | 6.50 |

Admission rules. `admit` exits 2 and writes nothing when:

- any recorded command has no dollars, because remaining dollars are then unknown. An unpriced entry blocks every
  later admission in that ledger. Record `--dollars` whenever the cost is known.
- remaining dollars or remaining hours is below the class cap. Remaining is the stage cap minus spent minus
  reserved. Open entries reserve their handed cap until recorded.
- an earlier entry for the same command is still open.

Overshoot: spend above the stage cap is reported as `overshoot`. Remaining may then go negative, and later
admissions are refused. The stage cap is best effort, as the spec says. Example: a $20 stage admits a T1 command
at $20 remaining. Recording $25 gives an overshoot of $5.00 and a remaining of -$5.00.

**Reading ambiguity, for the owner to decide before Stage 1.** Spec section 7 says two things. The ledger refuses
a command when remaining allowance is below that command's own cap. It also hands the command
`min(class cap, remaining)`. The implementation follows both literally. It refuses when remaining dollars or hours
are below the class cap. After that check passes, remaining is at least the class cap, so the handed cap is
always the class cap, and the `min` never lowers it.

Two readings are possible:

- **Implemented, reading A.** A command is admitted only when its full class cap fits, and it is handed the class
  cap. No trial gets less than its class cap.
- **Reading B.** A command is admitted whenever some allowance remains, and it is handed `min(class cap, remaining)`.
  The last trial of a stage can then run with a smaller cap than its class. The design rejected this, because
  it silently weakens that trial. Reading B also changes the refusal threshold, so it contradicts the spec's first
  sentence unless that sentence is read differently.

Class caps come from the spec's per-trial caps. All three hour caps, and the T2 and T3 dollar caps, are above
`TIERS.md`'s 20-dollar and 1-hour campaign ceilings. The owner waived those ceilings for this run only, in words.
No value in `TIERS.md` is changed in code.

## 13. `evalctl findings-check`

```sh
uv run --project evals evalctl findings-check FILE
```

`FILE` is a JSON array of finding records. The command prints `VALID: N finding record(s)` and exits 0, or
`INVALID:` followed by the reason, and exits 1. It validates format only. It runs no review and no verification, and it counts
nothing.

Each record has exactly these fields, and an unknown field is refused:

| Field | Rule |
|---|---|
| `finding_id` | nonempty string, unique within the file |
| `source` | nonempty string, for example the trial and review round |
| `summary` | nonempty string |
| `verification` | object with `status`, `method` and `evidence` (below) |
| `material` | boolean |
| `discovered_at` | ISO 8601 timestamp with a timezone |
| `already_known` | boolean |
| `disposition` | `fixed`, `accepted`, `rejected`, `deferred` or `open` |
| `false_positive` | boolean |
| `duplicate_of` | `null`, or the `finding_id` of another finding in the same file. Not its own ID. |

`verification.status` is `reproduced`, `not_reproduced` or `unverified`. `verification.method` is `test` or
`grader` for the first two, and `null` for `unverified`. `verification.evidence` is a nonempty string.

Example: `F-2` is a duplicate of `F-1`. This file is valid; it was checked while writing this page.

```json
[
  {
    "finding_id": "F-1",
    "source": "trial T1-native-1 review round 1",
    "summary": "empty input returns None instead of raising ValueError",
    "verification": {"status": "reproduced", "method": "test", "evidence": "hidden test test_empty_input fails"},
    "material": true,
    "discovered_at": "2026-10-19T12:00:00+00:00",
    "already_known": false,
    "disposition": "fixed",
    "false_positive": false,
    "duplicate_of": null
  },
  {
    "finding_id": "F-2",
    "source": "trial T1-native-1 review round 1",
    "summary": "same empty-input defect, reported again",
    "verification": {"status": "unverified", "method": null, "evidence": "not run"},
    "material": false,
    "discovered_at": "2026-10-19T12:05:00+00:00",
    "already_known": true,
    "disposition": "rejected",
    "false_positive": false,
    "duplicate_of": "F-1"
  }
]
```

The validator checks that `duplicate_of` names a finding in the same file. It does not check that the duplicate
is right.

## 14. Command index

| Command | What it does | Runs a trial or a model? |
|---|---|---|
| `evalctl compare SUMMARY... --out DIR [--calibration RECEIPT]` | Paired report over single-trial summaries | No |
| `evalctl grader-calibrate CASE --grader-model M --driver-model M --case-check-receipt F --near-miss-workspace D --near-miss-requirement R --output F [--owner-labels F]` | Grader controls and owner-label receipt | Calls the grader model when run for real |
| `evalctl stage-ledger init LEDGER --stage-dollars D --stage-hours H` | Creates a stage ledger | No |
| `evalctl stage-ledger admit LEDGER --command TEXT --class T1\|T2\|T3 [--reason R]` | Admits a command and prints its handed cap | No |
| `evalctl stage-ledger record LEDGER ID --outcome O --hours H [--dollars D]` | Records an admitted command | No |
| `evalctl stage-ledger status LEDGER` | Prints spent, reserved, remaining and overshoot | No |
| `evalctl findings-check FILE` | Validates a finding ledger | No |
| `python -m scevals.phases RECORD` (run with `uv run --project evals python -m`) | Splits a Pi session record by phase and role | No |
| `docker build -f evals/software-change/image/Dockerfile.arms ...` | Builds the arms image | Network for npm and git; no model |

The arm launchers (`arm-native`, `arm-superpowers`, `arm-openspec`) run inside the trial container, through the
bridge. There is no separate host command for them.

## 15. What scripted proofs do not show

- **Mechanics only.** The tests use scripted or fake backends. They show that the code paths start, score, record
  and refuse as described. They do not show that any arm does useful work.
- **No paid trial.** No paid model call, smoke trial or Stage 1 trial has been run. Real S and O behaviour under
  Pi print mode is unverified. The arms image pins Pi 1.2.0, and the frozen pilot used 1.0.0.
- **Grader quality is unmeasured.** The calibration result depends on owner labels. This repository has none. A
  `normal` receipt from scripted data says nothing about the real grader.
- **Receipt mode is applied by `compare` only.** Campaign reports show raw `outcome_grade` values. Only
  `evalctl compare --calibration` labels them as diagnostic-only.
- **Session records are in the raw capture only.** Nothing extracts them from `workspace.tar` for you yet; extract
  `trial/pi-sessions` and point `scevals.phases` at it.
- **Trajectories are not reported.** Replay needs a Python call, and campaign reports do not show the trajectory.
- **Workflow adherence is null for every arm.** The `loop-engine` arm's `needed_role_aliases` is the nearest
  diagnostic.
- **Visible-test receipt is off by default.** The claim call sees hidden-test statuses, and a visible-test status only
  when `visible_test_command` is set. Its status is not stored as its own metric; only the grader sees it.
- **Image is not validated against the arm.** An arm without its launcher in the image fails at launch and is scored
  as an ordinary failure. Check the image before the run.
- **Owner words are proxies.** Counts and words are simulator-side proxies for human attention, not minutes.
- **Stage ledger reading A versus B** (section 12) is still an owner decision.

## Tests that cover this page

- `tests/test_arms_descriptor.py`: arm validation, prompt rendering, the golden default preamble.
- `tests/test_scoring_common.py`: the completion and success rule.
- `tests/test_target_seams.py`: option-gated target behaviour, the margin, the baseline, snapshots, the grade option,
  the grade detail, the visible-test receipt and the block label.
- `tests/test_grader.py`: grader calls, pointers, unstable verdicts, redaction, the family refusal, the controls and
  the owner-label rule.
- `tests/test_snapshots.py`: snapshot gating, index and replay.
- `tests/test_pi_session_and_phases.py`: the session hook, the golden unset command line and phase attribution.
- `tests/test_compare.py`: the paired report, the calibration receipt labels and the paired deltas.
- `tests/test_ledger.py`: admission, refusal, handed cap and overshoot.
- `tests/test_findings.py`: the finding schema.
- `tests/test_arm_commands.py`: each subcommand's help and a real run over fixtures.
