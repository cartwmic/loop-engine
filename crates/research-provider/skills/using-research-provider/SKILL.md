---
name: using-research-provider
description: Use when running the research workflow through Loop Engine with the research provider — confirming work-slot bindings and models with the user before start, scoping a question, gathering sources externally, commissioning adversarial review or invoking bound slots, synthesizing a cited conclusion, appending review-evidence, and clearing checked transitions.
---

# Using the research provider

## Overview

`research` is Loop Engine's research reference provider. Search, fetch, and writing happen outside the provider. The binary never retrieves URLs, invokes a model, or judges claim truth. It validates artifact schemas and revision links, then aggregates externally supplied review-evidence at verify and synthesize. Per-run obligations are frozen in immutable `initial_input`.

Workflow: `scope → gather → verify → synthesize → end`.

Drive engine CLI, JSON envelopes, and `show` / `append` / `event` with `skills/using-loop-engine/SKILL.md`. This skill is the research counterpart of `crates/software-change-provider/skills/using-software-change-provider/SKILL.md` and `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md`: same engine loop, different artifacts, gates, and primary work. Do not markdown-link outside this crate. Provider contract: `crates/research-provider/README.md`. Judging and adjudication: `crates/research-provider/data/reviewer-protocol.md`. Artifact shapes: `crates/research-provider/data/templates/`.

## Setup

```sh
cargo build -p loop-cli -p research-provider
```

Register `target/debug/research` under exact alias `research` (absolute `command` path) in uncommitted machine-local provider TOML:

```toml
[providers.research]
command = "/absolute/path/to/target/debug/research"
args = []
```

Copy `crates/research-provider/data/configs/standard.json` to a run-specific file. Shipped profiles have **no** `work_slot_bindings`. Cataloged slots are `scope`, `gather`, `verify`, and `synthesize`. Do not add bindings, and do not start, until the user confirms: (1) driver-performed (omit the key or `{}`) vs which slots to bind; (2) exact `{command, args}` if bound; (3) which models those CLIs will use, encoded in frozen args. Nested inner workers count. Do not call `start` while a bound model-bearing CLI has no model in argv unless the user has explicitly accepted that CLI's unpinned default. Bindings freeze and cannot be patched. Omit `artifact_root` in the usual case; the engine allocates the durable directory and records that absolute path in object `initial_input` (`show` reveals it). Pass `artifact_root` only to isolate files to a caller-chosen absolute existing directory. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Then:

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-standard.json" "my research"
```

Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`. Once the run exists, subject files live under the allocated (or caller) `artifact_root` using fixed filenames: `brief.json`, `sources.json`, `verification.json`, `report.json`. Shipped `config_version` is `research-1`.

For an installed binary, dump into an empty directory first (`research data-dump "$DATA_ROOT"`), then copy `$DATA_ROOT/crates/research-provider/data/configs/standard.json`.

## External research work

Do the primary work outside Loop Engine, then record it in the subject artifacts. Author from `crates/research-provider/data/templates/`.

1. **Scope** — write a question, not a chosen answer. Name observable acceptance, constraints, and non-goals.
2. **Gather** — search and fetch externally. Record sources with stable ids, locators, and extracts later verification can check. `brief_revision` must equal current `brief.json` revision.
3. **Verify** — author claims with cited `source_ids`, support, and a genuine challenge (or an explicit record that none was found after search). `sources_revision` must equal current `sources.json` revision. Request `verified` to clear schema and links before commissioning review.
4. **Synthesize** — write a cited conclusion that answers the brief, with `{claim_id, source_id}` pairs. Do not introduce material claims verification never checked. `verification_revision` must equal current `verification.json` revision. Request `completed` to clear schema and links before commissioning review.

## Gate map

| Event (from state) | Subject checked | Evidence gate |
|---|---|---|
| `scoped` (scope) | `brief.json` | schema only |
| `gathered` (gather) | `sources.json` | schema and `brief_revision` link |
| `verified` (verify) | `verification.json` | schema, `sources_revision` link, then `verify` (`claim-grounded`, `adversarial`) |
| `completed` (synthesize) | `report.json` | schema, `verification_revision` link, then `synthesize` (`cited-conclusion`, `scope-faithful`) |
| `revise` (gather) | — check-free | returns to scope |
| `revise` / `revise-brief` (verify) | — check-free | gather / scope |
| `revise` / `revise-sources` / `revise-brief` (synthesize) | — check-free | verify / gather / scope |

