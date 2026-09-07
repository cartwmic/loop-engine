# Recovery-run retrospective

Reviewed 2026-09-07 against v0.19.0 (`d756d74f00078a18fcb839674b7cee6055807407`). This is a retrospective and a set of proposals, not new product requirements. [The backlog](../BACKLOG.md) holds the remaining work; [PRD.md](PRD.md) remains the requirement-ID authority.

## Bottom line

Several custom scripts are worth extracting, but not as a new workflow engine:

1. **A reusable final-matrix runner, including command capture and verified resume.** This is the strongest project-utility candidate.
2. **Deterministic progress monitoring.** The owner subsequently requested coverage of software-change runs, invocations, bound/unbound graphs and fan-out, plus validation commands, without model-mediated status interpretation or presentation.
3. **Reliable native review-result export and recovery in pi-subagents.** That belongs to the harness, not Loop Engine core.
4. **Shared packaged-smoke and benchmark helpers**, where they eliminate existing duplication without freezing this run's versions or workload.
5. **The tag-to-canonical-skill sync utility in chezmoi**, not in Loop Engine.

Do **not** ship the run-specific report authors, blanket recovery scripts, `TASK_STATUS: complete` convention, old fixture copies, or the unexecuted PID-reuse simulation as general product machinery.

The run delivered most of its accepted scope, but did not establish clean whole-intent success. It ended **`completed-with-overrides`**, with four public overrides. The independent collection passed 13 criteria and failed **AC-4 and the whole-intent goal** on the accepted, unreproduced and unrepaired PID/PGID-incarnation concern. Publication did not turn those failures into passes.

## Evidence and review coverage

These external records remain outside Git:

| Locator | Meaning |
| --- | --- |
| `R` | `$HOME/.local/share/loop-engine/runs/run-1788563575729926000-1-37951` |
| Catalog | `$HOME/.local/share/loop-engine/loop.db` |
| `P` | `$HOME/.local/share/loop-engine/preflights/release-0.19.0` |
| `A` | `$HOME/.local/share/loop-engine/preflights/recovery-retrospective-2026-09-07` |
| Session | `$HOME/.pi/agent/sessions/--Users-cartwmic-git-loop-engine--/2026-09-04T21-12-30-543Z_01a06e44-144f-70df-91f8-9cfbba9bc303.jsonl` |

The review covered:

- Frozen `R/intent.json` r5, design r3 and plan r5; the 15-task/28-edge plan, with T00 and T14 driver-owned outside the worker graph.
- All **336** saved history entries, with targeted inspection of denials, dispositions, carries and overrides. `R/exception-completion/terminal-show.json` contains 295 context records, including 128 review-evidence records and 23 finding-ledger snapshots.
- The first **5,003 session entries**, ending before this retrospective request. Observable messages, tool calls/results and selected child sessions were inspected; private thinking was excluded.
- An inventory of **544 external code files** within the bounded scan in `A/external-code-inventory.json`. Most are generated fixtures or copied checkout scripts, not 544 distinct utilities. Important driver scripts were read directly; every generated copy was not independently audited.
- The final 46-command run matrix, document audits, benchmark evidence, release failures and corrections, successful hosted publication, and chezmoi deployment follow-up.

Grok and GLM supplied four fresh-context read-only audit slices. Three completed normally; the utilities audit file was delivered before its harness failed its turn budget. Its file and failed status are both retained. Parent reconciliation is necessary: for example, an audit incorrectly equated repository checkpoints with Package 11a's Git commit checkpoint. **11a remains open.** Likewise, the bootstrap's existing review carry is not proof that its frozen provider acquired v2 criterion enforcement.

Detailed audits are `A/{utilities,workflow,session,backlog}-audit.md`. Sol's subsequent challenge is `A/sol-challenge.md`. It identified two lost backlog obligations—fail-closed bypass recording and per-clause semantic audit scope—and an overstrong watcher recommendation. The parent restored both obligations and initially made watcher extraction conditional. The owner then requested broader deterministic progress monitoring, now reflected below and in the backlog. This document synthesizes the reviews and subsequent owner decisions; it does not adopt every suggestion or severity label.

## What the run actually established

