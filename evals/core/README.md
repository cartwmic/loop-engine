# Evaluation core

## Overview

This is the general part of the local Inspect AI harness, not a separate library.
It owns settings, matched assignment, Inspect sample execution, outcome accounting,
core metrics, and the `report.md` / `summary.json` pair. Case files, container setup,
simulated-owner behaviour, hidden-test scoring and intent calibration belong to the
software-change target, not to this directory. The fake target is test-only and
uses no containers, Pi, credentials or network model calls; it does not prove the
software-change adapter or trial isolation.

Use this directory when adding or changing a target inside this harness. It is
not a published or versioned library: the interface may change with the harness,
and software-change is the only real target. Campaign operation, real-model setup
and result publication are in checkout-root `evals/README.md`.

## Setup

From the checkout root, install the project-local environment (Python 3.12+, uv):

```sh
uv sync --project evals --locked
```

## Target contract

Implement `evalcore.target.Target` and export a zero-argument factory, addressed as
`module:factory` in `target` settings or `--target`. The factory must not launch
trials. A target supplies:

- `name`, `results_dir`, `logs_dir` (logs/captures must stay local and gitignored).
- `load_cases(path) -> list[Case]`: stable IDs, versions and host-side paths. Core
  sorts IDs, selects the configured count, and rejects shortages or duplicate IDs.
- `prepare_sample(TrialContext) -> inspect_ai.dataset.Sample`: input and sandbox
  definition. Core assigns a unique sample ID; each software-change sample must
  have its own mount-free container. Preparation must not run the workflow.
- `async run_trial(TaskState, TrialContext) -> TrialExecution`: execute using the
  context's role combination, profile and hard limit. `data` is opaque host-side
  state and is never exported by core.
- `async score(TaskState, TrialExecution, TrialContext) -> TrialScore`: return a
  success boolean and export-safe metrics. Core success requires both terminal
  execution and scorer success. A software-change target must additionally require
  every hidden test to pass. Export only IDs, metrics and safe evidence, not private
  brief/test/reference text.
- `versions(settings) -> dict[str, str]`: actual pinned engine/provider/target
  identities. `report_metrics(list[dict]) -> dict`: per-combination target metrics,
  including judge calibration figures when available. Per-trial metrics also appear
  in both reports; core does not guess how to aggregate target-specific scores.
- Optional `finalize_metrics(metrics, *, usage_by_role, terminal, outcome, reason)`:
  attaches export-safe target metrics after Inspect finalizes role usage and failure
  classification. It runs host-side after sandbox teardown, must not run models,
  and cannot change the core outcome. Software-change uses it for alias-use flags
  and timeout accounting; unknown external observations remain null.

`TrialContext` carries settings, the assignment, case and role combination. Core
binds Inspect models under `role-driver`, `role-implementer`, `role-reviewer`,
`simulator` and `judge`. Use `get_model(role=...)` or the equivalent bridge aliases;
Inspect's per-sample `role_usage` is the accounting source. Model identity alone
cannot distinguish workflow cost from simulator/judge overhead when they share a
model. These roles follow the current workflow; no future target abstraction is
extracted. Unknown/unattributed model calls must not be made by targets.

Raise `InfrastructureError` for harness, container, bridge or provider-service
failures, and `ModelError` for model behaviour (e.g. bad schemas/refusals). Public
Inspect sandbox/refusal exceptions and provider connection/429/5xx errors are
also classified. Idle hangs and model errors are failures, never exclusions. A trial or step stopped by a
budget (the per-trial hard limit, campaign wall time, dollars or tokens) is the outcome `limit`
with its reason; see Budgets and stops.
Unknown exceptions in the target are failures unless explicitly declared
infrastructure. Errors outside the target solver are Inspect lifecycle
infrastructure; `EvalError` does not expose typed categories, so core never parses
model-controlled error text. Subprocesses must respect cancellation and use their
own timeouts. The core time limit covers execution plus scoring. Failed samples are not automatically retried. Explicit `campaign --resume`
requires identical settings/assignment, carries clean trials, and retries only
missing, infrastructure and unclassified harness failures. Target-owned
`rescore`/`resume-trial` commands recover private captures; resumed segments are
excluded from headline success rate/pass^k and reported separately.

## Budgets and stops

All optional; unset means off. One campaign command runs exactly one campaign and starts no other.
Trials run one at a time, so the guard sees one trial's live spend.

A stop that fires after a trial's work has finished (a dollar stop reached by the final judge call after scoring, a campaign wall-time stop during publication, or any other post-finish stop) leaves the finished trial as `success`. The stop is recorded in the stop events, the cut-off steps and the limit-only record label, and no campaign-level `limit` outcome is emitted. A stop that fires while a trial's work is still running ends the trial as `limit`, as described above.

