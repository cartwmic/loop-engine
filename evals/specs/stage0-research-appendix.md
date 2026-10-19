# Stage 0 research appendix

Companion to [stage0-native-vs-workflow-vs-loop-engine.md](stage0-native-vs-workflow-vs-loop-engine.md).
Research date: 2026-10-19. Method labels used below:

- **Verified (API)**: read from the GitHub API or repository files on 2026-10-19.
- **Verified (doc)**: read from the vendor's own document in its repository.
- **Reported**: from a web-search summary that cites the source; the source page was opened only where
  stated. Treat as a lead, not a finding.
- **Anecdote**: a person's report. Not a measurement.

## A. Shortlist

### A.1 Current versions

| Tool | Latest release | Date | Archived | Method |
|---|---|---|---|---|
| Spec Kit (`github/spec-kit`) | v1.2.1 | 2026-10-16 | no | Verified (API) |
| OpenSpec (`Fission-AI/OpenSpec`) | v1.14.3 | 2026-10-17 | no | Verified (API) |
| Superpowers (`obra/superpowers`) | v7.0.0 | 2026-10-10 (release notes date it 2026-10-09) | no | Verified (API, release notes) |
| GSD original (`gsd-build/get-shit-done`) | v1.42.3 | 2026-05-16 | **yes, archived** (last push 2026-05-31) | Verified (API) |
| GSD Pi (`open-gsd/gsd-pi`) | v1.22.0 | 2026-10-12 | no | Verified (API) |
| BMAD-METHOD (`bmad-code-org/BMAD-METHOD`) | v6.12.4 | 2026-10-14 | no | Verified (API) |

"GSD Core" as a separate project was not found. A GitHub search for "gsd core" returned only a
third-party wrapper (`giovannimnz/omni-spec-driven`). `gsd-build/gsd-2` is archived and points to
`open-gsd/gsd-pi`, which is its own terminal agent ("GSD Pi ... local-first coding agent"), not a workflow
that runs on Pi. For this reason GSD sits in the whole-stack category in the owner's comment, not
the workflow-on-harness category.

### A.2 Native harness support (Pi, Claude Code, Codex, OpenCode / Oh My Pi)

| Tool | Pi | Claude Code | Codex | OpenCode | Oh My Pi | Method |
|---|---|---|---|---|---|---|
| Spec Kit | yes, `--integration pi`, installs **prompt templates** in `.pi/prompts`; no MCP | yes (`.claude/skills`) | yes (`.agents/skills`) | yes (`.opencode/commands`) | yes (`omp`, `.omp/commands`) | Verified (doc: docs/reference/integrations.md) |
| OpenSpec | yes, `pi`: `.pi/skills` and `.pi/prompts` | yes | yes (skills only) | yes | yes (`oh-my-pi`) | Verified (doc: docs/supported-tools.md) |
| Superpowers | yes, Pi **package**: `pi install git:github.com/obra/superpowers`; skills plus a small extension that injects the bootstrap at start and after compaction; subagents and task lists are optional companions | yes (plugin) | yes (plugin marketplace) | yes | not stated | Verified (doc: README, `.pi/extensions/superpowers.ts`, `skills/using-superpowers/references/pi-tools.md`) |
| GSD original | **not verified** (installer flags found: Claude Code, Codex, OpenCode) | yes | yes | yes | no | Reported |
| BMAD | yes, installer id `pi`, skills in `.agents/skills` | yes | yes | yes | not stated | Verified (doc: `tools/installer/ide/platform-codes.yaml` at v6.12.4) |

### A.3 Lighter or faster modes

