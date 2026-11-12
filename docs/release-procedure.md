# Authorized release procedure

This is the authorized release sequence. Checkout-root `AGENTS.md` links here and still governs Git safety, authorization and proof; the text below moved from its "Authorized release sequence" section.

## Authorized release sequence

The only normal generation/check path is `python3 scripts/generate-release.py` / `python3 scripts/generate-release.py --check`, using pinned cargo-dist 0.32.0. `mise.toml` pins it (`aqua:axodotdev/cargo-dist@0.32.0`); `generate-release.py` and `prepare-release.sh` install and use that pin through mise when mise is present, else `dist` on PATH. The wrapper reproduces stock output in an external disposable copy, tightening exactly one archive-check host clause to success-only. ONLY the vendor's stock CI-byte comparison is replaced (`allow-dirty = ["ci"]`), because pinned dist checks those bytes internally during plan/build/host. Wrapper exact reproduction and independent `python3 scripts/assert-release-gates.py` strict graph proof remain mandatory; other generation/version checks remain intact. Failed, skipped, cancelled or missing archive-check results block host on both dry-run and publishing dispatches.

Choose the release path first:

1. After a PR merges to main, run `scripts/release-after-merge.sh [--version X.Y.Z] [--notes FILE]` (`--notes` is needed only when a release-prep PR has to be made). For that run, the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization at every point below that asks for one (version, commit and push, merging the release-prep PR, dry-run dispatch, publication), and they satisfy the ask-first Git rule in checkout-root `AGENTS.md` (Document and Git safety) for the release-prep commit, push and merge. Without `--version`, the script reuses main's version when that version has no release yet (an unfinished release), otherwise it takes the next minor; the owner may name one instead. The grant does not cover Loop Engine run starts. The script performs the steps below in order: it refuses to prepare while `/tmp/loop-engine-dist-plan.json` exists (remove it once you have checked it is a finished release's plan) and removes the plan it made after committing. After a failed prepare, a resume removes only that prepare's leftovers: tracked changes to `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md`, and the plan file, and only when this script started the prepare (a marker in the worktree's git dir says so). Any other dirty path or an unmarked plan file stops with the exact command to clear it. The release-prep commit uses the caller's git identity, or `release-after-merge <release-after-merge@example.invalid>` for that commit when none is set. It prepares in worktree `<main checkout parent>/loop-engine-release-vX.Y.Z` on branch `release/vX.Y.Z` (`RELEASE_WORKTREE` picks another path); creating it and removing it after the merge are covered by the grant. It reuses only a clean worktree of this repo on that branch, and stops, changing nothing, on anything else at that path, on uncommitted changes, or on commits on that branch other than the release prep. `prepare-release.sh` is the release-prep proof; it commits only `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` (the CHANGELOG text is the caller's `FILE`) and stops on any other staged path. It also checks the staged diff: the Cargo files may change only `version = ` lines and `CHANGELOG.md` may only gain lines; the diff goes to its log. It stops at the first failed gate, names the step and prints the resume command with `--version` pinned and the same `--notes` path; run that command to resume, since a bare rerun picks the version again. Keep the notes file at that path until the release is published; the script checks it exists on every run. A resume does not repeat a merge or a dispatch that succeeded or is running, and a bare rerun stops when nothing has merged since the last release.
2. Use the manual steps below, with a fresh owner decision at each authorization point, when a rerun cannot clear a stop (for example main moved past the release SHA) or for a release that does not follow a merge. A lead can instead record a delegated approval ([Delegated approval](agent-usage.md#delegated-approval)). Ask the owner when no delegation covers the step, or the step adds, drops or changes an approved outcome.

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). [prepare-release.sh](../scripts/prepare-release.sh) requires a clean tree and owns version/lockfile preparation, locked builds, ordinary Bookends, fmt, wrapper reproduction before dist plan, plan assertions and strict graph assertions. It does not regenerate release.yml:

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
