---
name: using-generate-prd
description: Use when extracting a schema-valid living markdown PRD candidate from a repository through the research provider generate-prd profile — proposing per-requirement repository evidence for human accept-or-reject before any commit to docs/PRD.md.
---

# Using generate-PRD

## Overview

Generate-PRD is a recurring research profile plus this skill. It proposes a schema-valid living markdown PRD candidate with per-requirement repository evidence. It is not a fourth Loop Engine provider, not a software-change profile family, and not a software-change precondition. Repositories without a schema-valid git PRD can still complete software-change with bookends off.

Drive the run with the **existing** `research` binary and `crates/research-provider/data/configs/generate-prd.json`. Describe and evaluate stay on that binary. Never invoke a model from the research provider binary itself. Never call software-change evaluate.

`using-research-provider` (`skills/using-research-provider/SKILL.md`) and `using-loop-engine` (`skills/using-loop-engine/SKILL.md`) are required companions for engine driving, bindings, evidence, and envelopes. This skill does not replace them. Do not markdown-link outside this crate.

## Hard rules

A human must accept or reject the candidate before any commit to docs/PRD.md.
Never auto-commit.
Never mint IDs outside the published grammar.
Never call software-change evaluate.
Never invoke a model from the research provider binary itself.

Published grammar (prose path `crates/bookends-check/schema/prd.md`): live IDs are `LE-<n>` with `<n>` = `[1-9][0-9]*` and no leading zeros. Candidate IDs are proposals; they are not authoritative until a human accepts and commits them into the repository PRD. Do not allocate Compass `PREFIX-N` or `@spec:` tokens.

## Setup

```sh
cargo build -p loop-cli -p research-provider -p bookends-check
```

Register `target/debug/research` under exact alias `research` (absolute `command` path) in uncommitted machine-local provider TOML, the same way as `skills/using-research-provider/SKILL.md`. There is no generate-prd binary and no extract provider.

Copy `crates/research-provider/data/configs/generate-prd.json` to a run-specific file. Shipped profiles omit `work_slot_bindings` (or `{}`). Cataloged slots remain `scope`, `gather`, `verify`, and `synthesize`. Opt-in review bindings use the constructor in `skills/using-research-provider/SKILL.md` against this per-run generate-prd copy (`SLOT_ID=verify` or `SLOT_ID=synthesize`).

When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. Then:

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-generate-prd.json" "generate PRD candidate"
```

For an installed binary, dump into an empty directory first (`research data-dump "$DATA_ROOT"`), then copy `$DATA_ROOT/crates/research-provider/data/configs/generate-prd.json`.

Subject files under the allocated `artifact_root` stay `brief.json`, `sources.json`, `verification.json`, and `report.json`. Author from `crates/research-provider/data/templates/generate-prd/`. Shipped `config_version` is `research-1`. Extra profile identity is `generate-prd`; `template_root` is `crates/research-provider/data/templates/generate-prd`.

## External extract work

Do the primary work outside Loop Engine, then record it in the subject artifacts.

1. **Scope** — question is to extract a schema-valid living markdown PRD candidate for the current repository. Name observable acceptance, constraints, and non-goals. Do not present a chosen PRD as the question.
2. **Gather** — search this repository. Record sources with stable ids, locators to tracked files or tests, and exact extracts later verification can check. `brief_revision` must equal current `brief.json` revision.
3. **Verify** — author claims with cited `source_ids`, support, and a genuine challenge. Each proposed requirement's supporting evidence must be reviewable in the repository. Request `verified` before commissioning review.
4. **Synthesize** — write a cited conclusion that emits the candidate markdown plus per-requirement repository evidence citations. Canonical live-record spelling is `### LE-<n>:`, `- Status: live`, and `- Coverage: e2e/journey`. Then write the same human-facing candidate to `prd-candidate.md` under the run artifact root. Do not write it to `docs/PRD.md`. Request `completed` before commissioning review.
5. **Parse-check** — after the run reaches `end`, validate the candidate with the real parser-only command: `bookends-check candidate prd-candidate.md`. This checks grammar only; it does not claim coverage, completeness, or semantic correctness.

Every proposed requirement must have one and only one evidence mapping. The mapping's locator must be a tracked repository path and its extract must match the current bytes at that path. Missing, duplicate, extra, untracked, or mismatched evidence is a failed deliverable, not a reason to accept the candidate anyway.

## Human accept-or-reject

`prd-candidate.md` is a proposal, not authority. After the research run reaches `end`:

- Present `prd-candidate.md` and its evidence mapping to a human.
- A human must accept or reject the candidate before any commit to docs/PRD.md.
- On reject, leave `docs/PRD.md` unchanged. Do not commit the candidate.
- On accept, only that human (or an agent acting after explicit human acceptance) may commit the candidate to `docs/PRD.md`.
- Never auto-commit. Do not run `git commit` as part of this skill.

This extract path is not required to start a software-change run.

## Gate map

Same research topology as `standard.json`: `scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise*` edges. Checked events, evidence rules, and production-start isolation rules are those in `skills/using-research-provider/SKILL.md`. Do not treat generate-PRD as a software-change workflow (`intent` / `design` / `plan` / `implement` / `validation`) and do not run `scripts/software-change-journey.py` for this path.
