# Documentation impact rubric v1

Base-versioned focused rubric. Loaded only from the remote base revision's committed `quality/rubrics/manifest.json`. Applies to the following push only; never read from the candidate working tree for self-judgment.

## Criteria

### DOC-1. Publication-checkpoint documentation coherence

Every accepted push **MUST** leave its candidate destination tip coherent with behavior, architecture, contracts, testing policy, and development policy introduced by the aggregate remote-base-to-candidate-head change.

Cite: `docs/invariants.md` § I47; `docs/tenets.md` § 27.

### DOC-2. Exact base-to-head scope

Judgment **MUST** use the exact remote-base-to-candidate-head diff and resulting tree. When documentation is unnecessary, judge **MUST** affirm that conclusion from the aggregate change.

Cite: `docs/tenets.md` § 27; `docs/invariants.md` § I47.

### DOC-3. Internal repair allowed; later push repair forbidden

Commits inside one unpublished range **MAY** repair one another. A later push **MUST NOT** substitute for coherence required at an earlier accepted publication checkpoint.

Cite: `docs/invariants.md` § I47; `docs/tenets.md` § 27.

### DOC-4. Deterministic checks are complementary

Deterministic formatting, link, and schema checks remain separate and **MUST NOT** replace semantic judgment.

Cite: `docs/invariants.md` § I47; `docs/testing.md` § Git enforcement direction.