- `campaign_wall_limit_seconds` (D): measured from the start of the `evalctl` command to the report
  being written. `post_trial_reserve_seconds` (default 600) gives the stop line S = D - reserve, where trial work
  stops. `publication_reserve_seconds` (default 120) gives H = D - reserve, where capture, scoring and the judge
  must be done. Each post-trial step gets its own limit bounded by the time left before H; a step that runs out is
  abandoned and recorded as `limit` with the step named (`limit_step`), and the work already on disk stays.
- `campaign_budget_usd` with `dollar_margin_usd` (default 0.25): the dollar stop line is the cap minus the
  margin. **The dollar cap is best effort.** Spend is read by polling (`guard_poll_seconds`, default 5), so a
  campaign can very rarely overshoot by about one model call. The report states the actual spend against the cap and
  an overshoot fails the cost verdict. A dollar budget is refused before any model call unless every model in use
  (the three workflow roles, simulator and judge) has a price. `campaign_token_budget` is an extra stop, never a
  substitute for a dollar budget.
- Prices may carry per-request `tiers` (a request whose total input exceeds a threshold is billed at that tier's rates, as
  published for Sol above 272k input tokens). Live and final dollars are summed over individual model calls, each priced by
  its own size; where per-call data is missing the whole aggregate is priced at the highest tier, an upper bound.
- The first stop that fires wins. Reasons: `trial_time` (per-trial `hard_limit_seconds`), `campaign_wall_time`,
  `campaign_dollars`, `campaign_tokens`. After any of these stops, including a per-trial time stop, no further trial starts (recorded `limit`, step
  `not_started`) and no new model call is admitted, so the judge is skipped. The guard looks once more when a trial's
  work returns and again after scoring (which may have made the final judge call, so a stop it reaches is recorded), so spend or time reached in its last seconds still stops the judge and later trials. A target that ignores a stop is cancelled
  after `stop_grace_seconds` (default 30). Nothing is retried.
