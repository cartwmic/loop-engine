# Agent instructions

## Scope

This file governs the entire loop-engine checkout: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, `bookends-check`, the lean-change/policy-document/research providers, `tests/workspace-integration`, `tests/fixtures`, `tests/bounded_process.rs`, scripts, skills and release workflows. `crates/bookends-check` has no nested guide and follows this file.

Primary workflow work belongs outside the engine/provider. Do not invent engine policy, review orchestration or core semantics. Humans start at [README.md](README.md). Nested `AGENTS.md` files add crate-local procedure and must not contradict this file.

## Authority

Each source governs its named scope:

1. Root `AGENTS.md`: checkout operations.
2. Nested `AGENTS.md`: additional crate-local procedure.
3. [docs/PRD.md](docs/PRD.md): living engine product requirements.
4. Frozen provider requirements and protocols: provider requirements and protocol contracts.
5. Provider README: that provider's public contract.
6. Engine and provider skills: driving procedure.
7. [docs/agent-usage.md](docs/agent-usage.md): generic CLI forms and loop operations.

Where sources overlap on checkout operations (proof, CI, handoff), root `AGENTS.md` wins. Nested guides and `docs/agent-usage.md` defer to its Workflow and Boundary checks rather than restate them.

Catalog selection follows the [engine skill’s Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup); README examples are not current-session owner authorization for production isolation.

