# Changelog

## [0.72.0] - 2026-11-27

A release can no longer be tagged when the dotfiles pin bump would fail on it, and the `append` help names an origin the tool accepts.

- release.yml and scripts/check-downstream-bump.sh: every build job starts with a gate step that reads the dotfiles default branch and runs the importer's `check` mode on the commit being released, so a release that removes a file the importer still lists, leaves an orphan in a skill bundle, or breaks a pin stops before the tag with the file or pin named. (#238)
- loop-engine append: the help text names an origin the tool accepts. (#240)

## [0.71.3] - 2026-11-27

Code that nothing calls is removed, and `bookends-check` handles annotated-tag pushes.

- loop-core, loop-integrations, formal/: removes 838 non-test lines that no running program calls (fan-out recovery types, the retired carry request, the advice command contract, wrappers only tests used, the SQLite single-invocation reader and half of `proof_metadata.rs`), the `jsonschema` dependency only they used (74 fewer `Cargo.lock` packages) and the schema file only they read. No command, option, output or stored table changes: the help text of every command, the schema of a new database, the six boundary journeys and the formal model give the same results before and after. (#231)
- scripts/behavior-snapshot.py: prints one result line per check (format, lint, build, tests, journeys, help-text and schema hashes, line and package counts, and a search for names that should be gone) so a change can be compared before and after. (#231)
- bookends-check: a pushed annotated tag with a tip finding now blocks, because the tip is matched by the commit the tag points to; every check and candidate run appends one JSON line to a log in the state directory. (#232)
- bookends-check: the log line of a failed `check` run names the commit that was checked, the line of a failed `candidate` run names the file, and both carry the text of every finding; `--help` names the fallback log location. (#236)

## [0.71.2] - 2026-11-24

The formal check no longer fails main on a GitHub rate limit: it retries and caches its statechart and TLC downloads.

- ci.yml and formal/: the formal check fetches its statechart and TLC jar through a script that retries failed downloads and caches the downloads in CI, so a rate-limited download no longer fails `checks` on main. (#228)

## [0.71.1] - 2026-11-23

The Bookends range is defined in one place and guarded by a test, and the overlapping-attempts race has a TLA+ model that CI checks.

- docs: the range definition lives only in step 1 of docs/culprit-hunt.md and the docstring of scripts/bookends-base.py; ci.yml comments, AGENTS.md and docs/PRD.md point to it; a test in scripts/test_bookends_base.py fails when another file restates which runs the range skips, and it runs in the checks job so it gates every PR. (#221)
- scripts/test_bookends_base.py: the range-restatement test skips CHANGELOG.md, which records history. (#224)
- formal/: a TLA+ model of the overlapping-attempts race, and a CI step that fails `checks` when the model breaks. (#218)
- formal/: the formal check fails when a source file the model covers changes. (#222)

## [0.71.0] - 2026-11-23

A queue of merges to main runs the full suite once, and no merged commit goes unchecked.

- ci.yml: a newer push to main cancels the push run before it; the Bookends step of the run that finishes checks every commit since the last push run whose own Bookends step finished (found by scripts/bookends-base.py), so a cancelled run leaves no commit unchecked. A release-prep commit keeps its own, never cancelled run, because release.yml needs a green push run on that exact commit. (#219)

## [0.70.2] - 2026-11-23

Docs only: the merge-queue rule in AGENTS.md and the release procedure.

- AGENTS.md and docs/release-procedure.md: read the full suite once at the end of a merge queue, not once per merged change. (#213)

## [0.70.1] - 2026-11-23

Tests no longer fail when a slow CI runner misses the engine's read budget.

- Tests that run a bounded `show` or `read` rerun when the engine reports a missed read budget, for up to 30 seconds, so a starved CI runner no longer fails them. The product's 1.1 s budget is unchanged. (#211)

## [0.70.0] - 2026-11-23

The eval harness gains the lean-change arms, and images follow the Docker daemon's own chip.

- evals: lean-b and lean-c arms (the lean-change provider, profiles b and c) with their prompts, the lean image layer (image/Dockerfile.lean, which builds loop-engine and lean-change from the checked-out source), lean metrics that count the intent check, and a scripted lean driver; lean-campaign-kgate.sh runs the scripted campaign for both profiles as one kgate Docker job, part of issue 61 (#166).
- evals: images are built and run for the Docker daemon's chip (amd64 or arm64) instead of a fixed linux/arm64, an image built for a different chip is refused, and the chip is recorded as `architecture` in the image check and results (#166).

## [0.69.0] - 2026-11-23

Follow-ups to the eval reader step: rescores and case checks no longer fail for reasons the delivery did not cause, and case-check logs each run.

- evals: the reader step builds its model only for uncached texts, refuses a non-UTF-8 guide at load, counts failed readings for this run only, scores a failed forms.json write as an infrastructure failure, and snapshot replay refuses a case with a [reader] (#200).
- evals: case-check decodes collector output with replacement characters, exits 3 on stale saved readings, and appends one JSON line per run to $XDG_STATE_HOME/loop-engine/case-check.jsonl (#200).

## [0.68.0] - 2026-11-23

The release script waits for the CI run the release workflow checks, and stops when its own copy is out of date.

- scripts/release-after-merge.sh: before dispatching release.yml, waits until the same run search the workflow's exact-commit CI gate uses returns the green push run, logging each miss; a failing search stops the release (#206).
- scripts/release-after-merge.sh: stops at step fetch, changing nothing, when its own file differs from origin/main's, and prints the fast-forward command (#202).

## [0.67.0] - 2026-11-22

Finding triage reads the same in every skill, and eval skips must give a listed reason.

- research and policy-document skills: a candidate fails only on findings that fail a test (any of the four); an in-intent finding is not declined but moved to a follow-up (#199).
- scripts/test_triage_skills.py: pins the triage text the four skills share; it runs in ci.yml's tests job and scripts/baseline.sh (#199).
- evals: the proof runner accepts a skip only with a reason listed for that test's file, and fails the unit on any other skip; the software-change target writes reports to `SC_RESULTS_DIR` when set, so the check-in test leaves the checkout clean (#198).

## [0.66.0] - 2026-11-22

The release script can prepare a release on a host with no Rust toolchain, by running the prepare step on kgate.

- scripts/release-after-merge.sh: with `RELEASE_PREPARE_CMD` unset, the prepare step runs `scripts/prepare-release.sh` when cargo is on PATH, as before. With no cargo and kgate on PATH, it runs the new `scripts/prepare-release-kgate.sh` (#195).
- scripts/prepare-release-kgate.sh: runs `prepare-release.sh VERSION` as a kgate job on the `kgate-verify.toml` profile, with cargo-dist at the version `mise.toml` pins, and copies `Cargo.toml` and `Cargo.lock` back. A kgate exit 69 or 75 stops the step as a job that did not run; any other failure stops with its exit code. Nothing is copied back unless the job exited 0 (#195).
- docs/release-procedure.md describes both routes (#195).

## [0.65.0] - 2026-11-22

The software-change scorer can read texts with a model before it scores.

- evals/software-change: an optional reader step. A case with a `[reader]` table has the scorer read the texts a delivery writes, with a pinned model, and score from those readings. A case with no `[reader]` table scores exactly as before (#190).
- `read-texts` saves the readings beside the capture and reuses them on a rescore. `case-check --save-forms` writes a case's saved forms, and stale saved readings are INVALID until it is run again (#190).
- A model call that still fails after its retries is an infrastructure failure (exit 3), not an empty reading. A text with no usable vote keeps its readings and text, and the failure names its id (#190).
- anthropic is pinned to 1.17.0 for the reader, which sends `cache_prompt=False` (#190).
- run_proof_groups.py registers test_intent_checkin_trial.py and test_owner_context.py (#190).

## [0.64.0] - 2026-11-21

The release script runs the merge gate before it merges a release-prep PR.

- scripts/release-after-merge.sh: just before it squash-merges the release-prep PR, it runs `intent-review-check cartwmic/loop-engine <PR>`. A failing check, or a missing command, stops the release at the merge step with the check's output and a resume hint; nothing is merged. `INTENT_REVIEW_CHECK` overrides the command (#188).
- docs/release-procedure.md and AGENTS.md describe the new step (#188).

## [0.63.0] - 2026-11-21

The kgate profiles pin the current kgate base image, which has jq.

- kgate.toml and kgate-verify.toml: the image pin moves from `41dc1792…` to `9d1f6308…`, the newest homelab kgate base image. It has jq 1.7.1, so the nested lean-change scenario no longer downloads jq. Rust, Python and Node versions are unchanged (#186).
- scripts/lean-change-journey.py: the nested scenario's message no longer says the kgate image has no jq (#186).

## [0.62.0] - 2026-11-21

Owner-decision and nested-run docs, and a CI fix so PR checks run Bookends on their own range.

- ci.yml: PRs now run the Bookends gate over the commits they add on top of main, so a requirement edit that Bookends reads as a changed identity fails the PR's own checks instead of turning main red. A manual run on main checks its tip commit from its first parent (#182).
- docs/agent-usage.md and the lean-change skill: for an approval in the owner's own words, put the ask id in `reason`. Never put it in `text`, and never add a top-level `source`: the engine rejects it as an unknown field and refuses `approved` with `owner-decision-missing` (#184).
- docs: a nested child may start, explore and draft its intent before the parent reaches implement. Its intent quotes a parent criterion only once that criterion is final (#184).
- docs: commit before `lean-change verify`, because the tree fingerprint includes HEAD (#184).

## [0.61.0] - 2026-11-21

A duplicate start is refused clearly, `terminate` takes a reason, and a manual CI run can prove a fix PR when main is red.

- `loop-engine start` with a run id that already exists is now refused with status `rejected`, code `run-already-exists`, exit 10 and a message that names the id. It was `error` / `persistence-failure` / exit 20 with the raw `UNIQUE constraint failed` text. The existing run is unchanged (#177).
- `loop-engine terminate RUN [REASON]` takes an optional reason. It is stored on the termination history entry and `show` displays it as `termination_reason` in every view and as a `terminated:` line in the default text view. Without a reason nothing changes, and runs terminated before this still load. The catalog schema does not change (#178).
- ci.yml: new `workflow_dispatch` trigger. `gh workflow run ci.yml --ref BRANCH` runs `checks`, `tests` and `release-tests` on a branch head, with Bookends over the commits the branch adds on top of main. A manual run on main refuses, because it would check an empty range. This gives a fix PR the full-suite run the red-main merge gate needs (#179).
- docs/PRD.md: LE-214 records the terminate reason. LE-15's title also carries it, because #178 extended that title and Bookends reads a changed title as a changed identity (#179).
- These came out of a real nested-runs proof: a parent run and two child runs, one per fix, each approved on its own.

## [0.60.0] - 2026-11-20

## [0.60.0] - 2026-11-20

Nested runs: split a plan too big for one run into a parent run and child runs, with guidance only. The engine does not change.

- docs/agent-usage.md: new "Nested runs" section. A parent is an ordinary lean-change run whose part checks read each child run's state with `loop-engine show --view full` and `jq`. It gives the two check lines (the owner's own words, or delegated allowed), the `child-run` and `parent-run` link records, the child's first constraint quoting its parent criterion, and how a child starts, reports, retries and is replaced. The machine that runs `lean-change verify` on a parent needs `jq`.
- using-loop-engine, using-lean-change-provider and coordinating-loop-engine skills point to the new section.
- scripts/lean-change-journey.py: new `nested` scenario, a parent with two children walked through the real CLIs, reading both check lines from the docs. It needs `jq` and fails with a clear message when it is missing. (#174)

## [0.59.0] - 2026-11-20

- lean-change docs: "the owner" means the user. When the user's instructions name someone else to approve a repo's intents, that approver records a standing-grant delegation citing those instructions and quoting the user's words (#172).

## [0.58.0] - 2026-11-20

Finding triage in the provider skills: four tests, and name the one a finding fails.

- using-lean-change-provider, using-loop-engine, using-research-provider and using-policy-document-provider skills: each says to triage every review finding on four tests (real, useful, maintainable, in intent) and to name the failed test (`real:`, `useful:`, `maintainable:`, `in intent:`). A finding that fails "in intent" goes to a follow-up, not a decline. Each points to `finding-triage.md` when it exists. (#167)

## [0.57.0] - 2026-11-20

PRD prose check follow-ups: hidden prose, clearer labels, and the check now ships with its skill.

- using-generate-prd skill: `check-prd-prose.py` moved to `skills/using-generate-prd/scripts/`, so it ships with the skill and in `data-dump`. It now fails with the new label `hidden` when an accepted line appears in the converted file only inside an HTML comment or a code fence. A dropped list item reports as `dropped`, a converted line is named "closest" for one accepted line only, and a leading byte order mark on either file is ignored. Exit codes are unchanged. AGENTS.md lists its tests among what `baseline.sh` and the `tests` job run. (#168)

## [0.56.0] - 2026-11-20

Packet wording, a generate-PRD conversion section with a prose check, and documentation pointers.

- lean-change: the intent-check and review packets print each question as `- id: use` with its text on the next line, and the answer template asks for the question id only (`"question": "<question id, e.g. use>"`). The parse refusal quotes the same template. The using-lean-change-provider skill now says the engine records one intent check per run and one review per intent, each the first well-formed one. (#121 was partly out of date; engine behaviour unchanged.) (#164)
- using-generate-prd skill: new section "Converting an accepted PRD", and `scripts/check-prd-prose.py`, which fails when an accepted line is dropped, changed or moved in the converted file and allows added lines. Its tests run in the `tests` job and `scripts/baseline.sh`. (#162)
- Release docs point at each other once: `scripts/release-after-merge.sh`'s header and AGENTS.md's authority list name docs/release-procedure.md. The policy-document and research crate guides say the release dry run's archive check is the required archive proof and `scripts/packaged-smoke.py` is optional. (#161)

## [0.55.0] - 2026-11-17

Instruction text, the lean-change skill's clean-up block, and its journey; no engine or provider code changes.

- The step-worktree clean check now counts files git ignores: a step worktree is clean only when `git -C <worktree> status --porcelain --ignored` prints nothing. The lean-change skill's clean-up block refuses with `not clean: <step>` otherwise, so `git worktree remove` can no longer delete ignored files silently. AGENTS.md (grant 51), the skill and PRD LE-211 say the same. (#159)
- Before the release script's stash is dropped, it is checked with `git stash show -p --include-untracked`, which shows the contents, not only file names. (#159)
- The lean-change journey's parallel scenario covers a step holding only an ignored file. (#159)

## [0.54.0] - 2026-11-17

Instruction and documentation text only; no code changes.

- AGENTS.md: grant 51 (ask #1289) lets a lean-change step worktree and branch be removed once they are merged to main and clean, after checking both. This replaces the rule that all removals stay with the owner for that case. Requirement text can be decided by the lead as a two-way decision under playbook a4c181c4; a generate-PRD candidate still needs a human to accept it. A fourth deletion exception covers the release script's own cleanup. Code work uses lean-change profile b unless the approved plan names c. The Merge row gives the gated merge command. Citations corrected (90fe4b7b, grant 46 exclusions, #847). (#157)
- docs/release-procedure.md, docs/agent-usage.md, docs/PRD.md (LE-211) and the lean-change skill: wording brought in line with AGENTS.md; "driver" and "run driver" used consistently. (#157)

## [0.53.0] - 2026-11-17

Instruction text only; no code changes.

- AGENTS.md: creating a worktree or branch follows the recorded Git checkpoint decision; removing one is deleting and goes to the owner (culprit-hunt and release worktrees excepted). The checkpoint decision now names who stages, commits, pushes and opens the PR. Run starts follow playbook f3404f07; intent approvals, model-binding amendments, deleting, buying things and the work Mac stay with the owner. The release-worktree exception cites grant 19 and playbook d9fa9f06. (#155)
- lean-change skill, Parallel builders: defers to the repository's own rules on commits, worktrees and deleting; command blocks unchanged. (#155)

## [0.52.0] - 2026-11-17

- AGENTS.md: builders no longer ask for steps a standing grant or playbook rule already covers (merge and release under grant 46, Git steps under the recorded checkpoint decision). Destructive steps, deleting, buying things, the work Mac, run starts and intent approvals stay with the owner. Agent-run spend is reported, not asked about. Links the design principles. (#153)
- docs: `gh run watch` examples poll with `--interval 60`. (#153)

## [0.51.0] - 2026-11-16

lean-change: a malformed review or intent-check record is now refused when it is appended (profile b), with an error naming the fix, using the same check the gate runs. A plain-string `reviewer` is read as an agent of that name. New `lean-change review-check [--kind review|intent-check]` checks a record on stdin before you append it; both review packets name it. kgate profiles request 8Gi of ephemeral storage (#151).

## [0.50.1] - 2026-11-16

lean-change skill: intent-check step now tells the driver to fix every promise in a quoted sentence, not only the word (#149).

## [0.50.0] - 2026-11-15

### Changed

- `scripts/release-after-merge.sh` runs `ci-main-status loop-engine` before it merges the release-prep PR (#147). Exit 0 (green) or 2 (untested) goes on. Exit 1, any other exit, or a missing `ci-main-status` stops at step `merge` with no merge, quoting the tool's output; a rerun after main is green resumes and merges.

### Compatibility

- Only the release script, its tests and the release docs change. The engine, the providers, the profiles and the record formats are unchanged. Running the release script now needs `ci-main-status` on PATH.

## [0.49.0] - 2026-11-14

### Changed

- The lean-change skill's optional outcome-map link block publishes to shartifacts (`POST /api/artifacts`, kind `html`) and prints the `/view/<id>` link, instead of calling `viz publish`. It still runs only when the owner asks for a link (#145).

### Compatibility

- Only the lean-change skill text, the LE-212 PRD sentence and the skill journey change. The engine, the providers' commands, the profiles and the record formats are unchanged.

## [0.48.1] - 2026-11-13

### Fixed

- The lean-change skill's "Heavy checks run on kgate" block (#141) now says what `verify --runner kgate` does when a check's time limit runs out before kgate starts the check: the check is "not run" and no record is made. A timeout after kgate starts the check is a timed-out result, as before.

### Compatibility

- No code change. Release archives, the engine, the providers, record formats and workflow text are unchanged.

## [0.48.0] - 2026-11-13

### Added

- `lean-change verify --runner kgate [--kgate-profile FILE]` (#139) runs each criterion check on the cluster through kgate. The check digest, the old-tree runs and the `old` expectations are unchanged. A kgate job that did not run the check (exit 64, 69, 75, 130, 143, no exit code, or a time limit that ran out before kgate began the check) makes no verification: it is neither a pass nor a failure. kgate's own lines are left out of `output_tail`. The repository carries `kgate-verify.toml`, a 4-CPU profile.

### Compatibility

- `--runner host` is the default and behaves as before. Release archives, the engine, record formats and workflow text are unchanged.

## [0.47.0] - 2026-11-12

### Changed

- Release scripts (#137): `mise.toml` pins cargo-dist 0.32.0 and `generate-release.py` runs it through mise (falling back to `dist` on PATH). `scripts/release-after-merge.sh` commits the prep commit under its own git identity when none is set, cleans its own leftovers on resume and stops with the exact command for anything else, and ends by naming `loop_engine_bump.py vX.Y.Z`.

### Compatibility

- Release archives, the engine, the providers, record formats and workflow text are unchanged.

## [0.46.2] - 2026-11-12

CI split: PRs run checks only; tests and release-tests run on push to main (ask #847); red-main culprit hunt in docs/culprit-hunt.md.

## [0.46.1] - 2026-11-11

### Fixed

- `scripts/release-after-merge.sh` now reads the prepared release diff with explicit `a/` and `b/` prefixes (#133), so its check that only version lines and new CHANGELOG text changed no longer stops on a machine whose git config sets `diff.mnemonicPrefix`.

### Compatibility

- Release archives, the engine, the providers, record formats and workflow text are unchanged.

## [0.46.0] - 2026-11-11

### Added

- Releases now publish all five archives (`loop-cli`, `bookends-check`, `lean-change-provider`, `policy-document-provider`, `research-provider`) for Linux arm64 (`aarch64-unknown-linux-gnu`, #131), built natively on the `ubuntu-22.04-arm` runner and gated by the same archive version check before host. Asset names follow the existing `<app>-<target>.tar.xz` pattern.

### Compatibility

- The macOS arm64 and Linux x86_64 archives are unchanged. The engine, providers, record formats and workflow text are unchanged.

## [0.45.0] - 2026-11-09

### Added

- A delegated approval may reword criterion statements. The delegation takes an optional `rewrites` list of `{criterion, field, before, after, reason}` entries (`field` is `statement`, the default, or `check`). lean-change now compares the goal, the constraints and the criterion ids with the baseline (`delegation-changes-approved-outcome`, as before) and lets a statement change only when an entry names it with the exact old and new text and a reason. A declared check rewrite must match too; a check change still needs no entry. A mismatch, a duplicate, an entry for something unchanged, or an undeclared statement change is refused with the new `delegation-rewrite-mismatch`. A delegated approval whose rewrites hold is a valid baseline for the next intent. The outcome map shows each rewrite.

### Changed

- A delegated approval that changes a criterion statement with no `rewrites` is now refused with `delegation-rewrite-mismatch`; it was `delegation-changes-approved-outcome`. `differs` now lists `criteria` only when the ids differ.

### Compatibility

- Records without `rewrites` parse and serialize as before. The workflow text, the profiles and the other record formats are unchanged.

## [0.44.1] - 2026-11-08

## [0.44.1] - 2026-11-08

### Changed

- Repository CI profile only (#125, #126): the kgate baseline profile asks for 5.5 GiB of memory (was 3.5 GiB), because the baseline's working set is 3.6 to 4.5 GiB and the old request sat below it. CPU stays 12 and the memory limit stays 8Gi. Measured run on the cluster: exit 0, working set 4.56 GiB.

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.44.0] - 2026-11-07

## [0.44.0] - 2026-11-07

### Added

- Delegated owner approval (#123, #122). An `owner-decision` record may carry a `delegation` (`basis` `approval` or `standing-grant`, `source`, `quote`, `covers`) so a lead or driver can record an intent approval under an approval the owner gave earlier or a standing grant, quoting the owner's words. lean-change checks it at `approved`, `approved-hard` and `verified`: `covers` must equal the intent's criterion ids (`delegation-not-covering`), and the revision must not add, drop or change the goal, the constraints or any criterion statement of the latest validly approved earlier revision (`delegation-changes-approved-outcome`). A refused delegated approval never counts as a baseline. The outcome map says when an approval was delegated. `docs/agent-usage.md#delegated-approval` has the rule, the record, an example and a table of every place that needs the owner's words. The skills and guides that asked for the owner's own words point to it and keep the fallback: ask the owner when no delegation covers the step, or the step adds, drops or changes an approved outcome.

### Compatibility

- A decision without `delegation` is evaluated exactly as before. The workflow text, the profiles and the other record formats are unchanged, so runs already open keep working. Model-binding amendments still need the owner's own words.

## [0.43.5] - 2026-11-06

### Changed

- Repository guidance only (#116): AGENTS.md says each proof run needs a fresh, empty proof root; adds boundary rows for journey and driver scripts and for root `skills/`; names the release dry run's archive check as the required archive proof for packaging changes, with `packaged-smoke.py --mode archive` optional and `--mode installed` for installed executables only. The release procedure moved to `docs/release-procedure.md`, unchanged in meaning; AGENTS.md keeps a short pointer section.

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.43.4] - 2026-11-06

### Changed

- Repository guidance only: AGENTS.md explains the kgate baseline run and its handoff (#114).

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.43.3] - 2026-11-05

### Changed

- Repository tooling only: CI and the baseline run the release-after-merge Python tests in 8 parallel shards through `scripts/run-release-tests-sharded.py` (#112).

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.43.2] - 2026-11-05

### Changed

- Repository tooling only: `kgate.toml` runs the baseline with 12 CPUs instead of 4, and its stale comment is fixed (#110).

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.43.1] - 2026-11-05

### Changed

- Repository guidance only: AGENTS.md now runs the baseline as `run-and-wake -- kgate -- scripts/baseline.sh` (#108).

### Compatibility

- No change to the engine, the providers, bookends-check, the profiles or the record formats.

## [0.43.0] - 2026-11-05

### Added

- Releases now publish `bookends-check` archives for macOS arm64 and Linux x86_64 (#106), so the PRD and journey coverage gate can be installed like the other binaries. `bookends-check --version` prints its version.

### Compatibility

- `bookends-check --repo` behaviour, exit codes and output are unchanged. The engine, the providers, the profiles and the record formats are unchanged.

## [0.42.0] - 2026-11-05

### Added

- `kgate.toml`: the profile for running the baseline (`scripts/baseline.sh`) on the personal homelab cluster through kgate (#104). It was proven against the Mac at v0.41.0 (same PASS, 705 Rust tests and 43 Python tests). The engine, the contracts and the workflow in `AGENTS.md` are unchanged.

## [0.41.0] - 2026-11-04

### Fixed

- Lean-change review and intent-check records with `findings_schema: 1` accept the shared findings contract's optional `head` and `brief_sha256` fields (#102). A reviewer's contract-shape return can be appended unedited. No gate reads either field.

### Compatibility

- Records without `findings_schema` that carry `head` or `brief_sha256` are refused, as they were before. Everything else reads and gates as in 0.40.0.

## [0.40.0] - 2026-11-04

### Changed

- Lean-change review and intent-check findings use the shared finding fields of `findings-contract.md` v1 (#100). A finding has `severity` `blocking` or `note`. A blocking finding stays blocking only with a nonblank `obligation`, `evidence` and `consequence`; otherwise it is read as a note, with the reason recorded. Notes need no disposition. Every blocking finding names its `component`. The conformance fixture is vendored and pinned.
- The review packet gains "Do not invent. Pass every question if nothing material remains", the short form of the bar, and a seventh review question, `smaller-design`: "Is there a materially smaller design that meets the intent, including what the systems you call already do? Name it with evidence. If no materially smaller design exists, pass." The intent check keeps its six questions. The `edge-cases` question now asks for cases a user would likely hit.
- A v1 review with no effectively-blocking finding accepts a later edit after re-verification, without a fixed finding.
- The outcome map shows notes apart from blocking findings and labels a demoted finding with its reason. The lean-change skill points further review rounds to the shared reviewer brief and the review-round rule.

### Compatibility

- Review and intent-check records carry `findings_schema: 1` with `severity`; records without it read as before. A reviewer using the new packet returns v1.
- A record without `findings_schema` is checked against the original six review questions, and gates exactly as in 0.39.0. Such a record that carries `severity`, `obligation` or `component` is refused.

## [0.39.0] - 2026-11-01

### Fixed

- `cargo test --workspace` no longer leaves a full debug build in `$TMPDIR/loop-engine-workspace-binaries-*` after each run. The workspace-integration tests now remove the fresh target they build when the test process exits (#98). A run killed by a signal can still leave one.

### Compatibility

- Test harness only. The engine, the providers, the profiles and the record formats are unchanged.

## [0.38.0] - 2026-10-31

### Changed

- Lean-change `outcome-map` uses plain-English labels, first line, field names and findings heading (LE-212, #96). The lean-change skill tells the driver to write the intent and Accountable Review's page for a reader new to the project and to apply the no-ai-slop skill.

### Compatibility

- Lean-change output wording and its skill only. The engine, the other providers, the profiles and the record formats are unchanged.

## [0.37.0] - 2026-10-31

### Added

- Lean-change has `outcome-map`, which makes one page for the owner from a finished run: Accountable Review's page for the change with a section, "What was approved and what ran". The section shows each criterion's label, its check results and old-tree runs, and each finding with its disposition. The command prints HTML and writes no file; `--into` puts the section into AR's page at its anchors (LE-212, #94).
- The lean-change skill's end step makes that page. It runs AR's review-map skill when it is installed and the tree is clean, then opens the local file. It publishes the file with `viz` only when the owner asks for a link.

### Compatibility

- Lean-change gains one read-only command. The engine, the other providers, the profiles and the record formats are unchanged.

## Unreleased

### Added

- Lean-change has `outcome-map`, which makes one page for the owner from a finished run: Accountable Review's page for the change with a section, "What was approved and what ran". The section shows each criterion's label, its check results and old-tree runs, and each finding with its disposition. The command prints HTML and writes no file; `--into` puts the section into AR's page at its anchors (LE-212).
- The lean-change skill's end step makes that page. It runs AR's review-map skill when it is installed and the tree is clean, then opens the local file. It publishes the file with `viz` only when the owner asks for a link.

### Changed

- lean-change outcome-map: plain-English labels, first line, field names and findings heading (LE-212). The skill tells the driver to write the intent and Accountable Review's page for a reader new to the project and to apply the no-ai-slop skill.

## [0.36.0] - 2026-10-31

### Changed

- Release builds and archive checks for `aarch64-apple-darwin` run on the `macos-15` runner instead of `macos-14`, which GitHub retires on 2026-11-02 (#92). Archive and installer names are unchanged.

### Compatibility

- Release configuration only. The engine, the providers and the profiles are unchanged.

## [0.35.0] - 2026-10-30

### Added

- `scripts/release-after-merge.sh` runs the release sequence after a merge to main as one command. It prepares the release in a worktree with `scripts/prepare-release.sh` and the caller's CHANGELOG text (`--notes FILE`), opens the release-prep PR and merges it once its checks pass, and waits for push-event CI to pass on the exact merge SHA. It then dispatches the release.yml dry run, then the publish. It stops at the first failed gate, names the step and prints a resume command. A rerun reads GitHub and resumes without repeating a merge or a dispatch. An existing release counts as done only when its publish run succeeded.

### Changed

- AGENTS.md "Authorized release sequence" now starts with a choice of path. After a merge, run the script: the owner's standing release grant is the authorization at each step it performs, but not for Loop Engine run starts. Otherwise, use the manual steps, with a fresh owner decision at each authorization point.

### Known limits

- The script tells a dry run from a publish by run order. A release.yml run dispatched by hand on the release commit can make it stop with a misleading message.
- The script reads only the newest 50 release.yml dispatch runs. `--version` on an old release (v0.4.1 and older today) stops instead of reporting it released.

### Compatibility

- Script and docs only. The engine, the providers and the profiles are unchanged.

## [0.34.0] - 2026-10-30

### Added

- The lean-change skill has a "Parallel builders" recipe: a driver builds independent plan steps at the same time, each in its own git worktree outside the repository, lands them in plan order with `git merge --no-ff`, and runs the full build and test gates once on the joined tree (LE-211). Landing stops and fails on a failed commit or a conflict and leaves the repository as it was; cleanup refuses a worktree that still holds uncommitted work.
- The lean-change journey has a `parallel` scenario that runs the recipe's shell blocks as written, and an optional `--skill PATH`.

### Compatibility

- Skill text and journey only. The engine, the provider and the profiles are unchanged.

## [0.33.0] - 2026-10-29

### Changed

- Lean-change profile `b`'s intent check has a sixth fixed question, `principles-gap`, and its packet now carries the owner's design principles (frozen text and sha256): which section applies to what the intent builds, yet has no criterion or constraint covering it (LE-208, LE-209). The skill has the driver list each principles section as applies/n-a in the intent's constraints before `intent-ready`.

### Compatibility

- Profile versions are unchanged. An `intent-check` record evaluated by this release needs one answer for each of the six questions, so a profile `b` run that recorded a five-answer check under 0.31.0 or 0.32.0 is refused at `intent-ready` if continued with this binary. Profile `c` and the engine are unchanged.

## [0.32.0] - 2026-10-29

### Added

- Lean-change has an old-vs-new check, on by default: `lean-change setup` writes `old_vs_new` with the repeatable `--proof-path` values, and `verify` runs each criterion's check on the base commit with the proof files laid over it. Each criterion says what its check must do there (`fails`, `same`, `absent` or `none` with a reason), and `verified` judges the recorded old runs against that expectation (LE-210). Drivers add `--proof-path` for test files; with none, the old tree is the bare base.

### Compatibility

- Profiles set up by lean-change 0.31.0 have no `old_vs_new` and behave as before. New setups print profile versions `lean-b-3` and `lean-c-2`; their intents need `old` on every criterion, and a 0.31.0 profile refuses it.

## [0.31.0] - 2026-10-28

### Changed

- Lean-change profile `b`'s intent check has a fifth fixed question, `criteria-gap`: could a plausible delivery pass every criterion's check and still leave the goal unmet (LE-208, LE-209).
- The lean-change review's `docs` question also asks, where the repository keeps a living PRD, for each requirement the change adds, changes or relies on and the check that proves it; a requirement no check proves is a finding (LE-207).

### Compatibility

- Profile versions are unchanged. An `intent-check` record evaluated by this release needs one answer for each of the five questions, so a profile `b` run that recorded a four-answer check under 0.30.0 is refused at `intent-ready` if continued with this binary. Profile `c` and the engine are unchanged.

## [0.30.0] - 2026-10-27

### Added

- The lean-change provider (LE-202–LE-209) is the software reference workflow and ships as a release binary.
- A reference `model-binding-provider` fixture proves the engine's model-binding contract (LE-195–LE-200) through the public binary.
- Leaner run-context reads: `read --kind context` pages record headers; `read --kind record --id ID [--pointer P]` returns one record exactly or a JSON Pointer slice; history entries name the context kind; `list` takes `--lifecycle`, `--label` and `--provider`; a closed pipe ends output quietly.

### Removed

- The software-change provider, its skill, journeys and real-binary integration tests. Its PRD requirements are tombstoned with their IDs and titles kept.

### Compatibility

- Removing software-change changed nothing in the engine. Runs started under software-change keep their stored graphs, context and history and still list and show; continuing one needs the release binary its run names. Nothing is migrated.

## [0.29.0] - 2026-10-24

### Added

- An owner can withdraw, relax, defer or clarify a reviewed requirement by recording the owner's words as ordinary `user-steering`, without revising intent, design or plan (LE-201). A criterion or goal verdict may cite an earlier, unsuperseded steering record with or instead of command evidence; the provider checks only that the record exists, is steering and is not superseded. Validation commissions list the citable steering.
- Validation-review commissions list the command-evidence IDs a verdict may cite (`citable_evidence`), naming the original ID for each indexed applicability wrapper. Citing the wrapper itself is still refused.

### Changed

- Review commissions deliver other revisions' review evidence by reference (verdict, findings, grounds, size, digest and locator) instead of whole stored exchanges, so a commission's size follows the evidence applicable to its target revision.
- The refusal for a missing implementation proof history now says that only the checked hop into validation writes it, that an owner override does not, and how to recover.
- LE-182 states how challenge scope carries across rounds.

### Compatibility

- Existing runs keep their frozen profiles and review obligations. No profile, schema, template or reviewer-protocol change; no new record kind.

## [0.25.0] - 2026-10-11

### Added

- Software-change plans now expose safe parallel work through their existing dependencies. New `minimal-15`, `standard-15` and `high-rigor-15` profiles review missed parallelism, conflicting concurrent work and unnecessary task splitting with the `parallelism-honest` axis.
- New software-change runs can opt into reviewed Markdown narratives at design, plan and validation, and owner checkpoints at intent, design, plan, implementation and validation. Both options default off. JSON artifacts remain authoritative, and revised artifacts invalidate their earlier narratives and decisions.
- New runs of the shipped providers retain exact artifact bytes in the catalog at forward phase exits. Retained copies remain inspectable after artifact files are deleted; repeated identical content is deduplicated without losing exit history.
- Operators can preview and explicitly apply `reclaim` to remove bulky legacy invocation details from finished or terminated runs. It preserves active runs and retained artifacts, reports retryable catalog contention, and returns freed space to the filesystem.

### Changed

- Main CI now separates lightweight checks from workspace tests and journey self-tests. Release dispatches require successful push CI for the exact commit, reproducible workflow generation with pinned cargo-dist, and successful archive checks before publication. Full source journeys remain a separate informational workflow.

### Compatibility

- Existing runs keep their frozen profiles and review obligations; they are not migrated to the new parallelism, narrative or checkpoint behavior.
- Artifact retention applies to new capture-enabled runs and does not backfill old artifacts. Reclaim never runs automatically; applying it intentionally removes selected historical invocation details and any verification that depends on those details.

## [0.19.0] - 2026-09-07

### Recovery and review

- Driver dispositions can discharge specific reviewer failures while retaining original verdicts and blocking accepted unresolved findings.
- Durable steering reaches later workers and reviewers. Effective commissions are inspectable before launch.
- Owner-attested binding amendments affect future invocations without rewriting frozen policy or past attempts.
- Public cancellation stops an invocation without advancing the workflow; implementation can return directly to plan, design or intent once owned work is quiescent.
- Explicit owner overrides retain denied evaluations and failed evidence. Exceptional runs finish permanently as `completed-with-overrides`.
- Criterion-based validation indexes named command captures, separate criterion and goal judgments, and explicit carry of unaffected evidence.
- Shipped review guidance and opt-in bindings batch assigned axes into one commission per author per gate. Ordinary and challenge review remain separate.
- Independent journey jobs support bounded concurrency and a serial path. Test builds optimize digest computation without removing integrity checks or relaxing deadlines.
- The proof pool restores the caller's Linux child-subreaper setting after success, failure or timeout, so later workflows do not inherit its orphan-reaping responsibility.
- Capture-backend errors now include the backend's diagnostic output.
- Validation-command timeout cleanup disambiguates negative process-group operands for older Linux `kill` implementations, preserving the capture parent while stopping the intended command group.

### Compatibility

Software-change now ships contract-v2 `minimal-9`, `standard-9` and `high-rigor-9` profiles, unbound by default. Use matching engine, provider and embedded profile versions. Older evidence remains readable; existing runs are not silently migrated to the new semantics.

### Known limitation

Cancellation ownership still relies on numeric PID/PGID values. After an interrupted controller and a sufficiently long delay, process-ID reuse could cause resumed cleanup to misidentify an unrelated local process, signal it, or keep the run falsely blocked. This concern remains unreproduced and unrepaired. The owner accepted it for this release; the recovery workflow completed with overrides, not clean acceptance of the cancellation criterion or whole-intent goal.

Local scripted proof does not establish universal performance gains or semantic review quality. Later live dogfood and the pre-existing calibration attestations remain pending.