- Synchronous setup and version probes have explicit timeouts bounded by the remaining time (summary preparation shares the
  time left before D minus a few seconds; the replay container's shielded removal is bounded by the time left before D minus 30 s, at most 60 s; capture metadata probes share the post-trial allowance; a removal that runs out of time is handed to a detached process and named as a `replay_cleanup` cut-off in the report). If one runs out, the
  campaign ends as `limit` with that step named and publishes only the limit-only record.
- Publication: a small limit-only record (outcomes, reasons, caps, no private text) is written first. A separate
  child process then renders, scans and stages the full report and is killed at D. The staged report replaces the
  record only if it finished in time; otherwise the record is the result, and it names the cut-off (`publication_cut_off`, a `publication` step in
  `cut_off_steps` and a `campaign_wall_time` stop event); the fallback carries the real command-to-written time and `publication.json` is written with `full_report: false`. The renderer is stopped one second before D so the written time stays inside it. `campaign_wall_seconds` is the command-start
  to report-written time, written just before the move, and `publication.json` records the interval.
- `tier` and `report_note` appear in the report's Budget section with the caps, actual spend and time, per-stop
  events, cut-off steps and the cost verdict (`pass`, `fail_over_cap`, `fail_unpriced`, `fail_unknown_usage`, or
  `not_budgeted`). Core metrics count `limit` trials as scorable non-successes and list them by reason.
- A target may set `TrialExecution.stop_reason` when its own driver was stopped by a budget; core still scores what was
  captured and records the trial as `limit`. Targets read the guard from `TrialContext.guard`
  (`allowance(own_limit)`, `model_calls_allowed()`, `stop_reason`).

## Settings and reports

Supply explicit Inspect model IDs and pinned simulator/judge IDs: core invents no
paid-model defaults. `models` compares each model in all three workflow roles;
`combinations` instead accepts `{id, driver, implementer, reviewer}` objects.
Use exactly one. Defaults are `preset = "small"` (3 cases x 3 trials),
`profile = "standard-16"`, engine/provider ref `v0.26.0`, and limits `idle_limit_seconds = 1800`, `hard_limit_seconds = 28800`.
Core execution/scoring use the hard limit, enforced by the trial solver (Inspect's own sample limit is a 10-second backstop behind it); idle detection belongs to
the target. CLI overrides are `--idle-limit-seconds` and `--hard-limit-seconds`.
The old `time_limit_seconds` setting is no longer accepted. `medium` is 6 x 3; `case_count` and `trials_per_case` override either count.
Targets may supply settings TOML files; paths resolve relative to the invocation
working directory. `target_options` holds target-owned options such as a scripted
driver command. All effective counts, explicit/default setting sources and role
mappings are recorded. `[model_config."provider/model"] reasoning_effort = "high"`
sets Inspect generation config for that model in every workflow/simulator/judge
role, separately from provider `model_args`. Thinking settings are recorded in
reports; omitted entries keep provider defaults. The initial per-model surface
is reasoning effort only; unknown settings and unused model IDs are rejected. No profile or task decomposition policy is enforced here.

Every compared combination gets identical case IDs, versions and trial counts.
Success rate excludes infrastructure trials; `pass_k` uses the configured trials
per case as fixed k. Cases with fewer than k scorable trials are excluded and
listed as incomplete. Each rate includes numerator/denominator (null for an empty
denominator). Failures, timeouts, infrastructure failures and terminal workflow
completion are separate. Costs are tokens and **estimated** dollars from
`prices.<model>.input_per_million` / `output_per_million`; absent prices produce
null dollars, not zero. Workflow roles and simulator/judge overhead are separate.
Trial wall time includes setup/scoring; campaign wall time is also recorded.

Results go to the target's `results_dir/<campaign>/<UTC timestamp>/`; reruns
create fresh report pairs and never overwrite earlier reports. `run_campaign` has
an optional `report_directory` for grouped demonstrations; that explicit directory
must not already exist. Software-change AC5 uses it for its three driver reports. Targets may implement `begin_run`, `checkpoint_record`, `persist_record`,
`failure_metrics` and `can_resume_capture` to retain host-private attempt evidence
and support explicit continuation. Software-change checkpoints score/role usage
before later samples, then updates with finalized Inspect usage. Its captures are
never scanned as report exports; report refusal prints a recovery command.
The summary includes schema version 1, settings, matched assignment,
pinned identities, harness/target/case versions, comparisons and every trial.
Markdown presents the same information. Raw Inspect logs remain outside the result
pair, under the target's gitignored `logs_dir`.

Before creating any result directory or file, both rendered outputs are scanned
for credential environment values (API keys, tokens, secrets/passwords, etc.) and
JSON-escaped forms, resolved provider configuration content and config markers.
On a match, nothing in the results directory is written. Core cannot discover
arbitrary machine-local configuration: operators/targets must supply all resolved
config files with `provider_config_paths` settings, repeated `--provider-config`,
or the target's optional `provider_config_paths` attribute. Environment variables
whose names identify provider config may contain a path or inline text. Configs
are read only on the host, never exported. Any `codex/` or `pi/` role automatically adds
its host Pi auth file to settings protection (`pi/` honors `PI_CODING_AGENT_DIR`). Standalone software-change audits
also protect it whenever present. Environment credential values are protected at
any length. From provider configs, the whole file plus non-comment lines and parsed
string values of at least 16 characters (`MIN_SECRET_LENGTH`) are protected.
Shorter config-only values and comment lines are not, so a short secret that exists
only in a config file can reach a report. Operators and targets must not rely on
this guard for such values. The scan is otherwise fail-closed: credential values
can also refuse an otherwise safe report.
Targets must separately prevent private case text in exports and protect raw logs;
this core credential guard does not establish AC-7 or container isolation.

## Usage

From the checkout root:

```sh
uv run --project evals evalctl campaign --settings PATH.toml
```

Targets register commands by exporting `register_subcommands(subparsers)` and
setting each parser's `handler` to a callable taking its argparse namespace (zero
or None means success). Load via `--plugin MODULE` before the subcommand,
`EVALCTL_PLUGINS` (comma-separated modules), or a Python entry point in group
`loop_engine_evals.commands` referencing the registration callable. A campaign's
explicit target module is automatically registered when it has that function.
This is the contract for target-owned `case-check`, `image-check`, `ac5-demo`,
`calibrate`, and `check-results`; core does not implement those commands. Targets
will need to add their package to the uv project's wheel package list and register
an entry point for discovery without `--plugin`. Plugin modules are trusted code.

## Validation

```sh
uv run --project evals pytest -q evals/core/tests
uv run --project evals pytest -q evals/core/tests/test_fake_campaign.py
```

The black-box tests invoke the actual `evalctl` process and Inspect `ModelAPI`
backend, retain reports and command output under gitignored test `captures/`, and
exercise small/medium/explicit counts, actual role routing, injected infrastructure,
model failure, actual timeout, and pre-write credential refusal. For a manual fake
campaign, put `evals/core/tests` on `PYTHONPATH`, set `EVAL_FAKE_ROOT` to a local
capture directory, use target `fake_target:create_target`, models `fake/a` and
`fake/b`, and pin simulator/judge to either fake model.

## Troubleshooting

- **Settings are rejected.** Unknown keys, unused `model_config` IDs, the removed
  `time_limit_seconds`, or both `models` and `combinations` fail validation. Fix the
  TOML; nothing is guessed.
- **A campaign refuses a shortage.** Fewer cases than `case_count`, or duplicate IDs,
  are rejected rather than silently reducing the matched assignment.
- **`--resume` refuses.** Effective settings or matched assignment differ from the
  source run. Use the original settings file.
- **No report is written.** The pre-write credential/config scan matched. Remove the
  matching value from settings, or supply the right `--provider-config` files, and
  rerun or rescore. Short common credential values can cause false refusals.
