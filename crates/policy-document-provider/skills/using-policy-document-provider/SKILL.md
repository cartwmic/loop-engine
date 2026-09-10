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

**Shared controls:** follow the engine companion's **Choose an observation** and **Capture external commands** for passive `monitor`, optional summaries and capture/abort/resume. Monitor is JSONL; capture streams raw output. Neither supplies semantic verdicts or permission to advance.

**Envelopes:** `completed`, `rejected`, `error`, and `invalid-invocation`. Parse JSON even on nonzero exit. Treat only `completed` as success.

**Bound versus unbound:** a catalog slot ID present in frozen `work_slot_bindings` is bound — `invoke` it; do not perform the stored work body. An absent key is unbound — perform the stored instructions yourself, then append and request the event.

**Overlay meaning:** overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. You still triage worker output, append provider-shaped records, and request the shown event.

**Observation before mutation:** action `show` (default) gives instructions and arms the visit for `append`, `event`, `invoke`, and `terminate`; repeat after every transition. Use `show --view full` for frozen input, complete context, invocation/change reports and commission input. Status/compact, monitor, list, history and invocation-progress never arm. Wait through passive monitor; inspect retained outputs separately.

**Lock-in-before-start:** do not call `start` until the user confirms (1) bind or not (which slot IDs), (2) exact `{command, args}` per bound slot, and (3) model identity in those frozen args (nested `--worker` / `--task-worker` count) or explicit unpinned-default acceptance. Initial bindings freeze; `amend-binding` corrects future execution only. Before invoke controls, cancellation or override, read the engine companion's **Execution recovery minimum** and its referenced contract. Overrun/cleanup-pending work blocks retry and departure; override never supplies document conformance. Software-change-specific records/batching are unsupported. Apply simple-first/YAGNI/KISS.

Run `loop-engine preview-bindings` on the JSON you will freeze before `start`. `describe` and `evaluate` remain deterministic and do not invoke a model. Provider contract: `crates/policy-document-provider/README.md`. Target constraints: `crates/policy-document-provider/data/target-guidance.md`. Semantic evidence contract: `crates/policy-document-provider/data/reviewer-protocol.md`.

## Setup

Before start, load repository `skills/using-loop-engine/SKILL.md`, **Deterministic setup**, and follow its catalog/override inspection procedure. Use the normal user catalog; only an explicit human isolation request in this session authorizes database or artifact overrides. Other runs and old preferences do not authorize isolation.

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

Choose `crates/policy-document-provider/data/readme.json` or `crates/policy-document-provider/data/agents.json`. Copy it to a run-specific file; set `mode` to `draft` or `audit`; replace `target.path` with the absolute target path. Keep shipped `target.id` and `profile_version` unchanged unless intentionally authoring a custom profile. Reserved `artifact_root` is accepted and ignored, and the provider is not required to write artifact files. Other unknown `initial_input` keys still fail.

Shipped profiles omit `work_slot_bindings` (or `{}`). Cataloged slots are `deterministic-review` and `semantic-review`; both stay driver-performed until the caller adds a map. Bound workers are opt-in.

Review workers return judgments only. The driver owns deterministic checks, `show`, `append`, `event`, and progression. A worker process exiting 0 is not enough: contracted output must conform mechanically, and the driver must still verify its values and semantic fitness before appending evidence.

For a driver-performed run, omit `work_slot_bindings` or set `"work_slot_bindings": {}`. To bind review, use the constructor below instead of hand-writing one generic worker. It accepts only `SLOT_ID=semantic-review`, reads `.semantic_policies` from the same per-run `PROFILE` that will be started, and emits workers in policy order and then required-author roster order. Leave `semantic_policies[].required_authors` absent: this provider rejects the field even though the generic constructor can normalize it. Shipped policies require one current pass per axis. The ordered `ROSTER` file is a JSON array of exact `{author,model}` objects; author labels must be pairwise distinct. Every generated worker freezes the exact provider preamble and output schema from `data-dump`, exact profile `mode` and complete `target`, policy `id` and `example_prompt`, claimed author, model argv, provider, and slot.

The constructor fails before preview or start on unsupported/empty/malformed input, atomically rewrites the same `PROFILE`, hashes and displays its exact resulting bytes and extracted bindings, previews those bindings, and asks the caller to confirm by typing that hash. It rechecks the hash immediately before starting that unchanged file. There is no post-preview merge. Set every path variable to an absolute caller-local value; do not put machine-local paths in this skill.

