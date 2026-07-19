# Observability rubric v1

Parent-versioned focused rubric. Loaded only from the parent revision's committed `quality/rubrics/manifest.json`. Applies to the following commit only; never read from the candidate working tree for self-judgment.

## Criteria

### OBS-1. Trace is diagnostic, not mutation authority

Operational trace is diagnostic storage only. It **MUST NOT** become competing authority over authoritative SQLite state, journal rows, or export artifacts.

Cite: `docs/operational-trace.md` § Scope and authority; `docs/invariants.md` § I42, I46.

### OBS-2. Journal explains; current state remains authoritative

The immutable ordered journal explains changes. Stored current state remains authoritative; journal **MUST NOT** be folded to derive current state, and state/journal **MUST NOT** silently disagree about durable mutation.

Cite: `docs/tenets.md` § 8; `docs/invariants.md` § I12, I13, I14, I15.

### OBS-3. Visibility without silent engine work

Every CLI invocation **MUST** create an always-on structured operational trace before dispatch. Instrumentation belongs at stable operation-dispatch, provider-execution, and persistence boundaries; do not require logging in every helper.

Cite: `docs/tenets.md` § 26; `docs/invariants.md` § I46.

### OBS-4. Incomplete observation must not be overclaimed

Abrupt process death, storage failure, and rotation can limit trace completeness. The engine **MUST NOT** claim impossible complete observation.

Cite: `docs/invariants.md` § I46; `docs/operational-trace.md` § Scope and authority.
