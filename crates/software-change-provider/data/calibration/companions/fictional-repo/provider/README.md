# Fictional provider README

Provider in `fictional-repo` runs as one fresh local subprocess per request. Flat `describe` returns only exact static workflow topology. Caller selects one full canonical profile as start `initial_input`; Loop Engine freezes that value, projects it unchanged through `show`, and carries it through `evaluate`. `evaluate` checks contained authored artifacts, maps deterministic structural violations into checked-transition denial details before or without evidence satisfaction, and aggregates supplied external review evidence by gate, axis, subject revision, config version, and exact declared name-kind identity.

Provider does not invoke a semantic judge, read arbitrary repository paths, write authored artifacts, route workflow state, or persist engine state. Exact declared identity is unauthenticated: a subject can claim another identity. Provider preserves deterministic declared-identity checks; external process and provenance controls remain outside provider scope, so that residual is accepted.

All profiles consume the same bounded schema contract: object/array/string types, exact per-type keyword allowlists, literal-false additionalProperties, rejected unknown or misplaced keywords, and exhaustive path/rule violations in deterministic order. Profile axes and author counts are frozen run input supplied through caller-selected `initial_input`; `show` projects that frozen value unchanged, while flat `describe` does not expose selected policies or schemas. No inferred obligations supplement declared terminal validation.

Reference entry points:

- [Authoritative product PRD](../docs/PRD.md)
- [Repository README](../README.md)
- [Requirement-to-proof matrix](../implementation-evidence/requirement-to-proof.md)
- [Executable proof-map checker](../scripts/assert-requirement-proof.py)

Profiles, schemas, templates, reviewer protocol, and tests are versioned reference data. Supplied review records support deterministic mechanics checks only; they do not attest semantic review quality.
