# Task-packet artifact

Plan implementation as an executable dependency graph. Each task must let fresh capable worker act without inventing product or architecture decisions.

Every task includes:

- objective;
- dependencies;
- source-of-truth references;
- deliverables;
- out-of-scope boundaries;
- validation;
- handoff contract.

Keep contract-establishing tasks before parallel fan-out. Name ownership and interfaces where agents could collide. Make completion observable. Record dependencies honestly; do not hide work in a giant task or leave unresolved decisions for implementation. Include **doc integration** as explicit deliverable: authoritative repository documents must remain coherent with delivered behavior, and no change-scoped PRD may remain a parallel source of truth.

Required metadata: non-empty `revision`, `author`, and `design_revision` matching current `design.json`.
