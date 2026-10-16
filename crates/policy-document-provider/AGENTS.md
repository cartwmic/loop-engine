# policy-document-provider agent instructions

## Scope and authority

Root `AGENTS.md` governs checkout operations and authority; this guide adds only crate-local procedure. Do not change frozen provider requirements or protocols without the root owner-acceptance process. Product purpose and install are in [README.md](README.md), not this runbook.

## Workflow

When editing or driving this provider, load the [skill](skills/using-policy-document-provider/SKILL.md) for general external-loop concepts; the supported crate-specific command set below overrides the generic tool list in checkout-root `docs/agent-usage.md`; load the [reviewer protocol](data/reviewer-protocol.md) when commissioning judgments.

This provider supports `commission`, `review-candidates`, `author-counts`, `setup` and `data-dump`, plus stdin describe/evaluate transport. The generic references in checkout-root `docs/agent-usage.md` to `checkpoint`, `prepare-validation` and `source-identity` do not apply here.

For an executable packet example, use checkout-root `scripts/policy-document-journey.py` and `scripts/external_work_driver.py` (`record_reviews` demonstrates intake shape, but its synthetic judgments are mechanical fixtures, never semantic proof). Draft progression is `ready` from prepare, `passed` from deterministic-review, then `passed` from semantic-review; `revise` returns either review state to prepare.

`commission` takes the frozen profile as inline JSON, not a filename or `@file`; stdin is the completed full show envelope itself (top-level `result`), not a wrapper under `show`; nesting it would silently omit retained context and amendments. For example, after saving that full-show envelope externally:

```sh
BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
"$BIN_DIR/policy-document" commission "$(cat /absolute/proof/profile.json)" < /absolute/proof/full-show.json
```

When recording reviewer judgments, load the skill’s Run loop section for `review-candidates` intake and evidence append procedure. Declare all actual target authors through digest-bound `target-authorship` context before semantic coverage; commission `missing_inputs` blocks readiness when that declaration is missing/stale. Refresh it after rewrites. Current target authors count only under explicit frozen self-review permission, labeled as self-review, never independent coverage.

Two provider-local traps require explicit handling: keep target bytes quiescent throughout review because file edits are not atomically locked/versioned with workflow commit; compare the final file SHA-256 with the digest in accepted review evidence after completion, and obtain new evidence if it changed. Load [target guidance](data/target-guidance.md) for this assessment boundary. `data-dump DIR` preserves the `crates/policy-document-provider/data/` prefix under DIR; find exported profiles there, not directly under DIR.

After completion, compare current target bytes with the saved commission binding (replace both paths):

```sh
python3 - /absolute/target.md /absolute/proof/commission.json <<'PYHASH'
import hashlib, json, pathlib, sys
actual = hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest()
expected = json.loads(pathlib.Path(sys.argv[2]).read_text())["current_target"]["target_sha256"]
assert actual == expected, "target changed after commissioned review"
PYHASH
```

When delivering historical judgments during revisions, load the skill’s Explicit historical review context section. Full show alone does not select prior judgments for reviewer delivery; selected stale findings are historical context, not current proof.

From the repository root, inspect active catalogs and preserve frozen executable paths under checkout-root `AGENTS.md` before rebuilding changed provider behavior. For focused checks:

```sh
cargo build --locked -p loop-cli -p policy-document-provider
cargo test -p policy-document-provider
python3 scripts/run-central-tests.py --filter policy_document
```

Crate manifests set `autotests = false`: the provider Cargo command above runs unit tests, not the integration files under `tests/`. The central runner executes focused integration checks and builds the required binaries. A narrower unit check may use `cargo test -p policy-document-provider <filter>`.

Final proof is checkout-root `scripts/baseline.sh` plus the root `AGENTS.md` boundary-table union: policy-document changes require draft, audit and external-work journeys. Draft/audit README/AGENTS through this provider under the root document procedure, not direct unaudited edits. Release generation/checking uses checkout-root `python3 scripts/generate-release.py` / `python3 scripts/generate-release.py --check`, never stock workflow generation. Use the root packaged-smoke rule only for packaging changes. Keep `src/embedded_data.rs` inventories current when adding bundled data. Root `AGENTS.md` owns catalog/environment safety, observation rules and Git approval.

## Completion

Follow root completion/handoff and proof gates. Report focused test results, actual catalog/artifact paths, remaining blockers and whether bundled data changed. Preserve failed external judgments; changes to the target require fresh applicable evidence, not relabeled receipts. No staging, commit or push without root authorization.
