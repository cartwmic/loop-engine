---
name: using-generate-prd
description: Use when extracting a schema-valid living markdown PRD candidate from a repository through the research provider generate-prd profile — proposing per-requirement repository evidence for human accept-or-reject before any commit to docs/PRD.md.
---

# Using generate-PRD

## Overview

Generate-PRD is a recurring research profile plus this skill. It proposes a schema-valid living markdown PRD candidate with per-requirement repository evidence. It is not a fourth Loop Engine provider and not a precondition for any other run.

Drive the run with the **existing** `research` binary and `crates/research-provider/data/configs/generate-prd.json`. Describe and evaluate stay on that binary. Never invoke a model from the research provider binary itself. Never call another provider's evaluate.

`using-research-provider` (`skills/using-research-provider/SKILL.md`) and `using-loop-engine` (`skills/using-loop-engine/SKILL.md`) are required companions for engine driving, external work policy, evidence, and envelopes. This skill does not replace them. Do not markdown-link outside this crate.

## Hard rules

Keep the candidate outside `docs/PRD.md` until explicit human acceptance and separate commit authorization. Never auto-commit. Never mint IDs outside the published grammar.

Published grammar (prose path `crates/bookends-check/schema/prd.md`): live IDs are `LE-<n>` with `<n>` = `[1-9][0-9]*` and no leading zeros. Candidate IDs are proposals; they are not authoritative until a human accepts and commits them into the repository PRD. Do not allocate Compass `PREFIX-N` or `@spec:` tokens.

## Setup

```sh
cargo build -p loop-cli -p research-provider -p bookends-check
```

Register `target/debug/research` under exact alias `research` (absolute `command` path) in uncommitted machine-local provider TOML, the same way as `skills/using-research-provider/SKILL.md`. There is no generate-prd binary and no extract provider.

Prepare with `research setup --rigor generate-prd --output /absolute/profile.json`.
Confirm exact profile bytes/hash, policies, groups, self-review declarations and author
counts with the user. Confirm the external role/model manifest, ownership and owner-set
spend/resource limits (none by default) separately; rehash unchanged profile immediately before start. No engine executor is
configured. Follow the research companion for external review commissioning/intake.
Inspect LOOP_/XDG/home redirects and use explicit --config.

