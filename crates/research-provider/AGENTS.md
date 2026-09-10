# Agent instructions for research-provider

## Scope

This file covers work in this crate: the `research` binary, shipped configs/templates/protocol under `data/`, crate tests, and the research skills under `skills/`. Engine CLI behavior and workspace-wide checks are documented at the repository root.

The provider is deterministic only. It validates artifact schemas and revision links, then aggregates externally supplied `review-evidence` at verify and synthesize. It does not generate prompts, invoke a model, fetch the web, edit artifacts, or decide whether findings are true.

Providers author worker-facing role and output content; the engine only transports and mechanically enforces it.
Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression.
Exit 0 does not establish a valid deliverable.

## Authority

Drive a standard run with [README.md](README.md) and [skills/using-research-provider/SKILL.md](skills/using-research-provider/SKILL.md). Drive candidate extraction with [skills/using-generate-prd/SKILL.md](skills/using-generate-prd/SKILL.md). Evidence shape and adjudication rules are [data/reviewer-protocol.md](data/reviewer-protocol.md). Repository-root AGENTS.md and docs/agent-usage.md govern checkout-wide operation and CLI envelopes.

Per-run obligations are frozen in immutable `initial_input` (`review_policies`, `artifact_schemas`, `config_version`, `artifact_root`). Ordinary `show` is the focused action handoff in candidate source. Use `show --view full` for complete frozen input, context and invocation/change reports. Status-only `show --view status` and human `show --compact` do not arm mutation; action/full reads do. Changing a source profile does not change an existing run, and candidate functionality does not upgrade released v0.19.0 operations. No policy, schema, prompt, or artifact shape is baked into provider code — those arrive in config data.

Shipped profile version currently in-tree: `research-1`. Evidence `config_version` must match the run's frozen value, not whatever file is currently shipped. Verify axes are `claim-grounded` and `adversarial`; synthesize axes are `cited-conclusion` and `scope-faithful`. `data/configs/generate-prd.json` keeps the same research topology and schemas but points its authoring guidance at `data/templates/generate-prd/`; it emits a provisional candidate and never changes PRD authority.

Simple-first/YAGNI/KISS apply to this provider under the product-wide engine direction: added complexity needs a meaningful current reason and inadequate simpler alternative in the ordinary design, not a new justification framework. Existing implementation/protocol/dependency choices have no presumption of preservation. Do not add software-change finding, steering, grouped-review or criterion features here; Generate-PRD candidates remain provisional.

Common engine recovery uses repository `docs/agent-usage.md` and the required engine skill: owner-attested amend-binding corrects future execution only; invoke preview/controls remain per-attempt; cancellation requires recorded ownership and verified cleanup. Overrun never authorizes overlapping retry/departure. Ten-second cancellation applies per controller acquisition/resumption, not across an interrupted controller's unbounded operator-delay gap. Explicit event override is exceptional, permanently labeled, and never research evidence or reviewer pass. Normal provider evidence rules below remain unchanged.

Bound fan-out delivery defaults to compact context for all workers, omitting only engine-owned top-level `data.loop_engine_origin` from cloned records while preserving meaningful context and concise origins. Independently captured full verification inputs and core/provider history remain intact; actual stdin and raw ad-hoc bytes stay truthful. Follow the required engine skill for capture/monitor behavior; advisory summaries and coordinator guidance confer no research judgment or start authority.

The synthetic Generate-PRD journey fixture's three predetermined lookups are bounded test evidence collection, not exhaustive contrary-evidence search. Actual extraction follows the [Generate-PRD skill](skills/using-generate-prd/SKILL.md) and requires repository discovery without a predetermined requirement list. Candidates still require exact owner acceptance and authorized commit; source integration and synthetic journeys do not establish semantic completeness or final audit.

## Workflow

