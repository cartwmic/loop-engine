# Agent instructions for software-change-provider

## Scope

This file covers work in this crate: the `software-change` binary, shipped configs/templates/protocol/calibration under `data/`, crate tests, `docs/prd.md`, and `skills/using-software-change-provider/`. Follow repository-root `AGENTS.md` for checkout-wide procedure and completion gates. This file cannot weaken those obligations.

Keep `describe` and `evaluate` deterministic. They must not launch models, edit authored subject artifacts or judge semantic truth. Allowed evaluation into validation records checkpoint history as documented in [README.md](README.md#overview). The separate deterministic `setup` helper assembles per-run bindings from shipped review data and explicit caller commands; it does not start or progress a run.

## Authority

Frozen requirements this crate's acceptance suite traces to (R1–R29, A1–A16, including amendments) live in [docs/prd.md](docs/prd.md). [README.md](README.md) owns the public contract, [data/reviewer-protocol.md](data/reviewer-protocol.md) owns evidence shape, and the [provider skill](skills/using-software-change-provider/SKILL.md) owns driving procedure. Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and generic CLI forms. Bookends IDs belong to the enabled repository's configured living PRD; this crate must not mint them.

Operationally, before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative. This summary remains subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority. Apply LE-107's observed-ordinary-failure/smaller-mechanism burden; retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records. Preserve R16's independent-author aggregation and visible verdict history. R21's retained review, materiality, triage and source-visibility rules remain; normal disposition is exact-source driver judgment, and exceptional owner override is distinct engine history, never reviewer success; delivered stable references use context-record or invocation/assignment identities, with one explicit evidence-applicability declaration and no driver-copied mechanical coordinates.

Keep draft amendments explicitly pending until accepted. Preserve each older run's original runtime, skills and obligations, including frozen contract-v2 runs. Historical missing ownership is unsupported for cancellation control. Candidate source does not grant migration or arbitrary-PID cleanup authority.

When changing profile or evidence handling, verify the independent criterion/goal author floors of 1/2/2 and the configured review stages against the PRD. Accepted-unresolved findings block across revisions; retired-author needs recorded roster departure and replacement coverage. Use the owning protocols and tests for the full rules.

## Workflow

Run every command below from the repository root. Follow its tools/cache preflight before compiling. For crate changes, run focused checks and the full source journey:

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

### Maintenance checks

- Adding or renaming a `data/` file requires updating the hand-maintained `FILES` shipping inventory in [src/embedded_data.rs](src/embedded_data.rs). Run `python3 scripts/run-nextest.py --filter embedded_manifest_exactly_matches_on_disk_data_tree` from the root; an on-disk file is not automatically shipped by `data-dump`.
- When editing the CLI, check that `software-change --help`/`-h` names `describe`, `evaluate`, `setup`, `data-dump`, `checkpoint`, `review-candidates`, `commission`, `run-validation`, `prepare-validation`, and `run-plan-graph`. Keep hidden `stdin-exec` and `validation-command` out of help. Verify `--version`/`-V` against the Cargo package version. Run the executable check below after building.
- `setup --output PATH` atomically replaces an existing destination. Choose a new run-specific path, or inspect and back up an existing file before deliberately replacing it. `data-dump DIR` refuses existing target files; use a fresh empty dump root.
- Graph and ad-hoc repair execution delete `implementation-report.json` and `implementation-checkpoint.json` after preparation and before workers start. A failed worker can leave them absent. Use `invoke --preview` for non-mutating preparation, and execute only when intentionally regenerating that artifact root's implementation proof through the skill's [implementation correction route](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide).
- `run-validation` replaces `validation-report.json` before launching workers; failure can leave an incomplete report and stale checkpoint. Use [prepare-validation](skills/using-software-change-provider/SKILL.md#prepare-validation-from-retained-commands) for inert preparation from retained captures. Execute validation only when intentionally regenerating that artifact root's proof, then use the [validation correction/checkpoint procedure](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide). `checkpoint` writes or replaces the selected phase's checkpoint. Preserve retained history/captures; do not resurrect superseded proof as current after failed replacement.
- When changing hidden `stdin-exec`, preserve its session-directory contract and run `python3 scripts/run-nextest.py --filter stdin_exec`. For a stdin path with a directory parent, an unset `PI_CODING_AGENT_SESSION_DIR` creates the parent's `sessions` directory and injects its absolute path; an inherited value is preserved. Keep both branches covered in [tests/cli/stdin_exec.rs](tests/cli/stdin_exec.rs).
- Local Markdown links must resolve under this crate directory. Do not use `..` in them. Name repository-root files in prose.

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
| Select authoring/review slots | [Work-slot policy](skills/using-software-change-provider/SKILL.md#work-slot-policy-confirm-before-start): the default catalog has fifteen slots, including `intent-draft`, `intent-review`, `validation-draft` and `validation-review`. The workflow has sixteen states; `intent-draft` runs in `explore`, and `validation-draft` runs in `validation`. |
| Commission, capture or record review | [Reviewer protocol](data/reviewer-protocol.md) and [per-gate loop](skills/using-software-change-provider/SKILL.md#per-gate-loop) |
| Correct execution or route late findings | [Proportional late-finding guide](skills/using-software-change-provider/SKILL.md#proportional-late-finding-guide), plus the engine companion and CLI reference |
| Change artifact schemas | [Artifact templates](README.md#artifact-templates) and [schema implementation](src/schema.rs); verify the supported subset and designated ID patterns |
| Create or accept checkpoints | [README Overview](README.md#overview) and the skill's checkpoint procedure; account for the submodule-content boundary |
| Change calibration material | [Calibration procedure](data/calibration/PROCEDURE.md) and its manifest; preserve source identities and actual returned judgments |

Before authoring AC-N intent, task links or final proof, use the skill's [criterion spine and Bookends overlay](skills/using-software-change-provider/SKILL.md#criterion-spine-and-bookends-overlay). Enabled coverage requires one `prd_traceability` disposition per criterion. A `candidate` can block completion; `not-applicable` does not waive or fulfill its criterion.

`review-candidates` reads one completed `show --view full` envelope from stdin. Drivers inspect its output and record qualifying evidence through the per-gate procedure. Give the other helpers their documented full input too; compact delivery does not replace independent verification snapshots. For checked implementation/validation events, run from the intended repository checkout and follow the skill's checkpoint/Bookends checks.

Reviewers return judgments and consume retained proof. Drivers own deterministic checks, capture triage, `show`, `append`, `event` and progression. Follow the root Git checkpoint decision and staged-diff checks; workers do not stage or commit independently.

## Completion and Handoff

Crate work is complete only when the repository-root completion gate, applicable crate checks and source journey have passed, shipped data matches runtime behavior, and this crate's README/AGENTS.md are current. Complete calibration when its procedure applies. Integrate changes into the repository's authoritative documents.

Handoff must identify files changed and why, commands and outcomes, current Git revision and commit/push/uncommitted state, run IDs and database paths, coverage/revision identities, remaining risks and out-of-scope follow-up. Report missing or failing proof explicitly. Keep the limits visible: artifact reads are not locked through transition commit, synthetic journeys do not establish semantic quality, and durable round state belongs to the engine.
