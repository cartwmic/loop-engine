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

Run from the repository root with Rust/Cargo and Python 3 (CI uses 3.14.6). Load the engine and active provider skills; use policy-document for README/AGENTS authoring or assessment. The final-proof owner runs the baseline plus the boundary checks below on the current source. Workers run focused checks; missing required proof blocks completion.

```sh
scripts/baseline.sh
```

Keep its check list identical to [ci.yml](.github/workflows/ci.yml): fmt, locked all-target Clippy, locked workspace build, workspace tests, five bare journey self-tests, the lean-change mutation applicability check, the release-after-merge script tests (`python3 -m unittest scripts/test_release_after_merge.py`), Bookends with bypass unset. Local Bookends checks the working tree; push CI checks every introduced commit from main's previous tip. Bare journeys and `--self-test` prove collection/interface mechanics, not end-to-end progression. Full journeys in [journeys.yml](.github/workflows/journeys.yml) are informational and never gate Release; local boundary proof remains required.

Before parameterized journeys, build current source and resolve the actual Cargo target directory. Inspect active catalogs before any rebuild as required above. Use an external directory owned by the proof driver for journey output, not user state:

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
| Packaging/release (dist metadata, workflows, release scripts) | `python3 scripts/generate-release.py --check`<br>`python3 scripts/assert-release-gates.py`<br>`python3 scripts/archive-version-check.py --self-test`<br>`python3 scripts/research-journey.py --mode source --engine "$BIN_DIR"/loop-engine --provider "$BIN_DIR"/research --profile crates/research-provider/data/configs/standard.json` (LE-69 dist-plan proof) |
| Docs/scripts only (outside the release or other boundaries above) | Baseline only |

For focused unit tests use `cargo test -p <crate> <filter>`; for focused central integration tests use `python3 scripts/run-central-tests.py --filter NAME`. The central runner builds its required binaries; ordinary final workspace proof is `cargo test --workspace` in the baseline, not a separate handoff matrix.

For packaging boundaries, choose `scripts/packaged-smoke.py --mode archive` when validating release archives, supplying `--archive` and `--checksum`; choose `--mode installed` only for installed executables, supplying `--binary`. Both require actual package identity, expected version, platform and an external output root; use `--help` for their required per-application inputs. Never describe installed identity as archive identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs before using the central runner. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands.

