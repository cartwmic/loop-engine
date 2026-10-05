# Whole-workflow evaluations (draft)

This local, uv-managed Python harness uses Inspect AI to compare whole unattended
software-change runs, not isolated chat answers. Each trial starts with a vague
request in a fresh native arm64 Docker container. A pinned host-side simulated
owner answers questions; hidden tests score the delivered code after the run
reaches a terminal state. The owner chooses models, approves private cases and
paid calls, and decides which reports to publish. Small samples are a gut check,
not a statistically rigorous ranking.

## General core and software-change target

- [core/](core/README.md): campaign settings, matched case/trial assignment, Inspect
  execution, failure accounting, success rate, fixed-k pass^k, workflow token and
  estimated dollar cost, wall time, infrastructure-failure rate, and report format.
  Its small target interface is internal to this harness, not an extracted library.
- [software-change/](software-change/README.md): case format, mount-free image,
  Pi driver/worker commands, host simulated owner, hidden-test scoring, intent judge,
  calibration and software-change-specific health, discipline and honesty metrics.

Every compared model/role combination receives the same case IDs, versions and
trial counts; exploration and task decomposition within a trial are unconstrained.
Success requires both a terminal workflow and every expected hidden test passing.
Infrastructure failures are excluded from success rate and pass^k and counted
separately. Timeouts, hangs and model errors are failures. k stays the configured
trials per case; cases with fewer than k scorable trials are excluded from pass^k
and listed as incomplete. Rates show their numerators and denominators.
Simulator/judge usage is evaluation overhead, separate from workflow cost; unpriced
dollar estimates are null, not zero. Judge agreement/false-pass counts accompany
intent scores when owner-confirmed calibration is available, otherwise pending.

## Setup and focused commands

Run from the checkout root with uv, Python 3.12+ and native arm64 Docker available.
Only the project-local Python environment is needed:

```sh
uv sync --project evals --locked
uv run --project evals evalctl --help
uv run --project evals evalctl image-check
uv run --project evals evalctl case-check evals/software-change/example-case
```

`image-check` builds from a pinned local Git ref (default `v0.24.0`), records the
resolved commit and executable versions, and checks an empty mount list. Building
requires network access for image-layer dependencies, but no model credentials.
Use `--ref REF --tag TAG` for another pinned image. `case-check` requires every
reference test to pass and at least one no-op test to fail; an invalid case exits
nonzero. Private cases stay in a separate repository, never under this checkout.

These public real-Pi/Docker proofs use scripted Inspect backends, with no live or
paid model calls (build the image first):

```sh
uv run --project evals evalctl campaign --settings evals/software-change/campaigns/scripted-example.toml
uv run --project evals evalctl campaign --settings evals/software-change/campaigns/role-routing.toml
uv run --project evals evalctl ac5-demo
uv run --project evals pytest -q evals
```

The scripts prove lifecycle, role routing and metrics, not real-model semantic
quality. `ac5-demo` distinguishes never asking, asking/building, and asking without
building: elicitation and delivery recall change separately.

## Real campaigns (owner-gated)

Copy `software-change/campaigns/real-example.toml` to a local ignored capture
location and obtain owner approval before launching. The shipped example uses
`codex/gpt-6.1-sol` for workflow roles and `openrouter/openai/gpt-6-luna` for
simulator/judge; the latter still requires separate paid-call approval:

```sh
uv run --project evals evalctl campaign --settings LOCAL_SETTINGS.toml
```

Read model credentials on the host at runtime; never put keys or resolved
provider configuration into campaign settings, prompts or reports. The registered
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
is shipped `standard-14` at `v0.24.0`; other shipped profiles require exact
config-version IDs. Default per-trial limit is 3600 seconds, including scoring.
All settings and versions are recorded. Paths resolve relative to invocation cwd.
See the core README for price tables and the target README for target options.

## Calibration

```sh
uv run --project evals evalctl calibrate extract --database "$HOME/.local/share/loop-engine/loop.db"
uv run --project evals pytest -q evals/software-change/tests/test_calibration.py
uv run --project evals evalctl calibrate score --labels evals/software-change/calibration/labels.json
```

Extraction is read-only; the owner must confirm good/deficient labels and pin the
judge before scoring. Do not label on the owner's behalf. The default private
labels file and receipts are ignored. Pin the same judge model in calibration and
campaigns. See [calibration](software-change/calibration/README.md) for the gate,
counts and receipt integration.

## Results, captures and secrecy

A normal campaign writes exactly `report.md` and `summary.json` under
`software-change/results/<campaign>/<UTC timestamp>/`, for example
`20261005T024904.964511Z`. Each rerun gets a new directory; reports are never
replaced. `ac5-demo` groups its three report pairs under
`software-change/results/ac5-demo/<UTC timestamp>/<driver>/`.
The command prints the actual destination. **results/ is not gitignored**: the
owner selects only report/summary pairs to commit, never all runtime evidence.
Old report layouts are still scanned by the recursive audit; no old results are
migrated or deleted. Raw Inspect logs and captures remain under ignored `logs/`
and `captures/`, including private histories/artifacts and local proof receipts.
Normal campaign capture paths are keyed by campaign/sample, so use a distinct
campaign name when retaining raw evidence across reruns; timestamped reports alone
do not version those captures.

Before publication, scan all results against every public and private case used:

```sh
uv run --project evals evalctl check-results evals/software-change/results --cases evals/software-change/example-case
uv run --project evals evalctl check-results evals/software-change/results --cases /absolute/private-case-repository
```

`--cases` accepts one case or a repository with immediate case subdirectories.
The audit reads every result file recursively, including binary files, and searches
whole hidden files and every nonblank hidden-text line containing a letter/digit;
blank/punctuation-only lines are not evidence. It scans raw/JSON-escaped strings
and decoded JSON layers, runtime credential values and provider configuration
markers. A match exits 1 without printing matching content; missing/empty/invalid
inputs exit 2, never a vacuous pass. Conservative common-line matches are possible.
This is a content-string check, not detection of paraphrases or arbitrary encodings.

Codex campaign settings automatically add the host Pi auth file to provider
configuration protection when **any** workflow/simulator/judge role uses `codex/`.
Standalone `check-results` also protects that file whenever present, without
printing its values. Whole configs, meaningful (non-punctuation-only) lines and
parsed string values are protected; formatting braces alone are not secrets.

Supply every other resolved host provider config via repeated `--provider-config FILE`
(to `campaign` and `check-results`), `provider_config_paths` campaign settings, or
provider-config-named environment variables (path or inline content). The audit
cannot discover arbitrary config files or credentials absent from its environment.
Run it with the credentials used by the campaign still present. The report writer
also refuses credential/config matches before writing output. No host credentials
enter trial containers; a dummy bridge key is not a model credential. Hidden
briefs remain host-side; tests/reference enter only after terminal observation.

The owner's final repository proof route and README policy-document drafting/audit
are separate driver-owned gates; focused Python proofs do not replace them.
