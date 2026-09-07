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
- handoff contract;
- optional `criterion_ids`, a non-empty list of current intent `AC-N` IDs when this task usefully names its criterion scope.

Do not require every task to carry a criterion reference, and do not reproduce a complete criterion matrix or create a parallel PRD-ID spine.

Validation must use realistic black-box proof of the observable outcome when practical. If black-box proof is genuinely impractical, state the concrete reason and the nearest realistic substitute; a list of completed work, internal tests, or passing commands is not outcome proof by itself. Keep task packets specific enough to preserve acceptance without prescribing replaceable mechanisms. Implementation agents have freedom inside frozen intent, operating context, outside obligations, and design decisions; do not leave product or architectural decisions for them, and do not turn a preferred implementation into a requirement.

Keep contract-establishing tasks before parallel fan-out. Name ownership and interfaces where agents could collide. Make completion observable. Record dependencies honestly; do not hide work in a giant task or leave unresolved decisions for implementation. Include **doc integration** as explicit deliverable: authoritative repository documents must remain coherent with delivered behavior, and no change-scoped PRD may remain a parallel source of truth.

Required metadata: non-empty `revision`, `author`, and `design_revision` matching current `design.json`. Contract v2 also requires `proof_commands: [{id,command,args,owner,obligation}]`: unique named runnable deterministic obligations, not shell prose or invented pass claims. Each task may reference names with `proof_command_ids`; task-local validation is focused proof, not another mandatory full-suite rerun list. Name every required final command in proof_commands regardless of who executes it.

Workers run only assigned focused validation. One designated driver/proof owner performs the complete final matrix on the stable tree and repeats only checks invalidated by later changes. Reviewers consume retained command/outcome evidence rather than independently rerunning suites. Applicable structured steering may change a named proof's owner or command/args with a reason while retaining its accepted obligation; changed outcomes/decomposition require revision. Use the simplest adequate mechanism, not speculative hardening or preservation of incidental implementation choices.

At implementation execution, the provider may add a `finding_context` array to each task object. It contains only current driver-accepted, unresolved findings with `owner_phase: "implementation"` whose `task_ids` contains that exact task ID. Stale, resolved, rejected, advisory, and unrelated ledger entries are not copied. The provider may also add exact-recipient `steering_context` from the invocation-start selection. Task-targeted steering does not spread to dependants or summarizer, and later appends do not change a started attempt. This runtime enrichment does not change the plan graph, task dependencies, or worker boundary: the inner worker still receives only the compact `{artifact_root, task}` packet.
