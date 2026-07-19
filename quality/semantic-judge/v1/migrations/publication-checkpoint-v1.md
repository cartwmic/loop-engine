# Owner-authorized publication-checkpoint migration rubric v1

Authority: owner direction dated 2026-07-18. Scope is exactly one aggregate publication whose remote base is foundation revision `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`. This is a policy-migration rubric, not a second bootstrap or reusable exception.

## MIG-1. Aggregate checkpoint coherence

Judge one exact owner-authorized governance-repair delta from reviewed C1 checkpoint `30f210d2a064c679c44f7880b67958fc23efe21e` to candidate head. Deterministic evidence must bind the complete foundation-base-to-candidate range by SHA-256, byte count, changed-path count, and candidate quality; this bounded projection exists only because the one-time 2 MiB migration request exceeds judge context. Internal unpublished commits are audit context only and may be incomplete or repaired by later commits in the same range. Do not require independent per-commit coherence.

## MIG-2. Governance correction completeness

Candidate must consistently replace current per-commit publication scheduling across governing invariants, tenets, intent, development policy, testing policy, task tracking, hooks, publication tooling, semantic request scheduling, CI, and tests. Frozen foundation artifacts may remain unchanged when clearly identified as historical and superseded.

## MIG-3. Fast local commit path

Default pre-commit must use bounded deterministic exact-staged checks only. Full quality and semantic publication judgment must not run automatically on every local commit. Explicit staged semantic advice may remain available.

## MIG-4. Exact aggregate publication authority

Existing-branch publication must use advertised destination SHA as base and candidate pushed head as endpoint. New-branch publication must fail closed unless an exact integration base is resolved. Candidate-head deterministic quality runs once and one semantic request uses this migration rubric. Fail, unavailable, or indeterminate blocks.

## MIG-5. Security and CI isolation

Pre-push gate code must bind to pushed candidate. Ambiguous multi-content pushes must be rejected. Authoritative CI must keep semantic credentials unavailable to candidate-controlled build, test, and tooling processes; trusted base gate code consumes revision-bound deterministic evidence before invoking semantic judge.

## MIG-6. Evidence and validation

Request must include deterministic test/check evidence, complete-range digest binding, and one revision-bound semantic response. Task tracking must require blind-review closure before publication. Claims must cite changed or resulting-tree paths; unnumbered paths are required when the bounded migration projection omits duplicate resulting-document snapshots. Missing deterministic evidence or complete-range binding blocks migration.
