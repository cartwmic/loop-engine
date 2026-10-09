# loop-cli

## Purpose

The `loop-engine` executable exposes durable workflow state to external drivers. It never executes agents, advisors or proof commands. Use it when you own execution and semantic acceptance; it is not an autonomous agent runner. Providers are separate processes implementing describe/evaluate transport. The CLI/process boundary is public; internal Rust APIs are not a stability contract.

## Installation

From the repository root with Rust/Cargo installed:

```sh
cargo build --locked -p loop-cli -p policy-document-provider
target/debug/loop-engine --help
```

No model service is built in. Bring your own harness and any service credentials for external reviews; the provider's reviewer protocol defines judgment input/output. Use root `README.md` (relative to the checkout root) for a first policy-document run with a dedicated database and explicit provider configuration.

## Usage

Primary commands: `start`, `list`, `show`, `append`, `event`, `history`, `terminate`. Additional commands: targeted `read`, terminal `explore`, passive `monitor`, explicit `reclaim` (preview by default, `--apply` to clear finished runs' legacy invocation payloads and shrink the catalog file). Monitoring observes durable state, not external execution. Approved artifacts of capturing providers are retained as `retained-artifacts` context records, readable through full `show` after their files are deleted. Active runs retain frozen obligations and provider command/args, not a binary snapshot. Preserve a compatible executable at that stored path; no migration is supplied.

```sh
target/debug/loop-engine --help
```

For driving procedure and exact observation/mutation forms, load root `docs/agent-usage.md` and `skills/using-loop-engine/SKILL.md`. For repository edits and operator traps (unarmed mutations, catalog redirects, test isolation), load root `AGENTS.md`; do not use a user catalog for tests. An action/full observation is required before mutations; compact/status views do not arm them. Early provider commissions return `missing_inputs`, not ready work. Typed advice answers are externally authored `advice-answer` context.

## Validation

From the repository root:

```sh
cargo test -p loop-cli
```

These mechanical tests do not establish semantic deliverable acceptance. Root `AGENTS.md` owns workspace proof and Git approval rules; agents must not stage or commit independently.
