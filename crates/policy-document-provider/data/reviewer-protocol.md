# Policy-document reviewer protocol

Semantic judgment and reviewer execution stay external. Reviewers return judgments only; drivers run checks, observe, triage, append, and progress. The driver computes the current digest and appends one ordinary `review-evidence` per accepted policy judgment:

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
    "profile_version": "readme-4"
  }
}
```

`result` is `pass` or `fail`; failure requires non-empty findings. Author kind is `human`, `agent`, or `script`. Provider validates shape, current policy/target/profile/digest, and latest verdict per exact reviewer. The effective distinct-author floor and no standing fail are required for every axis. Target-author evidence is excluded by default; explicit frozen-profile permission lets eligible target authors count once as labeled self-review, not independent or cold review. Required counts change only through authorized durable author-count amendment. Explicit applicability may reuse prior judgments, but deterministic checks always rerun. Values are caller claims, not signatures or provenance; provider never invokes a reviewer, model, or editor.