| Accepted work | Evidence and disposition |
| --- | --- |
| Driver dispositions | AC-1 passed. Exact-source rejection/resolution/retirement, independent authors and unresolved blocking were exercised; original failures survived. |
| Steering / 9a | AC-2 passed. Later recipients, selected tasks, observable changed output and immutable launched commissions were exercised. |
| Execution controls / 10a | AC-3 passed. Future binding amendments and ordinary invocation controls preserved frozen policy and past attempts. |
| Cancellation / 10b | Public cancellation and its ordinary mechanics shipped. **AC-4 failed on process identity after a long interrupted-controller gap.** Keep that residual open. |
| Implement backtracking / 10c | AC-5 passed. All three report-free owning-phase routes and live-work refusals were exercised. The shared ownership-identity limitation still matters. |
| Explicit override / 10d | AC-6 passed. Named-edge attestations, original denials and permanent exceptional labeling were exercised, including in this run. |
| Criterion validation / 8b | AC-7/8 passed. Current IDs, separate goal judgment, invalid-coverage refusals, failure blocking and unaffected carry were exercised. The bootstrap's collection was driver-owned; new v2 enforcement was proved separately. |
| Proof scheduling / 11b | AC-9/10 passed. Bounded independent jobs, ordered shared-run work, retained coverage, comparable measurements and a designated final proof owner were established. |
| Composed recovery | AC-11 passed. A separate deterministic public-CLI run reached normal completion with zero overrides; an exceptional branch retained failure and override labels. This bootstrap was not that clean run. |
| History, docs and simplicity | AC-12/13 passed in their assessed scope. This was not an exhaustive semantic Bookends audit or proof that every inherited mechanism was optimal. |
| Batched reviews | AC-14 passed. Shipped guidance/opt-in bindings batch assigned axes per author/gate while retaining distinct verdicts and ordinary/challenge separation. Profiles remain unbound by default. |

Primary evidence: `R/reviews/validation-r5-exception-1/grok-judgment.json`, both ordinary reviews and Sol's challenge in that directory, `R/proof/receipts.json`, and `R/proof/software-change/recovery-composed-2vg6pohh/proof.json`.

### Closure happened in stages