Owning-phase routes for accepted upstream defects:

| From | Event | Goes to | Use when |
|---|---|---|---|
| verify | `revise` | gather | sources-owned defect |
| verify | `revise-brief` | scope | brief-owned defect |
| synthesize | `revise` | verify | verification-owned defect |
| synthesize | `revise-sources` | gather | sources-owned defect |
| synthesize | `revise-brief` | scope | brief-owned defect |

Verification-local `verification.json` corrections stay in verify: edit, recheck, retry `verified`. Report-local `report.json` corrections stay in synthesize: edit, recheck, retry `completed`. Do not waive known defects.

## Per-gate loop

1. `show` — read `current_state`, `current_state_instructions`, frozen `initial_input` (including `work_slot_bindings`, `review_policies`, `artifact_schemas`), `work_slots`, and `work_slot_invocations`.
2. If this state is a **bound** slot, do not author the room yourself. `invoke` it, poll overlay until `succeeded` / `failed` / `overrun`, and on `overrun` invoke again. If **unbound**, author or revise the subject artifact. Material content changes require a revision bump — a bump retires standing verdicts for that subject; keeping the revision asserts the edit was immaterial.
3. For unbound evidence gates, request the event once before commissioning review. Schema denial means fix the artifact and retry. Evidence denial after a valid shape means schema and links cleared — do not treat that denial as a review failure. Do not append review-evidence until schema and links have cleared; a later material shape fix would bump `revision` and retire the new verdicts. For a bound slot, `invoke` and reach overlay `succeeded` before requesting the checked event; do not request that event to “probe schema” while overlay is `running`, `failed`, or `overrun`.
4. Then obtain the axis's `required_authors` count of distinct external judgments (default 1): fresh context, not the artifact's author, each judging only that axis using its `example_prompt`. Unbound: you commission those reviewers. Bound: `invoke` already ran the frozen CLI; read its output, then you still triage and append. Follow `crates/research-provider/data/reviewer-protocol.md`: triage candidates before append or mutation; append only accepted in-scope material failures or conforming passes.
5. Append one `review-evidence` record per axis judgment:

```sh
loop-engine --json append "$RUN_ID" review-evidence @verdict.json
```

```json
{
  "gate": "verify",
  "policy_id": "claim-grounded",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "verification.json",
  "subject_revision": "3",
  "config_version": "research-1"
}
```

All eight fields required; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` non-empty on `fail`; `config_version` must match the run's frozen config. Out-of-enum values make the record nonconforming and block the axis until a conforming record supersedes it.

6. Request the event. Interpret the outcome:
   - **Schema denial** (`rejected`) — artifact shape or link failed; evidence was not judged: fix shape first.
   - **Evidence denial** (`rejected`) — names unsatisfied policy axes and diagnostics for nonconforming/ignored records.
   - **Error** — invalid or inaccessible `artifact_root`, or provider failure; nothing advanced.

## Evidence rules (condensed)

- Latest conforming verdict per `(axis, subject_revision, author)` stands. Evidence is not a vote; one standing `fail` blocks even when others pass.
- Distinct-author counts use exact `(name, kind)`; the subject's author never counts toward its own review.
- Stale `subject_revision` never satisfies; wrong `config_version` counts as neither pass nor fail.
- Nonconforming records block the axis with a malformed diagnostic until a later conforming record supersedes them.
- No waivers: a material finding stands until fixed or the revision changes.
- Late findings remain actionable when they provide current evidence, violated obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked. Comprehensive-first review and scope/materiality burdens still bar drip-feeding and unrelated reopening.

## Production proof boundary

Use `scripts/research-journey.py` for repository and archive checks. Source mode drives separate Loop Engine processes across provider TOML, SQLite, production provider, shipped standard artifacts, deterministic denials, owning-phase revise, evidence aggregation, and terminal `end`. Packaged mode starts extracted binaries, materializes embedded data with `data-dump` into an empty `--data-root`, and runs the checked prefix from that dump. `--self-test` proves invalid packaged usage fails before mutating work roots. Synthetic pass records prove schema/evidence shape, independence, routing, and persistence only; they are not semantic review judgments.
