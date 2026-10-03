# software-change-provider agent instructions

## Scope and authority

Root `AGENTS.md` governs checkout operations and authority; this guide adds only crate-local procedure. Do not change frozen provider requirements or protocols without the root owner-acceptance process. Product purpose and install are in [README.md](README.md), not this runbook.

## Workflow

For engine-driven runs, use the allocated `artifact_root` in full show unless the driver must own a specific pre-existing external directory; only then set it explicitly in a copied profile. For direct provider requests, always supply the explicit root. Load [README Usage](README.md#usage) for allocation prerequisites.

When editing or driving this provider, load the [skill](skills/using-software-change-provider/SKILL.md) for input preparation and external evidence procedure; load the [reviewer protocol](data/reviewer-protocol.md) when commissioning judgments, using the current packet contract rather than the protocol's obsolete plain-show pipe/bound-worker examples. `review-candidates` requires a JSON packet with `show`, `assignment`, `request`, and `original` (plus a driver-confirmed `projection` when needed); the assignment declares `gate`, `stage`, `group`, `author: {"name": ..., "kind": "agent"}`, `subject` and the exact key `subject_revision`. Both `request` and `original` require exactly one envelope field: `{"text": "..."}` for retained inline text or `{"reference": "/absolute/capture"}` for a retained file. Use this packet shape instead of the obsolete pipe example. For executable preparation/intake examples, load checkout-root `scripts/software-change-journey.py`, its review loop; those synthetic judgments demonstrate transport only, never semantic review. Substitute retained genuine request/output, freeze the assignment to commission's policy group/stage/subject/revision, then inspect the returned `records` before explicit evidence append:

```sh
"$BIN_DIR/software-change" review-candidates < /absolute/proof/review-packet.json
```

The packet's original judgment JSON uses `assessment_id` and `judgments` with `axis`, `result`, `findings` and grounded `grounds` (`reason` and evidence `locator`s). Non-JSON judgments need a driver-confirmed top-level projection while preserving original text; no formatter may invent substantive outcomes.

Checkpoint creation is a separate required operation, not a Git commit. After authoring current `implementation-report.json` and before the checked implementation transition, create the implementation checkpoint; after the validation report is ready, create the validation checkpoint. Replace ARTIFACT_ROOT with the absolute run artifact directory and BIN_DIR with Cargo's metadata-resolved debug binary directory:

```sh
: "${ARTIFACT_ROOT:?absolute run artifact directory}"
: "${BIN_DIR:?absolute active Cargo debug binary directory}"
"$BIN_DIR/software-change" checkpoint --phase implementation --artifact-root "$ARTIFACT_ROOT" --working-directory "$PWD"
# Run this only after validation-report.json is ready:
"$BIN_DIR/software-change" checkpoint --phase validation --artifact-root "$ARTIFACT_ROOT" --working-directory "$PWD"
```

Run checkpoint creation and the engine/provider evaluations from the same repository root: verification binds the provider process's current repository, not merely checkpoint's `--working-directory`. Align the external driver's cwd and provider launch cwd with that root; do not move a checkpoint across repositories or recreate it to hide a Git/source change. Root Git approval rules remain separate.

From the repository root, rebuild before testing changed provider behavior:

```sh
cargo build --locked -p loop-cli -p software-change-provider
cargo test -p software-change-provider
python3 scripts/run-central-tests.py --filter software_change
```

Crate manifests set `autotests = false`: the Cargo command above runs unit tests, not the integration files under `tests/`. The central runner above is required after provider changes and builds its own fresh binary handoff.

Final software-change calibration has an ignored completeness gate; the final-proof owner must explicitly run it with the central runner's fresh handoff:

```sh
python3 scripts/run-central-tests.py --nextest --nextest-run-ignored only --filter calibration_manifest_has_no_pending_rows_for_final_validation
```

Ordinary tests alone do not establish complete calibration. If the gate reports pending rows, preserve the failure and report incomplete calibration rather than relabeling it.

Use the root guide's source journey after provider changes; use its packaged-smoke rule only for packaging changes. Keep `src/embedded_data.rs` inventories current when adding bundled data. Root `AGENTS.md` owns catalog/environment safety, observation and Git approval rules.

## Completion

Follow root completion/handoff and proof gates. Report focused test results, actual catalog/artifact paths, remaining blockers and whether bundled data changed. Preserve failed external judgments; changes to the target require fresh applicable evidence, not relabeled receipts. No staging, commit or push without root authorization.

Preserve review groups, self-review declarations and effective author counts. Declare evidence applicability explicitly; check ordinary review clearance before challenge review.
The engine never executes agents, advisors or proof commands. Supply genuine external `command_results` to `prepare-validation`; typed external advice is context kind `advice-answer`.
