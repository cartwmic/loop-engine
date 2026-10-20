# Whole-workflow evaluations

## Overview

This local, uv-managed Python harness uses Inspect AI to compare whole unattended
software-change runs, not isolated chat answers. Each trial starts with a vague
request in a fresh native arm64 Docker container. A pinned host-side simulated
owner answers questions; hidden tests score the delivered code after the run
reaches a terminal state. The owner chooses models, approves private cases and
paid calls, and decides which reports to publish. Small samples are a gut check,
not a statistically rigorous ranking.

Use it when one owner wants comparable signal about how models handle the whole
software-change workflow on their own private cases. Do not use it when you need:

- A public benchmark, leaderboard or statistically significant ranking. Defaults
  are 3 cases x 3 trials, and the first pilot ran 3 cases x 1 trial.
- Cheap or fast runs by default. One full real trial can take one to three hours and
  millions of workflow tokens. Budgeted small campaigns exist for that reason:
  [software-change/TIERS.md](software-change/TIERS.md) defines Tier 0, a 30-minute, 5-dollar
  cost-and-time floor, and the single-step build-up from it.
- Another host setup. It needs native arm64 Docker, uv, host Pi/Node for `pi/`
  models, and an image built from a pinned local Loop Engine ref (default `v0.26.0`).
- A trustworthy intent-quality verdict. The first owner-labelled calibration of the
  intent judge agreed with the owner on 4 of 10 labelled intents (0 of 2 deficient
  intents passed). Treat intent-judge scores as low-confidence signal.
- Isolation against a hostile model. The threat boundary is a single trusted owner.
- A reusable library. The core/target split is internal to this harness.

## General core and software-change target

- [core/](core/README.md): campaign settings, matched case/trial assignment, Inspect
  execution, failure accounting, success rate, fixed-k pass^k, workflow token and
  estimated dollar cost, wall time, infrastructure-failure rate, and report format.
  Its small target interface is internal to this harness, not an extracted library.
- [software-change/](software-change/README.md): case format, mount-free image,
  Pi driver/worker commands, host simulated owner, hidden-test scoring, intent judge,
  calibration and software-change-specific health, discipline and honesty metrics.
- [software-change/ARMS.md](software-change/ARMS.md): opt-in Stage 0 arm comparison (native,
  Superpowers, OpenSpec and Loop Engine arms under one completion rule), the independent grader,
  snapshots, paired `compare` reports, the stage ledger and the finding check. Nothing changes
  unless those options or commands are used.

Every compared model/role combination receives the same case IDs, versions and
trial counts; exploration and task decomposition within a trial are unconstrained.
Success requires a final (not terminated) workflow and every expected hidden test passing.
Terminated workflows still receive hidden-test delivery metrics and remain failures.
Resumed trials are reported separately and excluded from headline success rate/pass^k.
Infrastructure failures are excluded from success rate and pass^k and counted
separately. Timeouts, hangs and model errors are failures. k stays the configured
trials per case; cases with fewer than k scorable trials are excluded from pass^k
and listed as incomplete. Rates show their numerators and denominators.
Simulator/judge usage is evaluation overhead, separate from workflow cost; unpriced
dollar estimates are null, not zero. Judge agreement/false-pass counts accompany
intent scores when owner-confirmed calibration is available, otherwise pending.

## Setup

Run from the checkout root with uv, Python 3.12+ and native arm64 Docker available.
Only the project-local Python environment is needed:

```sh
uv sync --project evals --locked
uv run --project evals evalctl --help
uv run --project evals evalctl image-check
uv run --project evals evalctl case-check evals/software-change/example-case
```

`image-check` builds from a pinned local Git ref (default `v0.26.0`), records the
resolved commit and executable versions, and checks an empty mount list. Building
requires network access for image-layer dependencies, but no model credentials.
Use `--ref REF --tag TAG` for another pinned image. `case-check` verifies and reuses the existing pinned arm64 image; it never builds.
If the image is missing/mismatched, run `image-check` first (only when image changes
are authorized). Every reference test must pass and at least one no-op test must
fail; an invalid case exits nonzero. Private cases stay in a separate repository, never under this checkout.

## Usage

### Real campaigns (owner-gated)

