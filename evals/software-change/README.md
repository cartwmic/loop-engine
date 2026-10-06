# Software-change evaluation target (draft)

This directory supplies the software-change-dependent parts of the Inspect
harness. Checkout-root `evals/README.md` describes general campaign commands,
accounting, result locations and secrecy checks; checkout-root
`evals/core/README.md` defines the internal target interface. This is not a new
engine provider, shared library or change to provider calibration.

## Eval-case format

A case is one problem given to a run, not one of the implementation tasks that
the model creates within that run:

```text
case.toml
public/request.md
public/repo/...
hidden/brief.md
hidden/tests/test_*.py
hidden/reference/...
```

```toml
id = "stable-case-id"
version = "1"
allowed_edit_scope = ["labels.py"]
test_timeout_seconds = 60  # optional; raise it for cases that build first (e.g. Rust)
```

- IDs are safe identifiers; versions are nonempty strings. Increment the version
  when case content changes. Allowed scope contains relative path patterns.
- `public/repo/` is the starting repository; `public/request.md` deliberately
  leaves requirements for the driver to elicit. Only public files enter before
  terminal state. Cases have no symlinks, absolute scope paths or `..` escapes.
- `hidden/brief.md` is the private owner specification, with unique `## REQ-*`
  headings. It stays host-side. The pinned simulator answers only what is asked,
  recording revealed requirement IDs rather than exporting brief/answer text.
- Hidden tests are synchronous Python `test_*` functions/methods, each tagged
  once with `@requirement("REQ-ID")` imported from `case_runtime`. Every brief
  requirement must have a test; unittest assertions are supported by the runner.
  IDs and statuses, not test source/failure messages, enter public results.
- `hidden/reference/` is a repository-relative overlay within allowed scope. It
  is an example solution, not a prescribed model decomposition. It must pass every
  tagged test. The untouched no-op repository must fail at least one test.

Private cases live in a separate owner-approved repository. A cases path is either
one case directory or a repository containing immediate case subdirectories.
Agents may draft cases; the owner approves them before use. The
[public example](example-case/case.toml) covers label keys with three requirements
and six tests; it is intentionally public, not a private pilot substitute.

From the checkout root:

```sh
uv run --project evals evalctl case-check evals/software-change/example-case
uv run --project evals evalctl case-check /absolute/private-case-repository/CASE
uv run --project evals evalctl case-check --expect-invalid evals/software-change/tests/fixtures/broken-case
```

Reference/no-op attempts use separate mount-free containers. Invalid references,
collection mismatches and skips reject the case. `--expect-invalid` succeeds only
for an invalid case, not an infrastructure error.

## Image and unattended execution

The [image](image/README.md) contains native arm64 Linux binaries built from the
campaign's pinned local Git ref, Dagu, Pi, the two shipped skills, and role/owner
commands. Run `uv run --project evals evalctl image-check` from the checkout root;
no host mounts or image volumes are permitted. An image's commit identity must
match the configured engine/provider refs; changing refs requires a matching image
(selected with `target_options.image`). Credentials stay host-side behind Inspect's
sandbox agent bridge, including `pi/claude-compat/claude-opus-5-5` via the installed
host Pi SDK and `codex/gpt-6.1-sol` via the Codex subscription. The approved
`campaigns/pilot.toml` sets per-model medium/high thinking respectively, with
Codex high simulator/judge; loading this file does not authorize launching it.

`scevals.target.SoftwareChangeTarget` implements the core interface. Every sample
gets its own container and records the actual mount/bind configuration and copy
log. The real driver uses `sc-driver` (Pi print mode); implementation/review use
`sc-implement` and `sc-review`. Inspect aliases `role-driver`, `role-implementer`
and `role-reviewer` map to the chosen combination, not model choices made by the
driver. `ask-owner` reaches host service `sc_owner.ask_owner(question=...)`; only
its answer, never the hidden brief, returns to the container. Simulator and judge
roles are host-side evaluation overhead.

