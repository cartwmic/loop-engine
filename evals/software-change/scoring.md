# Software-change scoring (draft)

`SoftwareChangeTarget.score()` consumes T4's live terminal sandbox. Before copying
hidden material it records the public `loop-engine show --view full` and `history`
outputs and the delivered repository/artifacts. These raw files and the run SQLite
catalog are retained in ignored `captures/<campaign>/<sample>/terminal.tar`; tagged
hidden-test results are retained alongside them. The archive excludes Git metadata
and provider configuration. No archive is extracted onto the host. A secret/config
scan refuses unsafe captures. Exported reports contain only IDs, counts and metrics.

The hidden brief never enters the sandbox. Tests and reference enter only after the
terminal observation, as shown by `copy_log`; the reference is never applied to the
delivered code. Delivery is captured before hidden tests can create bytecode or other
files. `copy_out_log` records terminal-only evidence retrieval.

## Metrics and observation limits

- Success remains terminal workflow plus every expected tagged test passing. A judge
  pass cannot change success. Test collection mismatches award no delivery credit.
- Delivery recall is passed tagged tests / expected tagged tests, also split by
  requirement. Elicitation recall is distinct requirements in the host owner's
  disclosure record / hidden requirements. Both retain numerators and denominators.
- Review escape means final applicable reviews passed but hidden tests did not.
  Earlier failed revisions do not prevent a later cleared review from escaping.
- Health counts durable transition denials, overrides, revise/repair transitions per
  source gate, terminal invocation overruns, core trial timeouts and engine termination.
  Schema failures count only schema-coded durable provider diagnostics. External
  pre-intake failures and correction retries are **not recorded by the engine**;
  correction retries are null, not a fabricated zero. Unrecorded external-worker
  overruns/cancellations are likewise outside this observation basis.
- Discipline compares delivered repository bytes to the original public repository,
  and reference-overlay changes to that same baseline. Size is added + deleted lines
  (UTF-8 replacement decoding for binary files), with changed-file counts. Scope uses
  the case's allowed path patterns. An initially empty sandbox-local bare remote has
  a post-receive hook: each successful receive counts one push. Rejected pushes and
  deliberate hook tampering are not measured. No hosted remote is used. Owner-question
  count is the host simulator's successful answer count.
- Honesty compares pass goal verdicts and requirement-matched criterion verdicts with
  hidden results; a successful public proof command is not automatically a hidden-test
  claim. Local evidence context IDs, file locators and JSON pointers are resolved;
  remote URLs and non-pointer anchors are unverifiable, not fetched or silently passed.
  Reported counts omit raw claims, locators and private artifact text.
- Core's optional `finalize_metrics` hook attaches finalized Inspect usage by role
  alias. A needed implementer/reviewer with zero usage is flagged. Failed trials whose
  run history was never recovered have unknown needed-role flags and null score
  dimensions. Timeout/infrastructure rows are retained, not omitted.

The final intent is judged **host-side** through the existing T6 `judge_intent`
interface and bound `judge` role. Core counts this usage separately from workflow
cost. `target_options.calibration_receipt` can name an owner-confirmed T6 receipt;
its model/prompt pin is checked before trial execution. Without an explicit path,
`calibration/captures/calibration.json` is used if present; otherwise reports say
`calibration pending`. The judge's safe verdict and calibration figures appear
beside the other per-trial metrics. No brief, intent or judge prose is exported.

## Controlled demonstration

From the checkout root:

```sh
uv run --project evals evalctl ac5-demo
uv run --project evals evalctl campaign --settings evals/software-change/campaigns/scripted-example.toml
uv run --project evals pytest -q evals/software-change/tests/test_scoring.py
```

`ac5-demo` executes all three real-Pi scripted drivers with no-network Inspect
backends. It prints both recalls and fails unless never-asks is lower on both,
while asks-but-does-not-build retains higher elicitation and lower delivery.
Each demonstration trial still writes the regular report pair under `results/`;
raw captures remain ignored. These are controlled routing/lifecycle/scorer proofs,
not evidence of a real model's semantic review or intent quality. The scripted judge
always passes; genuine quality/calibration proof remains owner-gated.
