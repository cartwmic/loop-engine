# Documentation impact and consistency rubric

## Owner

This axis owns semantic judgment of documentation impact and consistency for the exact aggregate change and resulting candidate tree. Ordinary tests own objective formatting, link, schema, and other executable contracts.

## Criteria

### DOC-1. Assess documentation impact

Determine whether changed behavior, architecture, contracts, operator workflow, testing policy, or development policy requires documentation changes. A conclusion that no documentation change is needed must be supported by the changed lines and resulting behavior, not assumed from file type.

### DOC-2. Keep the resulting tree coherent

The resulting candidate tree must describe behavior, architecture, contracts, testing policy, and development policy consistently. New text must agree with implemented behavior and existing authoritative documents; stale or contradictory claims block.

### DOC-3. Judge the exact aggregate scope

Judge the complete base-to-candidate change. Commits within that unpublished range may repair one another, but follow-up work outside the candidate cannot excuse a documentation gap in this candidate.

### DOC-4. Keep deterministic and semantic responsibilities separate

Deterministic documentation checks prove objective properties only. They do not establish that explanations are sufficient, consequences are addressed, or documents remain semantically consistent.

## Evidence expectations

Cite changed or resulting-tree paths and explain concrete inconsistency or missing impact. Do not invent behavior, build, or test results beyond supplied deterministic evidence.
