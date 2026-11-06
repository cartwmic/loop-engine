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

Catalog selection follows the [engine skill’s Deterministic setup](skills/using-loop-engine/SKILL.md#deterministic-setup); README examples are not current-session owner authorization for production isolation.

README.md is the human overview. Requirement changes need exact owner acceptance and an authorized commit. Unrelated recovery drafts remain drafts. Source integration alone establishes no requirement authority or semantic approval.

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

Run from the repository root with Rust/Cargo and Python 3 (CI uses 3.14.6). Load the engine and active provider skills; use policy-document for README/AGENTS authoring or assessment. The final-proof owner runs the baseline plus the boundary checks below on the current source. Workers run focused checks and run the baseline on the homelab cluster with kgate (profile [kgate.toml](kgate.toml)); missing required proof blocks completion.

```sh
run-and-wake -- kgate -- scripts/baseline.sh
```

Mac undo: to revert this switch, change the command above back to `scripts/baseline.sh` and run it from the repository root. kgate never runs its command on the Mac, so a baseline that did not run on the cluster is missing proof, not a Mac run.

`run-and-wake` returns at once: it prints the job id, the pid and the output file, and its own exit status says nothing about the baseline. The baseline's real exit lands later as an `end` record for that job id in `${XDG_STATE_HOME:-~/.local/state}/run-and-wake/log.jsonl`. The wake goes to the Pi/Herdr pane or T3 thread that started the job, and it is best effort: `run-and-wake` prints `nothing will wake you` when it has no target (a session outside Pi and T3). A subagent that has no T3 thread of its own cannot be woken: ending its turn ends the subagent and the wake goes to its parent, which has none of the job context. If a wake will reach you, end the turn and wait for it. If not (no wake target, a subagent, or the wake never arrives), do not end the turn: poll that log until a line with your job id and `"event": "end"` appears, and read its `exit`. Do not read the immediate return as a pass. Then read kgate's record for the job in `${XDG_STATE_HOME:-~/.local/state}/kgate/log.jsonl` (its `job` field is the run-and-wake job id). `reason` is JSON `null` when the baseline ran, and then its `exit` is the baseline's status. A non-empty `reason` means the job did not run to its end on the cluster, whatever the exit code (69, or 130/143 when interrupted): report missing proof. Exit 69 with a `null` `reason` is the baseline itself exiting 69, which is a failure. A job with no kgate record (kgate missing or killed) is also missing proof.

Keep the check list in `scripts/baseline.sh` identical to [ci.yml](.github/workflows/ci.yml): fmt, locked all-target Clippy, locked workspace build, workspace tests, five bare journey self-tests, the lean-change mutation applicability check, the release-after-merge script tests in 8 parallel shards (`python3 scripts/run-release-tests-sharded.py`; it runs the tests of `scripts/test_release_after_merge.py`, keeps one log per shard and fails if any shard fails or any test is missing), Bookends with bypass unset. Local Bookends checks the working tree; push CI checks every introduced commit from main's previous tip. Bare journeys and `--self-test` prove collection/interface mechanics, not end-to-end progression. Full journeys in [journeys.yml](.github/workflows/journeys.yml) are informational and never gate Release; local boundary proof remains required.

Before parameterized journeys, build current source and resolve the actual Cargo target directory. Inspect active catalogs before any rebuild as required above. Use an external directory owned by the proof driver for journey output, not user state. Each proof run needs a fresh, empty `PROOF_ROOT`, or a fresh subdirectory per run (for example `PROOF_ROOT=/abs/proof/run-2` after a fix): `scripts/external-work-journey.py` refuses a non-empty `--output-root` at once, and the boundary commands below pass `"$PROOF_ROOT/external-work"`.

```sh
set -eu
: "${PROOF_ROOT:?absolute driver-owned proof directory outside the checkout}"
cargo build --workspace --all-targets --locked
BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
```

### Boundary checks

Every change runs the baseline. Changes touching several areas run the union of their rows. The commands use the metadata-resolved `BIN_DIR` and external `PROOF_ROOT` above; do not select stale installed binaries or hard-code a target directory. Research source proof requires cargo-dist 0.32.0 already provisioned in an approved environment (`dist --version`); a missing prerequisite blocks the journey. Obtain approval before changing persistent tooling; any disposable tool root must be driver-owned and external.

| Changed area | Additional required commands |
|---|---|
| Engine (`loop-cli`, `loop-core`, `loop-integrations`) | `python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"`<br>`python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode draft`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode audit`<br>`python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json`<br>`python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json` |
| policy-document provider | `python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode draft`<br>`python3 scripts/policy-document-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/policy-document --profile crates/policy-document-provider/data/readme.json --mode audit`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| lean-change provider | `python3 scripts/lean-change-journey.py --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/lean-change`<br>`python3 scripts/lean-change-mutations.py --engine "$BIN_DIR"/loop-engine`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| research provider (including generate-PRD) | `python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json`<br>`python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| bookends-check | `python3 scripts/generate-prd-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --profile crates/research-provider/data/configs/generate-prd.json`<br>`python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` |
| Journey or driver script (`scripts/*-journey.py`, `scripts/external_work_driver.py`, `scripts/lean-change-mutations.py`) | Run each changed script parameterized, with its commands from the rows above: `policy-document-journey.py` (both the draft and the audit command, policy-document row), `lean-change-journey.py` and `lean-change-mutations.py` (lean-change row), `research-journey.py` and `generate-prd-journey.py` (research row), `external-work-journey.py` (any provider row). The external-work journey loads or runs `lean-change-journey.py`, `research-journey.py`, `policy-document-journey.py` and `generate-prd-journey.py`, and `external_work_driver.py` is imported by all five journeys, so a change to any of these five scripts also runs the external-work journey, and a change to `external_work_driver.py` runs every journey in the Engine row. The baseline runs these scripts bare, which does not exercise a changed full path. |
| Root `skills/` (engine and coordinator skills) | `python3 scripts/external-work-journey.py --engine "$BIN_DIR"/loop-engine --lean "$BIN_DIR"/lean-change --policy "$BIN_DIR"/policy-document --research "$BIN_DIR"/research --checker "$BIN_DIR"/bookends-check --output-root "$PROOF_ROOT/external-work"` (it asserts phrases from `skills/using-loop-engine/SKILL.md`; the baseline's bare run returns before them) |
| Packaging/release (dist metadata, workflows, release scripts) | `python3 scripts/generate-release.py --check`<br>`python3 scripts/assert-release-gates.py`<br>`python3 scripts/archive-version-check.py --self-test`<br>For dist, packaging or release-archive changes the required archive proof is the `release.yml` dry run's archive check (`archive-check.yml`, `scripts/archive-version-check.py`), which blocks host and publish<br>`python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json` (LE-69 dist-plan proof) |
| Docs/scripts only (outside the release or other boundaries above) | Baseline only |

For focused unit tests use `cargo test -p <crate> <filter>`; for focused central integration tests use `python3 scripts/run-central-tests.py --filter NAME`. The central runner builds its required binaries; ordinary final workspace proof is `cargo test --workspace` in the baseline, not a separate handoff matrix.

For packaging boundaries, `scripts/packaged-smoke.py --mode archive` is an optional deeper check on downloaded dry-run archives, supplying `--archive` and `--checksum`; it is not the required archive proof. `--mode installed`, supplying `--binary`, is only for installed executables. Both require actual package identity, expected version, platform and an external output root; use `--help` for their required per-application inputs. Never describe installed identity as archive identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs before using the central runner. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands.

The baseline builds and selects the current checker and clears inherited Bookends bypass/update-stream settings for ordinary local proof. BYPASS is not proof. Do not substitute an installed checker for the current source build.

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-4`/`agents-4` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Load the policy-document skill for profile preparation and evidence rules. Local references must remain within the target directory; normalized paths such as `docs/../exists.md` are allowed, but escapes outside the directory are rejected. For root guidance in crate documents, use an explicit checkout-root-relative prose path instead of an escaping markdown link.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures. These markdown boundaries are advisory, not an enforcement mechanism; inspect staged contents and retain sensitive/runtime artifacts outside the checkout.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and generate only with `python3 scripts/generate-release.py`; use `python3 scripts/generate-release.py --check` for read-only reproduction. Pushes to main and pull requests into main run blocking CI ([ci.yml](.github/workflows/ci.yml)); pushes to main also run the separate informational [journeys.yml](.github/workflows/journeys.yml). Publication uses the separately authorized sequence in [docs/release-procedure.md](docs/release-procedure.md). Do not force-push main.

### Authorized release sequence

Before preparing, committing, pushing, dispatching or publishing a release, or regenerating release metadata, load [docs/release-procedure.md](docs/release-procedure.md). After a PR merges to main, run `scripts/release-after-merge.sh [--version X.Y.Z] --notes FILE`: the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization for that run's steps, but the grant does not cover Loop Engine run starts.

## Completion and handoff

Report changed files, exact command outcomes and missing checks, the kgate baseline evidence (the run-and-wake job id, the `tree` hash from the job's kgate record, and the log path `${XDG_STATE_HOME:-~/.local/state}/kgate/<output>`), Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. No checked transition establishes semantic review. A baseline run covers only the tree it shipped: any edit after it needs a fresh run, and a handoff that cites an older run is not baseline proof. To check that a cited `tree` hash is of the tree being handed off, compute kgate's hash for it from the repository root and compare:

```sh
t=$(mktemp -u); git ls-files -co --exclude-standard -z | GIT_INDEX_FILE=$t git update-index --add --remove -z --stdin && GIT_INDEX_FILE=$t git write-tree; rm -f "$t"
```
