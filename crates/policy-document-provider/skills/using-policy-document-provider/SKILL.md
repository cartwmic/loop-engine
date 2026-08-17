---
name: using-policy-document-provider
description: Use when drafting or auditing README.md, AGENTS.md, or another policy document through Loop Engine with the policy-document provider — confirming work-slot bindings with the user before start, selecting a profile, clearing deterministic checks, commissioning semantic review or invoking a bound review slot, appending digest-bound evidence, and finalizing the run.
---

# Using the policy-document provider

## Overview

`policy-document` is Loop Engine's deterministic document provider. It never edits the target, invokes a reviewer, or judges semantic quality. It reads exact UTF-8 target bytes, applies run-frozen deterministic policies, computes their SHA-256 digest, and aggregates externally supplied semantic verdicts bound to that digest.

Workflow: `prepare → deterministic-review → semantic-review → end`. `ready` and both `revise` events are check-free. Both `passed` events are checked; final semantic approval reruns deterministic checks against current bytes before evaluating evidence. `mode` is frozen as `draft` or `audit`, but both modes use the same topology and checks.

## Required companion and engine driving minimum

`using-loop-engine` (`skills/using-loop-engine/SKILL.md`) is a **required companion**. This skill does not replace it. The closed driving minimum below is what you cannot skip when this skill is loaded alone; load the companion for full engine semantics.

**Run-state commands:** `start`, `list`, `show`, `append`, `event`, `history`, `terminate`, and `invoke`.

**Non-run-state commands:** `fan-out` and `preview-bindings`. They do not start, advance, or record a run.

**Envelopes:** `completed`, `rejected`, `error`, and `invalid-invocation`. Parse JSON even on nonzero exit. Treat only `completed` as success.

**Bound versus unbound:** a catalog slot ID present in frozen `work_slot_bindings` is bound — `invoke` it; do not perform the stored work body. An absent key is unbound — perform the stored instructions yourself, then append and request the event.

**Overlay meaning:** overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. You still triage worker output, append provider-shaped records, and request the shown event.

**Lock-in-before-start:** do not call `start` until the user confirms (1) bind or not (which slot IDs), (2) exact `{command, args}` per bound slot, and (3) model identity in those frozen args (nested `--worker` / `--task-worker` count) or explicit unpinned-default acceptance. Bindings freeze and cannot be patched.

Run `loop-engine preview-bindings` on the JSON you will freeze before `start`. `describe` and `evaluate` remain deterministic and do not invoke a model. Provider contract: [README](../../README.md). Target constraints: [target guidance](../../data/target-guidance.md). Semantic evidence contract: [reviewer protocol](../../data/reviewer-protocol.md).

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

Choose [readme.json](../../data/readme.json) or [agents.json](../../data/agents.json). Copy it to a run-specific file; set `mode` to `draft` or `audit`; replace `target.path` with the absolute target path. Keep shipped `target.id` and `profile_version` unchanged unless intentionally authoring a custom profile. Omit `artifact_root` in the usual case; the reserved key is accepted and ignored, and the provider is not required to write artifact files. Other unknown `initial_input` keys still fail.

Shipped profiles omit `work_slot_bindings` (or `{}`). Cataloged slots are `deterministic-review` and `semantic-review`; both stay driver-performed until the caller adds a map. Bound workers are opt-in.

Copy-paste templates below into the **per-run** profile JSON after replacing `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL`. Do not put machine-local paths in the skill file. Keep `--no-skills --no-extensions` and add explicit `-e` so cursor-provider and claude-bridge load. Review `pi` examples include `--tools read,grep,find,ls` and must not pass `--no-context-files`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning.

Do not add bindings, and do not start, until the user confirms: (1) driver-performed (omit the key or `{}`) vs which slots to bind; (2) exact `{command, args}` if bound, after filling placeholders; (3) which models those CLIs will use, encoded in frozen args. Nested inner workers count. Do not call `start` while a bound model-bearing CLI has no model in argv unless the user has explicitly accepted that CLI's unpinned default. Bindings freeze and cannot be patched. Run `loop-engine preview-bindings` on any `work_slot_bindings` JSON you will freeze.

Opt-in review binding (same pattern for `deterministic-review`; every model-bearing worker names a model):

```json
"semantic-review": {
  "command": "loop-engine",
  "args": [
    "fan-out",
    "--worker", "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--tools\",\"read,grep,find,ls\",\"--model\",\"MODEL\"]}"
  ]
}
```

Driver-performed run: omit `work_slot_bindings` or set `"work_slot_bindings": {}`.

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start policy-document @"$PROFILE" "document audit"
```

Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`. Reuse the returned run ID for every later operation.

## Profile map

| Profile | Deterministic floors | Semantic axes |
|---|---|---|
| `readme-2` | non-empty document; H1; purpose, onboarding, usage, and validation sections; commands in onboarding and validation; resolving local references | product-fidelity, onboarding-sufficiency, audience-navigation, clarity-scope, honest-fitness, verifiable-claims, troubleshooting-sharp-edges |
| `agents-2` | non-empty document; scope/authority, workflow/validation, and completion/handoff sections; workflow command; resolving local references | success-path-completeness, operational-precision, authority-resolution, risk-boundary-sufficiency, completion-handoff, non-discoverable-sharp-edges, ambiguity-resolution, signal-density, living-config |

Heading aliases are case-insensitive; profiles do not require exact heading spelling. Commands must be non-comment content inside a fenced block within the matching section. Keep local references relative to target parent; web, mail, data, fragment-only, and protocol-relative links are ignored by local resolution.

## Run loop

1. `show` and read current instructions plus immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, and `work_slot_invocations` (`overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, `inner_workers`), especially `mode`, target, profile version, deterministic policies, and semantic policies.
2. In `prepare`, author or revise target externally. Request `ready` to enter deterministic review.
3. In `deterministic-review`, if that slot is bound, `invoke` it and poll overlay until `succeeded` / `failed` / `overrun`; on `overrun` invoke again; overlay `succeeded` is worker exit 0, not provider acceptance. Request `passed` only after overlay `succeeded`, or immediately if unbound. On `policy-document-nonconforming`, fix every reported violation, request check-free `revise`, then repeat from `prepare`.
4. After deterministic approval, compute lowercase SHA-256 over exact current bytes:

   ```sh
   TARGET_SHA256=$(shasum -a 256 "$TARGET" | awk '{print $1}')
   ```

5. For semantic review: if `semantic-review` is bound, `invoke` it, poll overlay until `succeeded` / `failed` / `overrun`, on `overrun` invoke again, and read worker output; if unbound, commission external review yourself. Overlay `succeeded` is collector/worker exit 0, not that the review passed. Either way, cover every frozen semantic policy. Give each reviewer current target bytes or path, policy `description`, `example_prompt`, and relevant project evidence. Reviewer judges one axis and returns `pass` or actionable `fail` findings. You still triage and append; a bound worker does not write records.
6. Append one `review-evidence` record per axis judgment, bound to exact target ID, digest, and profile version.
7. Request semantic `passed`. On denial, use diagnostics to supersede malformed evidence with a later conforming record, address standing failures, or supply missing current passes. Any target byte change invalidates prior evidence: request `revise`, rerun deterministic review, recompute digest, and commission fresh semantic verdicts.

## Evidence record

Append `data` as the eight-field object below; Loop Engine supplies context record `kind` separately:

```sh
loop-engine --json append \
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
  "profile_version": "readme-2"
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
