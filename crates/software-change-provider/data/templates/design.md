# Design artifact

Write `design.json` as structural design for accepted intent.

Required machine-checked shape:

- `revision`, `author`, and `intent_revision`.
- `approach`: concise description of resulting shape.
- `elements`: named parts, each with `name` and `responsibility`.
- `decisions`: choices with `choice`, `rationale`, and optional `rejected` alternatives.
- `risks`: design-specific risks with `risk` and `mitigation`.
- `coverage`: each design coverage row has `acceptance` and `delivered_by`, with an optional `criterion_id` referring to a current intent `AC-N` record.

Describe boundaries, responsibilities, relationships, invariants, and decisions. Do not turn design into task schedule or implementation diary. `intent_revision` must equal current `intent.json` revision when checked. Criterion references are optional and are checked only for AC-N shape, local duplicate-freeness, and current-intent membership; do not reproduce a complete criterion matrix or maintain a separate PRD-ID spine.
