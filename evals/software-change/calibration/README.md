# Intent judge calibration

## Overview

Calibration is software-change-specific and host-side. It compares the same
`intent-judge-v1` prompt used in trial scoring against owner-confirmed labels from
past Loop Engine runs. No engine/provider changes or paid model defaults are used.

The first real pass (2026-10-11) scored ten owner-confirmed pairs (8 good,
2 deficient) with `codex/gpt-6.1-sol` at high reasoning: agreement 4/10 and
false-pass rate 0/2. The judge failed 6 of the 8 good intents. That is a strict,
weakly calibrated judge; two deficient labels are too few to trust the false-pass
figure. The pilot report's `judge-calibration.md` supplement shows the figures.

Limits: `calibrate score` does not apply campaign `model_config`, so it uses the
provider's default reasoning effort. That first pass bound high effort with a
driver-side wrapper to match the pilot judge. Reasoning models also ignore the
requested `temperature=0`.

`judge_intent` and `calibration_for_report` are internal harness hooks with no
versioning or API-stability promise. The internal target contract is in
checkout-root `evals/core/README.md`; campaign operation and model authentication
are in checkout-root `evals/README.md`; contributor procedure is in checkout-root
`AGENTS.md`.

## Setup

Prerequisites: Python 3.12+ and uv, run from the checkout root, plus a judge
model you can authenticate on the host (bring your own Inspect model ID; for
`codex/...` run Pi once so `~/.pi/agent/auth.json` is valid, as described in
checkout-root `evals/README.md`). Extract candidate pairs read-only:

```sh
uv run --project evals evalctl calibrate extract --database "$HOME/.local/share/loop-engine/loop.db" --sessions "$HOME/.pi/agent/sessions"
```

## Usage

After the owner gate below, score the labels (makes one judge call per label):

```sh
uv run --project evals evalctl calibrate score --labels evals/software-change/calibration/labels.json
```

## Validation

Fixture proofs with a scripted judge backend (no network or paid calls):

```sh
uv run --project evals pytest -q evals/software-change/tests/test_calibration.py
```

## How extraction works

Extraction opens SQLite with URI `mode=ro` and `query_only=ON`, queries only recent
software-change run IDs/artifact roots with **both** `lifecycle = 'final'` and
`current_state = 'end'`, then their indexed `user-steering` records and linked
`intent.json`. Active, paused, failed/non-end and other-workflow runs are excluded
before the scan limit. The database stores artifact locations, not intent bytes.
The JSON extraction summary and CLI report completed runs examined, candidates,
and skipped completed runs with separate `skipped_reasons` counts:
`artifact_folder_missing`, `no_owner_request` (no usable steering instructions),
and `intent_unreadable` (missing, unreadable, invalid JSON or missing revision).
Without `--sessions`, reasons are exclusive, checked in that order; malformed/blank steering records
are ignored. Counts cover examined completed runs, stopping at either bound. All owner steering
instructions are retained in sequence as the request plus clarifications. This is
not a hidden brief: the owner must check that the captured request is sufficient
for labelling. Live artifacts can change later; extraction retains their bytes,
revision and SHA-256. Completion is an eligibility gate, not a quality label;
extraction does not infer quality from run state or review verdicts.

Default bounds are 20 candidates from at most 200 runs. `--limit`, `--scan-limit`
and `--output PATH` override them. Without `--sessions`, existing files are never
overwritten. With `--sessions DIR`, missing artifact roots can be reconstructed:
extraction first requires a usable owner request, then uses `subject_revision` in
the latest (by sequence) `review-evidence` data whose `subject` is `intent.json`.
It streams read-only JSONL transcripts mentioning the run ID and raw-decodes
message text for objects with `revision`, `outcome`, and `acceptance`, matching
only that final revision. It chooses the most frequent distinct object (stable
lexical tie-break), recording all ambiguity rather than inferring owner approval.
Recovered `source` metadata includes `reconstructed: true`, `session_file_count`,
`match_count`, `selected_match_count`, `ambiguous`, and `distinct_match_count`.
The reconstructed hash covers canonical sorted-key JSON, not lost original bytes.
Counts include `recovered`, `not_found`, `ambiguous`, and `skipped_no_request`.
Session-file count is the number of transcripts mentioning the run; match count
is every matching object occurrence, including repeated tool output.

