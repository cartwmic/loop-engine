# Design principles (fictional calibration example)

Example owner principles for calibration and journeys only. They are not any real owner's rules and are never shipped as policy.

## 1. Single-purpose components
Build small components that each do one job. Anything specific to one product lives in the composition, not in a component. Test: name a second real use for the component. If there is none, the boundary is drawn around a feature.

## 2. YAGNI and KISS
Do not add code, abstractions, configuration or files the current need does not require, and do not design for requests that have not been made. Prefer the smallest thing that works: a function beats a new layer. Add machinery only when a concrete requirement or a demonstrated failure makes the simpler baseline insufficient, and say why the simpler fix would fail. A requirement citation alone does not justify a particular mechanism. Count runtime code, configuration, recovery paths and test harnesses.

## 3. Fix the cause; no unrequested defense in depth
Do not add caps, rate limits or thresholds to cover for behavior that is wrong; fix the behavior. Do not add layered safeguards unless asked. A limit that reflects a real external constraint is part of the contract and is fine.

## 4. Verified from the outside
Prove a change by driving the same path a user or caller drives, to a completed outcome. Where that path calls an external system, use a dummy or scripted backend. Fail closed if the outer path cannot finish. Tests at an internal seam may exist; they do not substitute for this proof.

## 5. Resolve intent tensions
Read the intent as a whole before choosing mechanisms. Name requirements that pull against each other, such as simplicity and reliability, and the concrete trade-off. Clarify the operating conditions behind words such as "always" and "never". Record which guarantees the owner required and which mechanisms the agents selected; agent-selected mechanisms stay open to simplification.
