# Foundation seed rubric v1

Parent revision: `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`

This rubric is frozen by T012. Focused rubric files under `quality/rubrics/*.md` apply only to commits after T025.

## docs/invariants.md — I47

Every commit **MUST** independently leave relevant documentation coherent with behavior, architecture, contracts, testing policy, and development policy it introduces. Versioned semantic judge **MUST** evaluate exact parent-to-commit diff and resulting documentation through generic replaceable executable contract. Candidate commit is judged by parent revision's rubric; accepted rubric change applies to following commit. Bootstrap rule: initial foundation commit and first publication **MAY** proceed through explicit owner approval without parent rubric or judge executable; that commit becomes parent rubric for every following commit.

Determinate local judge failure **MUST** block commit. Local unavailable/indeterminate judge **MAY** warn and allow commit. Before publication, every commit **MUST** receive determinate pass; fail, unavailable, or indeterminate result blocks pre-push/authoritative gate. Later commit **MUST NOT** substitute for required same-commit documentation. Deterministic formatting/link/schema checks remain separate and cannot replace semantic judgment.

## docs/testing.md — Git enforcement direction

Git hooks can enforce cooperative local workflow but cannot be unbypassable on user-controlled machine.

Settled semantic policy:

- one generic versioned judge-executable contract supports focused rubrics for documentation impact, observability, architecture/tenet adherence including KISS, and behavioral evidence;
- each commit is judged independently from exact parent-to-commit diff and resulting tree;
- parent revision's rubric judges candidate revision, so changed rubric applies only to following commit; initial foundation commit and first publication use explicit owner-approved bootstrap exception and become parent rubric thereafter;
- determinate local failure blocks commit; unavailable/indeterminate local result warns and permits commit;
- pre-push and authoritative remote gate fail closed on failed, unavailable, or indeterminate judgment for any commit;
- semantic judges receive deterministic build/test/check evidence and must cite changed lines/rubric rules rather than invent compilation or test claims;
- deterministic documentation, architecture, and quality checks remain separate from semantic judgment.

Candidate local mechanism:

- version hooks under `.githooks/`;
- fast deterministic checks plus semantic-judge attempt against exact staged content and parent rubric at pre-commit;
- full pre-push gate judges every unpublished commit against its parent in temporary detached worktree;
- no duplicated gate logic between hooks and CI;
- non-shipping Rust `xtask` installs hooks and runs canonical gate.

Candidate authoritative mechanism after remote exists:

- protect `main` from direct writes;
- require branch current with `main`;
- require canonical gate before merge;
- make releases depend on same gate;
- prevent bypass where hosting platform supports it.

Server-side controls are authority. Local hooks provide earlier feedback and accidental-regression protection.

## docs/tenets.md — 27. Documentation evolves with every commit

Every commit must leave relevant documentation coherent with behavior, architecture, contracts, testing policy, and development policy introduced by that commit. When documentation is unnecessary, semantic judge must affirm that conclusion from exact parent-to-commit change. Later commit cannot repair earlier commit for publication-gate purposes.

Deterministic documentation checks and semantic judgment are complementary. Judge remains replaceable development tooling and must not create engine runtime or harness dependency.

## docs/architecture.md — Composition and enforcement

Concrete construction occurs only in CLI composition root. Core does not instantiate process or persistence integrations.

Cargo manifests enforce crate-level direction. Automated architecture check must enforce core's internal model/capabilities/operations direction, prohibit outer dependencies in core, and prevent direct provider/persistence/dispatch bypass outside approved integrations and composition root.

Development tooling invokes replaceable semantic judges through one versioned generic executable contract. Focused rubrics cover documentation impact, observability, architecture/tenet adherence including KISS, and behavioral evidence. Every commit is evaluated against parent revision's rubric and exact diff; judge tooling remains outside product runtime and creates no core dependency.

Do not create generic repositories, catch-all services, `util`, `common`, or interfaces beside implementations. Add capability only for external side effect or genuine contract boundary.