The baseline builds and selects the current checker and clears inherited Bookends bypass/update-stream settings for ordinary local proof. BYPASS is not proof. Do not substitute an installed checker for the current source build.

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-4`/`agents-4` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Load the policy-document skill for profile preparation and evidence rules. Local references must remain within the target directory; normalized paths such as `docs/../exists.md` are allowed, but escapes outside the directory are rejected. For root guidance in crate documents, use an explicit checkout-root-relative prose path instead of an escaping markdown link.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures. These markdown boundaries are advisory, not an enforcement mechanism; inspect staged contents and retain sensitive/runtime artifacts outside the checkout.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and generate only with `python3 scripts/generate-release.py`; use `python3 scripts/generate-release.py --check` for read-only reproduction. Pushes to main and pull requests into main run blocking CI ([ci.yml](.github/workflows/ci.yml)); pushes to main also run the separate informational [journeys.yml](.github/workflows/journeys.yml). Publication uses the separately authorized sequence below. Do not force-push main.

### Authorized release sequence

The only normal generation/check path is `python3 scripts/generate-release.py` / `python3 scripts/generate-release.py --check`, using pinned cargo-dist 0.32.0. The wrapper reproduces stock output in an external disposable copy, tightening exactly one archive-check host clause to success-only. ONLY the vendor's stock CI-byte comparison is replaced (`allow-dirty = ["ci"]`), because pinned dist checks those bytes internally during plan/build/host. Wrapper exact reproduction and independent `python3 scripts/assert-release-gates.py` strict graph proof remain mandatory; other generation/version checks remain intact. Failed, skipped, cancelled or missing archive-check results block host on both dry-run and publishing dispatches.

Choose the release path first:

1. After a PR merges to main, run `scripts/release-after-merge.sh [--version X.Y.Z] --notes FILE`. For that run, the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization at every point below that asks for one (version, commit and push, merging the release-prep PR, dry-run dispatch, publication), and they satisfy the ask-first Git rule above for the release-prep commit, push and merge. Without `--version`, the script reuses main's version when that version has no release yet (an unfinished release), otherwise it takes the next minor; the owner may name one instead. The grant does not cover Loop Engine run starts. The script performs the steps below in order: it refuses to prepare while `/tmp/loop-engine-dist-plan.json` exists (remove it once you have checked it is a finished release's plan) and removes the plan it made after committing. It prepares in worktree `<main checkout parent>/loop-engine-release-vX.Y.Z` on branch `release/vX.Y.Z` (`RELEASE_WORKTREE` picks another path); creating it and removing it after the merge are covered by the grant. It reuses only a clean worktree of this repo on that branch, and stops, changing nothing, on anything else at that path, on uncommitted changes, or on commits on that branch other than the release prep. `prepare-release.sh` is the release-prep proof; it commits only `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` (the CHANGELOG text is the caller's `FILE`) and stops on any other staged path. It also checks the staged diff: the Cargo files may change only `version = ` lines and `CHANGELOG.md` may only gain lines; the diff goes to its log. It stops at the first failed gate, names the step and prints the resume command with `--version` pinned and the same `--notes` path; run that command to resume, since a bare rerun picks the version again. Keep the notes file at that path until the release is published; the script checks it exists on every run. A resume does not repeat a merge or a dispatch that succeeded or is running, and a bare rerun stops when nothing has merged since the last release.
2. Use the manual steps below, with a fresh owner decision at each authorization point, when a rerun cannot clear a stop (for example main moved past the release SHA) or for a release that does not follow a merge.

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). [prepare-release.sh](scripts/prepare-release.sh) requires a clean tree and owns version/lockfile preparation, locked builds, ordinary Bookends, fmt, wrapper reproduction before dist plan, plan assertions and strict graph assertions. It does not regenerate release.yml:

```sh
VERSION="X.Y.Z"
TAG="v$VERSION"
test ! -e /tmp/loop-engine-dist-plan.json || { echo "Release plan destination already exists; obtain ownership/cleanup approval before proceeding" >&2; exit 1; }
scripts/prepare-release.sh "$VERSION"
```

Review the prepared result and obtain separate authorization for commit/push. Committing, pushing and any manual tagging require explicit owner authorization; preparation grants none. After the authorized push to main, require completed successful push-event CI for the EXACT SHA to be dispatched:

```sh
set -eu
COMMIT="$(git rev-parse HEAD)"
CI_RUN="$(gh run list --workflow ci.yml --branch main --event push --commit "$COMMIT" --limit 1 --json databaseId,status,conclusion --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId')"
test -n "$CI_RUN" || { echo "Exact-commit push CI is missing, pending or failed" >&2; exit 1; }
gh run view "$CI_RUN" --json headSha,status,conclusion,url
CI_SHA="$(gh run view "$CI_RUN" --json headSha --jq .headSha)"
test "$CI_SHA" = "$COMMIT"
```

Retain that run URL and logs. Dispatch uses the prepared main tip: if main moves, stop and obtain current exact-SHA proof and authorization rather than dispatching a different tree. Only after a separate owner-authorized dry-run dispatch:

```sh
test "$(gh api 'repos/{owner}/{repo}/commits/main' --jq .sha)" = "$COMMIT"
gh workflow run release.yml --ref main -f tag=dry-run
```

Inspect the new Release run's `headSha` with `gh run view RUN_ID --json headSha,status,conclusion,url`; require it to equal `COMMIT` and the dispatch to complete successfully, including archive checking. Retain its URL/logs. Informational journeys need not succeed or finish before Release. Publication is a separate owner decision, not implied by a successful dry run. Only after that authorization, using the same prepared SHA and owner-approved version:

```sh
: "${TAG:?set TAG from the owner-approved VERSION}"
test "$(gh api 'repos/{owner}/{repo}/commits/main' --jq .sha)" = "$COMMIT"
gh workflow run release.yml --ref main -f "tag=$TAG"
```

## Completion and handoff

Report changed files, exact command outcomes and missing checks, Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. No checked transition establishes semantic review.
