---
name: using-policy-document-provider
description: Use when drafting or auditing README.md, AGENTS.md, or another policy document through Loop Engine with the policy-document provider — selecting a profile, clearing deterministic checks, commissioning semantic review, appending digest-bound evidence, and finalizing the run.
---

# Using the policy-document provider

## Overview

`policy-document` is Loop Engine's deterministic document provider. It never edits the target, invokes a reviewer, or judges semantic quality. It reads exact UTF-8 target bytes, applies run-frozen deterministic policies, computes their SHA-256 digest, and aggregates externally supplied semantic verdicts bound to that digest.

Workflow: `prepare → deterministic-review → semantic-review → end`. `ready` and both `revise` events are check-free. Both `passed` events are checked; final semantic approval reruns deterministic checks against current bytes before evaluating evidence. `mode` is frozen as `draft` or `audit`, but both modes use the same topology and checks.

Drive engine commands and outcome handling with the [using-loop-engine skill](../../../../skills/using-loop-engine/SKILL.md). Provider contract: [README](../../README.md). Target constraints: [target guidance](../../data/target-guidance.md). Semantic evidence contract: [reviewer protocol](../../data/reviewer-protocol.md).

## Setup

```sh
cargo build -p loop-cli -p policy-document-provider
```

Register `target/debug/policy-document` under exact alias `policy-document` using an absolute command path in uncommitted machine-local provider TOML:

```toml
[providers.policy-document]
command = "/absolute/path/to/target/debug/policy-document"
args = []
```

For an installed provider, materialize embedded profiles and guidance into an empty destination:

```sh
policy-document data-dump "$DATA_ROOT"
```

Choose [readme.json](../../data/readme.json) or [agents.json](../../data/agents.json). Copy it to a run-specific file; set `mode` to `draft` or `audit`; replace `target.path` with the absolute target path. Keep shipped `target.id` and `profile_version` unchanged unless intentionally authoring a custom profile.

```sh
loop-engine --database "$DB" --config "$PROVIDER_CONFIG" --json \
  start policy-document @"$PROFILE" "document audit"
```

Reuse that database and returned run ID for every operation.

## Profile map

| Profile | Deterministic floors | Semantic axes |
|---|---|---|
| `readme-1` | non-empty document; H1; purpose, onboarding, usage, and validation sections; commands in onboarding and validation; resolving local references | product fidelity, onboarding sufficiency, audience navigation, clarity/scope |
| `agents-1` | non-empty document; scope/authority, workflow/validation, and completion/handoff sections; workflow command; resolving local references | success-path completeness, operational precision, authority resolution, risk boundaries, completion/handoff |

Heading aliases are case-insensitive; profiles do not require exact heading spelling. Commands must be non-comment content inside a fenced block within the matching section. Keep local references relative to target parent; web, mail, data, fragment-only, and protocol-relative links are ignored by local resolution.

## Run loop

1. `show` and read current instructions plus immutable `initial_input`, especially `mode`, target, profile version, deterministic policies, and semantic policies.
2. In `prepare`, author or revise target externally. Request `ready` to enter deterministic review.
3. In `deterministic-review`, request `passed`. On `policy-document-nonconforming`, fix every reported violation, request check-free `revise`, then repeat from `prepare`.
4. After deterministic approval, compute lowercase SHA-256 over exact current bytes:

   ```sh
   TARGET_SHA256=$(shasum -a 256 "$TARGET" | awk '{print $1}')
   ```

5. Commission external review for every frozen semantic policy. Give each reviewer current target bytes or path, policy `description`, `example_prompt`, and relevant project evidence. Reviewer judges one axis and returns `pass` or actionable `fail` findings.
6. Append one `review-evidence` record per axis judgment, bound to exact target ID, digest, and profile version.
7. Request semantic `passed`. On denial, use diagnostics to supersede malformed evidence with a later conforming record, address standing failures, or supply missing current passes. Any target byte change invalidates prior evidence: request `revise`, rerun deterministic review, recompute digest, and commission fresh semantic verdicts.

## Evidence record

Append `data` as the eight-field object below; Loop Engine supplies context record `kind` separately:

```sh
loop-engine --database "$DB" --json append \
  --record-id "$RECORD_ID" "$RUN_ID" review-evidence @verdict.json
```

```json
{
  "gate": "semantic-review",
  "policy_id": "product-fidelity",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "target_id": "README.md",
  "target_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "profile_version": "readme-1"
}
```

All fields are required; unknown fields are rejected. `result` is exactly `pass` or `fail`; `author.kind` is `human`, `agent`, or `script`; a fail needs non-empty findings; digest is exactly 64 lowercase hexadecimal characters.

## Evidence rules

- Every semantic axis needs at least one current pass and no current standing fail.
- Latest conforming current verdict per exact `(policy_id, author.name, author.kind)` stands. A later pass clears only that same author's fail; another author's fail remains standing.
- Wrong profile version, target ID, or digest is stale and never satisfies an axis.
- Attributable malformed evidence blocks its axis until a later shape-conforming record for that axis supersedes the malformed record. A stale conforming record can clear the malformed diagnostic but cannot provide the required pass.
- Unattributable `review-evidence` is inert. Non-`review-evidence` context is ignored by semantic aggregation.
- Reviewer identity and verdict are caller claims, not signatures. Provider does not prove independence or provenance.
- Provider reads one snapshot per evaluation but cannot lock target bytes through engine transition commit. Serialize `append` and `event`; avoid editing target during evaluation.

## Validation

Run source journeys against shipped profile bytes after provider or skill-contract changes:

```sh
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

Synthetic pass evidence proves deterministic mechanics, evidence shape/freshness, routing, and persistence only. It does not prove semantic verdict quality.