Reconstruction mode merges an existing output by ID, preserving `owner_label`,
`owner_confirmed`, the judge pin and previously extracted entries not found this
time. Content is secret-scanned before atomic replacement; extraction never
assigns or confirms a label. Use `--limit 200 --scan-limit 200` to examine the
whole current catalog instead of stopping at the default 20 candidates. The
private default `labels.json` is gitignored; do not publish it. Receipts and raw
logs belong under ignored `captures/`. Only synthetic test labels are public.
The secret/config guard scans label content before writing it or sending it to a
judge; a match refuses the operation without exporting the matched text.

## Owner gate

Every extracted entry starts with `owner_label: null` and
`owner_confirmed: false`; `judge_model` also starts null. The owner checks the
pairs, sets each assessed entry's label to `good` or `deficient`, and sets
`owner_confirmed: true`. Unassessed null/false entries can remain and are counted
separately. Set top-level `judge_model` to the chosen pinned Inspect model ID,
or supply `--judge-model PROVIDER/MODEL`. No model is chosen on the owner's behalf.

Scoring validates the whole set before any model call and refuses missing labels,
unconfirmed/invalid labels, duplicate IDs, or a missing pin. The exact owner
command above exits 2 while pending; that is not completed AC-6 proof. It makes
no judge calls until the owner gate is satisfied. Pin the same judge model for the
campaign and labels.

The safe metric receipt defaults to `captures/calibration.json` beside the labels;
`--output PATH` overrides it. It contains:

- Agreement: matching judge/owner verdicts divided by confirmed labelled entries.
- False-pass rate: judge passes among owner-deficient entries, divided by all
  owner-deficient entries (not by judge passes).
- Both numerators/denominators, owner good/deficient and judge pass/fail counts,
  unlabelled count, model/prompt pin, and labelled-file SHA-256.
- Judge token usage as separate evaluation overhead, never workflow cost. Missing
  usage or an empty denominator is null, not a fabricated zero.

No requests, intents, brief text or judge prose enter that receipt. Invalid judge
output raises `ModelError`; no partial metric receipt is written. Raw output
schema is exactly `{"passed": true}` or `{"passed": false}`. This is a binary
intent-quality rating, not a hidden-test or overall trial-success verdict.

## Target integration

In the target's host-side scorer, read the final intent artifact and call:

```python
judgment = await scevals.judge.judge_intent(intent_dict, hidden_brief)
metrics["intent_judge"] = judgment.metrics()
```

Do not pass a model override inside a campaign: this uses the bound Inspect
`judge` role, whose usage core already accounts as evaluation overhead. Do not
copy the hidden brief into the trial container or export judge inputs/outputs.
The call's missing/invalid intent or malformed verdict must not silently count as
a judge pass. Campaign time limits also cover scoring.

`calibration_for_report(receipt_path, judge_model=settings.judge_model)` returns
only report-safe figures, checks the same model/prompt pin and recomputes the
figures against stored verdicts. Attach that result beside intent scores in the
target's `report_metrics`. If no receipt exists, explicitly report calibration
pending, not zero agreement. Calibration usage is historical overhead, separate
from current campaign judge usage. The software-change target loads the default
receipt `captures/calibration.json` beside the labels, or
`target_options.calibration_receipt`.

The existing `scevals.cases` evalctl command plugin registers `calibrate` alongside
`case-check`. The fixture proof invokes the actual CLI with a trusted scripted
Inspect ModelAPI backend and separately verifies bound judge-role usage in a real
Inspect Task; no network, paid calls or whole software-change/Pi trial is needed
for these host-side mechanics.

## Troubleshooting

- **`calibrate score` exits 2.** Labels are missing, unconfirmed, invalid, or no
  judge model is pinned. Confirm labels and set `judge_model` or `--judge-model`.
- **`calibrate score` refuses because the receipt exists.** Receipts are never
  overwritten. Pass `--output NEW_PATH` for a new receipt.
- **Reports still say calibration pending.** No receipt is at the default path
  `captures/calibration.json` and no `target_options.calibration_receipt` is set.
- **A campaign fails before any trial (`ValueError`).** A receipt exists but its
  judge model or prompt does not match the campaign's `judge_model`. Point
  `target_options.calibration_receipt` at a receipt scored with that judge, or move
  the mismatched receipt away from the default path.
- **`calibrate score` fails with only `Command failed (SomeError)`.** The CLI hides
  exception text. Check judge authentication first (see checkout-root
  `evals/README.md` troubleshooting).
- **Many completed runs are skipped.** Their artifact folders are gone. Pass
  `--sessions` to reconstruct intents from Pi transcripts; ambiguous
  reconstructions are flagged for the owner, not resolved automatically.