`target_options.scripted_driver` accepts `never-asks`, `asks-and-builds` or
`asks-but-does-not-build`. The shipped scripted campaigns and `ac5-demo` use real
Pi, Docker, public engine CLI and no-network Inspect backends. They demonstrate
lifecycle/routing/metric behavior, not semantic reviewer quality. A trusted
`target_options.driver_command` argv array may replace `sc-driver`; the harness
appends the fixed prompt. Normal drivers start from the vague request, with no
prewritten intent and no human input after launch. No new engine/provider code
is required.

After terminal observation, scoring copies tests and reference into the sandbox;
it never applies reference to delivered code. Public show/history data, SQLite,
artifacts and the pre-test delivery snapshot are copied out as ignored captures.
Idle/hard stops and non-terminal driver exits retain DB (including any WAL),
artifacts, show/history and the delivery snapshot as `nonterminal.tar` in the same
ignored campaign/sample capture location. If no run was created yet, show/history
retain the CLI error and no nonexistent DB/artifacts are invented. Hidden tests
and reference are still never copied before terminal observation.
Private artifacts and raw Inspect logs must never be selected for publication.
The reports contain only metrics/IDs and safe evidence, not source or brief prose.

## Target-specific scores and limits

Defaults are idle 1800 seconds and hard 28800 seconds, set by
`idle_limit_seconds` / `hard_limit_seconds`. While the driver runs, the host polls
public engine show/history and Inspect sample role usage about every 60 seconds
(more frequently for small idle limits). Changed history/context sequence or an
increase in any role's total tokens resets the idle clock. A long progressing
trial is not idle. An idle stop kills the driver process group; hard stops retain
`DRIVER_LIMIT_MARGIN_SECONDS = 300` (minimum driver budget 60 seconds) for capture
and scoring. Driver timeout failures report `driver_timeout_kind` and
`run_health.timeout_kind` as `idle` or `hard`. Inspect's sample limit is the hard
limit. Abrupt infrastructure/cancellation failure outside the driver hook can
still prevent capture; unknown dimensions remain null.

See [scoring](scoring.md) for exact definitions:

- Hidden-test outcome and tagged delivery recall, independently from elicitation
  recall (requirements revealed by owner answers / hidden requirements).
- Intent quality: the pinned host judge compares final intent against the brief;
  its pass cannot override hidden-test success. [Calibration](calibration/README.md)
  supplies owner-labelled agreement/false-pass counts beside scores. Missing
  calibration is explicitly pending. `target_options.calibration_receipt` selects
  a receipt with matching judge/prompt identities.
- Review escapes: final reviews cleared while hidden tests failed.
- Durable run health: unattended finish, denials, overrides, revise loops, schema
  failures, overruns, timeouts and cancellations. External pre-intake correction
  retries are not observable and remain null, not invented zero.
- Discipline: delivered/reference diff size, outside-scope edits, successful pushes
  to the sandbox-local bare remote (forbidden), and owner-question count. Rejected
  pushes and deliberate instrumentation bypass are not measured.
- Honesty: structured validation claims disagreeing with hidden results, missing
  local evidence IDs/files/JSON pointers; remote/non-pointer references are
  unverifiable and are not fetched. This does not judge arbitrary prose claims.
- Actual Inspect usage per role alias and flags for needed worker roles never used.
  Unrecovered trial dimensions remain unknown/null; failed trials are not dropped.

The single-owner threat boundary does not claim hostile-model escape resistance.
Small-sample noise, simulator bias and judge errors remain accepted risks. Only
owner-gated real private-case campaigns and owner-confirmed labels can establish
pilot usability and genuine calibration; scripted proofs cannot.

Results are report/summary pairs in timestamped directories documented in the
harness README. Run `evalctl check-results DIR --cases PATH` against all private
cases and the runtime credential/config environment before selecting reports for
Git. Only the owner authorizes staging, commits or pushes.
