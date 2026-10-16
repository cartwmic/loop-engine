# research-provider agent instructions

## Scope and authority

Root `AGENTS.md` governs checkout operations and authority; this guide adds only crate-local procedure. Do not change frozen provider requirements or protocols without the root owner-acceptance process. Product purpose and install are in [README.md](README.md), not this runbook.

## Workflow

When preparing artifacts, load [README Usage](README.md#usage) for root allocation prerequisites; author only under the `artifact_root` shown in frozen initial input.

Choose up front: investigations use `data/configs/standard.json` and the [research skill](skills/using-research-provider/SKILL.md); requirement extraction uses `data/configs/generate-prd.json` and the [generate-PRD skill](skills/using-generate-prd/SKILL.md). Extraction does not authorize implementation or requirement acceptance. For generate-PRD, use the selected profile's `extra.template_root` (`data/templates/generate-prd`), not the standard template paths printed by generic workflow show instructions. Load its `brief.md`, `sources.md`, `verification.md` and `report.md` when authoring those phase artifacts, and retain the template's `prd-candidate.md` as the human handoff alongside machine artifacts. Ordinary research-shaped output is not a substitute for the requirement candidate.

Generate-PRD provider gates validate machine JSON and external evidence, not the candidate Markdown grammar. Before handoff, inspect active catalogs under checkout-root `AGENTS.md`, build current `bookends-check`, resolve BIN_DIR from Cargo metadata, and validate the actual candidate (not the journey's synthetic fixture):

```sh
cargo build --locked -p bookends-check --bin bookends-check
BIN_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
: "${ARTIFACT_ROOT:?absolute run artifact directory}"
unset BOOKENDS_BYPASS
"$BIN_DIR/bookends-check" candidate "$ARTIFACT_ROOT/prd-candidate.md" > "$ARTIFACT_ROOT/candidate-check.txt"
grep -qx GREEN "$ARTIFACT_ROOT/candidate-check.txt"
```

A human must accept or reject the candidate before any commit to checkout-root `docs/PRD.md`; a parser pass is not acceptance. For focused profile checks, use checkout-root `RESEARCH_PROVIDER="$BIN_DIR/research" python3 scripts/assert-generate-prd-profile.py` against a freshly built research binary; preserve failures rather than relabeling them.

When editing or driving this provider, load the selected skill for general external-loop concepts; the supported crate-specific command set below overrides the generic tool list in checkout-root `docs/agent-usage.md`; load the [reviewer protocol](data/reviewer-protocol.md) when commissioning judgments. The engine never executes agents, advisors or proof commands. Drivers own external execution and semantic acceptance.

This provider supports `commission`, `review-candidates`, `author-counts`, `setup` and `data-dump`, plus stdin describe/evaluate transport. The generic references in checkout-root `docs/agent-usage.md` to `checkpoint`, `prepare-validation` and `source-identity` do not apply here. Drivers author artifacts externally; checked events evaluate them and current external evidence, without a prepare-validation step.

When recording reviewer judgments, load the selected skill’s Per-gate loop for `review-candidates` intake and evidence append procedure. For executable packet examples, use checkout-root `scripts/research-journey.py` and `scripts/external_work_driver.py` (`record_reviews` demonstrates intake only; its synthetic judgments never establish semantic review). `research commission verify` or `research commission synthesize` reads a full show envelope directly from stdin.

Commission enumerates all four artifact inputs, including downstream `report.json` during verify. Do not invent synthesis to make that list empty: prepare phase-required `brief.json`, `sources.json` and `verification.json`, inspect the declared verification subject/policies, and treat missing downstream report as a later synthesis input. During synthesis, author `report.json` against retained verification. Follow the journey's phase ordering; early diagnostics are not a mandate to fabricate future results.

From the repository root, inspect active catalogs and preserve frozen executable paths under checkout-root `AGENTS.md` before rebuilding changed provider behavior. For focused checks:

```sh
cargo build --locked -p loop-cli -p research-provider
cargo test -p research-provider
python3 scripts/run-central-tests.py --filter research
```

Crate manifests set `autotests = false`: the provider Cargo command above runs unit tests, not the integration files under `tests/`. The central runner executes focused integration checks and builds the required binaries. A narrower unit check may use `cargo test -p research-provider <filter>`.

The source research journey checks wrapper reproduction before `dist plan` and independently asserts the release graph. It requires cargo-dist 0.32.0 provisioned in an approved environment (`dist --version`). Obtain approval before changing persistent tooling; any disposable tool root must be driver-owned and external. If the prerequisite is unavailable, report the journey as blocked rather than complete. Release generation/checking uses checkout-root `python3 scripts/generate-release.py` / `python3 scripts/generate-release.py --check`, never stock workflow generation; only the vendor CI-byte comparison is replaced, not other generation/version checks.

Final proof is checkout-root `scripts/baseline.sh` plus the root `AGENTS.md` boundary-table union: research/generate-PRD changes require research, generate-PRD and external-work journeys. Draft/audit README/AGENTS through policy-document under the root document procedure. Use the root packaged-smoke rule only for packaging changes. Keep `src/embedded_data.rs` inventories current when adding bundled data. Use a fresh external test directory and explicit provider config; root `AGENTS.md` owns catalog/environment safety and Git approval rules. Do not substitute compact/status observations for fresh action/full observations before mutations.

## Completion

Follow root completion/handoff and proof gates. Report focused test results, actual catalog/artifact paths, remaining blockers and whether bundled data changed. Preserve failed external judgments; changes to the target require fresh applicable evidence, not relabeled receipts. No staging, commit or push without root authorization.
