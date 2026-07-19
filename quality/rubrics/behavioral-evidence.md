# Behavioral evidence rubric v1

Base-versioned focused rubric. Loaded only from the remote base revision's committed `quality/rubrics/manifest.json`. Applies to the following push only; never read from the candidate working tree for self-judgment.

## Criteria

### BEH-1. CLI E2E is behavioral authority

Only black-box production-driver tests **MAY** satisfy behavioral acceptance. Lower-level tests **MUST NOT** substitute for missing production-driver coverage.

Cite: `docs/invariants.md` § I28; `docs/testing.md` § Behavioral authority; `docs/tenets.md` § 20.

### BEH-2. No mock-based behavioral authority

Required behavioral tests **MUST** use production CLI, persistence, and provider-process integrations. Mock frameworks and mock-based behavioral tests **MUST NOT** be used.

Cite: `docs/invariants.md` § I29; `docs/testing.md` § No-mock policy; `docs/tenets.md` § 21.

### BEH-3. No invented compilation or test claims

Semantic judges receive deterministic build/test/check evidence and **MUST** cite changed lines and rubric rules. Judges **MUST NOT** invent compilation or test claims.

Cite: `docs/testing.md` § Git enforcement direction.

### BEH-4. Direct fixture setup is not behavioral proof

Direct fixture construction for migration/corruption or narrowly for schema-valid prerequisite state is never behavioral evidence for an operation that would create that state; such setup must later be repeated through the production CLI after the owning operation exposes.

Cite: `docs/testing.md` § Isolation requirements.
