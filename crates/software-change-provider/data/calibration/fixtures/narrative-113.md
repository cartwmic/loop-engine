# Proposed provider design

The operator needs to see a run's fixed review rules and cannot advance a checked step without a structurally valid artifact and the required independent reviews (intent.json#/outcome, #/acceptance).

The provider is a stateless local program: the engine invokes it once per request and owns workflow progress. A caller passes the complete selected profile as initial_input, meaning the run's starting configuration. The engine preserves it unchanged; show exposes it and evaluate receives it. describe returns only the existing workflow map, not the selected rules (design.json#/approach, #/decisions/4).

Five components divide the work: a strict request/response boundary, a generic structural checker, an external-evidence evaluator, versioned reference data and a checked-transition evaluator. The checker reports all applicable structural violations in stable path/rule order, before evidence can satisfy a gate. Its deliberately limited schema language supports objects, arrays and strings, with only the listed keywords; extra object fields and unsupported schema keywords are refused. Revision links are separate configuration, not custom schema keywords (design.json#/elements, #/decisions/0, #/decisions/1).

Reviews are claims supplied by external actors, not judgments made by this program. A claim counts only for the exact gate, axis (named review obligation), subject, current revision and configuration version. Identity is the declared name/kind pair; the subject's author is excluded. The latest conforming verdict from each author stands, repeated claims do not add authors, and a standing failure or missing independent author blocks progress. Malformed current-axis records block until superseded by a conforming record for that gate/axis. The existing final step requires every configured validation obligation (design.json#/decisions/2, #/decisions/5, #/coverage).

The solution keeps the existing workflow and one-request process protocol. It does not authenticate identities or judge semantic truth. Material edits require a new revision and renewed review by the external actor; dishonest same-revision edits remain undetectable. Startup validation rejects unsupported schemas and malformed links. These are limits, not promises of hashing, locking, enterprise security, or a new workflow (design.json#/decisions/3, #/risks; intent.json#/constraints, #/non_goals).

The product will authenticate every reviewer cryptographically and provide enterprise authorization for mutually distrusting tenants as part of this design.
