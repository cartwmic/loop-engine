# Agent instructions

## Scope

This file governs the entire loop-engine checkout: `loop-cli` (`loop-engine`), `loop-core`, `loop-integrations`, `bookends-check`, the software-change/policy-document/research providers, `tests/workspace-integration`, `tests/fixtures`, `tests/bounded_process.rs`, scripts, skills and release workflows. `crates/bookends-check` has no nested guide and follows this file.

Primary workflow work belongs outside the engine/provider. Do not invent engine policy, review orchestration or core semantics. Humans start at [README.md](README.md). Nested `AGENTS.md` files add crate-local procedure and must not contradict this file.

## Authority

Each source governs its named scope:

1. Root `AGENTS.md`: checkout operations.
2. Nested `AGENTS.md`: additional crate-local procedure.
3. [docs/PRD.md](docs/PRD.md): living engine product requirements.
4. Frozen provider requirements and protocols: provider requirements and protocol contracts.
5. Provider README: that provider's public contract.
6. Provider skill: driving procedure.
7. [docs/agent-usage.md](docs/agent-usage.md): generic CLI forms and loop operations.

README.md is the human overview. Requirement changes need exact owner acceptance and an authorized commit. Unrelated recovery drafts remain drafts. Source integration alone establishes no requirement authority or semantic approval.

### Required referential reminders

The journey self-test checks the following LE-107 and execution-ownership reminders. Their linked PRD and skills govern policy and procedure; these reminders do not replace those sources.

Operationally, before the commit introducing `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, that PRD is authoritative. This AGENTS summary is subordinate and referential, not a second product policy: for any new provenance burden, name the observed ordinary-use failure and why a smaller mechanism using existing durable state, history, capture, or driver judgment is insufficient. Keep driver-authored metadata small, trust explicit materiality and applicability declarations except for cheap mechanical identity mismatches, prefer the narrowest honest correction, and preserve rich engine-generated history. Apply these limits with reference to [`docs/PRD.md`](docs/PRD.md) LE-107.

## External-driver loop

The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. The driver may use any agents or harness.

1. Inspect action/full `show` and provider `commission`. Early commissions return `missing_inputs`; supply those inputs before treating a commission as ready.
2. Do the work externally. Reviewers return judgments; the driver runs deterministic checks and owns progression.
3. Submit external judgments through `review-candidates`, external `command_results` through `prepare-validation`, and accepted context through engine `append`.
4. Read action/full `show`, then send the checked `event`; re-read after each transition. A successful process exit alone does not prove the deliverable.

Preserve review groups, self-review declarations, author counts and amendments. Declare evidence applicability explicitly; check ordinary review clearance before challenge review. Respect declared source scopes and current source identity. Record residual risks honestly and cover every applicable sufficiency axis; zero axes do not require invented reviews. Reuse evidence only while its declared applicability remains true. Typed advice answers are externally authored context of kind `advice-answer`.

## Workflow and proof

Run from the repository root. Load the engine and active provider skills. Use policy-document for README/AGENTS authoring or assessment. Requirement changes require owner acceptance. The designated final-proof owner supplies tool/cache startup, then runs the current stable-tree matrix, nextest/doctests, stock-Cargo compatibility, dependency audit, clippy and formatting. Workers run focused checks only. Missing proof blocks completion.

```sh
cargo build --workspace --all-targets
python3 scripts/external-work-journey.py --engine target/debug/loop-engine --software target/debug/software-change --policy target/debug/policy-document --research target/debug/research --checker target/debug/bookends-check --output-root /absolute/fresh-external-work
python3 scripts/software-change-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/software-change --profile crates/software-change-provider/data/configs/high-rigor.json --traversal-depth full --jobs 2
for mode in draft audit; do
  python3 scripts/policy-document-journey.py --engine target/debug/loop-engine --provider target/debug/policy-document --profile crates/policy-document-provider/data/readme.json --mode "$mode"
done
python3 scripts/research-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/research --profile crates/research-provider/data/configs/standard.json
python3 scripts/generate-prd-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/research --checker target/debug/bookends-check --profile crates/research-provider/data/configs/generate-prd.json
```

For packaging boundaries additionally run the applicable installed/archive packaged-smoke mode with actual package identity. Keep hand-maintained embedded_data.rs FILES inventories current. Register central integration modules in tests/workspace-integration/tests/workspace.rs and verify inventory/topology. Public tests need eligible bookends.toml pathspecs and recognized CI collection commands.

Before push, build the current checker and run the repository gate:

```sh
CHECKER_BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
cargo build --locked -p bookends-check --bin bookends-check
PATH="$CHECKER_BIN_DIR:$PATH" scripts/bookends-check-gate.sh </dev/null
```

## Document and Git safety

- Draft or revise README.md/AGENTS.md through policy-document using a copied `readme-3`/`agents-3` profile with `mode: "draft"`. Use `mode: "audit"` for assessment. Preserve `target.id` and `profile_version` unless intentionally authoring a custom profile. Local links resolve under the target's directory; `..` is rejected, so crate docs cannot markdown-link outside the crate.
- After implementation triage, obtain the explicit human Git checkpoint decision before pre-review proof. A repository checkpoint does not create a commit. Workers never stage or commit independently. Authorized Git identity changes require fresh applicable proof; do not relabel receipts.
- Ask before committing, pushing, destructive actions or worktree lifecycle changes. Before a commit, inspect `git diff --cached --name-status` and `git diff --cached`; this checkout has no tracked secret/runtime-artifact detection hook. Never commit secrets, machine-local provider TOML, run databases or captures.
- Never hand-edit `.github/workflows/release.yml`. Change dist metadata and regenerate. Direct pushes to main run read-only preflight. Publication uses the separately authorized sequence below. Do not force-push main.

### Authorized release sequence

Only after owner authorization, replace `X.Y.Z` with the approved version (without `v`). [prepare-release.sh](scripts/prepare-release.sh) requires a clean tree and owns version/lockfile preparation, locked builds and release assertions:

```sh
VERSION="X.Y.Z"
TAG="v$VERSION"
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

## Completion and handoff

Report changed files, exact command outcomes and missing checks, Git identity and staged/committed/pushed state, actual run/catalog paths, residual risks and owner follow-up. No checked transition establishes semantic review.
