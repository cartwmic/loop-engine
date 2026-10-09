# Loop Engine

## Purpose

Loop Engine stores durable workflow state and evaluates explicit transitions for external drivers. It is for operators who want retained context, evidence and history while choosing their own agent harness. It never executes agents, advisors or proof commands. It is not an autonomous runner or an agent service; bring your own execution and review tools (and their service credentials, if needed).

Software-change delivers code, policy-document drafts or audits documents, and research investigates claims; generate-PRD extracts requirement candidates. Engine requirements are living; provider requirements and protocols are frozen. CLI/provider process transport is the integration boundary, not a promise that internal Rust APIs are stable. Active runs retain frozen workflow/obligation data and the original provider command/args association, not a copy of the executable. Drivers must preserve a compatible executable at that path; no active-run migration is supplied. Each run assumes one logical mutating driver: serialize mutations rather than running independent concurrent mutators against the same run.

## Installation

From a source checkout with Rust/Cargo and Python 3 installed:

```sh
cargo build --locked --workspace --all-targets
target/debug/loop-engine --help
```

The executable is `loop-engine`; providers are separate binaries. No bundled model server is required: external drivers own model selection and any compatible service setup. See each provider's reviewer protocol and skill below for the judgment contract.

## Usage

For a first durable run, use policy-document to assess this README. Choose a fresh directory outside the checkout for the catalog and artifacts, prepare with `target/debug/policy-document setup --rigor readme --output /absolute/profile.json`, preserve `target.id` and `profile_version`, and set `target.path` to this README's absolute path with `mode: "audit"`. Save it as `profile.json` in that directory. Put the absolute path to the built `target/debug/policy-document` in `providers.toml`:

```toml
[providers.policy-document]
command = "/absolute/checkout/target/debug/policy-document"
args = []
```

After replacing `/absolute/proof` with that directory, these commands create and inspect the run (they do not execute a reviewer or complete the assessment):

```sh
target/debug/loop-engine --database /absolute/proof/catalog.sqlite --config /absolute/proof/providers.toml --json start --id readme-audit policy-document @/absolute/proof/profile.json "README assessment"
target/debug/loop-engine --database /absolute/proof/catalog.sqlite --config /absolute/proof/providers.toml --json show --view full readme-audit
```

Continue with the [policy-document operator guide](crates/policy-document-provider/AGENTS.md), which states the supported commands and provider-specific packet contract, then its [skill](crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md) and [reviewer protocol](crates/policy-document-provider/data/reviewer-protocol.md). Code work starts with the [software-change operator guide](crates/software-change-provider/AGENTS.md) and [skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md); investigations start with the [research operator guide](crates/research-provider/AGENTS.md) and [skill](crates/research-provider/skills/using-research-provider/SKILL.md). The operator guides own artifact-root preparation (the bundled research/software profiles omit that required machine-local directory) and current packet/tool contracts. These own driving procedure, not this overview.

Current source software-change profiles (`minimal-15`, `standard-15`, `high-rigor-15`) let you opt into reviewed markdown narratives at design, plan and validation, and owner checkpoints at intent, design, plan, implementation and validation. Both options default off and freeze at start. The JSON stays authoritative; a configured checkpoint waits for a recorded decision on the current narrative or artifact. These options require the current source provider; the installed 0.24.1 predecessor lacks them. Use the [software-change skill](crates/software-change-provider/skills/using-software-change-provider/SKILL.md#optional-narratives-and-owner-checkpoints) for setup, review and rejection recovery.

Primary commands are `start`, `list`, `show`, `append`, `event`, `history`, `terminate`; targeted `read`, terminal `explore` and passive `monitor` expose retained state, and explicit `reclaim` shrinks a catalog holding legacy payloads. Monitoring does not track external execution. Providers implement describe/evaluate transport and may opt in to `capture`, which keeps each approved artifact's exact bytes in the catalog so a run's approved documents survive deletion of its files.

## Validation

A local mechanical smoke check (not semantic product proof):

```sh
python3 scripts/software-change-journey.py --self-test
```

Contributors and agents follow [AGENTS.md](AGENTS.md) for build, journey and proof gates. Human CLI operators use [agent usage](docs/agent-usage.md) for argument forms and observation/mutation rules. If a mutation is unarmed, a commission is incomplete, or a catalog unexpectedly changes location, use those operator instructions and the active provider skill; tests must not touch your user catalog. External judgments and commands remain your driver's responsibility; successful transitions alone do not establish deliverable fitness.
