# Changelog

## [0.19.0] - 2026-09-07

### Recovery and review

- Driver dispositions can discharge specific reviewer failures while retaining original verdicts and blocking accepted unresolved findings.
- Durable steering reaches later workers and reviewers. Effective commissions are inspectable before launch.
- Owner-attested binding amendments affect future invocations without rewriting frozen policy or past attempts.
- Public cancellation stops an invocation without advancing the workflow; implementation can return directly to plan, design or intent once owned work is quiescent.
- Explicit owner overrides retain denied evaluations and failed evidence. Exceptional runs finish permanently as `completed-with-overrides`.
- Criterion-based validation indexes named command captures, separate criterion and goal judgments, and explicit carry of unaffected evidence.
- Shipped review guidance and opt-in bindings batch assigned axes into one commission per author per gate. Ordinary and challenge review remain separate.
- Independent journey jobs support bounded concurrency and a serial path. Test builds optimize digest computation without removing integrity checks or relaxing deadlines.
- The proof pool restores the caller's Linux child-subreaper setting after success, failure or timeout, so later workflows do not inherit its orphan-reaping responsibility.
- Capture-backend errors now include the backend's diagnostic output.
- Validation-command timeout cleanup disambiguates negative process-group operands for older Linux `kill` implementations, preserving the capture parent while stopping the intended command group.

### Compatibility

Software-change now ships contract-v2 `minimal-9`, `standard-9` and `high-rigor-9` profiles, unbound by default. Use matching engine, provider and embedded profile versions. Older evidence remains readable; existing runs are not silently migrated to the new semantics.

### Known limitation

Cancellation ownership still relies on numeric PID/PGID values. After an interrupted controller and a sufficiently long delay, process-ID reuse could cause resumed cleanup to misidentify an unrelated local process, signal it, or keep the run falsely blocked. This concern remains unreproduced and unrepaired. The owner accepted it for this release; the recovery workflow completed with overrides, not clean acceptance of the cancellation criterion or whole-intent goal.

Local scripted proof does not establish universal performance gains or semantic review quality. Later live dogfood and the pre-existing calibration attestations remain pending.
