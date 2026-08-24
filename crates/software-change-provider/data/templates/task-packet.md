# Task-packet artifact

Plan implementation as an executable dependency graph. Each task must let fresh capable worker act without inventing product or architecture decisions.

Every task includes:

- objective;
- dependencies;
- source-of-truth references, including the frozen intent and any predecessor contract needed by a fresh worker;
- affected user or operator paths and the observable completion outcome;
- deliverables;
- out-of-scope boundaries;
- validation;
- handoff contract.

Validation must use realistic black-box proof of the observable outcome when practical. If black-box proof is genuinely impractical, state the concrete reason and the nearest realistic substitute; a list of completed work, internal tests, or passing commands is not outcome proof by itself. Keep task packets specific enough to preserve acceptance without prescribing replaceable mechanisms. Implementation agents have freedom inside frozen intent, operating context, outside obligations, and design decisions; do not leave product or architectural decisions for them, and do not turn a preferred implementation into a requirement.

Keep contract-establishing tasks before parallel fan-out. Name ownership and interfaces where agents could collide. Make completion observable. Record dependencies honestly; do not hide work in a giant task or leave unresolved decisions for implementation. Include **doc integration** as explicit deliverable: authoritative repository documents must remain coherent with delivered behavior, and no change-scoped PRD may remain a parallel source of truth.

Required metadata: non-empty `revision`, `author`, and `design_revision` matching current `design.json`.

At implementation execution, the provider may add a `finding_context` array to each task object. It contains only current driver-accepted, unresolved findings with `owner_phase: "implementation"` whose `task_ids` contains that exact task ID. Stale, resolved, rejected, advisory, and unrelated ledger entries are not copied. This runtime enrichment does not change the plan graph, task dependencies, or worker boundary: the inner worker still receives only the compact `{artifact_root, task}` packet.
