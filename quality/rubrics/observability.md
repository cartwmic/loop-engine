# Observability consequences rubric

## Owner

This axis owns judgment of observability consequences introduced by the exact aggregate change. It evaluates diagnostic authority, stable instrumentation boundaries, failure visibility, correlation, and truthful completeness claims.

## Criteria

### OBS-1. Trace remains diagnostic, not mutation authority

Operational trace must stay diagnostic, not mutation authority. It must not become competing authority over SQLite current state, immutable journal facts, or export artifacts.

### OBS-2. Consequential work remains visible at stable boundaries

Changes must preserve useful correlated visibility at dispatch, provider-execution, and persistence boundaries. New alternate paths must not silently bypass the boundary that owns their observable request, result, timing, and failure facts. Pure helpers do not require per-function logging.

### OBS-3. Failure timing and authority stay truthful

Trace initialization failure must remain pre-dispatch. Later sink failure must not retroactively change an authoritative operation outcome, and diagnostics must distinguish failure before durable work from failure after commit.

### OBS-4. Observation must not overclaim completeness

Abrupt process death, sink failure, bounded capture, and rotation can limit observation. Documentation, code, and evidence must not overclaim completeness or imply that missing trace data proves work did not occur.

### OBS-5. Sensitive and unrelated process context stays out

Operational evidence should contain bounded contract-relevant payloads and correlation facts without leaking inherited environment or unrelated process context. Any changed retention or exposure consequence must be addressed explicitly.

## Evidence expectations

Cite changed boundaries and supplied deterministic or behavioral evidence. Block when a consequential path loses required visibility, authority becomes ambiguous, or claimed evidence exceeds what instrumentation can prove.
