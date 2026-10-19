# Opus continuation outcome

The six-assignment comparison in `report.md` and `summary.json` remains unchanged.
The original Opus review-policy attempt remains an infrastructure outcome:
Anthropic's provider policy block. It is not a scored model failure.

One subsequent owner-authorized ordinary continuation restored that active run,
without changing the request, model or workflow policy. It completed naturally:

- Assignment: `m1-c2-t1`, review-policy v2.
- Model: Opus 5.5 via claude-compat, medium thinking; Codex Sol high as owner/judge.
- Outcome: **failure**; terminal state `end`, lifecycle `final`.
- Hidden tests: **15/16 passed**, delivery recall 15/16.
- Failed check: setup knobs and review-axis edits (REQ-6).
- Elicitation recall: 7/7; intent judge passed.
- Additional execution time: about 152 minutes; original blocked segment about 33 minutes.
- No provider policy block recurred during this continuation.
- Report: `../20261011T030831.922694Z/report.md`.

This trial is marked resumed and excluded from headline success rate and pass^k.
Its original infrastructure segment is retained, not erased or relabelled.

The original comparison also uses differing execution limits. The four carried
records retain their original 1800-second idle and 28800-second hard limits.
Only the new authorized Opus assignments had those outer limits disabled.
Historical recap-preview records came from prior bare-snapshot recovery; their
original workflow usage, elapsed execution time and elicitation metadata are
unavailable. Their recorded recovery wall times must not be treated as original
model execution times. None of these historical scored fields were rewritten.

## Owner hold

Michael via pleb has placed all further Opus eval launches on hold after this
trial: no retries, replacement trials, result-improvement runs, extra Opus
scoring calls or replays of blocked requests without new explicit authorization.

No integration into main, rebase or push occurred.
