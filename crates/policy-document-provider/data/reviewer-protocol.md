# Policy-document reviewer protocol

Semantic judgment stays external. A bound review worker is read-only and returns only a judgment object with top-level `axis`, `author`, `result`, and `findings`; it does not run deterministic checks, call `show`, `append`, or `event`, or progress the workflow. The worker follows the frozen assignment immediately before the separator and treats later state text as driver context only. The driver verifies the returned values against that assignment, computes the current target digest, and appends one ordinary `review-evidence` context record per configured semantic policy:

```json
{
  "kind": "review-evidence",
  "data": {
    "gate": "semantic-review",
    "policy_id": "product-fidelity",
    "result": "pass",
    "findings": "",
    "author": {"name": "reviewer", "kind": "agent"},
    "target_id": "README.md",
    "target_sha256": "<64 lowercase hexadecimal SHA-256 of exact target bytes>",
    "profile_version": "readme-2"
  }
}
```

`result` is `pass` or `fail`; failure requires non-empty findings. Author kind is `human`, `agent`, or `script`. Provider validates shape, current policy/target/profile/digest, and latest verdict per exact reviewer. One pass and no standing fail are required for every axis. Any target byte change requires fresh evidence. Values are caller claims, not signatures or provenance; provider never invokes a reviewer, model, or editor. Mechanical worker-output conformance establishes only that the required top-level keys exist; exit 0 and conformance do not establish semantic validity or reviewer independence.