When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. Then:

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-generate-prd.json" "generate PRD candidate"
```

For an installed binary, dump into an empty directory first (`research data-dump "$DATA_ROOT"`), then select that file with `setup --profile FILE --output /absolute/profile.json` for full-custom preparation.

Subject files under the allocated `artifact_root` stay `brief.json`, `sources.json`, `verification.json`, and `report.json`. Author from `crates/research-provider/data/templates/generate-prd/`. Shipped `config_version` is `research-1`. Extra profile identity is `generate-prd`; `template_root` is `crates/research-provider/data/templates/generate-prd`.

## External extract work

Do the primary work outside Loop Engine, then record it in the subject artifacts. Follow the research companion's per-gate loop: action `show` for driving, full for frozen input/context, passive `monitor` for waiting. Retain external command results and source identities through the driver; execution receipts and summary prose do not establish requirement truth or candidate acceptance. Global --timeout-ms bounds provider transport, not external extraction time. Status/compact, monitor, list and history do not arm mutations; re-observe action/full after each transition.

1. **Scope** — question is to extract a schema-valid living markdown PRD candidate for the current repository. Name observable acceptance, constraints, and non-goals. Do not present a chosen PRD as the question.
2. **Gather** — investigate this repository without a predetermined requirement list. This is discovery and synthesis, not lookup of an already-known PRD. Separate observed behavior, stated obligations, conflicting evidence and unknown intent; do not promote every implementation detail into a requirement. Search this repository. Record sources with stable ids, locators to tracked files or tests, and exact extracts later verification can check. `brief_revision` must equal current `brief.json` revision.
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

This extract path is not required to start a lean-change run.

## Converting an accepted PRD

Use this when a human has already accepted a PRD written as prose and you are asked to turn it into a repository PRD with requirement ids. It is a different job from the extraction above: the accepted prose is the authority, and the conversion must not change it. It needs no new run or provider. Do the five parts below in the session.

### Inclusion test

A record is a requirement only if it is a current, independently falsifiable product obligation: someone could check the product and show it false. A goal, non-goal, journey, rationale, measurement plan or open question is not a requirement, even when it is worded with "must" or "must not". Do not measure completeness by "every source identifier is mapped": that pushes goals and journeys into requirements and still misses a real obligation. Measure it by "every obligation in the prose has a record". When you cannot tell, stop and ask the owner. Do not decide it yourself.

### Keep the accepted prose

Agree the document shape with the owner before you extract anything: the accepted prose kept as it is, plus a requirements-only index derived from it, and which one wins when they disagree (recommend the prose). Never paraphrase or compress the prose. You may add ids, status lines and the index around it. Copy defects of the source, such as a contradiction or an unverifiable statement, as written, label them next to the text, and return them to the owner. Do not fix them silently.

### Owner sign-off on the outline

Before any id is given out and before any expensive review, show the owner the outline (the sections of the prose and of the index), the inclusion rules, and a few representative records, including one you left out and why. Allocate ids only after the owner approves the outline. The first check should be the cheap one.

### Defective samples

Test the conversion against each of these before you hand it over. Keep one small sample file per kind; a process that passes one of them is wrong.

1. A non-goal worded as "must not" became a requirement.
2. A goal that looks measurable (a number and a unit) became a requirement with no obligation behind it.
3. A journey repeats a requirement as a second record.
4. A source contradiction: two statements disagree and the conversion quietly picked one or smoothed them together.
5. An unverifiable requirement: no check could show it false. Work out how each record could be verified before you assign proof; one proof class for every record is wrong.
6. An omitted obligation hidden behind complete-looking id coverage: every source id has a record, but one obligation sentence has none.

The script catches the prose-level part: prose that was dropped, changed or moved, which includes a contradiction that was smoothed by editing the text (4) and the loss of the sentence behind an omitted obligation (6) when the sentence itself was dropped. The script does not catch wrong inclusion (1, 2, 3), a record with no way to be falsified (5), an obligation whose prose was kept but never got a record (6), or a contradiction left unlabelled (4). Those need the owner or a reviewer. Give at least one reviewer the job of challenging the framing and the requirement boundary, not only checking conformance, and give each reviewer the converted file itself with its sha256, not a report about it.

### Check the prose

Run `python3 scripts/check-prd-prose.py ACCEPTED.md CONVERTED.md`. That path is relative to this skill's directory, where the script ships beside this `SKILL.md`; from any other directory, give the full path. It reads both files and checks that every non-blank line of the accepted prose is a whole line of the converted file, in the same order, and still visible: prose the accepted file shows as plain text must not end up inside an HTML comment or a fenced code block in the converted file. Added lines anywhere (ids, status lines, an index, lines between list items) are allowed. Only line endings, trailing spaces and a leading byte order mark are ignored; a reflowed line counts as changed. It exits 0 when the prose is kept, 1 with a report of each line that was dropped, changed (with the closest converted line), moved or hidden, and 2 when a file cannot be read. Run it after every edit, before every review.

Report three results separately, and never let one stand for another: grammar validity (`bookends-check candidate`, which checks grammar only), prose preservation (this script) and whether each record can really be verified (a reviewer's judgment).

## Gate map

Same research topology as `standard.json`: `scope → gather → verify → synthesize → end`, plus check-free owning-phase `revise*` edges. Checked events, evidence rules, and production-start isolation rules are those in `skills/using-research-provider/SKILL.md`. Do not treat generate-PRD as a lean-change workflow (`explore` / `intent-approval` / `implement` / `verify`).

Small pre-start overrides use `--set review.GATE.AXIS.required_authors=N`,
`--set review.GATE.AXIS.self_review=true`, and `--set review.GATE.AXIS.group=NAME`.
Policy-document uses gate `semantic-review`; research uses `verify` or `synthesize`.
Use `--add-axis GATE.NEW=GATE.EXISTING` or `--remove-axis GATE.AXIS` for membership.
Inspect the output and `FILE.explain.json` (base identity, overrides, effective policy).
To select a newer base, replay the options with `--previous-explain OLD.explain.json`;
inspect drift and resolve refused missing targets before start. Raw prompt/schema
replacement remains a full custom file; preparation never launches work or changes a run.
