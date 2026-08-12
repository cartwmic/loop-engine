---
name: using-software-change-provider
description: Use when running the software-change workflow through Loop Engine with the software-change provider — selecting a config profile, authoring gate artifacts, commissioning external reviews, appending review-evidence records, and clearing checked transitions.
---

# Using the software-change provider

## Overview

`software-change` is Loop Engine's repo-local reference provider. Workflow: `explore → design → design-review → plan → plan-review → implement → implementation-review → validation → end`, with check-free `revise` edges from each review state back to its authoring state.

The provider is deterministic only: it validates artifact schemas and revision links, then aggregates externally supplied review evidence. It never generates prompts, invokes a model, or judges findings — **you** commission semantic review and append the verdicts. Per-run obligations are frozen in immutable `initial_input`.

Drive the engine itself with the [using-loop-engine skill](../../../../skills/using-loop-engine/SKILL.md). Provider contract details: [README](../../README.md); frozen requirements: [docs/prd.md](../../docs/prd.md).

## Setup

```sh
cargo build -p loop-cli -p software-change-provider
```

Register `target/debug/software-change` under alias `software-change` (absolute `command` path) in an uncommitted machine-local `providers.toml`.

Pick a profile from [data/configs/](../../data/configs/) — `minimal.json` (validation gate only), `standard.json` (intent, design-review, validation axes), or `high-rigor.json` (all axes; two distinct reviewers on design-review and validation axes). Copy it to a run-specific file, replace the placeholder `artifact_root` with an absolute existing directory, then:

```sh
loop-engine --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start software-change "@/tmp/software-change-standard.json" "my change"
```

The artifact directory uses fixed subject filenames: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, `validation-report.json`.

## Gate map

| Event (from state) | Subject checked | Evidence gate |
|---|---|---|
| `intent-ready` (explore) | `intent.json` | `intent` |
| `design-ready` (design) | `design.json` | schema/links only |
| `approved` (design-review) | `design.json` | `design-review` |
| `plan-ready` (plan) | `plan.json` | schema/links only |
| `approved` (plan-review) | `plan.json` | `plan-review` |
| `implementation-ready` (implement) | `implementation-report.json` | schema/links only |
| `approved` (implementation-review) | `implementation-report.json` | `implementation-review` |
| `passed` (validation) | `validation-report.json` | `validation` |
| `revise` (any review state) | — check-free | — |

## Per-gate loop

1. `show` — read state instructions plus run-frozen `initial_input.review_policies` (each gate lists axes with `id`, `description`, `example_prompt`) and `initial_input.artifact_schemas`.
2. Author or revise the subject artifact in `artifact_root` using its template from [data/templates/](../../data/templates/). Material content changes require a revision bump — a bump retires all standing verdicts for that subject by design; keeping the revision asserts the edit was immaterial.
3. For evidence gates, commission the axis's configured `required_authors` count of distinct external reviewers (default 1; high-rigor design-review and validation axes require 2): fresh context, not the artifact's author, each judging only that axis using its `example_prompt`. Follow [data/reviewer-protocol.md](../../data/reviewer-protocol.md) — the canonical judging and adjudication rules.
4. Append one record per axis judgment — `kind` is `review-evidence`, `data` is the eight-field object:

```sh
loop-engine --database "$DB" --json append "$RUN_ID" review-evidence @verdict.json
```

```json
{
  "gate": "design-review",
  "policy_id": "intent-faithful",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "design.json",
  "subject_revision": "3",
  "config_version": "standard-3"
}
```

All eight fields required; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` non-empty on `fail`; `config_version` must match the run's frozen config. Out-of-enum values make the record nonconforming and block the axis until a conforming record supersedes it.

5. Request the event. Interpret the outcome:
   - **Schema denial** (`rejected`) — artifact shape or link failed; evidence was not judged: fix shape first.
   - **Evidence denial** (`rejected`) — names unsatisfied policy axes and diagnostics for nonconforming/ignored records.
   - **Error** — invalid or inaccessible `artifact_root`, or provider failure; nothing advanced.

## Evidence rules (condensed)

- Latest conforming verdict per `(axis, subject_revision, author)` stands. Evidence is not a vote; one standing `fail` blocks even when others pass.
- Distinct-author counts use exact `(name, kind)`; the subject's author never counts toward its own review.
- Stale `subject_revision` never satisfies; wrong `config_version` counts as neither pass nor fail.
- Nonconforming records block the axis with a malformed diagnostic until a later conforming record supersedes them.
- No waivers: a material finding stands until fixed or the revision changes.