- **Spec Kit:** independent bug-fix and assessment flows exist (assess, fix, test); an opt-in `lean`
  preset exists, which a maintainer says is not behavior-equivalent to the full workflow (issue
  [#1401](https://github.com/github/spec-kit/issues/1401), Verified API for existence; text Reported).
  A `specify workflow run` runner provides step sequencing, state, resume and approval gates (Verified doc).
  Whether the runner can dispatch headless to Pi was not found.
- **OpenSpec:** default `core` profile has `propose`, `explore`, `apply`, `update`, `sync`, `archive`; shortest
  path is propose, apply, archive. Extra commands (`ff`, `continue`) via `openspec config profile`
  (Reported from docs/opsx.md).
- **Superpowers v7:** brainstorming now sizes the work after understanding it: quick tasks go straight
  to doing, small changes are agreed in chat, larger ones get a design document; native
  `executing-plans` runs tasks inline with one fresh whole-branch review instead of a subagent per
  task (Verified: release notes, v7.0.0 and v6.4.2). A builder-check subagent replaces the spec
  reviewer in v7.
- **BMAD v6.12:** `bmad-quick-dev` was renamed `bmad-build` in v6.11; single-session path for simple
  changes; Build defaults to a single-reviewer `quick` review (Reported).
- **GSD original:** `--profile=core`, `/gsd-quick`, `budget` model profile (Reported).

### A.4 What each enforces vs. only prompts

Own-claim, from vendor docs and release notes; none of this was independently audited by code reading. "None found" means I found no such mechanism in the documents I read, not proof that none exists.

| Duty | Loop Engine | Spec Kit | OpenSpec | Superpowers | BMAD | GSD (original) |
|---|---|---|---|---|---|---|
| Durable intent / steering | enforced: schema'd intent artifact, provider-checked transitions | prompt (spec files); runner state when the runner is used | prompt plus CLI structure validation of spec/change files | prompt (design doc) | prompt; Python state scripts for sprint plans | prompt plus state files |
| Independent review | enforced contract: separate reviewer run, evidence locators, denials | prompt (checklist command); runner can gate | prompt | prompt; fresh-context subagent when a subagent tool exists; scripts check commit ranges when invoked | prompt; reviewer prompts | optional agents |
| Evidence / provenance | enforced: locators resolved by the engine | none found | none found | none found | none found | none found |
| Authority / approval | enforced gates | runner gates pause for approval (runner only) | prompt | prompt | prompt | prompt |
| Recovery | durable run, resume | runner resume | filesystem artifacts | none | partial (state scripts) | state files |
| Validation | engine-checked records of commands and results | prompt | CLI validate | prompt (TDD skill) | prompt | prompt |

Spec Kit's own docs say workflow `requires` declarations are advisory and shell steps have no capability
sandbox. Superpowers' README acknowledges sessions can ignore plans. Functional similarity of steps is
not equivalent enforcement; the spec's decision rules (section 10, rule 3) depend on this.

### A.5 Token and latency evidence, labelled

No controlled, independently replicated three-way comparison with matched model, tasks, budgets and
cache accounting was found. What exists:

| Source | Date | What it is | Label |
|---|---|---|---|
| J. Telin, [Spec Kit vs OpenSpec token test](https://ro.linkedin.com/pulse/spec-driven-development-your-safe-choice-burning-budget-jamie-telin-bo1tf) | 2026-02-25 | same app, two runs, GPT-5.2: OpenSpec 57,740 and 91,729 tokens; Spec Kit 120,947 and 181,040 | small matched measurement (Reported; page not opened by me) |
| Uvik Software, [SDD benchmark](https://uvik.net/blog/spec-driven-development-benchmark/) | page says updated 2026-10-08 | 50 private Python tickets, one model; accepted: control 36/50 ($2.43 per merged ticket), Spec Kit 40/50 ($3.33), BMAD 41/50 ($4.23), OpenSpec 42/50 ($2.71); the page states a noise band of plus or minus 10 points and says it does not rank arms inside it | agency report, private data, not replicated (**opened and read by me**) |
| Developer Toolkit, [frameworks compared](https://developertoolkit.ai/en/ecosystem/frameworks/spec-driven-frameworks/) | 2026-09-26 | idle context footprint only: Spec Kit 1.0.12 about 300 tokens, OpenSpec 1.13.2 about 550, Superpowers 6.4.2 about 838; invocation: subagent-driven-development about 11.8k, brainstorming about 6.3k | static measurement, not task cost (Reported; page opened, status 200) |
| Rapoport Studio, [OpenSpec vs Spec Kit](https://rapoport.studio/en/lab/openspec-vs-speckit) | measured 2026-05-10 | files after init 9 vs 27; artifact floor per feature 94 vs 247 lines | artifact size, not tokens (Reported; page opened, status 200) |
| Spec Kit issue [#1401](https://github.com/github/spec-kit/issues/1401) | opened 2025-12-29, closed 2026-09-24 | 18.6K standing context reported for Claude Code; a maintainer later said skills-based integrations load metadata only and the standing-cost description no longer applies | anecdote plus maintainer correction (issue existence and closure Verified API) |
| Superpowers issues [#762](https://github.com/obra/superpowers/issues/762) (closed), [#1120](https://github.com/obra/superpowers/issues/1120) (open), [#1152](https://github.com/obra/superpowers/issues/1152) | 2026-03 to 2026-04 | "cost too many tokens"; "6-7 agent invocations for a five-line file", estimated 10-15x direct; Codex subagent-driven run exhausted a five-hour allowance | anecdotes; #1120 maintainer reply defends fresh reviewer contexts (existence Verified API) |
| Reddit threads on Superpowers overhead (April, July 2026) and a four-framework one-feature comparison (about September 2026; the page was blocked to my fetch) | 2026 | unquantified complaints; one small matched diff comparison without a no-framework control | anecdotes |
| Hacker News "Superpowers 6" thread | 2026 | users disagree: 5.x costly, 6.x more efficient | anecdotes |
| arXiv [2604.05278](https://arxiv.org/abs/2604.05278) Spec Kit Agents | 2026-04-07 | authors' own extension; SWE-bench Lite 56.5% vs 58.2% with hooks; custom tasks 3.46 vs 3.51 vs 3.66 of 5 | controlled, but not stock Spec Kit (title and date Verified; numbers Reported) |
| arXiv [2606.30689](https://arxiv.org/abs/2606.30689) Citation Discipline | 2026-06-28 | traceSDD vs Spec Kit vs OpenSpec, output determinism and hallucination detection | controlled, different outcome (title and date Verified; content Reported) |
| Superpowers v6.4.2 release notes | 2026-09-25 | maintainer: leaner plans took about a quarter of the time and a third of the tokens in one reproduced case | maintainer-reported (Verified: read in repo) |

Reading for this experiment: the owner's "substantial tokens for little benefit" is plausible and
unmeasured; the matched numbers found (Telin in February 2026 on older versions and not opened by me; Uvik opened and read, private data) show OpenSpec lighter than Spec Kit; neither measures the current Spec Kit v1.2.1, and neither is evidence about its lean preset, and Uvik shows
a spec arm costing about 11% to 74% more than a no-spec control per merged ticket with a gain inside
its own stated noise band. Superpowers' overhead is documented by users and partly acknowledged by
its maintainer; v6 and v7 changed it. Hence the spec measures instead of assuming.

### A.6 Selection

1. **Superpowers v7.0.0.** Native Pi package (not just prompts), the tool the owner named, v7
   brainstorming targets intent elicitation, which is exactly a metric the harness scores, and it has
   the most-discussed overhead. Little is mechanically enforced, so it tests "prompted process"
   against LE's "enforced process".
2. **OpenSpec v1.14.3 (`core` profile).** Supported Pi integration with skills and prompts, the lightest
   spec-first option in the matched data above, and some mechanics (CLI validation, artifact state).
   It is the fairest "spec method at low weight" arm.

Held for Stage 2: Spec Kit v1.2.1 (Pi support is prompt templates only; heavier artifacts; the
enforcing part is the workflow runner, whose headless Pi dispatch I could not confirm) and BMAD v6.12.4
(Pi supported; persona-heavy). Excluded: GSD (native Pi support **not verified** for the archived original; the Pi lineage is a separate
agent, not a workflow on Pi).

Adjacent categories from the owner's comment (OmO, Gas Town/Gas City, Agent Orchestrator, Open SWE,
OpenHands, T3 Code) were **not** researched for Stage 0. They are whole-stack or UI comparisons, not
plug-in arms.

## B. Neutral base prompt (all arms)

```
You are working on a change in /workspace/public/repo.

Owner request:
{request}

The owner cannot read your messages. The only way to reach the owner is the command
ask-owner QUESTION. Do not guess where the request leaves a behavior open; ask.
Do the work in the repository. Run whatever checks you think are appropriate.
Do not commit and do not push. Edit only what the request needs.
When finished, reply with a short summary of what you changed and what you checked.
```

Added for S and O only: "Follow the {Superpowers|OpenSpec} workflow that is installed in this
environment for this change." L uses the existing software-change preamble (profile, run id, paths,
terminal-state requirement), minus its Git setup lines (G3). The sentence "Do not guess ... ask" is the
same opportunity-to-elicit statement the L preamble gives; if the owner prefers N without it, that
is a different arm.

## C. Harness evidence used

All under `evals/software-change/results/` at `96ee638`:

- `tier-0-full-sonnet/20261018T132346.405605Z`: `pi/claude-compat/claude-sonnet-5-5`, `minimal-16`, `label-keys`.
  Success; hidden 6/6; elicitation 3/3; workflow $3.641 and 8,257,921 tokens; evaluation overhead
  $0.014 and 3,753 tokens; wall 1189 s; provider denials 5; revise loops design-review 1,
  validation-review 1; cited evidence checked 297, not resolving 11, unverifiable 75.
- `tier-0-full-sol/20261018T135956.603628Z`: `codex/gpt-6.1-sol`, same case. Outcome failure, lifecycle
  terminated at `explore`, one cancellation; hidden 6/6; workflow $12.095 and 7,755,655 tokens; wall
  2162 s; 7 owner questions; diff 6 changed lines vs reference 2.
- `pilot/20261011T003331.702469Z` and `pilot/20261011T030831.922694Z` (profile `standard-14`):
  followup-import success for both models (5498 s Sol, 5348 s Opus; 15.2 M and 16.4 M workflow
  tokens); review-policy 15/16 for Sol (11438 s, 55.2 M tokens) and for Opus on the second attempt
  (9149 s; the first Opus attempt was an infrastructure failure).

Read-only checks run for this spec: GitHub API reads; `evalctl --help`; `docker image ls`; `pi
--help`; listing available classifier models (no classifier call was made). No model trial, image
build, or System 1 call was made.
