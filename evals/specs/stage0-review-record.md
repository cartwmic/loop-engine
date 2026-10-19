# Stage 0 review record

One independent review, as the owner chose: `openai-codex/gpt-6.1-sol` at high thinking, fresh read-only
session, no Opus or Haiku. Model guard (checked against the session file after the return): one
provider/model pair, thinking level `high`, 15 assistant turns. Verdict returned: **not ready**, with
13 findings (1 blocker, 10 major, 2 minor).
Each finding, and what was done:

| # | Sev | Finding (short) | Disposition |
|---|---|---|---|
| 1 | blocker | G1 kept the engine-terminal scoring gate for L, contradicting section 5 | **Accepted.** One scoring path for all arms including L; engine lifecycle, `intent.json` validity, adherence are diagnostics; scripted parity cases for terminated, active, crashed, completed drivers added to G8/G1 (sections 7, 8). |
| 2 | major | Grader lacked evidence its output needs (hidden-test truth, unchanged context, pointers for absence) | **Accepted.** Full baseline and final file contents; fidelity call has no test results, separate claim call with receipts; null pointers with reason; "not checkable" apart from "contradicted" (5.2). |
| 3 | major | Owner labels did not gate use; no adjudication of sample disagreement; no confidence field | **Accepted.** Predeclared acceptance (10 of 12, no deficient graded faithful, scope violations flagged); diagnostic-only fallback; unstable verdict rule; probabilities in schema (5.2). |
| 4 | major | "Defects caught" counted ordinary fail-to-pass; G5 optional but table said "full" | **Accepted.** Renamed delivery trajectory; deduplicated finding ledger with verification, materiality, false positives; "unavailable" for arms without review output (6). |
| 5 | major | Whole-product screening cannot attribute value to single obligations; profile confounded with task; weak rules at n=3 | **Accepted.** Section 10 rewritten: Stage 1 decides on whole configurations and produces candidates; obligation attribution moved to Stage 2 ablations; explicit comparison hierarchy; T3-alone and zero-yield findings exploratory. |
| 6 | major | Approval correctness and human attention not operationalized | **Partly accepted.** Marked untested rather than building approval scenarios (YAGNI for a screen); counts relabelled as proxies; incremental-value calculation added where denominators exist (6, 10). |
| 7 | major | Some System 1 uses not evidence-contained; #32 feasibility unanswered; unmeasured context claim | **Accepted.** Only question-label classification recommended; others "unproven"; feasibility unknowns listed (price, licensing, latency); T3 claim softened; interface described as proposed (5.3). |
| 8 | major | #33 deferred, not answered | **Accepted.** Bounded recommendation added: setup mode over existing profiles, input descriptor, floor invariants, three tasks, decision rule (5.4). |
| 9 | major | $0.44 and $1.56 per M tokens are blended costs, not prices; budget underestimated; $25/$70 caps too low | **Accepted.** Reworded; budget rebuilt with low/central/high; T2 central $15 and T3 $50 for L; total about $240 (range $165-$365); caps raised to $45/$120 (3 and 11). The "3.5x cheaper" claim removed. The model recommendation stays Sonnet 5.5 for continuity with the Tier 0 anchor, now justified by that, not by price. |
| 10 | major | No enforcement design for the stage cap | **Accepted.** Stage ledger and admission rule (G10, section 7); best-effort wording; all spend in one ledger. |
| 11 | major | Cap semantics did not match the harness (idle, hard limit covers scoring, 300 s margin, post-finish stops) | **Accepted.** Boundaries restated; margin made a target option (G9); idle is a failure; post-finish stops keep success (7). |
| 12 | minor | Research qualifiers lost ("not verified" became "no support"; old token test attached to current version) | **Accepted.** Wording fixed in spec section 2 and appendix A.5/A.6; permalinks added where the API gave them. |
| 13 | minor | Opus T3 result was resumed; recap-preview 8-11 s is a metadata limitation | **Accepted.** Resumed noted; recap-preview note corrected (4). |

Rejected: none outright. Finding 6 was only partly accepted, for the reason given. The reviewer's repricing
numbers ($25 and $93 for L on T2/T3 under Sonnet bucket rates; about $8 and $29 for Opus buckets) were
used as sensitivity bounds; I did not recompute them from the raw summaries. The review was read-only and
ran no commands, so none of the version/date claims in the appendix were re-verified by it; those were
checked by me against the GitHub API (appendix A.1).