```bash
set -eu

: "${PROFILE:?absolute per-run profile JSON path}"
: "${ROSTER:?JSON file containing an ordered array of {author,model}}"
: "${SLOT_ID:?must be semantic-review}"
: "${LOOP_ENGINE:?absolute loop-engine command path}"
: "${POLICY_DOCUMENT:?absolute policy-document command path}"
: "${PI:?absolute pi command path}"
: "${CURSOR_EXTENSION_PATH:?absolute cursor-provider extension path}"
: "${CLAUDE_BRIDGE_EXTENSION_PATH:?absolute claude-bridge extension path}"
: "${PROVIDER_CONFIG:?absolute provider TOML path}"
: "${RUN_LABEL:?run label}"

case "$PROFILE" in /*) ;; *) echo "PROFILE must be absolute" >&2; exit 1;; esac
case "$ROSTER" in /*) ;; *) echo "ROSTER must be absolute" >&2; exit 1;; esac
[ "$SLOT_ID" = semantic-review ] || { echo "unsupported slot: $SLOT_ID" >&2; exit 1; }

DATA_ROOT=$(mktemp -d)
PROFILE_DIR=$(dirname "$PROFILE")
TMP_PROFILE=$(mktemp "$PROFILE_DIR/.policy-document-profile.XXXXXX")
BINDINGS_FILE=$(mktemp)
cleanup() {
  rm -rf "$DATA_ROOT"
  rm -f "${TMP_PROFILE:-}" "$BINDINGS_FILE"
}
trap cleanup EXIT HUP INT TERM

"$POLICY_DOCUMENT" data-dump "$DATA_ROOT"
PREAMBLE_FILE="$DATA_ROOT/crates/policy-document-provider/data/semantic-review-worker-preamble.md"
SCHEMA_FILE="$DATA_ROOT/crates/policy-document-provider/data/semantic-review-worker-output-schema.json"

JQ_FILTER=$(cat <<'JQ'
def nonblank: type == "string" and test("\\S");
def require($condition; $message): if $condition then . else error($message) end;
def worker($profile; $policy; $reviewer; $schema):
  {
    command: $pi,
    args: [
      "--print", "--no-skills", "--no-extensions",
      "-e", $cursor_extension,
      "-e", $claude_bridge_extension,
      "--tools", "read,grep,find,ls",
      "--model", $reviewer.model
    ],
    preamble: (
      $base_preamble
      + (if ($base_preamble | endswith("\n")) then "" else "\n" end)
      + "\nFrozen assignment (authoritative):\n"
      + ({
          provider: "policy-document",
          slot: $slot,
          axis: $policy.id,
          example_prompt: $policy.example_prompt,
          author: $reviewer.author,
          mode: $profile.mode,
          profile_version: $profile.profile_version,
          target: $profile.target
        } | tojson)
    ),
    output_schema: $schema
  };

. as $profile
| require($slot == "semantic-review"; "unsupported slot")
| require(($roster_documents | length) == 1; "ROSTER must contain one JSON value")
| require(($schema_documents | length) == 1; "output schema must contain one JSON value")
| ($roster_documents[0]) as $roster
| ($schema_documents[0]) as $schema
| require(($profile | type) == "object"; "PROFILE must be a JSON object")
| require($profile.mode == "draft" or $profile.mode == "audit"; "PROFILE mode must be draft or audit")
| require(
    ($profile.target | type) == "object"
    and (($profile.target | keys_unsorted | sort) == ["id", "path"])
    and ($profile.target.id | nonblank)
    and ($profile.target.path | nonblank)
    and ($profile.target.path | startswith("/"));
    "PROFILE target must be the complete {id,path} object with an absolute path"
  )
| require(($profile.semantic_policies | type) == "array" and ($profile.semantic_policies | length) > 0; "semantic_policies must be non-empty")
| require(all($profile.semantic_policies[]; (.id | nonblank) and (.example_prompt | nonblank)); "every semantic policy needs a non-empty id and example_prompt")
| require(([$profile.semantic_policies[].id] | unique | length) == ($profile.semantic_policies | length); "semantic policy ids must be unique")
| require(
    all($profile.semantic_policies[];
      .required_authors == null
      or ((.required_authors | type) == "number"
          and (.required_authors | floor) == .required_authors
          and .required_authors >= 1));
    "required_authors must be a positive integer when present"
  )
| require(($roster | type) == "array" and ($roster | length) > 0; "ROSTER must be a non-empty array")
| require(
    all($roster[];
      type == "object"
      and ((keys_unsorted | sort) == ["author", "model"])
      and (.author | nonblank)
      and (.model | nonblank));
    "every ROSTER entry must contain exactly non-empty author and model strings"
  )
| require(([$roster[].author] | unique | length) == ($roster | length); "ROSTER author labels must be pairwise distinct")
| require(
    ([ $profile.semantic_policies[] | (.required_authors // 1) ] | max) <= ($roster | length);
    "ROSTER does not contain enough authors"
  )
| require($base_preamble | nonblank; "provider preamble must be non-empty")
| require($schema == {"required": ["axis", "author", "result", "findings"]}; "provider output schema is invalid")
| require($profile.work_slot_bindings == null or ($profile.work_slot_bindings | type) == "object"; "work_slot_bindings must be an object when present")
| ([
    $profile.semantic_policies[] as $policy
    | range(0; ($policy.required_authors // 1)) as $roster_index
    | ["--worker", (worker($profile; $policy; $roster[$roster_index]; $schema) | tojson)]
  ] | add) as $worker_args
| .work_slot_bindings = (
    ($profile.work_slot_bindings // {})
    + {($slot): ({command: $loop_engine, args: (["fan-out"] + $worker_args)}
        + (if $profile.work_slot_bindings[$slot].context_filter != null
           then {context_filter: $profile.work_slot_bindings[$slot].context_filter}
           else {} end))}
  )
JQ
)

jq \
  --arg slot "$SLOT_ID" \
  --arg loop_engine "$LOOP_ENGINE" \
  --arg pi "$PI" \
  --arg cursor_extension "$CURSOR_EXTENSION_PATH" \
  --arg claude_bridge_extension "$CLAUDE_BRIDGE_EXTENSION_PATH" \
  --rawfile base_preamble "$PREAMBLE_FILE" \
  --slurpfile schema_documents "$SCHEMA_FILE" \
  --slurpfile roster_documents "$ROSTER" \
  "$JQ_FILTER" "$PROFILE" >"$TMP_PROFILE"
mv -f "$TMP_PROFILE" "$PROFILE"
TMP_PROFILE=

profile_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
PROFILE_SHA256=$(profile_sha256 "$PROFILE")
jq -e '.work_slot_bindings as $bindings | if ($bindings | type) == "object" then $bindings else error("missing work_slot_bindings") end' \
  "$PROFILE" >"$BINDINGS_FILE"

printf '\nPROFILE=%s\nPROFILE_SHA256=%s\nExact resulting profile bytes follow:\n' "$PROFILE" "$PROFILE_SHA256"
cat "$PROFILE"
printf '\nExtracted work_slot_bindings (%s):\n' "$BINDINGS_FILE"
cat "$BINDINGS_FILE"
printf '\npreview-bindings output:\n'
"$LOOP_ENGINE" preview-bindings "@$BINDINGS_FILE"

printf '\nConfirm this exact profile, bindings, and model roster by typing %s: ' "$PROFILE_SHA256" >&2
IFS= read -r CONFIRMED_PROFILE_SHA256
[ "$CONFIRMED_PROFILE_SHA256" = "$PROFILE_SHA256" ] || { echo "confirmation hash did not match" >&2; exit 1; }

CURRENT_PROFILE_SHA256=$(profile_sha256 "$PROFILE")
[ "$CURRENT_PROFILE_SHA256" = "$PROFILE_SHA256" ] || {
  echo "PROFILE changed after preview; refusing start" >&2
  exit 1
}
"$LOOP_ENGINE" --json --config "$PROVIDER_CONFIG" \
  start policy-document "@$PROFILE" "$RUN_LABEL"
```

