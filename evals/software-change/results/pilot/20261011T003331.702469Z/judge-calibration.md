# Intent judge calibration (supplement)

This supplement adds the judge calibration that `report.md` lists as pending.
That report and `summary.json` are unchanged. No trial score, outcome or label changed.

- Judge: `codex/gpt-6.1-sol`, reasoning effort high (same as the pilot judge), contract `intent-judge-v1`.
- Label set: ten owner-confirmed pairs from past Loop Engine runs (8 good, 2 deficient). Seven unlabelled candidates were not scored. Labels SHA-256: `4fc2bf68b1017ed5064e81aa286348e79c97ddc82138e0259d6f72bfa9eb2827`.
- **Agreement: 4/10 (0.40).**
- **False-pass rate: 0/2 owner-deficient intents passed (0.00).**
- Judge verdicts: 2 pass, 8 fail. It failed 6 of 8 intents the owner labelled good.
- Calibration usage (evaluation overhead, not workflow cost): 92071 input, 2438 output, 94509 total tokens over ten calls.
- One pass only. temperature=0 was requested, but the provider ignores temperature for reasoning models.

## Read next to the judge's pilot scores

| Trial | Model | Case | Outcome | Intent judge |
|---|---|---|---|---|
| m0-c0-t1 | Sol | followup-import | success | pass |
| m0-c1-t1 | Sol | recap-preview | failure | pass |
| m0-c2-t1 | Sol | review-policy v2 | failure | pass |
| m1-c0-t1 | Opus | followup-import | success | fail |
| m1-c1-t1 | Opus | recap-preview | success | fail |
| m1-c2-t1 | Opus | review-policy v2 | infrastructure (unscored) | none |
| m1-c2-t1 continuation (separate, resumed) | Opus | review-policy v2 | failure, 15/16 | pass |

Interpretation: with 4/10 agreement, the judge is strict. It rejected most owner-good intents. With only two deficient labels, the 0/2 false-pass rate is weak evidence. Treat the intent-judge column as low-confidence signal, not a quality verdict. Hidden-test and workflow outcomes are separate and unaffected.
