# Task-packet artifact

Plan implementation as useful externally driven work with meaningful dependencies. The driver may group work freely; one task need not mean one agent call. Each task must let a fresh capable actor act without inventing product or architecture decisions.

Every task includes:

- objective;
- dependencies;
- source-of-truth references, including the frozen intent and any predecessor contract needed by a fresh worker;
- affected user or operator paths and the observable completion outcome;
- deliverables;
- out-of-scope boundaries;
- validation;
- handoff contract;
- `criterion_ids`, a non-empty unique list of current intent `AC-N` IDs naming the criteria this task serves. This is required for every task in a contract-v4 plan; other artifact links remain optional.

For contract-v4 plans, every task carries its required criterion reference; other intermediate artifacts remain optional. Do not reproduce a complete criterion matrix or create a parallel PRD-ID spine. With Bookends enabled, the plan phase uses the existing AC/task/proof-command spine to explain which applicable accepted requirement obligations and explicitly named authoritative documents the decomposition covers. Put actionable missing configuration, live wording, or eligible CI/public-location prerequisites in the plan before approval; do not require proof of behavior that has not yet been implemented. Bookends-off plans add no PRD-ID duty.

Validation must use realistic black-box proof of the observable outcome when practical. If black-box proof is genuinely impractical, state the concrete reason and the nearest realistic substitute; a list of completed work, internal tests, or passing commands is not outcome proof by itself. Keep task packets specific enough to preserve acceptance without prescribing replaceable mechanisms. Implementation agents have freedom inside frozen intent, operating context, outside obligations, and design decisions; do not leave product or architectural decisions for them, and do not turn a preferred implementation into a requirement.

Keep contract-establishing work before dependent work. Name ownership and interfaces where agents could collide. Make completion observable. Record dependencies honestly; do not hide work in a giant task or leave unresolved decisions for implementation. Include **doc integration** as explicit deliverable: authoritative repository documents must remain coherent with delivered behavior, and no change-scoped PRD may remain a parallel source of truth.

Required metadata: non-empty `revision`, `author`, and `design_revision` matching current `design.json`. Contract v4 also requires `proof_commands: [{id,command,args,owner,obligation}]`: unique named runnable deterministic obligations, not shell prose or invented pass claims. Every task must carry a non-empty unique `criterion_ids` set of current intent IDs; the provider checks membership and reviewers judge relevance. Each task may reference names with `proof_command_ids`; task-local validation is focused proof, not another mandatory full-suite rerun list. Name every required final command in proof_commands regardless of who executes it.

Workers run only assigned focused validation. One designated driver/proof owner performs the complete final matrix on the stable tree and repeats only checks invalidated by later changes. Reviewers consume retained command/outcome evidence rather than independently rerunning suites. Applicable structured steering may change a named proof's owner or command/args with a reason while retaining its accepted obligation; changed outcomes/decomposition require revision. Use the simplest adequate mechanism, not speculative hardening or preservation of incidental implementation choices.

The driver inspects current findings and exact-recipient steering through provider commissions, supplies necessary context to external actors, and preserves their actual requests and returns. Only current driver-accepted, unresolved findings routed to the work belong in its correction context; stale, resolved, rejected, advisory and unrelated entries are not obligations. Grouping or delegating work does not alter dependencies, accepted outcomes or review eligibility. The driver/harness owns external work and cleanup; neither engine nor provider launches tasks or a summarizer.
