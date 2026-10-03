# policy-document-provider

## Purpose

Draft or audit README.md and AGENTS.md against deterministic and externally judged semantic policies. This provider is for external workflow drivers, not autonomous execution. The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. Bring your own agent/reviewer harness, model selection and any required service credentials. The judgment contract is the [reviewer protocol](data/reviewer-protocol.md).

Provider requirements/protocols are frozen; use the process boundary rather than depending on internal Rust APIs. Runs retain frozen obligations and the original command/args association, not executable bytes. Drivers must preserve a compatible runtime at the stored path; no active-run migration is authorized. Checked transitions validate obligations, not the truth of reviewers' judgments. Target-file edits are not atomically locked or versioned with workflow commits: keep the document quiescent during review and verify its final digest afterward. This is not a transactional document assessment service; see [target guidance](data/target-guidance.md).

## Installation

From the repository root with Rust/Cargo installed:

```sh
cargo build --locked -p loop-cli -p policy-document-provider
```

This builds `target/debug/loop-engine` and `target/debug/policy-document`. Source examples use the [standard profile](data/readme.json); packaged users can export bundled data to a fresh, driver-owned directory outside the checkout:

```sh
target/debug/policy-document data-dump /absolute/fresh-provider-data
```

Replace the destination with your chosen fresh directory. This is data export, not model installation or user configuration.

## Usage

Start a run with the profile through `loop-engine start`, using a dedicated database and explicit provider TOML whose command is the absolute built provider path. The [skill](skills/using-policy-document-provider/SKILL.md) owns input preparation, commission, review intake and progression; load it before driving a run. Early commissions with `missing_inputs` are not ready. Before semantic review, declare actual target authors using context kind `target-authorship`, data `{"target_sha256":"CURRENT_DIGEST","author":{"name":"ACTUAL_AUTHOR","kind":"agent"}}` (one record per author; kind is human, agent or script). Missing/stale declarations block coverage. Target authors are excluded unless the frozen policy permits self-review, which counts only as labeled self-review; every rewrite needs a declaration bound to its new bytes. `initial_input.target_author` is not supported. External reviewers return judgments and drivers explicitly append accepted evidence and send events after fresh action/full observations.

```sh
target/debug/loop-engine --help
```

Use root `README.md` for a first-run catalog example and root `AGENTS.md` for repository operations (paths relative to the checkout root). Follow the [local agent guide](AGENTS.md) when editing this crate.

## Validation

From the repository root:

```sh
cargo test -p policy-document-provider
```

These are mechanical tests, not genuine semantic review. The root agent guide owns journey and final-proof requirements. For stalls involving missing inputs, stale evidence, author floors or observation permits, load the [skill](skills/using-policy-document-provider/SKILL.md); do not infer acceptance from process exit. Typed advice answers, where applicable, are externally supplied context of kind `advice-answer`.

## Base-derived profiles

Use `target/debug/policy-document setup --rigor standard --output /absolute/profile.json`
(or `--profile FILE` for a selected custom base). `setup --help` lists review
count, eligible-self, group and axis overrides. Inspect `profile.json.explain.json`
for base/override/effective identity; replay options with `--previous-explain FILE`
when selecting a newer base to see drift. No active run follows changed defaults.
