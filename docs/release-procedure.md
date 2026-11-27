# Authorized release procedure

This is the authorized release sequence: the steps, their order and their commands. Checkout-root `AGENTS.md` links here and governs what authorizes each step, Git safety and proof; where the two overlap on those, `AGENTS.md` wins (its Authority section lists this file as source 8). The text below moved from its "Authorized release sequence" section, which keeps a short pointer here.

## Authorized release sequence

The only normal generation/check path is `python3 scripts/generate-release.py` / `python3 scripts/generate-release.py --check`, using pinned cargo-dist 0.32.0. `mise.toml` pins it (`aqua:axodotdev/cargo-dist@0.32.0`); `generate-release.py` and `prepare-release.sh` install and use that pin through mise when mise is present, else `dist` on PATH. The wrapper reproduces stock output in an external disposable copy, tightening exactly one archive-check host clause to success-only. ONLY the vendor's stock CI-byte comparison is replaced (`allow-dirty = ["ci"]`), because pinned dist checks those bytes internally during plan/build/host. Wrapper exact reproduction and independent `python3 scripts/assert-release-gates.py` strict graph proof remain mandatory; other generation/version checks remain intact. Failed, skipped, cancelled or missing archive-check results block host on both dry-run and publishing dispatches.

Choose the release path first:

1. After a PR merges to main (the last one, when several are queued, and only once the push run of every merge in the queue is green, `cancelled` by a newer push (its commits are covered by the later run), or handled under the red-main procedure; see checkout-root `AGENTS.md`, Authorized release sequence), run `scripts/release-after-merge.sh [--version X.Y.Z] [--notes FILE]` (`--notes` is needed only when a release-prep PR has to be made). For that run, the owner's grant 19 ("cut the release when a PR merges to main") and playbook rule d9fa9f06 are the owner authorization at every point below that asks for one (version, commit and push, merging the release-prep PR, dry-run dispatch, publication), and checkout-root `AGENTS.md` (Document and Git safety) says the release steps under its Authorized release sequence need no further ask, which covers the release-prep commit, push and merge. Without `--version`, the script reuses main's version when that version has no release yet (an unfinished release), otherwise it takes the next minor; the owner may name one instead. The grant does not cover Loop Engine run starts. The script performs the steps below in order: it refuses to prepare while `/tmp/loop-engine-dist-plan.json` exists (remove it once you have checked it is a finished release's plan) and removes the plan it made after committing. A resume never deletes anything. If the prep worktree is dirty (staged, unstaged or untracked paths) or the plan file exists, it stops, changes nothing, lists each path and prints one command that stashes the worktree (`git stash push --include-untracked`, with a throwaway identity) and removes the plan file; run it as printed, keep the stash, and drop it only when `git stash show -p --include-untracked` (the contents, not just the file names) shows, once you have read them, that it holds nothing but the release script's own output; if it holds anything else, keep it and report it, then resume. The release-prep commit uses the caller's git identity, or `release-after-merge <release-after-merge@example.invalid>` for that commit when none is set. It prepares in worktree `<main checkout parent>/loop-engine-release-vX.Y.Z` on branch `release/vX.Y.Z` (`RELEASE_WORKTREE` picks another path); creating it and removing it after the merge are covered by the grant. It reuses only a clean worktree of this repo on that branch, and stops, changing nothing, on anything else at that path, on uncommitted changes, or on commits on that branch other than the release prep. `prepare-release.sh` is the release-prep proof; it commits only `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` (the CHANGELOG text is the caller's `FILE`) and stops on any other staged path. It also checks the staged diff: the Cargo files may change only `version = ` lines and `CHANGELOG.md` may only gain lines; the diff goes to its log. It stops at the first failed gate, names the step and prints the resume command with `--version` pinned and the same `--notes` path; run that command to resume, since a bare rerun picks the version again. Keep the notes file at that path until the release is published; the script checks it exists on every run. A resume does not repeat a merge or a dispatch that succeeded or is running, and a bare rerun stops when nothing has merged since the last release. After it reads the push CI run green on the merge SHA, and before any dispatch, it waits until the run search that `release.yml`'s "Require successful push CI for this exact commit" step reads (`gh api` on `actions/workflows/ci.yml/runs` with `event=push`, `head_sha` and `status=success`) returns that run too, because that search can lag the run's completion by more than half a minute and `release.yml` refuses the SHA until it shows. It logs each miss and how long the run took to show, polls every `RELEASE_POLL_SECONDS` for at most `RELEASE_APPEAR_POLLS` polls (about 10 minutes by default), and then stops at step `ci` with no dispatch; a rerun resumes. Before it merges the release-prep PR it runs `ci-main-status loop-engine`: exit 0 (green) or 2 (untested) goes on; exit 1, any other exit or a missing tool stops it at step `merge` with no merge, quoting the output. Then it runs the merge gate `intent-review-check cartwmic/loop-engine <PR>` (the command comes from `INTENT_REVIEW_CHECK`, default `intent-review-check` on PATH): any non-zero exit or a missing command stops it at step `merge` with no merge, quoting the output. A rerun after main is green and the gate passes resumes and merges.
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