README.md is the human overview. Requirement changes need exact owner acceptance and an authorized commit. A lead can instead record a delegated approval ([Delegated approval](docs/agent-usage.md#delegated-approval)). Ask the owner when no delegation covers the step, or the step adds, drops or changes an approved outcome. Unrelated recovery drafts remain drafts. Source integration alone establishes no requirement authority or semantic approval.

When proposing provenance obligations, load [docs/PRD.md](docs/PRD.md) LE-107; this guide is referential, not a second product policy. Owner-accepted wording remains a proposal until committed in that PRD.

## External work

Before driving the CLI, load [the engine skill](skills/using-loop-engine/SKILL.md) and [agent usage](docs/agent-usage.md). Before commissioning provider work, load its crate-local guide to select supported commands and its skill. Each run has one logical mutating driver: serialize all mutations through that driver, rather than starting independent concurrent mutators. Test catalogs and captures belong in driver-owned directories outside the checkout: explicitly set database and provider config, and unset unintended LOOP/XDG redirects rather than using user state.

Before rebuilding or replacing a provider executable, inspect active runs in each driver-owned catalog with explicit database/config and full show. Runs freeze command/args paths, not executable bytes. If an active run points to a build output that would be overwritten, stop the build: retain that executable and its required runtime dependencies at the exact frozen path, and build new binaries into a separate `CARGO_TARGET_DIR`. Do not move the old binary and assume the run follows it; there is no migration. Resume only with the compatible runtime preserved or after an authorized termination of the affected run.

### Load before acting

| Work | Owning procedure |
|---|---|
| Start or resume a run | [Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup) and the active provider skill |
| Observe before mutation | [Choose an observation](skills/using-loop-engine/SKILL.md#choose-an-observation) |
| Deliver code | [Lean-change skill](crates/lean-change-provider/skills/using-lean-change-provider/SKILL.md) |
| Draft or audit README/AGENTS | [Policy-document skill](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md) |
| Research a question | [Research skill](crates/research-provider/skills/using-research-provider/SKILL.md) |
| Coordinate independent drivers | [Coordinator skill](skills/coordinating-loop-engine/SKILL.md) |
| Extract a PRD candidate | [Generate-PRD skill](crates/research-provider/skills/using-generate-prd/SKILL.md) |

## Workflow

Run from the repository root with Rust/Cargo and Python 3 (CI uses 3.14.6). Load the engine and active provider skills; use policy-document for README/AGENTS authoring or assessment. Proof runs in stages, each with one owner:

| When | Who | What runs |
|---|---|---|
| Before handoff | Worker (never stages, commits or opens the PR) | The `checks` job's commands, locally: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo build --workspace --locked`. Then the [Boundary checks](#boundary-checks) rows for the areas the change touches, and `cargo test -p <crate>` for each touched crate. `cargo test -p` runs unit tests only: every crate with a `tests/` directory sets `autotests = false`, and its `tests/*.rs` files run through `tests/workspace-integration`. So for a touched crate with `tests/*.rs`, also run `python3 scripts/run-central-tests.py --filter <module>` (module names are in `tests/workspace-integration/tests/workspace.rs`, for example `loop_cli_engine`, `research_cli`). The handoff reports those commands and their outcomes. Missing required boundary proof blocks completion. |
| On the PR | Driver, who opens the PR once the Git checkpoint below allows it | The `checks` job of [ci.yml](.github/workflows/ci.yml), on the PR head. The driver cites its run URL and head SHA. PRs do not run `tests` or `release-tests`. |
| Merge | Driver | When the PR `checks` run is green on the current head and the review has passed, the driver squash-merges (`gh pr merge --squash`). The authority is the owner's standing rule for his personal computer, from his global agent instructions: "Merging and deploying approved work is yours to do: if checks pass and it keeps the product requirements, ship it and tell me after." The driver reports the merge to the owner after. |
| After merge | Driver | The push run on main for the merge SHA: `checks`, `tests` (workspace tests, journey self-tests, Bookends over the pushed range) and `release-tests`. See [After merge](#after-merge-the-main-push-run). |

The full baseline on kgate (profile [kgate.toml](kgate.toml)) is not run before merge by default: the push run carries it. The driver or lead may decide, before the PR opens, to run it for changes to public tests, `bookends.toml` pathspecs, `scripts/bookends-check-gate.sh` or release scripts, or anything they judge risky; it includes the workspace tests and Bookends. A worker runs it only when told to. Run it when the owner asks. A culprit hunt on a red main does not run it: it runs only the failing check (see After merge). Load [docs/kgate-baseline.md](docs/kgate-baseline.md) first: it holds the run command, how to read the result and the tree-hash check, and a baseline counts as proof only per that file.

Keep the check list in `scripts/baseline.sh` identical to the union of the jobs in [ci.yml](.github/workflows/ci.yml) (`checks` runs on PRs and pushes; `tests` and `release-tests` run on pushes to main): fmt, locked all-target Clippy, locked workspace build, workspace tests, five bare journey self-tests, the lean-change mutation applicability check, the release-after-merge script tests in 8 parallel shards (`python3 scripts/run-release-tests-sharded.py`; it runs the tests of `scripts/test_release_after_merge.py`, keeps one log per shard and fails if any shard fails or any test is missing), Bookends with bypass unset. Local Bookends (the baseline) checks the working tree; the push run checks every introduced commit from main's previous tip, and nothing else runs it before merge. Bare journeys and `--self-test` prove collection/interface mechanics, not end-to-end progression. Full journeys in [journeys.yml](.github/workflows/journeys.yml) are informational and never gate Release; local boundary proof remains required.

Before parameterized journeys, build current source and resolve the actual Cargo target directory. Inspect active catalogs before any rebuild as required above. Use an external directory owned by the proof driver for journey output, not user state. Each proof run needs a fresh, empty `PROOF_ROOT`, or a fresh subdirectory per run (for example `PROOF_ROOT=/abs/proof/run-2` after a fix): `scripts/external-work-journey.py` refuses a non-empty `--output-root` at once, and the boundary commands below pass `"$PROOF_ROOT/external-work"`.

```sh
set -eu
: "${PROOF_ROOT:?absolute driver-owned proof directory outside the checkout}"
cargo build --workspace --all-targets --locked
BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
```

### After merge: the main push run

Check the push run for the merge SHA before merging anything else. Poll `gh run list --workflow ci.yml --branch main --event push --commit <merge sha> --json databaseId,status,conclusion` (every 30 seconds, as `release-after-merge.sh` does) until a run appears, then wait for it to finish: `gh run watch <run id> --exit-status`. Push runs are never cancelled, so a missing run is not restarted. If no run appears within 10 minutes, report it to the lead and merge nothing further. If the run is red:

1. Merge nothing else into main except the fix, and cut no release.
2. Rerun the failed jobs once: `gh run rerun --failed <run id>`. If it goes green, it was a flake: file an issue naming the test, and the freeze ends.
3. Otherwise get the last green SHA with `gh run list --workflow ci.yml --branch main --event push --status success -L 1 --json headSha --jq '.[0].headSha'`. The culprit is in `git log --first-parent --oneline <last green sha>..<red sha>`. If the range holds one commit, that commit is the culprit. For more, test each commit with only the failing check, earliest first:
   - `git worktree add --detach <scratch>/<sha> <sha>` in a driver-owned scratch folder outside the checkout.
   - `cd <scratch>/<sha>` and run only the failing check as `run-and-wake -- kgate -- <cmd>`. Exit 0 means that commit is green; read the result as in [docs/kgate-baseline.md](docs/kgate-baseline.md). The command depends on the red job: `tests` is the failing `cargo test` or journey command (for example `cargo test -p <crate> <filter>` or `python3 scripts/run-central-tests.py --filter <name>`); `release-tests` is `python3 scripts/run-release-tests-sharded.py`; Bookends is `printf 'refs/heads/main %s refs/heads/main %s\n' <after> <before> | scripts/bookends-check-gate.sh`, with `<after>` the red push's head and `<before>` the previous tip of main, and the same environment as the `tests` job's Bookends step in [ci.yml](.github/workflows/ci.yml).
   - Remove it after: `git worktree remove <scratch>/<sha>`.
   The earliest red commit is the culprit.
4. Revert the culprit with a normal PR: `git revert <sha>` for a squash commit, `git revert -m 1 <sha>` for a merge commit. It needs only `checks` plus a one-line review, and main's next push run must go green. The culprit's owner then fixes forward in a new PR. Never force-push main.

These steps are the driver's job under the owner's ask #847 and need no further ask: the flake rerun, the culprit-hunt worktrees and kgate runs, and the revert PR's commit and push.

### Boundary checks

Every change runs the rows for the areas it touches (see the Before handoff row above); the rest of the baseline runs in the push run on main, one per push to main. Changes touching several areas run the union of their rows. The commands use the metadata-resolved `BIN_DIR` and external `PROOF_ROOT` above; do not select stale installed binaries or hard-code a target directory. Research source proof requires cargo-dist 0.32.0 already provisioned in an approved environment (`dist --version`); a missing prerequisite blocks the journey. Obtain approval before changing persistent tooling; any disposable tool root must be driver-owned and external.

| Changed area | Additional required commands |
|---|---|
| Engine (`loop-cli`, `loop-core`, `loop-integrations`) | `python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"`<br>`python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode draft`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode audit`<br>`python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json`<br>`python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json` |
| policy-document provider | `python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode draft`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode audit`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| lean-change provider | `python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change`<br>`python3 scripts/lean-change-mutations.py --engine "$BIN_DIR"/loop-engine`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| research provider (including generate-PRD) | `python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json`<br>`python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| bookends-check | `python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| Journey or driver script (`scripts/*-journey.py`, `scripts/external_work_driver.py`, `scripts/lean-change-mutations.py`) | Run each changed script parameterized, with its commands from the rows above: `policy-document-journey.py` (both the draft and the audit command, policy-document row), `lean-change-journey.py` and `lean-change-mutations.py` (lean-change row), `research-journey.py` and `generate-prd-journey.py` (research row), `external-work-journey.py` (any provider row). The external-work journey loads or runs `lean-change-journey.py`, `research-journey.py`, `policy-document-journey.py` and `generate-prd-journey.py`, and `external_work_driver.py` is imported by all five journeys, so a change to any of these five scripts also runs the external-work journey, and a change to `external_work_driver.py` runs every journey in the Engine row. The baseline runs these scripts bare, which does not exercise a changed full path. |
| Root `skills/` (engine and coordinator skills) | `python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` (it asserts phrases from `skills/using-loop-engine/SKILL.md`; the baseline's bare run returns before them) |
| Packaging/release (dist metadata, workflows, release scripts) | `python3 scripts/generate-release.py --check`<br>`python3 scripts/assert-release-gates.py`<br>`python3 scripts/run-release-tests-sharded.py` (the `release-tests` job)<br>`python3 scripts/archive-version-check.py --self-test`<br>For dist, packaging or release-archive changes the required archive proof is the `release.yml` dry run's archive check (`archive-check.yml`, `scripts/archive-version-check.py`), which runs during the release (after merge, not at handoff) and blocks host and publish<br>`python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json` (LE-69 dist-plan proof) |
| Docs/scripts only (outside the release or other boundaries above) | `checks` only |

For focused unit tests use `cargo test -p <crate> <filter>`; for focused central integration tests use `python3 scripts/run-central-tests.py --filter NAME`. The central runner builds its required binaries. `cargo test --workspace` runs only in the push run (the `tests` job) and in the baseline; it is not a handoff requirement.

For packaging boundaries, `scripts/packaged-smoke.py --mode archive` is an optional deeper check on downloaded dry-run archives, supplying `--archive` and `--checksum`; it is not the required archive proof. `--mode installed`, supplying `--binary`, is only for installed executables. Both require actual package identity, expected version, platform and an external output root; use `--help` for their required per-application inputs. Never describe installed identity as archive identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs before using the central runner. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands; Bookends checks them only in the push run and the baseline.

The baseline builds and selects the current checker and clears inherited Bookends bypass/update-stream settings for ordinary local proof. BYPASS is not proof. Do not substitute an installed checker for the current source build.

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-4`/`agents-4` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Load the policy-document skill for profile preparation and evidence rules. Local references must remain within the target directory; normalized paths such as `docs/../exists.md` are allowed, but escapes outside the directory are rejected. For root guidance in crate documents, use an explicit checkout-root-relative prose path instead of an escaping markdown link.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes, except the driver's squash merge (Merge row above) and red-main steps (see [After merge](#after-merge-the-main-push-run)), which need no further ask. A lead can instead record a delegated approval ([Delegated approval](docs/agent-usage.md#delegated-approval)). Ask the owner when no delegation covers the step, or the step adds, drops or changes an approved outcome. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures. These markdown boundaries are advisory, not an enforcement mechanism; inspect staged contents and retain sensitive/runtime artifacts outside the checkout.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and generate only with `python3 scripts/generate-release.py`; use `python3 scripts/generate-release.py --check` for read-only reproduction. Pull requests into main run the `checks` job of [ci.yml](.github/workflows/ci.yml). Pushes to main run `checks`, `tests` and `release-tests`, and the separate informational [journeys.yml](.github/workflows/journeys.yml). Publication uses the separately authorized sequence in [docs/release-procedure.md](docs/release-procedure.md). Do not force-push main.

### Authorized release sequence

Before preparing, committing, pushing, dispatching or publishing a release, or regenerating release metadata, load [docs/release-procedure.md](docs/release-procedure.md). After a PR merges to main, first confirm that merge's push run is green (`gh run list --workflow ci.yml --branch main --event push --commit <merge sha>`; red means the red-main procedure and no release), because the script waits only on the push run of its own release-prep merge. Then run `scripts/release-after-merge.sh [--version X.Y.Z] [--notes FILE]` (`--notes` is needed only when a release-prep PR has to be made; see the script's usage): the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization for that run's steps, but the grant does not cover Loop Engine run starts.

## Completion and handoff

Report changed files, exact command outcomes and missing checks, the local pre-handoff commands (fmt, Clippy, build, `cargo test -p` for touched crates) with outcomes, the boundary checks run and their outcomes, Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. A worker has no PR, so it lists the PR `checks` run as pending. The driver adds that run (its CI run URL and head SHA) once the PR exists, and after merge the push run's URL and conclusion for the merge SHA. For a kgate baseline also report the run-and-wake job id, the `tree` hash from the job's kgate record and the log path `${XDG_STATE_HOME:-~/.local/state}/kgate/<output>` ([docs/kgate-baseline.md](docs/kgate-baseline.md)). No checked transition establishes semantic review. A `checks` run or boundary run covers only the tree it ran on (the `checks` job checks out the PR head, not the merge ref): any edit after it needs a fresh run, and a handoff that cites an older run is not proof.
