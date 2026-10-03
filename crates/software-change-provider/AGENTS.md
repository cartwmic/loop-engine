# software-change-provider agent instructions

## Scope and authority

Root `AGENTS.md` governs checkout operations and authority; this guide adds only crate-local procedure. Do not change frozen provider requirements or protocols without the root owner-acceptance process. Product purpose and install are in [README.md](README.md), not this runbook.

## Workflow

For engine-driven runs, use the allocated `artifact_root` in full show unless the driver must own a specific pre-existing external directory; only then set it explicitly in a copied profile. For direct provider requests, always supply the explicit root. Load [README Usage](README.md#usage) for allocation prerequisites.

When editing or driving this provider, load the [skill](skills/using-software-change-provider/SKILL.md) for input preparation, review intake and phase checkpoint procedure. For the read-only reviewer packet contract, consult the [reviewer protocol](data/reviewer-protocol.md). For executable transport examples, load checkout-root `scripts/software-change-journey.py`; its synthetic judgments never establish semantic review.

For checkpoint operations, align the external driver's cwd and provider launch cwd with the same repository root: verification binds the provider process's current repository, not merely checkpoint's `--working-directory`. Do not move a checkpoint across repositories or recreate it to hide a Git/source change. Root Git approval rules remain separate.

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

Before review intake, load the skill's `review-candidates` procedure and its “Preserve review groups” instructions. Before evidence handoff, consult its “Declare evidence applicability explicitly” rule; before challenge commissioning, consult “check ordinary review clearance before challenge review”. These are conditional procedure pointers, not independent policy.
For validation execution, load the skill's `prepare-validation` procedure; for typed external advice, load its `advice-answer` instructions. For the boundary that the engine “never executes agents, advisors or proof commands”, consult [README.md](README.md).