## The downstream bump check

The dotfiles repository (`cartwmic/dotfiles`) installs every release: its skill importer copies the release's skill bundles and moves the tool pins. A release the importer cannot take turns dotfiles red once the tag is out (v0.71.3 removed a file the importer still listed).

So `release.yml` runs `scripts/check-downstream-bump.sh` as the first gate step of every build job (`downstream-bump-check`, from `.github/release-gate-steps.yml`). It runs on the dry-run and the publishing dispatch, on both release paths above, and before `host` can create the tag; `scripts/assert-release-gates.py` fails if the step is missing, conditional or after the build. It reads the dotfiles default branch (public: no secret, nothing written there) and runs the importer's `check` mode on the exact commit being released. It stops the release, with the file or pin named, when:

- the importer lists a file the commit no longer has (`ROOT_REFERENCES`);
- a file in the commit is empty, a symlink, or has a name chezmoi would read as an attribute;
- the dotfiles skill bundles hold a file the commit no longer ships (the bump stops on such an orphan);
- a Loop Engine pin's `matching_regex` in the dotfiles mise config matches none of the archives in this release's dist plan, so mise could not resolve the tool.

When it stops, change dotfiles first (drop the list entry and delete the bundle file, or fix the pin), merge that, and rerun the release; or restore the file here. A dotfiles repository that cannot be read also stops the release, so a rerun picks it up. The same check runs by hand with `scripts/check-downstream-bump.sh --plan <dist plan json> [--rev <commit>]` from a checkout.

## Running release-after-merge.sh

`scripts/release-after-merge.sh` runs `ci-main-status loop-engine` before it squash-merges the release-prep PR: exit 0 (green) or 2 (untested) go on; exit 1 (red, or main's state cannot be read), any other exit or a missing `ci-main-status` stop it at step `merge` with no merge, quoting the output, and a rerun after main is green resumes and merges. Right before the squash merge it runs the merge gate `intent-review-check cartwmic/loop-engine <PR>`, the command is `intent-review-check` on PATH; `INTENT_REVIEW_CHECK` overrides it, and its value is split on spaces, so a `node ... intent-review-check.ts` command line works. Any non-zero exit (a refusal, or exit 2 when the check could not decide) or a missing command stops it at step `merge` with no merge, quoting the output; a rerun after the gate passes resumes and merges. The check passes an exempt loop-engine release-prep PR without an agent record: its diff is only the version lines of `Cargo.toml` and `Cargo.lock`, and every bullet of its CHANGELOG section cites a loop-engine PR merged since the previous tag that has a passing review round. Any other PR needs a passing review round on its head.

`scripts/release-after-merge.sh` runs `$RELEASE_PREPARE_CMD VERSION` in the release worktree. With `RELEASE_PREPARE_CMD` unset, it runs `scripts/prepare-release.sh` on the host when `cargo` is on PATH. When `cargo` is not on PATH and `kgate` is (the agent-host pod, which has mise but no Rust toolchain), it runs [prepare-release-kgate.sh](../scripts/prepare-release-kgate.sh) instead. That script runs `scripts/prepare-release.sh VERSION` as a kgate job on the 4-CPU `kgate-verify.toml` profile (the 12-CPU baseline often has no node with room), fetching the cargo-dist version that `mise.toml` pins into the pod, then copies the bumped `Cargo.toml` and `Cargo.lock` back from the job's `/out` into the release worktree. A kgate exit 69 or 75 stops the release at step `prepare`, because the job did not run. Set `RELEASE_PREPARE_CMD` only to replace that command.

The script is release logic that runs from the checkout you call it in, while the release prepares from `origin/main`. So right after `git fetch` it compares its own file with `origin/main:scripts/release-after-merge.sh`; if they differ it stops at step `fetch`, changing nothing, and prints the command that brings the checkout up to date (`git -C <checkout> merge --ff-only origin/main`). Run that and rerun. A checkout behind main would otherwise run old logic, for example the pre-kgate prepare route that stopped the v0.67.0 release on the pod at `cargo: command not found`.