Copy `software-change/campaigns/real-example.toml` to a local ignored capture
location and obtain owner approval before launching. The shipped example uses
`codex/gpt-6.1-sol` for workflow roles and `openrouter/openai/gpt-6-luna` for
simulator/judge; the latter still requires separate paid-call approval and an
`OPENROUTER_API_KEY` in the host environment (create a key at
<https://openrouter.ai/keys>; model IDs are listed at <https://openrouter.ai/models>):

```sh
uv run --project evals evalctl campaign --settings LOCAL_SETTINGS.toml
```

Any Inspect model ID can fill any role (bring your own provider). The
[Inspect model providers guide](https://inspect.aisi.org.uk/providers.html) lists
each provider's `provider/model` ID form and the host environment variable it
reads for credentials. Set that variable on the host and pin the exact model ID
in settings. Models used here must support tool calling (workflow roles) and plain
text completion (simulator/judge). Read model credentials on the
host at runtime; never put keys or resolved provider configuration into campaign
settings, prompts or reports. The registered
Inspect `codex` provider reads `~/.pi/agent/auth.json` (`openai-codex`) at model
creation and uses the owner's ChatGPT/Codex subscription, never container auth.
Expired/missing/invalid credentials fail with instructions to run Pi once; there
is no OAuth refresh implementation. Requests use the fixed Codex Responses endpoint,
`stream=true`, `store=false`, system instructions and encrypted reasoning replay.
Completed streamed output items fill an empty terminal response, preserving text,
parsed tool calls and cached-token usage. Inspect's extra organization probe is
disabled. Batch/background are unsupported, and the backend's unsupported
`max_output_tokens` field is omitted; use campaign time limits to bound execution.
No paid model is invented and no real campaign is launched by tests. Offline
provider proofs use scripted SSE through the real SDK, including a public Inspect
bridge/real-Pi bash tool loop:

```sh
uv run --project evals pytest -q evals/core/tests/test_codex.py evals/software-change/tests/test_image_bridge.py
```

The host-only `pi/<pi-provider>/<model>` Inspect provider forwards one completion
(messages, tools and usage including cache read/write) to a long-lived Node helper.
It loads the globally installed `@earendil-works/pi-coding-agent` SDK beside host
Node, the normal Pi agent directory (including `PI_CODING_AGENT_DIR`), extensions,
`models.json` and Pi-managed OAuth. The helper uses the public SDK session factory
only to load/register providers, then calls `ModelRuntime.completeSimple`: no
agent loop, lifecycle startup, tool execution or container credentials. Signed
native assistant blocks are retained for conversation replay. Error completions use
Pi's installed `isRetryableAssistantError` classifier; transient failures are retried
inside the provider for up to six attempts (2/4/8/16/32-second exponential backoff,
capped at 60 seconds). Non-retryable failures remain model errors; exhausted
transients are infrastructure errors. Raw error messages stay only in ignored host
`software-change/logs/pi-helper-errors.log` (timestamp/model, mode 0600), with bearer
tokens, `sk-*` keys and JWT-looking strings redacted. They never enter completion
frames, results or containers. Cancellation, including backoff, kills/reaps the helper. This reuses claude-compat's own
request transport/envelope rather than reconstructing it in Python. Forced tool
choices are supported for Anthropic/claude-compat and OpenAI APIs; unsupported
APIs fail rather than silently downgrade. Host Node/Pi and the owner's trusted
provider extensions must already be installed; the harness installs none.

Campaigns may pin thinking separately for each model, across **all** roles:

```toml
[model_config."codex/gpt-6.1-sol"]
reasoning_effort = "high"
[model_config."pi/claude-compat/claude-opus-5-5"]
reasoning_effort = "medium"
```

These become Inspect `GenerateConfig` at model creation (Codex Responses effort,
Pi thinking level) and appear in recorded report settings. Unlisted models keep
Inspect/provider defaults; unknown model IDs/settings are rejected. The approved
`software-change/campaigns/pilot.toml` pins 3 private cases x 1 trial, both workflow
models, Codex high simulator/judge, and default limits. It is settings only:
**do not run the pilot without owner approval**. Offline proofs:

```sh
uv run --project evals pytest -q evals/core/tests/test_pi.py evals/core/tests/test_pilot.py evals/software-change/tests/test_pi_bridge.py
```

Settings require `cases_path`, `simulator_model`, `judge_model`, and either
`models = ["provider/model", ...]` (each model in every workflow role) or explicit
combinations, for example:

```toml
[[combinations]]
id = "chosen-roles"
driver = "provider/driver-model"
implementer = "provider/implementation-model"
reviewer = "provider/review-model"
```

Default small is 3 cases x 3 trials per model/combination; medium is 6 x 3. The
public single-case example explicitly selects 1 x 1; a small campaign against only
that case is rejected rather than silently reducing counts. `--preset medium`,
`--case-count N`, `--trials-per-case K`, `--profile CONFIG_VERSION`, repeated
`--model ID`, or repeated `--combination JSON` override settings. Default profile
is shipped `standard-16` at `v0.26.0`; other shipped profiles (`minimal-16`, `high-rigor-16`) require exact
config-version IDs. Defaults are `idle_limit_seconds = 1800` and `hard_limit_seconds = 28800`.
The hard limit includes scoring; software-change stops idle drivers on unchanged
engine history/sequence and role token usage. The former `time_limit_seconds`
setting is replaced (update local TOMLs).
All settings and versions are recorded. Paths resolve relative to invocation cwd.
See the core README for price tables and for campaign budgets, stops and the `limit`
outcome (`campaign_wall_limit_seconds`, `campaign_budget_usd`); see the target README for
target options. Trials run one at a time.

### Calibration

```sh
uv run --project evals evalctl calibrate extract --database "$HOME/.local/share/loop-engine/loop.db" --sessions "$HOME/.pi/agent/sessions"
uv run --project evals pytest -q evals/software-change/tests/test_calibration.py
uv run --project evals evalctl calibrate score --labels evals/software-change/calibration/labels.json
```

Extraction is read-only; the owner must confirm good/deficient labels and pin the
judge before scoring. Do not label on the owner's behalf. The default private
labels file and receipts are ignored. Pin the same judge model in calibration and
campaigns. See [calibration](software-change/calibration/README.md) for the gate,
counts and receipt integration.

### Results, captures and secrecy

A normal campaign writes exactly `report.md` and `summary.json` under
`software-change/results/<campaign>/<UTC timestamp>/`, for example
`20261005T024904.964511Z`. Each rerun gets a new directory; reports are never
replaced. `ac5-demo` groups its three report pairs under
`software-change/results/ac5-demo/<UTC timestamp>/<driver>/`.
The command prints the actual destination. **results/ is not gitignored**: the
owner selects only report/summary pairs to commit, never all runtime evidence.
Old report layouts are still scanned by the recursive audit; no old results are
migrated or deleted. Raw Inspect logs and captures remain under ignored `logs/` and `captures/`,
including private histories/artifacts, owner answers and proof receipts. **Never
publish captures.** Each attempt has its own private run directory:

```text
captures/<campaign>/<UTC run timestamp>/run.json
captures/<campaign>/<UTC run timestamp>/<sample_id>/workspace.tar
captures/<campaign>/<UTC run timestamp>/<sample_id>/meta.json
captures/<campaign>/<UTC run timestamp>/records.json
```

Immediately after driver stop (exit, crash, idle or hard stop), before host parsing
or scoring, `workspace.tar` captures all of `/workspace`, including trial SQLite
(and WAL/SHM when present), artifacts, raw show/history receipts, local-push-log,
local-remote.git and public/repo **including .git**. Cache-tagged directories (`CACHEDIR.TAG`, such as Cargo's build
`target/`) and directories named `.cache` are excluded; source directories merely
named `target` are retained. Sandbox reads and host writes are chunked; an interrupted
copy retains `workspace.tar.partial`. `meta.json` contains the settings snapshot,
matched assignment, role models, driver timings/exit/stop kind, harness commit,
private host-only owner receipt/answers, Inspect role usage and continuation
segments. Raw receipts are generated without interpreting them; no hidden tests
or reference files have entered this workspace. Run directories are mode 0700.
Completed trial scores are checkpointed host-side before starting later trials.

Captures are never subjected to publication scans. All scoring restores the tar
into a **fresh mount-free pinned trial-image container**, observes the restored DB,
and copies hidden tests/reference only for final or terminated lifecycles. The
campaign and rescore commands use the same scorer. Even a refused report retains
the capture and prints its rescore command; use `--settings` with safe reportable
settings to remove irrelevant case/config-looking settings content if necessary.
A lost Docker daemon/container or host disk failure can prevent a complete copy;
a partial copy is retained when possible, not claimed complete.

### Rescore and continue (scripted proof examples)

```sh
uv run --project evals evalctl rescore evals/software-change/captures/CAMPAIGN/RUN_TIMESTAMP
uv run --project evals evalctl rescore evals/software-change/captures/CAMPAIGN/RUN_TIMESTAMP --settings SETTINGS.toml
uv run --project evals evalctl campaign --settings SETTINGS.toml --resume evals/software-change/results/CAMPAIGN/RUN_TIMESTAMP
uv run --project evals evalctl campaign --settings SETTINGS.toml --resume evals/software-change/captures/CAMPAIGN/RUN_TIMESTAMP --resume-interrupted
uv run --project evals evalctl resume-trial evals/software-change/captures/CAMPAIGN/RUN_TIMESTAMP/SAMPLE_ID
uv run --project evals evalctl merge-captures evals/software-change/captures/CAMPAIGN/MERGED SOURCE_CAPTURE_RUN_1 SOURCE_CAPTURE_RUN_2 --settings SETTINGS.toml
uv run --project evals evalctl campaign --settings SETTINGS.toml --resume evals/software-change/captures/CAMPAIGN/MERGED
```

Each writes a fresh timestamped report pair, never replaces a prior report.
Rescore re-evaluates every captured trial with current harness code. Original
workflow/simulator usage comes from host `meta.json` (snapshotted Inspect
`sample.role_usage`), not model-controlled receipts or reconstructed estimates;
current judge usage replaces prior judge overhead. This implementation does not
need to read `.eval` logs for accounting. External snapshots without metadata have
unknown original workflow/simulator tokens and dollars (null), not fabricated zero.
Without host owner receipts, elicitation recall and owner-question/answer counts
are also null; unattended driver completion is unknown without an exit receipt.

For an external bare tar of `/workspace` (members relative to `/workspace`,
including `trial/`, `public/repo/.git` and the local remote), the exact command is:

```sh
uv run --project evals evalctl rescore /ABSOLUTE/workspace.tar --settings /ABSOLUTE/SETTINGS.toml --sample external-trial --case /ABSOLUTE/CASE --model sc-scripted/driver
```

`--case` is a case directory or a case ID from settings. `--model` identifies the
configured driver combination; without `--settings` it is used for all roles
(including simulator/judge) and the case path must be a single case. Only use
scripted models for offline proofs. `--run-id` defaults to `eval-trial`; select the
actual run ID for older/external snapshots. Replaying requires the pinned image,
case ID/version and host case files; it does not turn a tar into a standalone case.

`merge-captures OUT_DIR SRC_DIR... --settings SETTINGS.toml` creates a new private
capture run from disjoint scored capture runs (including those produced by
`rescore`). It copies workspace snapshots and host metadata and preserves each
source's scored record/usage keyed by `sample_id`. It refuses duplicates, unknown
or mismatched assignments, missing snapshots, existing output and differing
effective settings. Only the campaign label is exempt from settings comparison;
recorded timestamps/provenance are not settings. All scoring/execution/model,
price and target options must match. The target settings determine the full matched
assignment even when each source scored only a subset. Subsequent campaign resume
carries clean trials and runs only missing/retryable ones, with one combined report.
No models or Docker execution occur during merging. Outputs and inputs are private;
never place the merged directory in published results.

Campaign `--resume` requires **identical effective settings and matched assignment**.
It carries clean successes and failures attributed to the model, re-running only
missing trials, infrastructure failures and unclassified harness errors. Reports
list carried sample IDs and their source run. `--resume-interrupted` continues
eligible captured active trials instead of re-running them; other retryable
trials start fresh. `resume-trial` accepts only non-terminal infrastructure/harness
interruptions, restores the repo/DB/artifacts and prior owner answers, and launches
a fresh driver instructed to continue `eval-trial` from durable records, not start
a new run. Each segment records time, role usage and stop reason. Resumed trials
measure a new driver taking over mid-run, not one uninterrupted run: they have a
separate report section/rate and are excluded from headline success rate and
fixed-k pass^k; mixed cases may therefore be incomplete. Scorable original trials
carried unchanged remain eligible. Terminal scoring failures should be rescored,
not resumed as a driver. Replay preserves ordinary symlinks (including absolute
and `..` targets) inside the disposable mount-free container; it rejects unsafe
archive member paths/devices, workspace-escaping hardlinks and any member that
would be extracted through an archived symlink.

Offline public-command proofs (real Pi/engine/Docker, no paid calls):

```sh
uv run --project evals pytest -q evals/software-change/tests/test_lifecycle.py
```

They check same-capture campaign/rescore/repeat parity, fault recovery, provider
markers, terminated delivery scoring, matched continuation, restored history and
owner answers, separate resumed accounting and report-refusal recovery. Equality
normalizes only execution identity, timing and provenance, never scored values.

Before publication, scan all results against every public and private case used:

```sh
uv run --project evals evalctl check-results evals/software-change/results --cases evals/software-change/example-case
uv run --project evals evalctl check-results evals/software-change/results --cases /absolute/private-case-repository
```

`--cases` accepts one case or a repository with immediate case subdirectories.
The audit reads every result file recursively, including binary files, and searches
whole hidden files and every hidden-text line of at least 24 characters that
contains a letter/digit. Shorter standalone fragments are not detected. It scans raw/JSON-escaped strings
and decoded JSON layers, runtime credential values and provider configuration
markers. A match exits 1 without printing matching content; missing/empty/invalid
inputs exit 2, never a vacuous pass. Conservative common-line matches are possible.
This is a content-string check, not detection of paraphrases or arbitrary encodings.

Codex and Pi campaign settings automatically add their host Pi auth file to provider
configuration protection when **any** workflow/simulator/judge role uses `codex/`
or `pi/` respectively (Pi honors its normal agent-directory override).
Standalone `check-results` also protects these files whenever present, without
printing their values. Environment credential values are protected at any length.
From provider configs, the whole file plus non-comment lines and parsed string
values of at least 16 characters are protected. Shorter config-only values and
comment lines are not; do not rely on this guard to catch short secrets.

Supply every other resolved host provider config via repeated `--provider-config FILE`
(to `campaign` and `check-results`), `provider_config_paths` campaign settings, or
provider-config-named environment variables (path or inline content). The audit
cannot discover arbitrary config files or credentials absent from its environment.
Run it with the credentials used by the campaign still present. The report writer
also refuses credential/config matches before writing output. No host credentials
enter trial containers; a dummy bridge key is not a model credential. Hidden
briefs remain host-side; tests/reference enter only after terminal observation.

## Validation

These public real-Pi/Docker proofs use scripted Inspect backends, with no live or
paid model calls (run `image-check` first):

```sh
uv run --project evals pytest -q evals
uv run --project evals evalctl campaign --settings evals/software-change/campaigns/scripted-example.toml
uv run --project evals evalctl campaign --settings evals/software-change/campaigns/role-routing.toml
uv run --project evals evalctl ac5-demo
```

The scripts prove lifecycle, role routing and metrics, not real-model semantic
quality. `ac5-demo` distinguishes never asking, asking/building, and asking without
building: elicitation and delivery recall change separately. The repository's
own Rust proof route is in checkout-root `AGENTS.md`; these Python proofs do not
replace it.

## Troubleshooting

- **`case-check` says the image is missing or mismatched.** It never builds. Run
  `uv run --project evals evalctl image-check` (needs build-network access).
- **A `codex/` model fails with missing or expired credentials.** The harness does
  not refresh OAuth. Run Pi once on the host to refresh `~/.pi/agent/auth.json`.
- **A `pi/` model fails repeatedly.** Read the redacted host log
  `software-change/logs/pi-helper-errors.log`. Provider policy blocks and rejected
  bridged requests are classified as infrastructure, not model failures.
- **The report is refused and no report is written.** The pre-write scan found a
  credential or provider-config match. The private capture is kept and the command
  prints a `rescore` command; rescore with safe `--settings`.
- **A campaign fails with only `Command failed (SomeError)`.** The CLI hides
  exception text because it can contain keys or private case text. A common first-run
  cause is a missing provider credential, such as `OPENROUTER_API_KEY` for the
  real example's simulator/judge. Set it on the host and rerun.
- **`check-results` exits 1 or 2.** Exit 1 means protected content matched (content
  is not printed). Exit 2 means missing, empty or invalid inputs.
- **A trial stops as idle or hard timeout.** Adjust `idle_limit_seconds` or
  `hard_limit_seconds` in settings; recover with `rescore` or `resume-trial`
  (resumed trials are reported separately).
