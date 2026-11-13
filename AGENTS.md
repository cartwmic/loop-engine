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

README.md is the human overview. Requirement changes need exact owner acceptance and an authorized commit, or a delegated approval as described in Document and Git safety. Unrelated recovery drafts remain drafts. Source integration alone establishes no requirement authority or semantic approval.

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
| Before handoff | Worker (never stages, commits or opens the PR) | The `checks` job's commands, on kgate (see Where proof runs below): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo build --workspace --locked`. Then the [Boundary checks](#boundary-checks) rows for the areas the change touches, and `cargo test -p <crate>` for each touched crate. `cargo test -p` runs unit tests only: every crate with a `tests/` directory sets `autotests = false`, and its `tests/*.rs` files run through `tests/workspace-integration`. So for a touched crate with `tests/*.rs`, also run `python3 scripts/run-central-tests.py --filter <module>` (module names are in `tests/workspace-integration/tests/workspace.rs`, for example `loop_cli_engine`, `research_cli`). The handoff reports those commands and their outcomes. Missing required boundary proof blocks completion. |
| On the PR | Driver, who opens the PR once the Git checkpoint below allows it | The `checks` job of [ci.yml](.github/workflows/ci.yml), on the PR head; a PR runs nothing else. The driver cites its run URL and head SHA. |
| Merge | Driver | When the PR `checks` run is green on the current head and the review has passed, the driver squash-merges (`gh pr merge --squash`). The authority is the owner's standing rule for his personal computer, from his global agent instructions: "Merging and deploying approved work is yours to do: if checks pass and it keeps the product requirements, ship it and tell me after." The driver reports the merge to the owner after. |
| After merge | Driver | The push run of [ci.yml](.github/workflows/ci.yml) on main for the merge SHA runs `checks`, `tests` (workspace tests, journey self-tests, Bookends over the pushed range) and `release-tests`, one run per push, never cancelled. See [After merge](#after-merge-the-main-push-run). |

**Where proof runs.** fmt, Clippy, build, `cargo test -p`, `scripts/run-central-tests.py`, the Boundary checks journeys and release prep run on kgate, not on the host (playbook rule 83fbe006). Run from the repository root; kgate ships that tree's tracked and untracked-not-ignored files, so an edit after a run needs a fresh run. Start the job with `run-and-wake`. If a wake can reach you, end your turn and wait for it; a subagent or a session with no wake target cannot be woken, so it must not end its turn but poll the job's `end` record in `${XDG_STATE_HOME:-~/.local/state}/run-and-wake/log.jsonl`. Read the result as [Read the result](docs/kgate-baseline.md#read-the-result) says; it applies to every kgate job here, not only the baseline.

```sh
run-and-wake -- kgate --profile kgate-verify.toml -- sh -c 'set -e; cargo fmt --all -- --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo build --workspace --all-targets --locked'
run-and-wake -- kgate --profile kgate-verify.toml -- cargo test -p <crate> --locked
```

[kgate.toml](kgate.toml) is the 12-CPU baseline profile; [kgate-verify.toml](kgate-verify.toml) (4 CPUs, same image and tools) fits everything smaller. A journey needs the binaries it runs, so build and run it in the same job: resolve `BIN_DIR` in the pod with the `cargo metadata` block under Workflow (further down) and set `PROOF_ROOT` to a fresh directory such as `/tmp/proof`. A job whose kgate record has a non-empty `reason`, or has no kgate record, did not run to its end: that is missing proof, neither a pass nor a failure. A pod's `/tmp` is lost when the job ends, so write anything the handoff must cite to `/out`; it comes back to `~/.local/state/kgate/<id>.out/`. The pod has no mise and no `dist`. A job that runs a row needing cargo-dist (`research-journey.py --mode source`, `generate-release.py --check`, `assert-release-gates.py`) downloads cargo-dist 0.32.0, the version `mise.toml` pins, in that job and puts it first on `PATH`, before the build and the journey commands:

```sh
export HOME=/tmp/h; mkdir -p "$HOME" /tmp/dist
python3 -c 'import urllib.request, tarfile, io; u = "https://github.com/axodotdev/cargo-dist/releases/download/v0.32.0/cargo-dist-x86_64-unknown-linux-gnu.tar.xz"; tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(u).read())).extractall("/tmp/dist")'
export PATH="$(dirname "$(find /tmp/dist -name dist -type f | head -1)"):$PATH"
```

The host runs one of these jobs only under one of the rule's three exceptions: (1) kgate cannot run the job (it needs the host's own state, apps or hardware), (2) kgate has no capacity for it, or (3) the owner explicitly asks for a local run instead of the cluster. The handoff names which of the three applies, in one line.

The full baseline on kgate (profile [kgate.toml](kgate.toml)) is not run before merge by default: the push run carries it. The driver or lead may decide, before the PR opens, to run it for changes to public tests, `bookends.toml` pathspecs, `scripts/bookends-check-gate.sh` or release scripts, or anything they judge risky; it includes the workspace tests and Bookends. A worker runs it only when told to. Run it when the owner asks. A culprit hunt on a red main does not run it (see After merge). Load [docs/kgate-baseline.md](docs/kgate-baseline.md) first: it holds the run command, how to read the result and the tree-hash check, and a baseline counts as proof only per that file.

Keep the check list in `scripts/baseline.sh` identical to the union of the jobs in [ci.yml](.github/workflows/ci.yml): fmt, locked all-target Clippy, locked workspace build, workspace tests, five bare journey self-tests, the lean-change mutation applicability check, the release-after-merge script tests in 8 parallel shards (`python3 scripts/run-release-tests-sharded.py`; it runs the tests of `scripts/test_release_after_merge.py`, keeps one log per shard and fails if any shard fails or any test is missing), Bookends with bypass unset. Local Bookends (the baseline) checks the working tree; the push run checks every introduced commit from main's previous tip, and nothing else runs it before merge. Bare journeys and `--self-test` prove collection/interface mechanics, not end-to-end progression. Full journeys in [journeys.yml](.github/workflows/journeys.yml) are informational and never gate Release; local boundary proof remains required.

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
3. If a push run of [ci.yml](.github/workflows/ci.yml) on main is still red after one `gh run rerun --failed <run id>`, find the culprit commit with [docs/culprit-hunt.md](docs/culprit-hunt.md) before reverting anything.
4. Revert the culprit with a normal PR: `git revert <sha>` for a squash commit, `git revert -m 1 <sha>` for a merge commit. It needs only `checks` plus a one-line review, and main's next push run must go green. The culprit's owner then fixes forward in a new PR.

These steps are the driver's job under the owner's ask #847 and need no further ask: the flake rerun, the culprit-hunt worktrees and kgate runs, and the revert PR's commit and push.

### Boundary checks

Every change runs the rows for the areas it touches (see the Before handoff row above); the push run covers the rest. Changes touching several areas run the union of their rows. The commands use the metadata-resolved `BIN_DIR` and external `PROOF_ROOT` above; do not select stale installed binaries or hard-code a target directory. Research source proof requires cargo-dist 0.32.0 already provisioned in an approved environment (`dist --version`), or downloaded in the kgate job as in Where proof runs; a missing prerequisite blocks the journey. Obtain approval before changing persistent tooling; any disposable tool root must be driver-owned and external.

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

For focused unit tests use `cargo test -p <crate> <filter>`; for focused central integration tests use `python3 scripts/run-central-tests.py --filter NAME`. The central runner builds its required binaries. `cargo test --workspace` is not a handoff requirement.

For packaging boundaries, `scripts/packaged-smoke.py --mode archive` is an optional deeper check on downloaded dry-run archives, supplying `--archive` and `--checksum`; it is not the required archive proof. `--mode installed`, supplying `--binary`, is only for installed executables. Both require actual package identity, expected version, platform and an external output root; use `--help` for their required per-application inputs. Never describe installed identity as archive identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs before using the central runner. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands.

The baseline builds and selects the current checker and clears inherited Bookends bypass/update-stream settings for ordinary local proof. BYPASS is not proof. Do not substitute an installed checker for the current source build.

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-4`/`agents-4` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Load the policy-document skill for profile preparation and evidence rules. Local references must remain within the target directory; normalized paths such as `docs/../exists.md` are allowed, but escapes outside the directory are rejected. For root guidance in crate documents, use an explicit checkout-root-relative prose path instead of an escaping markdown link.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes, except the driver's squash merge (Merge row above) and red-main steps (see [After merge](#after-merge-the-main-push-run)), which need no further ask. A lead can instead record a delegated approval ([Delegated approval](docs/agent-usage.md#delegated-approval)). Ask the owner when no delegation covers the step, or the step adds, drops or changes an approved outcome. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures. These markdown boundaries are advisory, not an enforcement mechanism; inspect staged contents and retain sensitive/runtime artifacts outside the checkout.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and generate only with `python3 scripts/generate-release.py`; use `python3 scripts/generate-release.py --check` for read-only reproduction. Publication uses the separately authorized sequence in [docs/release-procedure.md](docs/release-procedure.md). Never force-push main.

### Authorized release sequence

Before preparing, committing, pushing, dispatching or publishing a release, or regenerating release metadata, load [docs/release-procedure.md](docs/release-procedure.md). After a PR merges to main, first confirm that merge's push run is green (`gh run list --workflow ci.yml --branch main --event push --commit <merge sha>`; red means the red-main procedure and no release), because the script waits only on the push run of its own release-prep merge. Then run `scripts/release-after-merge.sh [--version X.Y.Z] [--notes FILE]` (`--notes` is needed only when a release-prep PR has to be made; see the script's usage): the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization for that run's steps, but the grant does not cover Loop Engine run starts.

Release prep runs on kgate. The script runs `$RELEASE_PREPARE_CMD VERSION` in the release worktree, so set `RELEASE_PREPARE_CMD` to a driver-owned wrapper that runs `scripts/prepare-release.sh VERSION` in a kgate job (`--profile kgate-verify.toml`) on a standalone clone of the release worktree with `origin` set to GitHub. The pod has no mise and no `dist`, so the job downloads cargo-dist 0.32.0 (the version `mise.toml` pins) and puts it first on `PATH`. It copies `Cargo.toml` and `Cargo.lock` to `/out`, and the wrapper copies them from the job's output directory (`~/.local/state/kgate/<id>.out/`, named by the `log=` path on kgate's last line) into the release worktree. The wrapper fetches `main` into its clone and runs `kgate` in the foreground (never `run-and-wake` inside it, which would return before the files exist); `run-and-wake` wraps the whole `scripts/release-after-merge.sh` instead. The script's own checks of the staged diff still run on the host. A wrapper that fails stops the script at its prepare step.

## Completion and handoff

Report changed files, exact command outcomes and missing checks, the pre-handoff commands (fmt, Clippy, build, `cargo test -p` for touched crates) with outcomes, the boundary checks run and their outcomes, Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. A worker has no PR, so it lists the PR `checks` run as pending. The driver adds that run (its CI run URL and head SHA) once the PR exists, and after merge the push run's URL and conclusion for the merge SHA. For every kgate job (the baseline included) also report the run-and-wake job id, the `tree` hash from the job's kgate record and the log path `${XDG_STATE_HOME:-~/.local/state}/kgate/<output>` ([docs/kgate-baseline.md](docs/kgate-baseline.md)). No checked transition establishes semantic review. A `checks` run or boundary run covers only the tree it ran on (the `checks` job checks out the PR head, not the merge ref): any edit after it needs a fresh run, and a handoff that cites an older run is not proof.
