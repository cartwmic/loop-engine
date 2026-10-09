# Authorized release procedure

Load this procedure when preparing or publishing an owner-authorized release. Root `AGENTS.md` governs authorization, Git safety and proof; this document supplies operating steps and creates no separate policy authority. Run commands from the checkout root. Paths such as `scripts/prepare-release.sh`, `Cargo.toml` and `.github/workflows/` are checkout-root-relative, including when this document lives under `docs/`.

## Prerequisites and stopping rule

The current checkout contains `.github/workflows/release.yml` but lacks `.github/workflows/archive-smoke.yml`, `.github/workflows/push-to-main.yml` and `.github/workflows/preflight.yml`. `Cargo.toml` also lacks the required dist `global-artifacts-jobs = ["./preflight", "./archive-smoke"]` hooks. `scripts/assert-release-gates.py` requires the archive smoke workflow, reusable preflight and generated proof-hook graph; `scripts/assert-push-main-preflight.py` requires the dispatcher and reusable preflight. Both assertions remain mandatory and cannot pass with the current inputs. Direct main pushes currently provide no preflight dispatcher, and the exact-commit lookup below is unavailable.

Stop before release preparation or publication until the owner resolves these prerequisites through separately authorized work and the required assertions pass. Retain actual failures and missing-proof evidence. Documentation changes authorize no CI restoration, gate changes or waiver; publication remains blocked. `scripts/prepare-release.sh` changes version/lockfile bytes before its release assertions, so invoking it cannot safely discover this known blockade. Root AGENTS's local matrix, pinned tools, journeys and Bookends GREEN remain required where applicable; local proof does not substitute for release's successful exact-commit hosted preflight.

Preflight's named “Install dist”, “Build central target for structure and inventory assertions”, “Assert one integration target and current test inventory”, and “Run stock Cargo compatibility gate” steps are unavailable while `preflight.yml` is absent. Root AGENTS retains the local startup and ordered central-handoff/topology/inventory/stock-Cargo commands. Resolve the missing prerequisites before final-proof or publication progression; preserve the commands and their proof obligations.

## Authorized release sequence

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). Checkout-root `scripts/prepare-release.sh` requires a clean tree and owns version/lockfile preparation, locked builds and release assertions:

```sh
VERSION="X.Y.Z"
TAG="v$VERSION"
test ! -e /tmp/loop-engine-dist-plan.json || { echo "Release plan destination already exists; obtain ownership/cleanup approval before proceeding" >&2; exit 1; }
scripts/prepare-release.sh "$VERSION"
```

Review the result and obtain separate authorization for the Git commit, tag and push. After those actions, locate preflight for the exact committed HEAD:

```sh
set -eu
COMMIT="$(git rev-parse HEAD)"
PREFLIGHT_RUN="$(gh run list --workflow push-to-main.yml --branch main --event push --commit "$COMMIT" --limit 1 --json databaseId,status,conclusion --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId')"
test -n "$PREFLIGHT_RUN" || { echo "Exact-commit preflight is missing, pending or failed" >&2; exit 1; }
gh run view "$PREFLIGHT_RUN" --json headSha,status,conclusion,url
PREFLIGHT_LOG="$(mktemp)"
FINAL_CACHE_LOG="$(mktemp)"
gh run view "$PREFLIGHT_RUN" --log >"$PREFLIGHT_LOG"
grep -F 'Start credential-free sccache GHA backend' "$PREFLIGHT_LOG" >/dev/null
grep -F 'Emit hosted sccache statistics and total preflight wall time' "$PREFLIGHT_LOG" >"$FINAL_CACHE_LOG"
grep -E 'preflight_total_wall_seconds=[0-9]+$' "$FINAL_CACHE_LOG"
grep -F '"cache_hits"' "$FINAL_CACHE_LOG" >/dev/null
printf 'Retain release evidence: %s %s\n' "$PREFLIGHT_LOG" "$FINAL_CACHE_LOG"
```

The commit filter selects the approved HEAD; require the displayed `headSha` to match it. These commands fail on missing successful preflight or required current log markers. Inspect and retain the logs as release evidence. Only after this check and publication authorization:

```sh
: "${TAG:?set TAG from the owner-approved VERSION}"
gh workflow run release.yml --ref main -f "tag=$TAG"
```