Do before editing `artifact_schemas`: use only the bounded object/array/string keyword subset in [Compatibility limits](README.md#compatibility-limits), and consult [src/schema.rs](src/schema.rs). Ordinary `$ref`, `oneOf`, `pattern`, and numeric schemas are unsupported and cause rejection; this is not general JSON Schema.

```sh
cargo test -p research-provider --lib
python3 scripts/run-nextest.py --filter research
cargo clippy -p research-provider --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/assert-generate-prd-profile.py
```

Run these commands from the repository root. The package `--lib` command covers units only (`autotests = false`). CLI, describe, evaluate, embedded-data and shipped-data integration suites belong to the central workspace target and use the fresh-binary filtered runner above. These focused checks do not replace the centralized topology and workspace completion gate in root AGENTS.md or the public-boundary journeys.

Do update the explicit [src/embedded_data.rs](src/embedded_data.rs) `FILES` manifest when adding shipped profiles, templates or skills: new files are not automatically embedded or dumped. Run `python3 scripts/run-nextest.py --filter research` from the repository root to exercise the shipped-data drift integration tests. After any crate change, also run the repository source journey from the repo root (build `loop-engine`, `research`, and `bookends-check` first):

```sh
python3 scripts/research-journey.py \
  --mode source \
  --engine target/debug/loop-engine \
  --provider target/debug/research \
  --profile crates/research-provider/data/configs/standard.json
```

That journey command is a harness example, distinct from the production start; do not copy isolation flags from it into production start.

Register `target/debug/research` under exact alias `research` with an absolute command path in uncommitted provider TOML. Copy the standard profile. Use the normal user catalog. No database or artifact override unless the human explicitly requests isolation in this session; other runs and old preferences do not authorize isolation. Before start, load [Setup](skills/using-research-provider/SKILL.md#setup), including canonical engine Deterministic setup. `start` allocates the durable directory and records its absolute path in object `initial_input`; `show` then reveals it. Artifact filenames are fixed: `brief.json`, `sources.json`, `verification.json`, `report.json`.

Topology: `scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise*` edges. Checked `scoped` and `gathered` schema-check the current subject and revision links. Checked `verified` and `completed` then aggregate evidence. Check-free `revise` does not evaluate.

At review states, follow `data/reviewer-protocol.md`: comprehensive first review, triage candidates before append or mutation, append only accepted in-scope material failures or conforming passes, confirmation review after fixes. No waivers. Verification-local `verification.json` corrections stay in verify (edit the artifact, retry `verified`). Report-local `report.json` corrections stay in synthesize (edit the artifact, retry `completed`). From synthesize, nearest `revise` is verification-owned only; use `revise-sources` or `revise-brief` for earlier owners.

For a current standing fail, obtain genuine conforming reconsideration from the same exact author (name and kind) for the current subject revision/config, or fix the actual subject, bump revision and obtain fresh independent evidence. Another reviewer's pass cannot clear that fail; a gratuitous revision bump cannot evade an accepted defect. Load the [evidence protocol](data/reviewer-protocol.md) for supersession and the distinct malformed-evidence rule.

The subject's declared author never counts toward its own review. Stale `subject_revision` never satisfies. A material edit without a revision bump is an accepted claim-trust residual.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use parent-directory segments in those links. Refer to repository-root files such as docs/agent-usage.md in prose.

`research --help`/`-h` names `describe`, `evaluate`, and `data-dump`. `--version`/`-V` prints the Cargo package version. `data-dump DIR` materializes embedded data and refuses to overwrite existing target files, including `generate-prd.json`, its templates, and its skill.

## Completion and Handoff

Crate work is complete when `python3 scripts/run-nextest.py --filter research` (including centralized integration coverage), `cargo clippy -p research-provider --all-targets -- -D warnings`, the source research journey, and the Generate-PRD profile assertion and source journey pass, shipped configs/templates/protocol/skills still match runtime behavior, and this crate's README/AGENTS.md remain accurate.

Handoff the files changed, commands run, and residuals: unread locked artifacts, synthetic test evidence is not semantic quality, and round state lives outside the provider.