This constructor intentionally does not bind `deterministic-review`; deterministic checking remains a driver duty. The generated reviewer preamble makes the frozen assignment authoritative and treats the later state instruction body as driver context only. The mechanically forwarded `artifact_root` context is irrelevant to policy-document review because the assignment already freezes the complete external target object.

Follow the required canonical setup above for normal-catalog start and owner-only isolation.

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start policy-document @"$PROFILE" "document audit"
```

Reuse the returned run ID for every later operation.

## Profile map

| Profile | Deterministic floors | Semantic axes |
|---|---|---|
| `readme-2` | non-empty document; H1; purpose, onboarding, usage, and validation sections; commands in onboarding and validation; resolving local references | product-fidelity, onboarding-sufficiency, audience-navigation, clarity-scope, honest-fitness, verifiable-claims, troubleshooting-sharp-edges |
| `agents-2` | non-empty document; scope/authority, workflow/validation, and completion/handoff sections; workflow command; resolving local references | success-path-completeness, operational-precision, authority-resolution, risk-boundary-sufficiency, completion-handoff, non-discoverable-sharp-edges, ambiguity-resolution, signal-density, living-config |

Heading aliases are case-insensitive; profiles do not require exact heading spelling. Commands must be non-comment content inside a fenced block within the matching section. Keep local references relative to target parent; web, mail, data, fragment-only, and protocol-relative links are ignored by local resolution.

## Run loop

1. Read action `show` for instructions, events and work locators. Use full for frozen mode, target, profile version/policies, context and invocation/change reports. Repeat actionable observation after every transition before mutation.
2. In `prepare`, author or revise target externally. Request `ready` to enter deterministic review.
3. In `deterministic-review`, if that slot is bound, `invoke` it, monitor until completion or attention, then inspect action/full show and captures; on `overrun`, wait or cancel owned work and verify cleanup, then observe before retry; on failure, inspect `capture_dir/summary.json` and captured stdout before stderr. Overlay `succeeded` is worker exit 0, not provider acceptance. Request `passed` only after overlay `succeeded`, or immediately if unbound. On `policy-document-nonconforming`, fix every reported violation, request check-free `revise`, then repeat from `prepare`.
4. After deterministic approval, compute lowercase SHA-256 over exact current bytes:

   ```sh
   TARGET_SHA256=$(shasum -a 256 "$TARGET" | awk '{print $1}')
   ```

5. For semantic review: if `semantic-review` is bound, `invoke` it, monitor until completion or attention, then inspect action/full show and captures; on `overrun`, wait or cancel owned work and verify cleanup, then observe before retry; on failure, inspect `capture_dir/summary.json` and captured stdout before stderr; then read worker output. If unbound, commission external review yourself. Overlay `succeeded` is collector/worker exit 0, not that the review passed. Either way, cover every frozen semantic policy. Give each reviewer current target bytes or path, policy `description`, `example_prompt`, and relevant project evidence. Reviewer judges one axis and returns `pass` or actionable `fail` findings. You still triage and append; a bound worker does not write records.
6. Append one `review-evidence` record per axis judgment, bound to exact target ID, digest, and profile version.
7. Request semantic `passed`. On denial, use diagnostics to supersede malformed evidence with a later conforming record, address standing failures, or supply missing current passes. Any target byte change invalidates prior evidence: request `revise`, rerun deterministic review, recompute digest, and commission fresh semantic verdicts.

## Explicit historical review context

No selection means no prior-findings attachment. Existing frozen catalogs and
already launched packets are unchanged. Historical context never supplies a
current pass or rewrites a verdict. Selecting is a driver decision, not relevance
inference by core.

For a **new** bound run that needs selection, set this filter under the per-run
profile's `work_slot_bindings.semantic-review` **before** running the constructor.
It preserves the supplied filter while constructing worker arguments, then displays,
hashes, previews and confirms the final profile. Freeze the profile JSON
(without `work_slot_bindings`) as the
literal second argument, not a mutable file path:

```json
{"context_filter":{"command":"/absolute/path/policy-document","args":["commission","<literal frozen profile JSON>"]}}
```

Do not splice this into a profile after confirmation. Unfiltered bindings retain
the old no-attachment catalog. For unbound review the new catalog advertises the
same eligible kinds; run the same `policy-document commission "$FROZEN_PROFILE_JSON"`
with a completed `loop-engine --json show --view full "$RUN_ID"` envelope on stdin.
The result contains selected original `context`, `current_target` and diagnostics.
Hand that packet to reviewers alongside current target bytes and frozen policies.
For bound review the existing engine filter transports those same original records;
the active selection's `receipt` delivers current-target identity and diagnostics
in bound stdin too. Successful filter stderr is not delivery. Legacy receipt-less
selections remain supported with unknown receipt freshness, not claimed current identity.
Reviewer framing must label every attached source historical, compare
its target/profile/digest to current bytes, and never treat attachment as proof.

After full `show`, prepare proposed `review-context-selection` data:

```json
{"target_id":"README.md","slot_id":"semantic-review","record_ids":["original-finding-id"],"supersedes":"previous-selection-id"}
```

Add this proposed data as top-level `selection_data` to the completed full-show
envelope, then pipe it to `policy-document commission "$FROZEN_PROFILE_JSON"`.
Save stdout as `selection.json`: it is appendable data with a provider-calculated
`receipt` (`current_target` and per-source `diagnostics`), not an engine record.
Append it through ordinary `append --record-id ... "$RUN_ID" review-context-selection
@selection.json`, observe again, then invoke. This receipt-bearing path is required
for current identity/diagnostic delivery. Never invent record IDs/timestamps inside
the data or hand-author the receipt. Both commission forms recheck a supplied receipt
against current bytes/profile and sources and refuse mismatches before launch.
After a target change, prepare a new proposal without the old receipt and append it
with a new record ID, superseding the previous selection; prior packets stay immutable.

Omit `supersedes` for the first selection. The latest sequence for the exact target
and slot controls future commissions; `record_ids: []` explicitly clears the
attachment. Supersession references must name an earlier selection for that same
target/slot. Selected IDs must name original earlier `review-evidence`; unknown,
duplicate, non-evidence or non-prior references refuse preparation. Unselected
material is omitted. Originals, including the active selection, are transported
in original context order without rewriting. Stale originals are explicitly
historical context, not current proof; diagnostics retain supersession identities.
A target edit still requires the normal deterministic recheck and fresh digest-bound
verdicts. Old selection records and launched packets are never changed.

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
