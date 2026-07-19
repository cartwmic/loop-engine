# Architecture and tenet adherence rubric v1

Base-versioned focused rubric. Loaded only from the remote base revision's committed `quality/rubrics/manifest.json`. Applies to the following push only; never read from the candidate working tree for self-judgment.

## Criteria

### ARCH-1. Dependencies point inward (KISS boundary)

Product code **MUST** keep three inward-pointing crates: core ← integrations ← CLI composition root. Core **MUST NOT** depend on integrations or CLI. Within core, dependencies point toward the model.

Cite: `docs/tenets.md` § 19; `docs/invariants.md` § I22, I23; `docs/architecture.md` § Composition and enforcement.

### ARCH-2. No catch-all abstractions

Do not create generic repositories, catch-all services, `util`, `common`, or interfaces beside implementations. Add capability only for an external side effect or genuine contract boundary.

Cite: `docs/architecture.md` § Composition and enforcement; `docs/tenets.md` § 14 (focused core / KISS).

### ARCH-3. Judge tooling stays outside product runtime

Replaceable semantic judges are invoked through one versioned generic executable contract. Judge tooling **MUST** remain outside product runtime and create no core dependency.

Cite: `docs/architecture.md` § Composition and enforcement; `docs/tenets.md` § 27.

### ARCH-4. Clean-room; no OpenSpec

Project work **MUST NOT** import prior loop-specific implementations or artifacts. OpenSpec-related skills, commands, and artifacts **MUST NOT** be used.

Cite: `docs/tenets.md` § 18; `docs/invariants.md` § I21.
