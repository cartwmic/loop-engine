# Backlog

This file records candidate work that has not yet been accepted as a product requirement. [docs/PRD.md](docs/PRD.md) remains the sole requirement-ID authority. Promote an item through the normal intent and design process before treating it as binding.

The initial entries come from the 22-Aug-2026 blind review of Bookends commit `531532c` by Claude Fable 5, OpenAI Codex Sol, and Cursor Grok 4.6.

## Release blockers

### Restore continuity in shallow required-CI checkouts

**Context:** Required CI checks out the repository at the default depth of one. The checker cannot resolve `HEAD^`, maps the missing parent to first adoption, and therefore reports GREEN for continuity violations that report RED in a full clone. Fable reproduced this with the same invalid PRD transition in full and depth-one clones.

**Evidence:**

- `.github/workflows/preflight.yml` uses `actions/checkout@v6` without `fetch-depth`.
- `crates/bookends-check/src/git.rs::first_parent` maps an unavailable parent to `None`.
- `crates/bookends-check/src/continuity.rs` treats no parent as no continuity baseline.
- Superseded run `run-1787334584350890000-1-98697` had accepted the need for non-shallow CI history.

**Smallest scope:**

- Set `fetch-depth: 2` for the required preflight checkout.
- Make parent unavailability in a shallow repository RED instead of treating it as first adoption.
- Preserve legitimate first adoption in repositories that truly have no parent requirement state.

**Done when:**

- A full clone and a depth-limited CI-equivalent clone both reject live-ID disappearance, tombstone removal, title reassignment, and revival.
- A real first adoption remains GREEN.
- The public required-CI path proves the behavior, not only an internal seam test.

### Persist local bypass invocation evidence

**Context:** The accepted intent requires every `class:reason` bypass to be visible and durably recorded. The landed checker prints the bypass, but the local pre-push path leaves no later-retrievable record. Sol identified this as a literal intent gap; the same concern was accepted in superseded run `run-1787334584350890000-1-98697`.

**Evidence:**

- `scripts/bookends-check-gate.sh` forwards `BOOKENDS_BYPASS` but persists nothing.
- `.githooks/pre-push` executes the wrapper without a durable sink.
- `crates/bookends-check/src/main.rs` prints `BYPASS`, class, and reason to stdout.

**Smallest scope:**

- Choose the smallest durable destination appropriate for a trusted sole operator.
- Record invocation time, repository, checked revision or tree identity, bypass class, reason, and outcome.
- Fail closed if a requested bypass cannot be recorded.
- Do not add a service, registry, multi-user audit system, or generalized policy framework.

**Done when:**

- A black-box local pre-push invocation using `BOOKENDS_BYPASS=class:reason` succeeds only after leaving retrievable evidence.
- A recording failure prevents the bypass from permitting the push.
- Normal GREEN and RED invocations keep their current behavior.
- Software-change continues to treat BYPASS as non-GREEN.

## Follow-ups

### Make Generate-PRD verification wording honest

**Context:** `scripts/generate-prd-journey.py` performs three predetermined exact-line lookups but writes that no contrary repository extract was found “after the deterministic search.” No contrary-evidence search occurs. The candidate extracts, parser validity, provisional status, and human gate remain valid.

**Smallest scope:** Replace the unsupported claim with wording that states the exact extract is narrow and does not establish completeness. Do not add search infrastructure.

**Done when:** Generated verification evidence describes only work the deterministic journey actually performed, and the Generate-PRD self-test and source journey still pass.

### Decide push-range continuity behavior

**Context:** Immediate-parent continuity checks only the pushed tip against its parent. An invalid middle commit in a multi-commit push can escape if a later commit becomes the tip. This is a routine trusted-operator workflow, but immediate-parent-only continuity was an accepted minimal-design boundary.

**Decision:** Choose one:

1. Check each commit in the bounded pushed range at pre-push and required-CI boundaries; or
2. Retain immediate-parent-only behavior and explicitly document that intermediate commits in a multi-commit push are outside v1 continuity guarantees.

Do not turn this into an unbounded history-authority scan.

**Done when:** The chosen boundary is explicit in intent, schema documentation, tests, and public gate behavior, with no claim that invalid intermediate commits are caught unless they are actually checked.

### Retain exhaustive audit matrices

**Context:** Two implementation workers reported independent LE-1 through LE-90 citation-placement audits as 90/90, but their row-level scratch matrices were removed. Later auditors can see the result claim but must redo the mapping to falsify it.

**Smallest scope:** For future exhaustive semantic-placement audits, retain a compact run artifact containing requirement ID, requirement text or digest, citation location, exercised scenario, and observable assertion. Keep this as review evidence, not checker product machinery.

**Done when:** A fresh reviewer can inspect or sample the retained matrix without reconstructing all mappings, and the artifact is bound to the reviewed repository revision.

### Add a post-commit run-to-commit pointer

**Context:** Final implementation and validation reports correctly describe the pre-commit state as `939c7d6+uncommitted-worktree`. After commit, no durable pointer links that exact reviewed surface to landed commit `531532c`. Historical reports should not be rewritten.

**Smallest scope:** Add a separate post-commit record that names the run ID, report revisions, landed commit, and proof that the committed pathname set and bytes match the reviewed worktree. Keep the run reports immutable.

**Done when:** A later auditor can move from the final run evidence to the landed commit without relying on chat history, while the original reports remain unchanged.
