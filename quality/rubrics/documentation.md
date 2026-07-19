# Documentation impact rubric v1

Parent-versioned focused rubric. Loaded only from the parent revision's committed `quality/rubrics/manifest.json`. Applies to the following commit only; never read from the candidate working tree for self-judgment.

## Criteria

### DOC-1. Per-commit documentation coherence

Every commit **MUST** independently leave relevant documentation coherent with behavior, architecture, contracts, testing policy, and development policy it introduces.

Cite: `docs/invariants.md` § I47; `docs/tenets.md` § 27.

### DOC-2. Exact parent-to-commit scope

Judgment **MUST** use the exact parent-to-commit diff and resulting documentation. When documentation is unnecessary, the judge **MUST** affirm that conclusion from that exact change.

Cite: `docs/tenets.md` § 27; `docs/invariants.md` § I47.

### DOC-3. No deferred repair for publication

A later commit **MUST NOT** substitute for required same-commit documentation or repair an earlier commit for publication-gate purposes.

Cite: `docs/invariants.md` § I47; `docs/tenets.md` § 27.

### DOC-4. Deterministic checks are complementary

Deterministic formatting, link, and schema checks remain separate and **MUST NOT** replace semantic judgment.

Cite: `docs/invariants.md` § I47; `docs/testing.md` § Git enforcement direction.
