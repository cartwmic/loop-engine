# software-change-provider

## Purpose

Track code delivery obligations, externally supplied reviews and validation evidence. This provider is for external workflow drivers, not autonomous execution. The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. Bring your own agent/reviewer harness, model selection and any required service credentials. The [reviewer protocol](data/reviewer-protocol.md) describes judgment obligations; its historical plain-show pipe and bound-worker examples are not current intake instructions. Use the [local agent guide](AGENTS.md) for the shipped external-review packet contract and the bundled profile for current version/axis/group obligations.

Provider requirements/protocols are frozen; use the process boundary rather than depending on internal Rust APIs. Runs retain frozen obligations and the original command/args association, not executable bytes. Drivers must preserve a compatible runtime at the stored path; no active-run migration is authorized. Checked transitions validate obligations, not the truth of reviewers' judgments.

## Installation

From the repository root with Rust/Cargo installed:

```sh
cargo build --locked -p loop-cli -p software-change-provider
```

This builds `target/debug/loop-engine` and `target/debug/software-change`. Source examples use the [standard profile](data/configs/standard.json); packaged users can export bundled data to a fresh, driver-owned directory outside the checkout:

```sh
target/debug/software-change data-dump /absolute/fresh-provider-data
```

Replace the destination with your chosen fresh directory. This is data export, not model installation or user configuration.

## Usage

For predictable artifact ownership, copy the profile and set `artifact_root` to an existing writable absolute directory outside the checkout. With engine `start`, an omitted root is allocated automatically: inspect full show initial input to find it before authoring artifacts. Direct provider requests need an explicit root; absent, relative or inaccessible roots fail artifact access. See [local operator preparation](AGENTS.md) for the owned procedure.

Start a run with the profile through `loop-engine start`, using a dedicated database and explicit provider TOML whose command is the absolute built provider path. The [skill](skills/using-software-change-provider/SKILL.md) owns input preparation, commission, review intake and progression; load it before driving a run. Early commissions with `missing_inputs` are not ready. External reviewers return judgments and drivers explicitly append accepted evidence and send events after fresh action/full observations.

```sh
target/debug/loop-engine --help
```

Use root `README.md` for a first-run catalog example and root `AGENTS.md` for repository operations (paths relative to the checkout root). Follow the [local agent guide](AGENTS.md) when editing this crate.

## Validation

From the repository root:

```sh
cargo test -p software-change-provider
```

These are mechanical tests, not genuine semantic review. The root agent guide owns journey and final-proof requirements. For stalls involving missing inputs, stale evidence, author floors or observation permits, load the [skill](skills/using-software-change-provider/SKILL.md); do not infer acceptance from process exit. Typed advice answers, where applicable, are externally supplied context of kind `advice-answer`.

Preserve review groups, self-review declarations and effective author counts. Declare evidence applicability explicitly; check ordinary review clearance before challenge review.
Use `review-candidates` for retained external judgments and `prepare-validation` for genuine external command results; inspect their candidates before appending evidence.
