# Changelog

## [0.36.0] - 2026-10-31

### Changed

- Release builds and archive checks for `aarch64-apple-darwin` run on the `macos-15` runner instead of `macos-14`, which GitHub retires on 2026-11-02 (#92). Archive and installer names are unchanged.

### Compatibility

- Release configuration only. The engine, the providers and the profiles are unchanged.

## Unreleased

### Changed

- Release builds and archive checks for `aarch64-apple-darwin` run on the `macos-15` runner instead of `macos-14`, which GitHub retires on 2026-11-02. Archive and installer names are unchanged.

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
