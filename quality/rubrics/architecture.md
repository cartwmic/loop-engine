# Architecture, tenet, and KISS rubric

## Owner

This axis owns semantic architecture judgment that cannot be established by Cargo metadata tests. Ordinary tests own product-crate dependency direction and sole-product-binary facts.

## Criteria

### ARCH-1. Core internals point toward the model

Within core, model must not depend on capabilities or operations, and capabilities must not depend on operations. Capability contracts should speak in core model types rather than transport, database, configuration, or CLI records.

### ARCH-2. External construction stays in owned adapters

Core must not construct provider processes or persistence integrations. Integrations must keep provider-process and persistence construction in their owned adapters, including executable-provider and SQLite details, and translate integration-specific errors before they cross inward.

### ARCH-3. CLI remains the concrete composition and dispatch boundary

Concrete integration construction belongs in CLI `composition.rs`. Operation-root dispatch belongs in CLI `dispatch.rs`. Changes must not create alternate provider, persistence, composition, or operation-dispatch paths that bypass those ownership points.

### ARCH-4. Integration details do not leak inward

Changes must ensure raw integration details do not leak into core policy or capability contracts. CLI must not select transitions, reinterpret provider verdicts, or make workflow-policy decisions.

### ARCH-5. Architecture remains focused and KISS

Apply architecture tenets and KISS judgment to the aggregate change. Reject catch-all services, generic repositories, speculative interfaces, `util`/`common` dumping grounds, needless layer growth, or framework machinery without a genuine contract or external-effect boundary.

### ARCH-6. Validation tooling stays outside product behavior

Git validation and replaceable semantic-judge mechanics must remain outside product runtime and must not expand Loop Engine product behavior.

## Evidence expectations

Cite concrete changed paths and explain dependency, construction, placement, leakage, or needless-complexity consequences. Do not replace judgment with source-text token counting or demand a retired scanner.
