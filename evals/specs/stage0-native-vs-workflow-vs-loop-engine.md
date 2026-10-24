# Stage 0 spec: native Pi vs established workflow vs Loop Engine

Status: **design only, proposed.** Written 2026-10-19 for the owner's review. It authorizes nothing.
No paid trial was run to write it, the frozen pilot and its results are untouched, and no adapter or
integration was built. Every execution step below needs its own owner go (see "What needs Michael").

Sources: loop-engine issues [#22](https://github.com/cartwmic/loop-engine/issues/22) (body and the
owner's three 2026-10-19 comments), [#32](https://github.com/cartwmic/loop-engine/issues/32) (System 1
judges), [#33](https://github.com/cartwmic/loop-engine/issues/33) (task-specific harness template), the
#42 harness merged in PR #50 (`evals/`), and current vendor repositories. Research detail and links:
[stage0-research-appendix.md](stage0-research-appendix.md).

Owner decisions this spec follows (verbatim): "Plain worker + 1 Sol review (Recommended)" for how this
spec is produced; "#22 + #32 + #33 (Recommended)" for scope.

## 1. Question, and the decisions it feeds

Question: on a Pi-first screen, does Loop Engine (LE) buy enough outcome per unit of cost, time and
human attention, compared with (a) native Pi and (b) a current established workflow on the same Pi?

Falsifiable hypothesis from the owner's comment: a strong harness with light persistent intent,
deterministic safeguards at real boundaries and risk-triggered independent review may beat both
elaborate spec/prompt methods and LE's heavier profiles on ordinary tasks, while LE's stricter
guarantees may pay off only on high-risk, ambiguous or authorization-sensitive work.

Success is not "LE wins". Each LE obligation ends in one of: **keep, simplify, condition, replace,
retire** (section 10). "Inconclusive, keep as is" is a legal outcome and the default.

## 2. Arms (Pi first)

One execution harness (Pi), one model for every role in every arm, so each arm differs from native
only in the workflow layer. Claude Code and Codex are out of Stage 1 (section 8, blockers).

| Arm | Id | What runs |
|---|---|---|
| Native | **N** | Pi print mode with its built-in tools (read, bash, edit, write). No skills, extensions or templates beyond Pi's default system prompt. |
| Established workflow 1 | **S** | N plus **Superpowers v7.0.0** installed as its Pi package (skills plus the bootstrap extension). Inline execution: no subagent tool, which is the documented Pi behavior without the optional `pi-subagents` companion. |
| Established workflow 2 | **O** | N plus **OpenSpec v1.14.3**, default `core` profile (`propose`, `apply`, `archive`), initialised with `openspec init --tools pi`. |
| Loop Engine | **L** | N plus the shipped software-change workflow at the pinned release (v0.28.0, current), driven by the existing `sc-driver`/`sc-implement`/`sc-review` commands. Profile `minimal-17` on the trivial task, `standard-17` on the others (the shipped IDs at v0.28.0, read from the tag's `crates/software-change-provider/data/configs/`). |

Why these two workflows, and why not the rest: [appendix A](stage0-research-appendix.md#a-shortlist). Short
version: both name Pi as a supported harness in their own docs, both are the tools the owner tried,
and both are current (S 2026-10-10, O 2026-10-17). Spec Kit (v1.2.1) supports Pi through prompt
templates only; a February 2026 report (older versions, not re-checked) measured about twice
OpenSpec's tokens, so it is held for Stage 2. GSD: the original is archived (2026-06-26) and I could not
verify native Pi support; "GSD Pi" is a separate agent, not a workflow on Pi. BMAD (v6.12.4) lists Pi in
its installer and is optional for Stage 2.

### 2.1 Deliberate differences (documented, not hidden)

| Difference | N | S / O | L |
|---|---|---|---|
| Added instructions | none | the workflow's skills/templates | two LE skills plus engine CLI |
| Roles and processes | one Pi process | one Pi process (S: inline; no fresh-context reviewer) | driver plus separate Pi worker processes for implement and review (same model) |
| Durable state | repo only | workflow files in repo (`openspec/`, `docs/superpowers/`) | engine SQLite catalog, artifacts, history |
| Mechanical enforcement | none | little: bootstrap injection (S); CLI structure validation (O); the rest is prompt | provider-checked transitions and denials, evidence locators, review contracts |
| Completion | driver exits | driver exits | engine reaches a terminal state, then driver exits |
| Opt-in line in the prompt | none | one line naming the workflow | LE preamble (profile, run id, paths) |
| Workflow-owned repo paths | none | `openspec/**`, `.pi/**`, `docs/superpowers/**`, `.superpowers/**` | none in repo (state is under `/workspace/trial`) |

The following are **not** differences and must be identical: model, thinking level, Pi version, container
image base, tool set, case version, starting repository bytes and baseline commit, request text,
simulated owner, owner context file (bytes), hidden tests, grader, caps. See section 3.

Limits we state up front. (1) Defining behavior is not disabled: S runs without subagents because Pi
ships none; if the owner wants S with `pi-subagents`, that is a labelled variant (S+), not S. (2) L
uses worker processes and S/O do not, so "independent review" differs by construction; that is part
of what is being priced. (3) Anything an arm changes beyond the workflow (extra processes, files) is
reported as part of its total effect. We do not claim the workflow text alone caused a difference.

## 3. Matched inputs

For every (task, repetition) block, all arms get:

- Model for every role: `pi/claude-compat/claude-sonnet-5-5`, thinking `high` (same as the Tier 0 Sonnet
  full trial, so a measured cost anchor exists). Michael may choose otherwise; the spec holds for any
  single model. Sonnet and Sol have the same base token rates in `tier0.toml` ($2 input, $10 output per M
  tokens); their observed per-trial costs differ because of cache mix, workflow behavior and the Sol
  high-input tier, so no per-token price advantage is claimed.
- Pi pinned to one version in the image for all arms (see G2: the current image pins 1.0.0).
- The same case id and version, the public request text verbatim, the same starting repository, and
  **a baseline commit made by the harness before the driver starts** (G3). Today LE's preamble tells the
  driver to initialise Git; that moves to the harness for every arm.
- The same simulated owner: the existing host-side simulator (`codex/gpt-6.1-sol`, high), same
  instruction text, reached only through the `ask-owner` command. It is the only human channel in
  every arm. The hidden brief never enters the container.
- The same neutral base prompt (appendix B). It states the request, the repo path, that the owner
  is reachable only through `ask-owner` (messages are not read), no commits or pushes, stay in the
  request's scope, and "finish with a short summary of what changed and what you checked".
- The same **owner context**: the owner's design principles (`design-principles.md` at chezmoi commit `3517fd6`,
  8734 bytes, sha256 `82062f0e68be91969f2b6557e20557fc6407ec5d0ba9d0a314624d31e5b770bf`) as one global Pi
  `AGENTS.md`, byte-identical in every arm and in every Pi process an arm starts (native, Superpowers,
  OpenSpec, and the Loop Engine driver and its `sc-implement` and `sc-review` children). It is written inline
  into each per-call Pi agent directory, so it is always in context and does not depend on the model choosing to
  read a path. It is not in the case repository, and the base prompt (appendix B) is unchanged. The arms image
  pins its hash. Delivery is proved without a live or paid model call (G11); every report states the hash. A labelled control,
  native without the principles (`SC_OWNER_CONTEXT=off` on the same image), measures the file's own effect. The
  principles shape design and review, so rows run without the file are not comparable with rows run with it.
- The same caps per task class (section 7) and the same price table for API-equivalent dollars.
- The same hidden tests and the same grader (section 5). Arms never see tests, brief or grader.

## 4. Tasks

Prefer existing cases. Three tasks; a fourth is held in reserve.

| Id | Class | Case (version) | Repo / size | Existing anchors |
|---|---|---|---|---|
| T1 | Trivial | `label-keys` (1), public example | 1 file, ref diff 2 lines, 3 REQs (2 hidden), 6 tests | Tier 0 full, Sonnet 5.5, LE minimal-16 (v0.26.0; its v0.28.0 equivalent is minimal-17): success, 6/6 tests, $3.64, 8.26 M tokens, 1189 s, 3 owner questions. Sol: 6/6 tests but workflow terminated early, so scored failure, $12.10, 2162 s |
| T2 | Small, hidden requirements | `followup-import` (1), private | 1 file, 3 REQs, 6 tests | Pilot, LE standard-14: Sol success 5498 s, 15.2 M tokens; Opus success 5348 s, 16.4 M tokens |
| T3 | Complex, high risk | `review-policy` (2), private | Rust, about 7.3k files incl. 143 vendored crates, ref diff +1502/-290 over 20 files, 7 REQs, 16 tests, test timeout 1200 s | Pilot, LE standard-14: both models 15/16 tests; Sol 11438 s, 55 M tokens; Opus a **resumed** continuation (9149 s) after an infrastructure-blocked first segment of about 1953 s |
| (R) | Reserve | `recap-preview` (1), private | 1 file, 3 REQs | Pilot rows for it show 8-11 s because they come from a recovery/scoring pass whose original execution metadata is unavailable (see the pilot `continuation-outcome.md`); its delivery results are not thereby invalid |

Honest gaps. (1) There is **no existing medium multi-file case**. T2 is medium in ambiguity (hidden
requirements to elicit), not in size. A real medium case needs the owner to approve a new case and a
`case-check`; no model cost, small effort. Recommendation: do not block Stage 1 on it; add it in
Stage 2 if T2 looks too easy. (2) T1 is a public example case: use it for the overhead floor and
elicitation behavior, not as a capability claim. (3) T3 is the only class where the pilot showed LE
missing a test; both pilot LE runs failed one of 16 tests, so the ceiling for LE on T3 is unproven.
Pilot data was gathered with profile `standard-14` at an older ref; it anchors cost and time, not
outcomes under v0.28.0.

## 5. Grading contract

Principle: **one grading path for all arms, and it never uses LE's terminal-state semantics.**

### 5.1 Layers (kept separate, as #22 requires)

1. **Binary outcome (primary).** All hidden tests pass on the final workspace after the driver exits
   by itself before its caps. Collection mismatch, skips and unexecuted tests fail, as today. The
   existing scorer's hidden-test runner is reused unchanged.
2. **Graded delivery.** Per-requirement test recall, and elicitation recall (requirements revealed by
   owner answers, from the host-side owner receipt). Both exist today.
3. **Intent fidelity (independent grader, new, section 5.2).** Secondary. It never overrides layer 1.
4. **Process and honesty signals.** Arm-specific where only some arms expose them; unavailable is
   `null`, never zero.

LE's own `intent.json` judge (`intent-judge-v1`) stays as an L-only diagnostic. It is not used across
arms: it needs LE's artifact, and its first owner calibration agreed with the owner on 4 of 10
intents (it failed 6 of 8 good ones).

### 5.2 Independent intent grader (`outcome-grade-v1`, new)

- **Inputs, host-side only:** the hidden brief; the baseline and final contents of every file the agent
  changed (full files, so unchanged context is visible); the diff with workflow-owned paths removed; the
  agent's final message. Not the owner Q&A (that is process, scored by elicitation recall) and not any
  workflow artifact.
- **Two separate grader questions, so evidence stays arm-neutral:**
  1. *Fidelity call.* Inputs above, **no test results**. Output (JSON, no prose exported): per
     requirement `satisfied | partial | violated | not_addressed` with a probability for the verdict;
     evidence pointers into the final files or diff, **null allowed with a stated reason** when the
     judgment is an absence (for example, a no-op repository); `unrequested_changes` as pointers with a
     `material` flag; `intent_faithful` boolean with a probability.
  2. *Claim call.* The final message's claims, plus trusted receipts: per-requirement hidden-test status
     and the harness's own re-run of the repository's visible tests, if any. Output: each claim as
     `supported | contradicted | not_checkable`. "Not checkable from supplied evidence" is reported apart
     from "contradicted", so an honest statement about checks the grader cannot see is not counted as false.
- **Independence:** a different model family from the driver. With a Sonnet driver the grader is
  `codex/gpt-6.1-sol` at high effort; if a later stage drives with Sol, swap to a Claude model. Arm names
  and workflow terms ("OpenSpec", "Superpowers", "Loop Engine", `intent.json`) are redacted from the final
  message; leakage is checked and reported.
- **Sampling and adjudication:** two independent samples per call. If they disagree on `intent_faithful`
  or on any requirement verdict, the trial's intent verdict is recorded as `unstable`, counted as not
  faithful in the headline and shown separately. Hidden tests carry correctness regardless.
- **Calibration gate, predeclared (before any headline use):**
  1. *Positive control:* each case's reference overlay: `intent_faithful = true`, all REQs `satisfied`.
  2. *Negative control:* each case's untouched no-op repository: not faithful, and every REQ with failing
     tests not `satisfied` (null pointers allowed).
  3. *Near-miss control:* the T3 pilot capture that failed 1 of 16 tests: the failing requirement (REQ-6,
     missed by both pilot models) not `satisfied`.
  4. *Owner labels:* about 12 delivered diffs chosen by the harness to include good, deficient,
     scope-violating and unsupported-claim examples (about 30 minutes of Michael's attention).
     Acceptance: agreement with Michael on at least 10 of 12 `intent_faithful` labels, **and** no
     owner-labelled deficient diff graded faithful, **and** every scope-violating diff graded with a
     material unrequested change.
  - **If controls 1-3 or the owner-label acceptance fail,** the grader is *diagnostic only*: hidden tests
    and elicitation recall carry every correctness and intent claim, intent-fidelity columns are shown but
    excluded from section 10 decisions, and the report says so. Stage 1 proceeds on that basis.
  - Report agreement, misses, false alarms and confident-wrong answers (probability 0.9 or more on the
    wrong label). Controls 1-3 cost under $1 and need no new trial.
- **The grader is evaluation overhead.** Its tokens and dollars are reported apart from workflow cost.

### 5.3 System 1 judges (#32): where they fit

Status: proposed, not established. Jev is available to this session as a classifier
(`typesafe/jev-1.13`, text only, 64k context) and Winnow 12B as a local one (65k context). Nothing was run
against them for this spec.

- **Suitable, and the only Stage 1 use:** classify each `ask-owner` question as *clarification*,
  *approval/gate*, or *status*. One question in, one label out, evidence fully in the packet. It feeds the
  human-attention proxy counts (section 6). Calibration: about 40 questions labelled by Michael; compare
  Jev, Winnow and Sol against his labels. Report agreement, misses, false alarms, confident-wrong answers
  and abstentions.
- **Unproven, not used for decisions:** workflow-adherence checks on Pi transcripts (need temporal
  reconstruction across turns) and per-requirement coverage from a brief section plus a hunk (may omit
  dependencies; hidden-test status is ground truth only for what the tests exercise). Candidate follow-ups
  only if the question-label calibration is good and a packet-sufficiency and abstention rule is written first.
- **Unsuitable:** whole-intent fidelity across brief, diff and message; verifying that a reported defect is
  real; deciding whether a review changed a decision. These need investigation or cross-artifact reasoning
  and stay with the Sol grader or tests. T3's source is likely far beyond either context window (not
  measured here), so System 1 on T3 would need chunking.
- **Open #32 questions this spec does not answer:** setup effort and per-call cost (Jev is served through
  OpenRouter; its price was not checked), licensing and terms for Jev and Winnow, latency and availability.
  Rule: unavailable is reported as `unavailable`, never silently replaced by another judge.
- **Interface:** the proposal is that the Sol grader and any System 1 judge share a "state plus typed
  questions in, typed verdict out" shape so a judge can be swapped without changing harness contracts.
  That is design intent; the harness today has one `judge_intent` function.
- **Default off in Stage 1.** Running Jev or Winnow is a new-model call that #32 says needs separate owner approval.

### 5.4 Task-specific harness template (#33): recommendation

**Not needed to run this comparison, and not recommended to build before Stage 1 results.** The comparison
needs one small **arm descriptor** per arm (image tag, role script, prompt template, workflow-owned
paths, completion rule), passed as data through `target_options`. That configures this evaluation; it is
not the #33 product.

For #33 itself, a bounded written recommendation, no implementation:

- *What carries over from the eval template:* a per-task harness assembled from a small declared
  description, with the same guarantees checked by the same machinery. What does not carry over: the
  simulated owner and hidden tests, which exist only because the evaluation has no real owner.
- *Shape, if built:* a **setup mode** of the existing software-change provider, not a new product. Input: a
  short task descriptor (change class; risk factors such as touching auth, data loss or a public
  interface; ambiguity; size; owner availability). Output: a validated configuration of the existing knobs
  (profile, rigor axes, review count and grouping, checkpoints), the same thing small profile
  customization (LE-180) produces today. Example: "one-line doc fix, owner present" gives a profile with
  one review and no design checkpoint; "touches the release workflow" gives standard-17 plus a mandatory
  independent validation review.
- *Guarantees survive tailoring only if* the template can choose only among existing validated profile
  values, cannot remove an evidence, owner-checkpoint or independent-review obligation below the shipped
  floor, and records the chosen profile with its descriptor as provenance. Check: the provider's own
  profile validation, plus a test that every descriptor maps to a valid profile at or above the floor.
- *Evaluation tasks for it (existing cases):* T1 (trivial: expect a lighter profile than standard-17 with
  the same hidden-test result), T3 (high risk: expect at least standard-17), and T2 (ambiguous: expect
  elicitation preserved). The value test is a Stage 2 arm "L-tailored" (descriptor-chosen profile) against
  the fixed profile, and against N-lite below.
- *Decision rule:* build only if Stage 1 shows LE's value concentrating by task class (section 10). If one
  profile is best everywhere, tailoring has nothing to choose between and the answer is a better default.

N-lite (Stage 2 only): native Pi plus a lightweight harness (short persistent intent note, one
deterministic check run, one risk-triggered fresh review). It tests the owner's falsifiable hypothesis and
is a native-harness variant, distinct from the #33 template.

## 6. Metrics and accounting boundaries

| Metric (#22) | Definition | Source | Arms |
|---|---|---|---|
| Correctness | hidden tests passed/total; all-pass boolean | existing scorer, common path (G1) | all |
| Intent fidelity | `intent_faithful`, per-REQ verdicts, elicitation recall | grader; owner receipt | all |
| Scope discipline | files and lines outside allowed scope; diff size vs reference; workflow-owned paths excluded and counted as *artifact volume* | existing `discipline` plus path filter | all |
| Defects escaped | final-workspace hidden-test failures, plus grader-found material defects | tests; grader | all |
| Delivery trajectory | per-requirement test status over snapshots (fail to pass is ordinary implementation progress, not defect prevention) | G5 snapshots | all, only if G5 is built |
| Defects caught / prevented | a deduplicated **finding ledger** from each arm's own review or check output: independent verification (reproduced by a test or the grader), materiality, discovery time, whether already known, disposition, false-positive and duplicate flags | arm logs plus verification (G10) | **unavailable for N (no review step) and for S/O unless their logs expose findings; L from engine records. Reported as "unavailable", never zero** |
| Human attention (proxy) | simulated-owner question and answer counts, words returned to the owner, by type (clarification / approval / status) | owner receipt; Sol or System 1 labelling | all |
| Wall time | driver start to driver exit; setup and scoring reported apart | harness clocks | all |
| Tokens and API-equivalent $ | Inspect per-role usage (input, output, cache read, cache write) times the pinned price table; unpriced is `null` | Inspect role usage | all |
| Workflow overhead | arm cost minus the paired native cost on the same block; plus Pi-session phase attribution where available (G6) | derived | S, O, L |
| Incremental value | per (arm vs N, class): change in hidden-test pass fraction and in verified escaped defects, over change in dollars, wall hours and owner words; shown only when numerator and denominator both exist and the numerator is nonzero | derived | S, O, L |
| Grading cost | simulator, grader, judge, snapshot replay: *evaluation overhead*, never added to workflow cost | existing split | all |
| Process diagnostics | idle and hard stops, workflow adherence (did S/O run their own steps), L denials and revise loops, engine lifecycle, `intent.json` validity | arm logs | arm-specific, never used for success |
| Honesty | L: existing claim/evidence checks; all: grader claim call | scorer; grader | L, all |
| Setup/maintenance | agent-hours to build and validate the adapter, files changed, pinned versions, install steps, re-validation per upstream release | build log | S, O, L |

**Not measured in Stage 1, and said so:** *actual human attention* (the owner is simulated; counts and
words are proxies, and any conversion to minutes is a labelled assumption); *incorrect approvals* (no
approval scenario with independent ground truth exists, and the simulator sees only the brief and the
current question, not delivered work, so authority-gate correctness is untested); *recovery effort* (no
interruption is injected); *defects prevented* (a defect that never existed cannot be observed).

Accounting rules, all taken from the existing harness: usage is Inspect `role_usage` only; simulator and
grader are overhead; failed, limit and infrastructure trials stay visible; missing usage or price is
unavailable, not zero. Dollar figures are *API-equivalent* from the price table in
`campaigns/tier0.toml`, not billing.

## 7. Design: repetitions, order, stopping, provenance

**Repetitions.** T1 x 3, T2 x 3, T3 x 1 per arm. Three is a screen, not statistics. We report paired
per-block deltas and the range across blocks, and make no significance claims. T3 gets a second
repetition only if Stage 1b shows a decision-relevant gap.

**Order.** Each (task, repetition) is a block that runs all four arms serially (the harness runs one trial at
a time, which also avoids provider contention). Arm order inside a block follows a 4x4 Latin square
(ABCD, BDAC, CADB, DCBA with A=N, B=S, C=O, D=L), cycling by repetition; the realised order is recorded.
Blocks run cheap to expensive: all T1, then T2, then T3. Time-of-day and provider-load drift are listed as
confounders.

**One trial = one campaign command**, as the Tier 0 full trials were. Each trial keeps its own caps,
report, capture and resume, and "one command runs exactly one campaign" holds.

**Time and dollar boundaries (stated separately, because the harness defines them separately).**
In the harness, the per-trial `hard_limit_seconds` covers driver, capture and scoring; the target stops the
driver `DRIVER_LIMIT_MARGIN_SECONDS` (300 s, a code constant) before it. Idle stops (no engine or token
progress for `idle_limit_seconds`) are ordinary **failures**. A hard-time stop or a campaign budget stop that
fires while work is running is outcome `limit`: delivery is scored on what exists, success is not credited,
never retried. A budget stop that fires after a trial's work finished leaves the trial a success and is
recorded. Dollar caps are best effort (a trial can overshoot by about one model call; the report says so).

| Class | Idle | Driver budget | Margin for capture + scoring | `hard_limit_seconds` | Dollars (best effort) |
|---|---|---|---|---|---|
| T1 | 1800 s | 3000 s | 1500 s | 4500 s | $12 |
| T2 | 1800 s | 12000 s | 1500 s | 13500 s | $45 |
| T3 | 1800 s | 21600 s | 1800 s | 23400 s | $120 |

The margin is longer than the fixed 300 s because T3's hidden-test timeout is 1200 s and scoring runs inside
the hard limit. This needs the margin to become a target option (G9). The dollar caps are above the
`TIERS.md` ceilings; see "What needs Michael".

**Stage ledger.** A tiny script (G10) keeps a ledger of every command in the stage: command, start,
end, reported dollars (best effort), hours, outcome, attempt number. It refuses to admit a new command
when remaining stage allowance is below that command's own cap, and the cap handed to the command is
`min(class cap, remaining)`. Smoke trials, calibration controls, grading, failed attempts and approved reruns
all count in the same ledger. Stage hard cap: $400 and 40 hours. Because dollar caps are best effort,
the ledger treats the stage cap as best effort too and reports any overshoot.

**Pause and report at the first of:** an arm failing its smoke test twice; infrastructure failures above
one third of any arm's trials; any trial reaching its cap; an arm that never invokes its own workflow in two
consecutive trials (adherence failure, reported, not dropped); the stage cap. "Stop at the first
surprise" from `TIERS.md` applies throughout.

**Failure and retry provenance.** The harness rules hold: no automatic retry; infrastructure trials are
excluded from success rate and counted apart; timeouts, hangs and model errors are failures; every original
attempt, pinned settings, image id and source commit, raw capture and report is retained. A rerun is allowed
only with the owner's explicit approval, is recorded as `attempt 2` with the reason, and **never replaces or
relabels attempt 1**. Both attempts are shown side by side. If a provider block hits one arm, that arm is
reported blocked for the block. A resumed trial is reported as resumed with both segments (the T3 Opus pilot
result is such a case).

**Completion boundary, one path for every arm including L.** A trial completes when the driver process exits
by itself before its driver budget. Delivery (hidden tests, recall, discipline) is scored on the final
workspace whatever the engine state is, including L runs whose engine lifecycle is active, terminated or
crashed. Engine lifecycle, `intent.json` validity and workflow adherence are process diagnostics, never
success conditions in this experiment. Success = driver exited by itself **and** all hidden tests pass.
This repairs the Tier 0 Sol case, where correct code (6/6) was scored a failure because the engine run ended
"terminated". The harness's existing engine-based success rule stays for existing campaigns.

## 8. Feasibility of the #42 harness

Findings from reading `evals/` at `96ee638` and running only `evalctl --help` and `docker image ls`.
Effort is agent-days of focused work (a day = one focused implementation session) plus owner review;
estimates, not measurements.

**What already works unchanged.** Mount-free arm64 container per trial; Inspect agent bridge to the
host models (it serves OpenAI Completions, OpenAI Responses, Anthropic and Google APIs); host-side
simulated owner behind `ask-owner`; hidden tests run in a fresh container after the fact; delivery
and elicitation recall; per-role token and dollar accounting with price tiers; campaign caps, idle and
hard limits; private captures, rescore and a pre-publication secrecy scan; a `driver_command`
override. So **(a) native Pi and (b) Pi plus a third-party workflow are both feasible without a new
execution harness**; neither needs Claude Code, Codex or a new sandbox.

**What is missing, and effort.**

| # | Gap | Detail | Effort |
|---|---|---|---|
| G1 | Arm-neutral scoring path, **for all four arms including L** | `score_capture` requires the engine DB (`show`, `history`), an `intent.json`, an engine terminal lifecycle, and success also needs lifecycle `final` and no driver timeout. Replace with one delivery/completion path: driver exited by itself plus hidden tests, scored on the final workspace for every arm. Engine state, `intent.json` validity and adherence become diagnostics (`null` where the arm has none). Existing campaigns keep the old rule. Scripted parity cases: terminated, active, crashed and naturally completed drivers score identically under L and N. | 3-4 days |
| G2 | Prompt, role scripts, image | `PREAMBLE` is LE-specific and hard-coded; `pi-role.py` always loads the two LE skills and passes `--no-extensions --no-skills --no-prompt-templates`. Need: a `prompt_template` option; role scripts for N, S, O; one image with Pi bumped (the image pins 1.0.0 and the S extension uses newer extension events; check against 1.2.0), Superpowers v7.0.0 and OpenSpec 1.14.3 installed at pinned versions, and LE rebuilt at v0.28.0 (`image-check --ref`, build network only). Print-mode expansion of `/opsx-*` prompt templates is **unverified**; fallback is plain instructions in the prompt. | 2-3 days |
| G3 | Baseline-commit parity | Harness makes the baseline commit and the empty local remote before the driver, for all arms. LE's preamble drops its Git lines (new preamble revision; the frozen pilot is unchanged). | 0.5 day |
| G4 | Independent grader | New host-side module, schema, redaction, pointer verification, two samples, controls, calibration receipt. Needs the driver's final message persisted (print-mode stdout) and a diff that excludes workflow-owned paths. | 2 days |
| G5 | Snapshot trajectory | Periodic workspace snapshots from the existing 60 s progress poll; replay hidden tests on each in fresh containers (free). Needed for "defects caught" on N, S, O. Optional for Stage 1; without it, caught is reported unavailable for those arms. | 2 days |
| G6 | Phase attribution | Keep Pi session JSONL (drop `--no-session`, set a session dir under `/workspace/trial`; the capture tar already includes it) and a script that splits tokens and time by skill/phase. Without it, overhead is the paired difference only. | 1.5 days |
| G7 | Paired report | `evalctl` reports per campaign and per combination; arms are not models. One campaign per trial means a small `compare` command that reads the trial summaries and renders the paired tables, ordering log and attempt history. | 1.5 days |
| G9 | Margin as option | `DRIVER_LIMIT_MARGIN_SECONDS` is a code constant (300 s) while the hard limit also covers scoring and T3's hidden-test timeout is 1200 s. Make it a target option; no default change. | 0.5 day |
| G10 | Stage ledger and finding ledger | Ledger script that admits commands against the remaining stage allowance (section 7) and a finding-ledger format with verification fields (section 6). | 1 day |
| G11 | Owner context file | One helper in `pi-role.py`, reused by `pi-arm.py`, writes the pinned `design-principles.md` text as `AGENTS.md` into each per-call agent directory (all six launchers); `Dockerfile.arms` pins its sha256. Free proof: real Pi in the image against a stub OpenAI-compatible server, system prompt read back for each launcher, with a negative control on an image built before the change (`tests/test_owner_context.py`). | 0.5 day |
| G8 | Adapter smoke tests (free) | Scripted-model proof per arm in the real container (existing scripted-backend pattern): arm starts, asks the owner, edits, exits, is scored. Proves mechanics only. | 1 day |

Total about **14-17 agent-days** before the first paid trial, no model spend (scripted backends and
`case-check`). Setup/maintenance for N is near zero; S and O each carry an install pin and re-check on
upstream release; L already has the pipeline.

**Significant new integration, flagged as #22 asks.** None for Pi. For the other harnesses:
- *Claude Code and Codex CLI arms:* plausible through the Inspect bridge (it exposes Anthropic and
  OpenAI endpoints; set the CLI's base URL), but unproven: install the CLI in the image, handle its
  login/telemetry checks, non-interactive mode, its own skills and plugins, and keep host credentials
  out of the container. Estimate 3-5 days per harness plus a free probe using the scripted backend, and
  a separate value/feasibility decision per #22. **Not in Stage 1.**
- *Superpowers with its subagent companion (S+):* the companion's child Pi processes must reach the
  same bridge model alias; unknown. Gate on a one-day probe; not in Stage 1.
- *Orchestrators and software factories (OmO, Gas Town, OpenHands, ...):* not investigated; they are
  whole-stack alternatives, not plug-in arms, per the owner's note.

## 9. Setup order inside Stage 1

1. Build G1-G3, G8 and the arm descriptors; run scripted proofs. No model spend.
2. Build G4; run its controls (under $1). Gate: controls pass, or the grader stays diagnostic.
3. One smoke trial per arm on T1 (about $7 total). Gate: each arm starts, asks, edits, exits and scores.
4. Stage 1a: T1 x 3 and T2 x 3. Report. Owner reads it.
5. Stage 1b (T3 x 1) only on the owner's go after reading 1a.

## 10. How results map to decisions

**What Stage 1 can decide.** It compares whole configurations (N, S, O, L at one profile per task class) on
a handful of blocks. It **cannot** attribute value to an individual LE obligation, because every LE
obligation changes together, and L runs minimal-17 on T1 and standard-17 on T2/T3, which confounds profile
with task. So Stage 1 produces (a) per-class verdicts on the configurations tested and (b) *candidates* for
follow-up per obligation. Obligation-level keep/simplify/condition/replace/retire decisions need ablations
(an L arm with one obligation changed) and are Stage 2 work; the table below names the follow-up each
Stage 1 pattern would motivate.

**Comparison hierarchy (applied per task class, in order).**
1. Success (all hidden tests pass with the driver exiting by itself). An arm with more successful blocks
   is better; a block where one arm succeeds and another fails is a decisive block.
2. Escaped defects: hidden-test failures at the end, and grader-found material defects if the grader is
   calibrated. Fewer is better.
3. Elicitation recall and intent fidelity (if calibrated).
4. Cost: workflow dollars, wall hours, owner words.
"Comparable" means the same level at steps 1-3 in every block of the class. "Cheaper" means at most half
the workflow dollars **and** at most half the wall time. A gap counts only if it has the same sign in every
block of the class and is at least one hidden test, one escaped defect, or one requirement, or a cost
ratio of 2x or more. With three blocks (one on T3), findings that rest on T3 alone, or on "zero yield
observed", are **exploratory**, stated as such, and never by themselves a retire decision.

| Stage 1 pattern | Reading | Follow-up it motivates |
|---|---|---|
| L succeeds where N/S/O fail in a class | LE's process is doing work there | keep for the class; ablate to find which obligation |
| N and L both succeed, equal elicitation, N far cheaper | LE overhead not earning its cost in this class | candidate to simplify or make conditional; Stage 2: L with fewer reviews |
| S or O match L on 1-3 at lower cost | prompted process may be enough | candidate to replace by or borrow from that workflow; check the enforcement difference in A.4 first |
| L worse than N on success | process harms | investigate harness and process failure before any conclusion |
| Review rounds cost much and the finding ledger shows no verified findings | review may be ceremony | candidate to condition on risk; needs more blocks |
| Honesty and unsupported-claim counts differ by arm | provenance may matter | keep provenance candidate; check materiality |
| Mixed, or any arm inadequately exercised | inconclusive | keep as is; name the missing evidence |

Not decidable in Stage 1, said plainly: recovery and durable state, authority-gate correctness and
incorrect approvals, rigor-profile choice, effect of any single review, and anything that depends on actual
(rather than simulated) human attention. Enforcement matters: a competitor that only *prompts* a duty does
not "replace" an obligation LE *enforces* unless the outcome gap shows the enforcement made no difference.
Nothing here changes the frozen pilot; any change to LE itself is a separate owner decision.

## 11. Stage 1 budget (the number)

**About $240 API-equivalent and about 23 unattended wall hours (serial), plus about 4 hours of Michael's
attention.** Plausible range **$165-$365**. Hard cap $400 and 40 hours (stage ledger, section 7). Staged:
Stage 1a (T1 and T2, smoke trials, controls) about **$130 and 14 h**; Stage 1b (T3) about **$110 and 9 h**,
only on the owner's go after reading 1a.

The figures are **estimates, with most inputs guessed**. What is measured and what is not:

- *Measured:* L on T1 with Sonnet 5.5 high: $3.64, 8.26 M tokens, 1189 s (Tier 0 full). Sol on T1: $12.10, 2162 s.
- *Observed blended cost, not a price:* realised dollars over total tokens was about $0.44 per M tokens for
  that Sonnet trial (cache-read heavy: 7.28 M of 7.61 M driver tokens were cache reads) and about $1.56 for
  that Sol trial (most input uncached). These depend on cache mix and are not model prices. The table in
  `tier0.toml` gives both models the same base rates ($2 input, $10 output per M tokens); the cache read
  rate is $0.10 (Sol) vs $0.20 (Sonnet), and Sol has a higher-rate tier above 272k input tokens.
- *Sensitivity, not prediction:* repricing the pilot's Sol workflow token buckets at Sonnet's rates gives about
  $25 for T2 and $93 for T3 (a reviewer computed this); repricing the pilot's Opus buckets gives about $8
  and $29. Sonnet's own behavior on T2 and T3 under L is unmeasured. The budget uses the low, central and
  high values below.
- *Guessed:* every N, S and O figure (anecdotes put Superpowers at several times a direct edit on tiny
  tasks, and Spec Kit at about 2x OpenSpec in one small test; not Stage 1 arms or current-version evidence).

| Per block, cost / wall time | N | S | O | L | Block |
|---|---|---|---|---|---|
| T1, central | $0.15 / 3 min | $2 / 20 min | $1 / 12 min | $3.7 / 20 min (measured) | $6.9 / 55 min |
| T2, low / central / high | $1 / 8 min | $4 / 35 min (central $6, high $8) | $2.5 / 25 min (central $4, high $5) | $8 / **$15** / $25, 90 min | $15.5 / **$26** / $39.5 |
| T3, low / central / high | $5 / 60 min | $14 / **$20** / $30, 2 h | $10 / $14 / $20, 1.5 h | $29 / **$50** / $93, 3 h | $58 / **$90** / $151 |

Central: T1 x 3 = $21 and 2.8 h; T2 x 3 = $78 and 7.9 h; T3 x 1 = $90 and 7.5 h; sum $189 and 18.2 h. Add smoke
trials (one per arm on T1, about $7 and 1 h), grading and controls (about $5; simulator about $0.02 per trial,
Sol grader with two samples per call about $0.10-0.60 by task, controls under $1; estimates), subtotal about
$200 and 19 h; add 20% contingency for infrastructure failures and approved reruns: **about $240 and 23 h**.
Low case (all low values): about $165. High case (all high values, 20% contingency): about $365.
System 1 shadow judges are off in Stage 1; if the owner enables the question-label check, estimate under $3
(Jev price unchecked). Everything priced as the Sonnet 5.5 row of the `tier0.toml` table, API-equivalent,
not billing.

Not included: agent time to build G1-G10 (about 14-17 agent-days, no paid model use), Michael's labelling time.
Re-estimate after the smoke trials and before Stage 1a, and again before Stage 1b.

## 12. Stage 2 expansion criteria

Expand only after Michael reads the Stage 1 report. Open Stage 2 when at least one of:

- a class shows a decision-relevant gap by the section 10 rule, and one more repetition set (3 more
  blocks of that class) would settle it; or
- N matches the others on T1 and T2 and the open question is T3-like work: add a second complex case
  and an L `high-rigor-17` arm; or
- results are mixed and the hypothesis arm N-lite (section 5.4) is the cheapest way to separate
  "process helps" from "this much process helps".

Candidate additions, each a separate go: Spec Kit v1.2.1 (lean preset) and BMAD v6.12.4 (`bmad-build`);
a real medium multi-file case; a second model (Sol) to check model dependence; S+ with subagents; the
Claude Code and Codex arms once their integration probes are done and separately approved.

Do **not** expand when: the grader fails its controls and no human-labelled fallback exists;
the arms' adherence failed (the workflow was not actually exercised); or infrastructure failures
exceed one third. Fix and re-run Stage 1 instead.

## 13. Confounders and risks

- **Simulator bias.** The owner simulator answers only what is asked. Arms that ask more reveal more.
  That is a real effect (elicitation) and also a simulator artifact; report both counts.
- **Workflows built for interactive humans.** S's brainstorming wants a conversation. Headless, the
  only human is `ask-owner`. A workflow that ends its turn asking a question in its final message
  stops with no delivery; that is recorded as the arm's behavior, not dropped.
- **Pi version.** The image pins Pi 1.0.0, the owner's installed Pi is 1.2.0. One version for all arms.
- **Contamination.** T1 is public. Models may know it. Capability claims use T2 and T3.
- **Ceiling and floor effects.** T1 is probably solved by every arm; T3 may be solved by none.
- **Owner context.** Every arm gets the same design principles (section 3), so they are not a variable between
  arms, but they are a variable against earlier rows: Stage 1 and check-in rows ran without them. They push
  toward the smallest design (§4, §13) and change what reviewers block on (§12), so they also act on the Loop Engine
  reviewers. The Loop Engine arm's standing instructions (concurrent tasks, carry-forward of review evidence) are
  workflow guidance and are given to that arm only; report them as such.
- **Small n.** Stage 1 is a screen. Effect size and consistency, not significance.
- **Workflow versions drift.** S, O, L pins are recorded; upstream releases (S shipped 7.0.0 on
  2026-10-10) can invalidate the arm.
- **Cost model.** API-equivalent, with cache pricing from a price table; the subscription cost the owner
  actually pays differs.
- **Grader independence.** Same family for simulator and grader (Sol) is shared bias; the grader sees
  neither Q&A nor arm identity, but both use one model family.

## 14. What needs Michael

1. **Waive or reset the `TIERS.md` ceilings for Stage 1.** Ceilings are $20 and 1 hour per campaign;
   L trials alone run 20 minutes to 3 hours. Stage 1 needs the per-trial caps in section 7 and a stage cap
   of $400 and 40 hours. This is the owner's rule; the Tier 0 full trials set the precedent.
2. **Approve the model** (recommended Sonnet 5.5 high for every driver and worker role) and the shortlist
   (S = Superpowers v7.0.0, O = OpenSpec v1.14.3 core).
3. **Approve building G1-G10** (about 14-17 agent-days, no paid model use), separately from running anything.
4. **Approve the Stage 1 budget:** about $240 and 23 hours (range $165-$365), hard cap $400 and 40 hours;
   Stage 1b (T3) gated on reading 1a.
5. **Provide labels:** about 12 delivered diffs for the grader (about 30 minutes) and about 40 owner
   questions if System 1 question-labelling is enabled (about 20 minutes).
6. Decide whether S should also run with `pi-subagents` (S+), and whether a real medium multi-file case
   should be added before Stage 1.

## 15. Review record

This spec had one independent review, `openai-codex/gpt-6.1-sol` at high thinking, as the owner chose
(session model guard checked: one provider/model, thinking `high`, 15 assistant turns, no Opus or Haiku).
Findings and dispositions: [stage0-review-record.md](stage0-review-record.md).
