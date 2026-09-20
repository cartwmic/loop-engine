# Agent instructions for software-change-provider

## Scope

This file covers work in this crate: the `software-change` binary, shipped configs/templates/protocol/calibration and `reconciliation-schema.json` under `data/`, crate tests, `docs/prd.md`, and `skills/using-software-change-provider/`. Follow repository-root `AGENTS.md` for checkout-wide procedure and completion gates. This file cannot weaken those obligations.

Keep `describe` and `evaluate` deterministic. They must not launch models, edit authored subject artifacts or judge semantic truth. Allowed evaluation into validation records checkpoint history as documented in [README.md](README.md#overview). The separate deterministic `setup` helper assembles per-run bindings from shipped review data and explicit caller commands; it does not start or progress a run.

## Authority

Frozen requirements this crate's acceptance suite traces to (R1–R29, A1–A16) live in [docs/prd.md](docs/prd.md). The accepted boundary is Sections 1–9 plus the separately accepted, committed new-run contract in Section 11. Section 10 remains an explicitly pending proposal, not authority; neither source integration nor an unrelated amendment accepts it. Uncommitted requirement additions acquire authority only after exact owner acceptance and an authorized commit. [README.md](README.md) owns the public contract, [data/reviewer-protocol.md](data/reviewer-protocol.md) owns evidence shape, and the [provider skill](skills/using-software-change-provider/SKILL.md) owns driving procedure. Repository-root `AGENTS.md`, the engine `docs/PRD.md`, and `docs/agent-usage.md` govern checkout-wide operation, live requirement authority, and generic CLI forms. Bookends IDs belong to the enabled repository's configured living PRD; this crate must not mint them. The provider's reconciliation state checks the mode-specific result shape; the driver and owner still own semantic classification, document edits, owner acceptance, Git authorization, and commit.

When sources overlap, follow the root authority order: repository-root `AGENTS.md` for checkout and completion gates; this nested file for additional crate procedure; engine `docs/PRD.md` for living engine requirements; the provider PRD and reviewer protocol for frozen provider requirements and evidence contracts; `README.md` for the public contract; the provider skill for driving procedure; and engine `docs/agent-usage.md` for generic CLI forms. Live engine requirements do not migrate an existing run's frozen provider obligations. Shipped schemas and source define exact accepted fields only where those higher-level documents leave the encoding open. A skill or README pointer is preferred to copying its full procedure here.

### Required referential reminders

The repository journey gate checks the following retained LE-107/evidence reminders. They are references to the governing PRDs and protocols, not a duplicate driving procedure. Load those sources before applying them.

Operationally, before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative. This summary remains subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority. Apply LE-107's observed-ordinary-failure/smaller-mechanism burden; retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records. Preserve R16's independent-author aggregation and visible verdict history. R21's retained review, materiality, triage and source-visibility rules remain; normal disposition is exact-source driver judgment, and exceptional owner override is distinct engine history, never reviewer success; delivered stable references use context-record or invocation/assignment identities, with one explicit evidence-applicability declaration and no driver-copied mechanical coordinates.

Keep draft amendments explicitly pending until accepted. Preserve each older run's original runtime, skills and obligations, including frozen contract-v2 runs. Historical missing ownership is unsupported for cancellation control. Candidate source does not grant migration or arbitrary-PID cleanup authority.

When changing profile or evidence handling, verify the independent criterion/goal author floors of 1/2/2 and the configured review stages against the PRD. Accepted-unresolved findings block across revisions; retired-author needs recorded roster departure and replacement coverage. Use the owning protocols and tests for the full rules.

Before implementing or changing reconciliation, load the [public contract](README.md#reconciliation-and-document-integration), [provider requirements](docs/prd.md#reconciliation-and-documentation-authority-le-142-r23-r27-and-a13), and provider skill. Use those sources for the `reconciliation-ready` branch checks and Bookends-on/off boundaries; older v10 runs retain their stored graph. Do not duplicate or reinterpret the driving procedure here.

## Workflow

Run every command below from the repository root. Follow its tools/cache preflight before compiling. Graph/fan-out execution requires an executable `dagu` on PATH with semver >=2.14.0. The required journey commands below exercise the runner's fail-closed prerequisite check before workers launch; no separate prose-only version approval is a gate. Use `dagu version` to diagnose a refusal and follow [Dagu setup](README.md#build-current-source). For crate changes, run focused checks and the full source journey:

```sh
cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check
cargo test -p software-change-provider --lib
cargo test -p software-change-provider --bin software-change
python3 scripts/run-nextest.py --filter software_change
cargo fmt --all -- --check
python3 scripts/software-change-journey.py --self-test
python3 scripts/software-change-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/software-change \
  --data-root "$PWD" \
  --work-root "${TMPDIR:-/tmp}/loop-engine-software-change-journey" \
  --profile crates/software-change-provider/data/configs/high-rigor.json \
  --traversal-depth full \
  --jobs 2
```

Run both library and binary unit tests; the binary includes modules absent from `src/lib.rs`. Integration suites are modules of the central workspace target and use the filtered runner above. These focused checks do not replace the root workspace completion gate. The self-test is required for crate, skill and journey-harness changes and must print `worker-data skill/root policy assertions passed`.

Source full mode must print `full software-change journey passed`, `stitched software-change journey passed`, and `contracted fan-out failure` after their corresponding completed scenarios. It must also print `Package 7b review-candidates scenario passed: selected retry, exhausted assignment, raw capture preservation, deterministic repeated inspection, inert-before-records, and driver-action-afterward progression` after the candidate-pipe proof. Synthetic journeys establish mechanics; semantic review remains separate.

Full-source validation needs all five sibling binaries built above. Ordinary provider setup needs only `loop-cli` and `software-change-provider`. Keep test isolation local to fixtures; production storage follows the root instructions and the skill's setup procedure.

Before generating or starting a new profile, load [review profiles](README.md#review-profiles) and the skill's [exact profile confirmation](skills/using-software-change-provider/SKILL.md#exact-profile-confirmation). Setup success is neither execution nor semantic acceptance.

### Maintenance checks

- When adding tests, register modules in their matching crate suite root (`tests/cli.rs`, `tests/contracts.rs`, `tests/provider.rs`, or `tests/plan_graph.rs`); register new suite roots in repository `tests/workspace-integration/tests/workspace.rs`. `autotests = false` means unregistered tests can be silently skipped. Follow the root topology/inventory gate, not just a passing filtered run.
- Before Bookends-enabled evaluation, inspect `BOOKENDS_BYPASS` and unset only unintended values. Evaluation inherits it despite its absence from provider help, and BYPASS cannot satisfy GREEN-only completion. Use the [overlay procedure](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay) for intentional bypass handling.
- Adding or renaming a `data/` file requires updating the hand-maintained `FILES` shipping inventory in [src/embedded_data.rs](src/embedded_data.rs). Run `python3 scripts/run-nextest.py --filter embedded_manifest_exactly_matches_on_disk_data_tree` from the root; an on-disk file is not automatically shipped by `data-dump`.
- When editing the CLI, check that `software-change --help`/`-h` names `describe`, `evaluate`, `setup`, `data-dump`, `checkpoint`, `review-candidates`, `commission`, `run-validation`, `prepare-validation`, and `run-plan-graph`. Keep hidden `stdin-exec` and `validation-command` out of help. Verify `--version`/`-V` against the Cargo package version. Run the executable check below after building.
- `setup --output PATH` atomically replaces an existing destination. Choose a new run-specific path, or inspect and back up an existing file before deliberately replacing it. `data-dump DIR` refuses existing target files; use a fresh empty dump root.
- Before graph execution, read [execution limits](README.md#limits) and [exact profile confirmation](skills/using-software-change-provider/SKILL.md#exact-profile-confirmation): omitting `--task-worker` launches an unpinned Pi default; direct execution defaults to four tasks in one shared checkout. Pin the worker/model unless the owner accepts the default, and use `--max-active 1` unless write ownership is disjoint.
- Before `run-validation`, ensure the run artifact root and its captures are outside the entire Git checkout, including gitignored subdirectories: the helper rejects in-checkout capture roots. Follow the engine companion's deterministic storage setup; the normal engine-allocated root is the default, not permission to add an override.
- Before graph/repair or validation execution, load [recovery warnings](README.md#troubleshooting) and the [correction route](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide): graph/repair deletes prior implementation proof; validation replaces its report and can leave a stale checkpoint. Use those owners for preview, regeneration and recovery.
- When changing hidden `stdin-exec`, preserve its session-directory contract and run `python3 scripts/run-nextest.py --filter stdin_exec`. For a stdin path with a directory parent, an unset `PI_CODING_AGENT_SESSION_DIR` creates the parent's `sessions` directory and injects its absolute path; an inherited value is preserved. Keep both branches covered in [tests/cli/stdin_exec.rs](tests/cli/stdin_exec.rs).
- Local Markdown links must resolve under this crate directory. Do not use `..` in them. Name repository-root files in prose.
- `data/reconciliation-schema.json` is a shipped source contract. If it is added, renamed, or changed, update `src/embedded_data.rs`, inspect the schema/result-field tests in `src/workflow.rs`, and rerun the provider self-test and source journey.

Run this check from the repository root after CLI changes:

```sh
python3 - <<'PY'
import json, re, subprocess
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--no-deps", "--format-version=1"
], text=True))
version = next(p["version"] for p in metadata["packages"]
               if p["name"] == "software-change-provider")
binary = "target/debug/software-change"
public = "describe evaluate setup data-dump checkpoint review-candidates commission run-validation prepare-validation run-plan-graph".split()
for flag in ("--help", "-h"):
    text = subprocess.check_output([binary, flag], text=True)
    for name in public:
        assert re.search(r"(?<![\w-])" + re.escape(name) + r"(?![\w-])", text), (flag, name)
    for hidden in ("stdin-exec", "validation-command"):
        assert hidden not in text, (flag, hidden)
for flag in ("--version", "-V"):
    assert subprocess.check_output([binary, flag], text=True).strip() == f"software-change {version}", flag
print("software-change public help/version assertions passed")
PY
```

### Load the procedure before acting

| Boundary | Required reading |
|---|---|
| Configure or start a run | [Setup](skills/using-software-change-provider/SKILL.md#setup) and [exact profile confirmation](skills/using-software-change-provider/SKILL.md#exact-profile-confirmation), including the required engine companion |
| Select authoring/review slots | [Work-slot policy](skills/using-software-change-provider/SKILL.md#work-slot-policy-confirm-before-start): bare union discovery has fifteen slots/sixteen states; v11 with all review states live has sixteen slots/seventeen states, including `reconciliation-draft` in `reconciliation`. Inspect the stored graph for the actual run. |
| Commission, capture or record review | [Reviewer protocol](data/reviewer-protocol.md) and [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) |
| Correct execution or route late findings | [Proportional late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide), plus the engine companion and CLI reference |
| Author plan tasks | IDs must match `[A-Za-z0-9_-]+` and cannot be `summarizer`; consult [plan-task troubleshooting](README.md#troubleshooting) and its owning repair route before accepting a plan. |
| Change artifact schemas | [Artifact templates](README.md#artifact-templates) and [schema implementation](src/schema.rs); verify the supported subset and designated ID patterns |
| Create or accept checkpoints | [README Overview](README.md#overview) and the skill's checkpoint procedure; account for the submodule-content boundary |
| Change calibration material | [Calibration procedure](data/calibration/PROCEDURE.md) and its manifest; preserve source identities and actual returned judgments |

Before authoring AC-N intent, task links or final proof, use the skill's [criterion spine and Bookends overlay](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay). Enabled coverage requires one `prd_traceability` disposition per criterion. A `candidate` can block completion; `not-applicable` does not waive or fulfill its criterion.

`review-candidates` reads one completed `show --view full` envelope from stdin. Drivers inspect its output and record qualifying evidence through the per-gate procedure. Give the other helpers their documented full input too; compact delivery does not replace independent verification snapshots. Before checkpointing, read [checkpoint limits](README.md#limits) and [mismatch recovery](README.md#troubleshooting): require valid HEAD, no unmerged entries, UTF-8 repository paths and supported entry types. `--working-directory` selects creation CWD only; checked evaluation uses provider CWD. Create/evaluate in the same checkout and keep its bytes/Git identity stable.

Reviewers return judgments and consume retained proof. Drivers own deterministic checks, capture triage, `show`, `append`, `event` and progression. Follow the root Git checkpoint decision and staged-diff checks; workers do not stage or commit independently.

### Document authoring and reconciliation

- Author or assess this crate's `README.md` and `AGENTS.md` through target-specific `policy-document` runs: use the shipped `readme-2` or `agents-2` profile, preserve target ID/profile version, use `mode: draft` for authoring and `mode: audit` for assessment, and obtain digest-bound evidence for every semantic axis. A policy-document journey is mechanics proof, not a substitute for the actual target run.
- Before starting or progressing a document workflow, load repository `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md` and its engine companion `skills/using-loop-engine/SKILL.md`, not the software-change skill as a substitute. The driver performs deterministic checks, target hashing, evidence triage, `show`, `append`, `event`, and final-envelope inspection; a worker exit 0 is not approval.
- For software-change document decisions, load [reconciliation and document integration](README.md#reconciliation-and-document-integration) and the skill's owning procedure before acting. Do not transplant the new state into an existing frozen run.

Choose the path before acting:

| Work | Use | Do not substitute |
|---|---|---|
| Edit or assess `README.md`/`AGENTS.md` | target-specific `policy-document` draft or audit run | a direct unreviewed edit or a generic fixture journey |
| Change provider behavior/data | current-source `software-change` procedure and focused crate checks | a policy-document run as code proof |
| Prove source behavior | changed `target/debug` binaries and the source journey | an installed/archive result |
| Prove a package/archive boundary | the packaged-smoke procedure and extracted binaries | source compilation alone |
| Resume an existing run | its frozen runtime, profile, and stored graph | replacing it with a new profile or binary |

### Risk boundaries

- **Always:** keep machine-local provider TOML, run databases, captures, generated profiles, credentials, and secret material outside committed source; use fresh dump/output roots when a command refuses replacement; preserve raw attempts and evidence.
- **Ask first:** before changing a frozen profile/run, applying requirement wording, staging or committing, using a database/artifact override, changing a target-specific policy profile, or choosing Bookends-on behavior.
- **Never:** let a worker invoke workflow controls, edit authoritative documents as semantic proof, stage/commit/push, mint Bookends IDs, or use a related ID/parser success/command exit as requirement coverage. Markdown is advisory; rely on the repository gate and CI for enforceable checks.

## Completion and Handoff

Crate work is complete only when the repository-root completion gate, applicable crate checks and source journey have passed, shipped data matches runtime behavior, and this crate's README/AGENTS.md are current. Complete calibration when its procedure applies. Integrate changes into the repository's authoritative documents. For the two provider policy documents, completion additionally requires the prepared target runs to reach checked `end`, with exact run/profile/target identities, current target digests, passing evidence for every configured axis, and retained full envelopes. The read-only `scripts/assert-policy-document-run.py` verifier proves those facts; it does not progress a run.

After each target reaches checked `end`, run this verifier from the repository root. Set the variables to that target's retained full-show file, approved reference/confirmation files, exact profile and approved hash, absolute target path, and reference key (`provider-readme` or `provider-agents`). Do not substitute another target's evidence:

```sh
python3 scripts/assert-policy-document-run.py \
  --show "$FULL_SHOW" --run-reference "$RUN_REFERENCES" \
  --confirmation "$CONFIRMATION_MANIFEST" --profile "$PROFILE" \
  --profile-sha256 "$APPROVED_PROFILE_SHA256" --target "$TARGET" \
  --evidence-schema crates/policy-document-provider/data/semantic-review-worker-output-schema.json \
  --run-key "$RUN_KEY"
```

Require exit 0 and JSON `status: "verified"`, `state: "end"`, `lifecycle: "final"` with the expected target, digest and run identity. Retain that result with the full envelope; this checker does not replace the checked transitions.

Handoff must identify files changed and why, commands and outcomes, current Git revision and commit/push/uncommitted state, run IDs and database paths, coverage/revision identities, remaining risks and out-of-scope follow-up. For policy-document work, include both exact target paths, profile hashes, artifact roots, checked end-state locators, target digests, and raw review evidence. Report missing or failing proof explicitly. Keep the limits visible: artifact reads are not locked through transition commit, synthetic journeys do not establish semantic quality, and durable round state belongs to the engine.
