# Behavioral-evidence sufficiency rubric

## Owner

This axis owns semantic judgment of behavioral-evidence sufficiency for the exact aggregate change. Ordinary tests own objective operation-catalog equality and schema, protocol, and other executable product contracts.

## Criteria

### BEH-1. Changed behavior has production-path proof

User-visible or operational behavior must have proportionate black-box production CLI evidence. Evidence must exercise the built CLI as a separate process and observe relevant output, exit status, persistence, provider invocation, later CLI query, and correlated trace consequences.

### BEH-2. Required behavior uses real integrations

Behavioral authority requires real provider-process and SQLite integrations plus production parsing, rendering, configuration, and dispatch. Mock-based or in-process substitutes cannot establish product behavior.

### BEH-3. Evidence closes affected facets and regressions

Changed operations must retain sufficient valid, rejection, failure, mutation, journal, lifecycle, provider, persistence, and trace coverage for applicable facets. Avoid both missing affected paths and wasteful cross-products that add no distinct behavioral claim.

### BEH-4. Lower-level tests remain supporting evidence

Lower-level schema, protocol, unit, integration, and property tests are valuable regression contracts but cannot substitute for missing production-driver evidence. Direct fixture setup can establish prerequisite state, not behavioral proof of an operation that would create that state.

### BEH-5. Claims match supplied evidence

Review only supplied deterministic evidence and resulting test content. Do not invent compilation, execution, platform, or passing-test claims. A named test without runtime observation is not proof that an operation executed.

## Evidence expectations

Cite changed behavior, exact relevant tests, and deterministic evidence. Block when evidence cannot support a changed behavioral claim or when test strategy bypasses production boundaries.
