# Design principles

Michael's standing architecture and design rules for software built with or for him. This is the canonical principles document, managed in chezmoi at `dot_local/share/agent-harness/design-principles.md` with destination `~/.local/share/agent-harness/design-principles.md`. Agent instructions, playbooks and review guidance should reference it by section number without maintaining competing copies. Working-style rules (voice, subagents, doors, git) stay in the global AGENTS.md.

## 1. Composable components
Build small tools that each do one job and can be combined. An application is a composition of components plus configuration. Anything specific to one use (coordinating agents, budgets, a game) lives in the composition. Keep it out of the components.

**Test:** name a second, real use for the component outside the project that motivated it. If there isn't one, the boundary is drawn around a feature; reconsider whether it needs a component.

## 2. Kernel and adapters
Each component has a core that knows nothing about the runtime or transport it is used from (Pi, a CLI, Matrix, Discord, HTTP). Runtime- and transport-specific code lives in thin adapters on top. The core is usable on its own as a library plus a CLI.

**Gate:** dependency checks fail the build if a core imports an adapter, a runtime or a transport.

## 3. Clean architecture inside each component
Within a component, structure the code in layers with dependencies pointing inward only:
- **Domain:** entities, value objects and invariants. Pure logic, no I/O, no frameworks.
- **Use cases:** the operations the component offers, written against the domain and against ports (interfaces) for anything external.
- **Adapters:** implementations of those ports (storage, network, clock, runtime) and the entry points that call use cases (CLI, Pi extension, HTTP).

Inner layers never import outer ones; external concerns are reached only through ports. Domain and use cases are testable without any real storage, network or runtime. Scale the layering to the component: a small component may have a handful of files per layer (§4 applies).

**Gate:** dependency checks fail the build on any outward import.

## 4. YAGNI and KISS
Don't add code, abstractions, configuration or files the current need doesn't require, and don't design for requests that haven't been made. Prefer the smallest thing that works: a function beats a new layer, a script beats a service. The structure in §2 and §3 defines dependency boundaries. It does not justify speculative abstraction.

Unless asked otherwise, start with the smallest design that completes the required user journey. Add machinery when a concrete requirement or demonstrated failure makes that baseline insufficient; explain why a simpler fix would fail. A requirement citation alone does not justify a particular mechanism.

Count runtime code, configuration, recovery paths, test harnesses, review work and operator burden when judging simplicity.

If a small feature grows into substantial infrastructure or repeated repair cycles, pause the affected work and reassess. Explain what changed, whether a simpler path remains, and whether an owner trade-off is needed. Continue independent work where safe. Do not silently weaken acceptance to finish.

## 5. Written contracts and invariants
Every core has a contract: its schema, its operations, and the invariants it guarantees (for example "status changes only along declared transitions" or "a write with the same idempotency key is applied at most once"). The contract is versioned alongside the code.

## 6. Conformance suites
Every adapter must pass the component's conformance suite before it can merge. A contract without a conformance suite is not finished.

## 7. No cross-component imports
Components never import each other. They are wired together only in a composition package, through their public interfaces and event streams. No circular dependencies.

**Gate:** dependency checks fail the build on any cross-component import.

## 8. Judgment in agents, mechanism in code
Code enforces structure: schemas, transitions, idempotency, delivery, storage. Interpretation, classification and decisions belong to the agent. Never encode judgment as keyword rules or heuristics in code; if a decision needs judgment, give the agent the information and a tool.

The same split applies to repeated work: a deterministic pattern (same inputs, same steps, no judgment) becomes a script or code; a recurring pattern that needs judgment becomes a reusable skill. Do not use an LLM for deterministic repeated work or leave recurring semantic work as a one-off prompt.

## 9. Fix the cause; no arbitrary caps or unrequested defense in depth
- Don't add budgets, caps, rate limits or thresholds to cover for behavior that is incorrect. If something is noisy, wrong or too frequent, fix its cause. Do not paper over it with an arbitrary limit.
- Don't add layered safeguards (defense in depth) unless explicitly asked, when the problem would be visible and easy to fix. Prefer visibility and fast fixes over redundant guards.
- Limits that reflect a real external constraint (an API's message size or rate limit) are fine; they are part of the contract.

## 10. Good enough for personal software
This is personal, one-person software. Good enough is the bar: it does not need to be large-scale, production-grade secure or enterprise-hardened. Theoretical attacks and hostile operators are out of scope unless they already fail in ordinary single-user use. Never commit secrets; read them at runtime.

Apply the stated operating environment and accepted risks throughout design, implementation and review. Do not silently strengthen guarantees or repeatedly demand elimination of accepted risks. Accepted risks do not waive promised outcomes; expose any conflict under §13.

## 11. Prior art before building
Before building a component, check for existing libraries, packages and projects that cover it, and record the finding: adopt, wrap, borrow ideas, or build, with the reason. Prefer adopting or wrapping when a candidate meets the contract and its footprint fits; borrow ideas when it doesn't. Avoid platforms that would replace the runtime, the memory or the composition.

When uncertainty could determine the architecture, resolve it with the smallest experiment through the real interface before building dependent machinery. Use existing evidence when sufficient; avoid speculative scaffolding.

## 12. Verified from the outside
Prove a change by driving the same path a user or caller drives, to a completed outcome: black-box journeys for apps (actually opening and using them), contract-level tests for components. Where that path calls an external system, use a dummy or scripted backend unless the test is about that system. Fail closed if the outer path cannot finish. Seam tests may exist; they don't substitute for this proof.

A blocking review finding must identify the violated obligation, supporting evidence and concrete consequence. Reviewers should also identify machinery that can be removed while preserving required behavior. Preferences and hypothetical improvements remain suggestions. Revalidate affected work and preserve applicable evidence.

## 13. Resolve intent tensions and preserve design freedom
Read intent as a whole before choosing mechanisms. Identify requirements that pull against each other, especially simplicity, autonomy, reliability and delivery time. Explain the concrete trade-off and recommend a resolution before building around an assumption. Clarify the operating conditions behind words such as "always", "never" and "automatic". Owner decisions are needed for changes to promised outcomes or unresolved product trade-offs.

Record which guarantees the owner required and which mechanisms the agents selected. Agent-selected mechanisms remain open to simplification. Review acceptance does not turn them into owner requirements. Changing a promised outcome still requires owner agreement.

## Enforcement (strongest first)
1. Build gates: dependency-boundary and layer checks (§2, §3, §7) and conformance suites (§6).
2. Scaffold: new components start with the core/adapter split, the clean-architecture layers, a contract file, an empty conformance suite and the boundary checks configured.
3. Design review: designs are reviewed against this document before code is written (§1, §3, §4, §8, §9, §10, §11, §12, §13).
4. Agent instructions: AGENTS.md files should reference this document. Instructions alone do not enforce compliance.

Use the proportionality and trade-off checks in the existing design and review steps. Do not add separate gates for each check.
