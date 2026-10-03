# research-provider

## Purpose

Investigate claims and retain source-bound findings; the generate-PRD profile extracts requirement candidates rather than implementing them. This provider is for external workflow drivers, not autonomous execution. The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. Bring your own agent/reviewer harness, model selection and any required service credentials. The judgment contract is the [reviewer protocol](data/reviewer-protocol.md).

Provider requirements/protocols are frozen; use the process boundary rather than depending on internal Rust APIs. Runs retain frozen obligations and the original command/args association, not executable bytes. Drivers must preserve a compatible runtime at the stored path; no active-run migration is authorized. Checked transitions validate obligations, not the truth of reviewers' judgments.

## Installation

From the repository root with Rust/Cargo installed:

```sh
cargo build --locked -p loop-cli -p research-provider
```

This builds `target/debug/loop-engine` and `target/debug/research`. Source examples use the [standard profile](data/configs/standard.json); packaged users can export bundled data to a fresh, driver-owned directory outside the checkout:

```sh
target/debug/research data-dump /absolute/fresh-provider-data
```

Replace the destination with your chosen fresh directory. This is data export, not model installation or user configuration.

## Usage

For predictable artifact ownership, prepare a resolved profile with `setup` and set `artifact_root` to an existing writable absolute directory outside the checkout. With engine `start`, an omitted root is allocated automatically: inspect full show initial input to find it before authoring artifacts. Direct provider requests need an explicit root; absent, relative or inaccessible roots fail artifact access. See [local operator preparation](AGENTS.md) for the owned procedure.

Start a run with the profile through `loop-engine start`, using a dedicated database and explicit provider TOML whose command is the absolute built provider path. The [local agent guide](AGENTS.md) selects the investigation versus generate-PRD profile and states the supported research commands; the [skill](skills/using-research-provider/SKILL.md) explains general external-loop concepts; load it before driving a run. Early commissions with `missing_inputs` are not ready. External reviewers return judgments and drivers explicitly append accepted evidence and send events after fresh action/full observations.

```sh
target/debug/loop-engine --help
```

Use root `README.md` for a first-run catalog example and root `AGENTS.md` for repository operations (paths relative to the checkout root). Follow the [local agent guide](AGENTS.md) when editing this crate.

## Validation

From the repository root:

```sh
cargo test -p research-provider
```

These are mechanical tests, not genuine semantic review. The root agent guide owns journey and final-proof requirements. For stalls involving missing inputs, stale evidence, author floors or observation permits, load the [skill](skills/using-research-provider/SKILL.md); do not infer acceptance from process exit. Typed advice answers, where applicable, are externally supplied context of kind `advice-answer`.

## Base-derived profiles

Use `target/debug/research setup --rigor standard --output /absolute/profile.json`
(or `--profile FILE` for a selected custom base). `setup --help` lists review
count, eligible-self, group and axis overrides. Inspect `profile.json.explain.json`
for base/override/effective identity; replay options with `--previous-explain FILE`
when selecting a newer base to see drift. No active run follows changed defaults.
