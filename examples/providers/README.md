# Provider contract validation

This directory documents how independent provider authors validate protocol v1 payloads against the published JSON Schema inventory. It does **not** ship a reference provider executable; scenario and reference providers remain planned under `test-support/providers/` (T135–T142).

## Normative contracts

| Document | Scope |
|---|---|
| [provider-protocol-v1.md](../../docs/provider-protocol-v1.md) | Subprocess transport, envelopes, roles, byte framing, same-major compatibility |
| [graph-projection.md](../../docs/graph-projection.md) | Graph wire parse rules, canonical bytes, duplicate-key rejection |
| [cli-contract.md](../../docs/cli-contract.md) | Named numeric bounds referenced by schema `x-loop-engine-bounds` markers |
| [schemas/index.json](../../schemas/index.json) | Deterministic index of all published provider v1 schema files |

## Published schema files

Provider protocol v1 JSON Schemas live under `schemas/provider/v1/`:

| File | Role / purpose |
|---|---|
| `graph.json` | Shared graph DTO (`GraphDto`) |
| `describe-request.json` / `describe-result.json` | `describe` |
| `validate-inputs-request.json` / `validate-inputs-result.json` | `validate_inputs` |
| `evaluate-gates-request.json` / `evaluate-gates-result.json` | `evaluate_gates` |
| `live-guidance-request.json` / `live-guidance-result.json` | `live_guidance` |
| `check-compatibility-request.json` / `check-compatibility-result.json` | `check_compatibility` |

Each envelope schema pins `protocol_major` `1`. Request schemas additionally pin the operation `role`. Bound marker names in `x-loop-engine-bounds` reference [cli-contract.md](../../docs/cli-contract.md#resource-bounds-d008); numeric limits are not duplicated here.

## Regenerating published schemas

From the repository root:

```bash
cargo run -p loop-engine-integrations --example generate_provider_schemas
```

This overwrites the eleven files under `schemas/provider/v1/` from integration DTO types (T084). It does not build or run a provider executable.

## Repository validation commands

These integration tests guard the published inventory:

```bash
cargo test -p loop-engine-integrations published_schema_inventory_is_parseable
cargo test -p loop-engine-integrations published_schemas_pin_protocol_major_and_operation_role
cargo test -p loop-engine-integrations published_schemas_carry_runtime_bound_markers
```

Additional protocol conformance (strict parse, duplicate-key rejection, trailing-value rejection, role-specific result kinds, and malformed fixtures) is covered by `crates/loop-engine-integrations/tests/provider_protocol.rs` and related unit tests in `provider_protocol/validation.rs`.

## Validating author payloads offline

Authors may validate draft request/result documents against the published JSON Schema files using any Draft 2020-12 validator. JSON Schema checks structural shape and declared bounds markers; it does **not** replace normative transport rules in [provider-protocol-v1.md](../../docs/provider-protocol-v1.md):

- exactly one UTF-8 JSON object on stdin and stdout with no trailing bytes;
- duplicate object keys rejected at any depth;
- trailing JSON values after the first complete document rejected;
- within major version `1`, unknown fields are additive only when optional and ignorable by readers.

Graph payloads additionally require [graph-projection.md](../../docs/graph-projection.md) wire parse rules (finite JSON numbers, duplicate-key rejection, canonical byte semantics) before `graph_revision` is meaningful.

## Production exposure

WP1 publishes schemas and private integration tests only. The production `loop-engine` binary does not expose application provider routes yet; provider subprocess invocation remains integration-owned until later work-package exposures.