1. The final run matrix passed 46 commands on one retained pre-commit tree. Genuine review still found the cancellation identity gap.
2. The owner accepted that gap and authorized exceptional completion. History retained the failure rather than manufacturing accepted implementation history.
3. Commit `45352dc` landed the recovery change. Hosted Linux then exposed additional defects; the pre-commit macOS matrix had not proved Linux correctness.
4. Release fixes landed in `ff4074d`, `19e6b07`, `36c11b1`, `5a0d3ce`, `7271f4a` and `6e99e12`. The final version commit is `d756d74`.
5. [Exact-commit preflight](https://github.com/cartwmic/loop-engine/actions/runs/34136892689) and [publication](https://github.com/cartwmic/loop-engine/actions/runs/34138511233) passed. [v0.19.0](https://github.com/cartwmic/loop-engine/releases/tag/v0.19.0) includes both native targets and packaged smoke. One non-failing nextest leak warning was retained; there is no zero-warning claim.
6. Chezmoi commit `fc16562b2376898aa0ff50919e55cf4b45730aa0` updated four pins and five canonical bundles. Targeted personal-profile apply, 75 file comparisons, 20 skill links and installed-binary packaged journeys passed. Existing sessions still needed refreshed PATH/skill discovery.

Thus Git, requirement-text commit, hosted/native proof, publication and pin/skill deployment are no longer pending. **The 138 calibration attestations, later ordinary v2 dogfood, the PID concern and independent acceptance review of the separate pi-subagents fixes remain outstanding.** Historical pending records should remain historical, not be rewritten.

## Utility extraction candidates

### 1. Final matrix execution and command receipts — first priority

**Source:** `R/run-final-matrix.py`; repeated release capture loops and `R/driver-corrections/capture-command.py`.

The concrete need is not another test runner. We already have `scripts/run-nextest.py`, `scripts/proof_pool.py`, the public journeys and `scripts/assert-implementation-report.py`. What is missing is their reusable **serial orchestration and receipt/resume wrapper**.

The original redirect-only matrix hid discovery progress. Owner interruption left the test group running and row 19 without a completed receipt. The repaired wrapper added live stdout/stderr, heartbeats, bounded abort cleanup, atomic status/receipt files and validation of a resumable successful prefix. Later release scripts still imported the capture helper through this particular run ID.

Extract the useful executor into a small repository utility with explicit matrix, checkout, output and environment inputs. Reuse the existing receipt checker. Remove the fixed 46-row invariant and this run's paths. Preserve failed attempts and require an explicit selection decision before retry; never relabel old identities or make semantic approval a process-exit side effect.

Do not separately ship the weaker 1 KB capture helper as a competing standard: it lacks the matrix runner's cancellation and live-output behavior. A one-command mode can share the same implementation.

**Acceptance before promotion:** real two-/three-command matrices must finish, fail without launching later rows, stream output, stop owned work on interruption, refuse stale/failed prefixes, and resume valid prefixes. Test detached descendants and Linux reaping explicitly: the existing abort probe covers a resisting child/grandchild in one group, not every ownership shape. Command completion, cleanup, receipt validity and reviewer acceptance remain separate.

**Owner:** repository utility. `software-change run-validation` already executes named proof commands as a CLI utility, but requires a validation-state packet and builds a criterion index. That does not directly replace the pre-implementation-review matrix. Provider evaluation itself must still not perform primary work.

### 2. Deterministic progress monitoring — broader than graph observation

**Sources:** `R/execution-preflight/{launch-graph,monitor-graph,guard-graph}.py`.

The session contains **830 bash calls mentioning `monitor-graph.py`**; this count includes setup/reference calls and is not an exact count of distinct status queries. It is enough to demonstrate excessive model-mediated polling. Short polls restored visibility after the owner objected to a foreground wait, but repeating them through the LLM is not the desired permanent design.

The custom observer served a **direct unbound utility graph**, which has no catalog invocation to inspect. The owner's subsequent request is broader: monitor software-change runs, general invocations, bound/unbound plan graphs and fan-out, and validation commands. Refresh the human display outside the model loop; notify the agent only on completion or when attention or judgment is needed. Reuse `show`, `invocation-progress` and structured captures rather than inventing another orchestration system. Keep workflow state, helper completion, worker outcome and review judgment distinct; missing information remains unknown.

Keep these roles separate:

- launcher retains the real process handle, captures and completion;
- observer reports facts; an optional local snapshot cache is not a workflow mutation;
- any stop controller needs explicit ownership and stop authority;
- the driver judges deliverables and requests workflow events.

Do not ship the current Dagu TTY-tree regex, private status-code assumptions, Pi session-body parsing or `TASK_STATUS: complete` header as engine policy. That header attested completion; it did not prove it. Prefer existing capture/conformance and provider-owned output contracts before adding a guard.

**Acceptance:** real bound and unbound graphs, unchanged polls, task completion/failure, missing/partial metadata and controller interruption. Observation must not mutate the catalog or auto-approve work. Stop proof must establish actual cleanup, not merely successful `dagu stop` exit.

### 3. Native review-result export/recovery — pi-subagents, not core

**Sources:** `R/bootstrap/capture-review.py`, `R/driver-audits/native-recovery/capture-batch.py`, `R/reviews/validation-r5-exception-1/capture.py`, and release review-capture snippets.

These are several versions of the same missing harness convenience: locate the selected attempt's output, verify its shape and subject/author/model identity, preserve the original status and return usable evidence without hand-parsing private runtime directories.

The fix must distinguish **execution state, output validity, harness acceptance and domain judgment**. A failed native run can contain a genuinely completed judgment, including a failing judgment. Conversely, a progress message, file's existence or exit zero does not supply one. Never drop every failed native step or convert a prose “pass” into a schema-conforming result automatically.

Prefer a supported pi-subagents result-export/status surface over maintaining three Loop-specific parsers. Remove `/var/folders/.../uid-501` assumptions. Resolve the `launchContractDigest` recovery-descriptor incompatibility there, not by editing descriptor JSON. Avoid asking a reviewer to satisfy incompatible structured-output and generic acceptance-report formats.

**Acceptance:** exercise the real public tool with scripted model/backend behavior: valid output followed by transport failure; no completed output; malformed/missing output; wrong subject; interrupted/revived attempt; and partial parallel success. Completed peers and original statuses must survive. No automatic evidence append or workflow approval.

### 4. Benchmark collector/comparator — useful if maintained

**Source:** `R/baseline/benchmark.py`, also delivered in T11's proof directory.

The useful part is repeatable collection, workload equivalence, job-limit/cleanup observations and honest arithmetic. This is not supplied by nextest or the cache proof. The pilot must remain non-accepting; missing measurements must remain unknown, not zero.

Do not copy the old T00 collector, freeze 22 rows forever, or make AST shape and monkeypatches into product contracts. Keep a versioned/caller-selected workload tied to the actual journey inventory. Promote this only with an owner for future comparable measurements; otherwise retain the current evidence and collector with the run.

### 5. Packaged smoke — share the existing sequence

**Source:** `P/packaged-smoke.py`, compared with `.github/workflows/archive-smoke.yml`.

The script verifies checksums, extracts archives and drives existing packaged journeys outside the checkout. Local archive and installed-binary follow-ups repeated the same sequence already present in CI.

A small shared utility is worthwhile: explicit archive/binary inputs, expected version, supported target and output root. CI and local use should call the same sequence. Remove the hard-coded Darwin target, `0.19.0` assertion and capture-helper dependency on this recovery directory. Do not add a second journey implementation or assume Generate-PRD has a packaged checker that is not distributed.

### 6. Skill vendoring — chezmoi

**Source:** `P/chezmoi/sync-source.py`.

This maps one immutable release tag's skills, docs and provider data into the established canonical layout, verifies bytes and refuses unexpected orphans. It replaces repeated copy instructions from earlier releases. Keep it in the dotfiles/tooling owner, not Loop Engine core or a second engine skill tree.

Acceptance includes immutable-tag verification, exact source/deployed comparisons, targeted apply, harness-link checks and installed-tool behavior. Preserve older installed versions needed by frozen runs. Refreshing configuration does not rewrite an already-running harness's inherited PATH.

### Keep run-specific authorship local

Do not promote `finalize-implementation-report.py`, `exception-completion/prepare-validation.py`, `driver-audits/finalize-audits.py` or summarizer repair scripts wholesale. They contain this run's criterion prose, exceptions, authors, matrices and paths. A mechanical report scaffold may reuse the existing checker, but a script must not invent criterion satisfaction or a whole-intent judgment.

`exception-completion/drive.py` is useful evidence of serial CLI use and per-edge attestation, not a general “recover anything” command. Empty blocker-ledger projections were specifically authorized bootstrap workarounds, not a normal pattern. The unexecuted PID script is a proposed simulation, not an observed OS-reuse reproduction; preserve its blocked status and do not silently repurpose it as acceptance proof.

## Problems and lessons, by owner

| Observation | Current status | Correct follow-up |
| --- | --- | --- |
| Resolved findings' original evidence rejected as stale | Fixed in recovery delivery. Denials at history 51, 143, 188, 226; BE records/projections at 52–53, 144–145, 189–190, 227–228. | Keep lineage and regression; retire the workaround from ordinary practice. |
| Implementation override skipped creation of accepted proof history | Intentional per-edge behavior. Later denials 315, 327, 334 required exceptions 316, 328, 336 after the first override at 314. | Explain this concrete downstream consequence in existing skill/denial guidance. Do not fabricate history or add automatic exception carry. |
| Headless compaction/continuation killed prematurely | Separate pi-subagents fixes `0fe2a797611bba9fe2fbe638c01e653489a14bd2` and `886c6af05f24ec435ce3aeb461de0fb8584a7c89` published. | Keep real foreground/background host-teardown regressions; independent acceptance remains separate. Do not disable compaction to hide it. |
| Intermediate recovery handle rejects `launchContractDigest` | Still an external harness issue. Original handles sometimes resumed while derived handles refused. | Fix descriptor-version compatibility in pi-subagents; never edit runtime descriptors to force admission. |
| Idle alerts and turn limits do not describe deliverable validity | Repeated during the run; some complete outputs survived failed harness statuses. | Inspect actual state/output, use live steering when appropriate, and bound packet scope. Do not blindly retry or merely raise all thresholds. |
| Matrix interruption hid output and left an owned group | Run-local runner corrected and tested. | Extract the reliable executor; show the running phase and idle duration. |
| macOS discovery stalled in a pathological `target/debug/deps` directory | Diagnosed using byte-identical executable comparison; owner approved a fresh target. | Diagnose before cleaning. A fresh target is a recovery option, not a per-session default that discards cache value. |
| Final report rows disagreed with checker/template expectations | Corrected; current profiles/checker use closed `{proof, criterion_id?}` rows. | Reuse current schemas and checker; do not keep a second run-local success-string inventory. |
| Linux test setup spent heavily in repeated unoptimized hashing | Test-profile SHA optimization shipped; checks remained intact. | Profile the hot path before altering deadlines or removing integrity checks. |
| Linux `kill` parsed a negative operand ambiguously | Reproduced on Ubuntu 22.04/procps 3.3.17; `--` fix shipped. | Preserve that public timeout regression and platform evidence; it is not the PID-incarnation fix. |
| Proof-pool Linux subreaper setting leaked into later work | Restoration fix and six state/outcome cases shipped. | Scope process-global settings to the operation; test a later unrelated workflow in the same driver process. |
| Long composed test crossed its deadline under contention | Slot reservation shipped; global four slots and 60-second limit retained. | Use measured scheduling changes or sensible test decomposition, not automatic timeout inflation. |
| Linux-specific failures appeared only after commit | Fixed during release, but costly to discover serially in CI. | Run focused native-Linux probes early for Linux-specific process behavior, then still require complete hosted proof. |
| Current docs still contain pending acceptance/commit/audit labels | Wording accepted and committed; some labels are now stale. | Verify accepted patch against committed bytes, then do a narrow documentation reconciliation. Do not rewrite old run reports. |

The provider refusal prevented the proposed PID reproduction from executing. The owner chose disclosure and exceptional completion; no rephrased/provider-switched execution was used to evade that restriction. The concern remains open.

## Practices to retain

- Freeze the exact selected profile and keep the separate owner-approved role/model manifest honest. Use existing execution provenance rather than copying model/command metadata into every human record.
- Evaluate existing `run-plan-graph` and invocation tools before building a chat dispatcher. One checkout writer does not prohibit parallel isolated proofs or read-only reviewers.
- Preserve completed tasks and reviewer peers. Retry the selected unfinished work only after quiescence; do not replace an unfavourable genuine judgment with another author.
- Use comprehensive first review, focused confirmation and explicit unaffected carry. Supply exact inputs and output contract instead of asking reviewers to discover their assignment.
- Give one driver ownership of the final stable-tree proof matrix. Workers run focused checks; reviewers inspect retained outcomes. Do not relabel old receipt identities after changes.
- Distinguish build, test discovery, execution, cleanup, artifact conformance and semantic acceptance. Print the failing layer's actual diagnostics.
- Keep raw failures, interrupted attempts and owner decisions. A durable error plus a later fix is better evidence than a cleaned-up success story.
- Prove through the same outer CLI/tool/packaged path the user drives. Unit seams and synthetic verdicts do not establish semantic review quality.
- Hold publication for the exact release commit's hosted gates. Local macOS proof is not Linux proof, and configured CI success is not proof that the backlog is empty.

## Measurements that can be stated honestly

- Final run matrix: **46 passing commands**, **1,593.66 seconds** on its recorded pre-commit tree. Failed/interrupted earlier attempts remain retained.
- Independent workload: three samples per mode, 22 rows per sample. Median predecessor serial **76.55s**, current serial **85.18s**, current parallel **47.78s**. Parallel was **37.6% lower than the predecessor** and **43.9% lower than current serial**. Current serial regressed. These are historical measurements of that workload, not universal or final-release speed guarantees.
- The expanded software-change source journey took longer than T00; no whole-journey speedup was established. Original final local cache counters were zero; later hosted cache hits do not retroactively change them.
- v0.19.0 exact-commit preflight reported **931 seconds**, 1,120 passed tests, two deliberate skips and one non-failing leak warning. Publication separately passed both native archive-smoke jobs.
- Session monitoring count is stated as tool-call mentions, not money saved or CPU time. Per-attempt/compaction usage exists, but no reliable aggregate model-cost saving is claimed.

## Backlog reconciliation and next decisions

The old backlog mixed delivered candidates, a historical execution queue and genuinely open work. At the owner's request, the backlog now contains concise open work only. Delivered history stays in Git; evidence and explanation stay in this retrospective and its referenced records. The full old charter remains in [the v0.19.0 backlog snapshot](https://github.com/cartwmic/loop-engine/blob/d756d74f00078a18fcb839674b7cee6055807407/BACKLOG.md).

Important boundaries:

- Cancellation remains **delivered with an accepted open identity gap**, not completed without qualification.
- Package **11a is not delivered** by `implementation-checkpoint.json`: a repository checkpoint is not a driver-owned Git commit checkpoint. The separate post-commit run-to-commit pointer is also not a standardized product capability merely because this release retained ad-hoc manifests.
- The shallow-CI continuity and local bypass-recording candidates remain open despite publication. The exhaustive semantic Bookends audit, circular-proof calibration, policy-document finding delivery, CLI-friction batch and PRD-migration work were not completed by this run.
- Generalized replay stays deferred until ordinary v2 use demonstrates a residual need. Do not use this bootstrap's special authorization as future standing permission.
- Other active catalog runs require their own owner decisions. This retrospective did not clean up or terminate them.

The strongest utility candidates are the matrix/capture runner and the owner's requested deterministic progress monitor, followed by native harness result/recovery fixes. Narrow documentation/profile visibility improvements and ordinary v2 dogfood also remain open. The accepted PID gap is a distinct owner decision with its own reproduction/correction proof. These are proposals for the next intent, not authorization to implement them now.
