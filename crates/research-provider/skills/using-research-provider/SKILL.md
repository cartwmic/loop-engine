---
name: using-research-provider
description: Use when running the research workflow through Loop Engine with the research provider — confirming work-slot bindings and models with the user before start, scoping a question, gathering sources externally, commissioning adversarial review or invoking bound slots, synthesizing a cited conclusion, appending review-evidence, and clearing checked transitions.
---

# Using the research provider

## Overview

`research` is Loop Engine's research reference provider. Search, fetch, and writing happen outside the provider. The binary never retrieves URLs, invokes a model, or judges claim truth. It validates artifact schemas and revision links, then aggregates externally supplied review-evidence at verify and synthesize. `describe` and `evaluate` remain deterministic and do not invoke a model. Per-run obligations are frozen in immutable `initial_input`.

Workflow: `scope → gather → verify → synthesize → end`.

## Required companion and engine driving minimum

`using-loop-engine` (`skills/using-loop-engine/SKILL.md`) is a **required companion**. This skill does not replace it. The closed driving minimum below is what you cannot skip when this skill is loaded alone; load the companion for full engine semantics.

**Run-state commands:** `start`, `list`, `show`, `append`, `event`, `history`, `terminate`, and `invoke`.

**Non-run-state commands:** `fan-out` and `preview-bindings`. They do not start, advance, or record a run.

**Envelopes:** `completed`, `rejected`, `error`, and `invalid-invocation`. Parse JSON even on nonzero exit. Treat only `completed` as success.

**Bound versus unbound:** a catalog slot ID present in frozen `work_slot_bindings` is bound — `invoke` it; do not perform the stored work body. An absent key is unbound — perform the stored instructions yourself, then append and request the event.

**Overlay meaning:** overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. You still triage worker output, append provider-shaped records, and request the shown event.

**Lock-in-before-start:** do not call `start` until the user confirms (1) bind or not (which slot IDs), (2) exact `{command, args}` per bound slot, and (3) model identity in those frozen args (nested `--worker` / `--task-worker` count) or explicit unpinned-default acceptance. Bindings freeze and cannot be patched.

Run `loop-engine preview-bindings` on the JSON you will freeze before `start`. This skill is the research counterpart of `crates/software-change-provider/skills/using-software-change-provider/SKILL.md` and `crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md`: same engine loop, different artifacts, gates, and primary work. Do not markdown-link outside this crate. Provider contract: `crates/research-provider/README.md`. Judging and adjudication: `crates/research-provider/data/reviewer-protocol.md`. Artifact shapes: `crates/research-provider/data/templates/`.

## Setup

```sh
cargo build -p loop-cli -p research-provider
```

Register `target/debug/research` under exact alias `research` (absolute `command` path) in uncommitted machine-local provider TOML:

```toml
[providers.research]
command = "/absolute/path/to/target/debug/research"
args = []
```

Copy `crates/research-provider/data/configs/standard.json` to a run-specific file. Shipped profiles omit `work_slot_bindings` (or `{}`). Cataloged slots are `scope`, `gather`, `verify`, and `synthesize`; all stay driver-performed until the caller adds a map. Bound workers are opt-in.

Review bindings must be constructed from the same **per-run** `PROFILE` that will be previewed and started. Set `DATA_ROOT` to the checkout root or to an empty directory previously populated by `research data-dump`. The constructor reads the provider-owned preamble and output schema from that tree and freezes their values inline; fan-out never resolves those files at invocation time. Keep `--no-skills --no-extensions`, load only the explicit cursor-provider and claude-bridge extensions, restrict review tools to `read,grep,find,ls`, and do not pass `--no-context-files`.

Do not add bindings, and do not start, until the user confirms: (1) driver-performed or selected bound slot; (2) the exact frozen command and args; and (3) every author label and model in the ordered roster. Bindings freeze and cannot be patched. A review slot with no configured axes must not be bound.

### Executable review-binding constructor

The constructor accepts only `SLOT_ID=verify` or `SLOT_ID=synthesize`. `ROSTER_JSON` is an ordered JSON array of pairwise-distinct, non-empty `author` labels and non-empty `model` ids. For every policy, the first `required_authors` roster entries are used in roster order; absent `required_authors` means one. Worker order is profile policy order, then roster order. The stable assignment block at the end of each preamble freezes provider `research`, selected slot id, exact axis id, exact profile `example_prompt` bytes, and required author claim.

Set every placeholder before running this complete snippet. It rejects unsupported or empty slots, malformed policies, missing prompts, invalid or insufficient rosters, invalid provider data, and missing machine-local inputs before start. It atomically rewrites `PROFILE`, computes its SHA-256, previews bindings extracted from that resulting file, displays the exact resulting bytes and hash, requires confirmation by retyping that hash, checks it again immediately before `start`, and starts that same file without a post-preview merge.

```sh
set -eu

PROFILE="${PROFILE:?set PROFILE to the selected per-run profile}"
SLOT_ID="${SLOT_ID:?set SLOT_ID to verify or synthesize}"
ROSTER_JSON="${ROSTER_JSON:?set ROSTER_JSON to an ordered author/model array}"
DATA_ROOT="${DATA_ROOT:?set DATA_ROOT to the checkout or data-dump root}"
LOOP_ENGINE="${LOOP_ENGINE:?set LOOP_ENGINE to the absolute loop-engine path}"
PI="${PI:?set PI to the absolute pi path}"
CURSOR_EXTENSION_PATH="${CURSOR_EXTENSION_PATH:?set CURSOR_EXTENSION_PATH}"
CLAUDE_BRIDGE_EXTENSION_PATH="${CLAUDE_BRIDGE_EXTENSION_PATH:?set CLAUDE_BRIDGE_EXTENSION_PATH}"
PROVIDER_CONFIG="${PROVIDER_CONFIG:?set PROVIDER_CONFIG to uncommitted providers.toml}"
RUN_LABEL="${RUN_LABEL:?set RUN_LABEL}"

case "$SLOT_ID" in
  verify|synthesize) ;;
  *) printf 'unsupported research review SLOT_ID: %s\n' "$SLOT_ID" >&2; exit 1 ;;
esac

require_nonblank() {
  name=$1
  value=$2
  case "$value" in
    *[![:space:]]*) ;;
    *) printf '%s must be non-empty\n' "$name" >&2; exit 1 ;;
  esac
}
require_nonblank PROFILE "$PROFILE"
require_nonblank DATA_ROOT "$DATA_ROOT"
require_nonblank LOOP_ENGINE "$LOOP_ENGINE"
require_nonblank PI "$PI"
require_nonblank CURSOR_EXTENSION_PATH "$CURSOR_EXTENSION_PATH"
require_nonblank CLAUDE_BRIDGE_EXTENSION_PATH "$CLAUDE_BRIDGE_EXTENSION_PATH"
require_nonblank PROVIDER_CONFIG "$PROVIDER_CONFIG"
require_nonblank RUN_LABEL "$RUN_LABEL"
[ -f "$PROFILE" ] || { printf 'PROFILE is not a file: %s\n' "$PROFILE" >&2; exit 1; }
[ -f "$PROVIDER_CONFIG" ] || { printf 'PROVIDER_CONFIG is not a file: %s\n' "$PROVIDER_CONFIG" >&2; exit 1; }

PREAMBLE_PATH="$DATA_ROOT/crates/research-provider/data/review-worker-preamble.txt"
OUTPUT_SCHEMA_PATH="$DATA_ROOT/crates/research-provider/data/review-worker-output-schema.json"
[ -s "$PREAMBLE_PATH" ] || { printf 'missing review preamble: %s\n' "$PREAMBLE_PATH" >&2; exit 1; }
[ -s "$OUTPUT_SCHEMA_PATH" ] || { printf 'missing output schema: %s\n' "$OUTPUT_SCHEMA_PATH" >&2; exit 1; }
jq -e '
  type == "object"
  and (keys == ["required"])
  and .required == ["axis", "author", "result", "findings"]
' "$OUTPUT_SCHEMA_PATH" >/dev/null || {
  printf 'invalid research review output schema\n' >&2
  exit 1
}

jq -e --arg slot "$SLOT_ID" --argjson roster "$ROSTER_JSON" '
  def nonblank: type == "string" and test("[^[:space:]]");
  def author_count: if has("required_authors") then .required_authors else 1 end;
  .review_policies[$slot] as $policies
  | type == "object"
    and ((has("work_slot_bindings") | not) or (.work_slot_bindings | type == "object"))
    and ($policies | type == "array" and length > 0)
    and ($policies | all(.[];
      type == "object"
      and (.id | nonblank)
      and (.example_prompt | nonblank)
      and (if has("required_authors")
           then (.required_authors | type == "number" and . >= 1 and floor == .)
           else true
           end)))
    and ($roster | type == "array" and length > 0)
    and ($roster | all(.[];
      type == "object"
      and ((keys | sort) == ["author", "model"])
      and (.author | nonblank)
      and (.model | nonblank)))
    and (($roster | map(.author) | unique | length) == ($roster | length))
    and (([$policies[] | author_count] | max) <= ($roster | length))
' "$PROFILE" >/dev/null || {
  printf 'invalid or insufficient policies/roster for %s\n' "$SLOT_ID" >&2
  exit 1
}

profile_dir=$(dirname "$PROFILE")
next_profile=$(mktemp "$profile_dir/.research-profile.XXXXXX")
bindings_file=$(mktemp "${TMPDIR:-/tmp}/research-bindings.XXXXXX")
trap 'rm -f "$next_profile" "$bindings_file"' EXIT HUP INT TERM

jq \
  --arg slot "$SLOT_ID" \
  --argjson roster "$ROSTER_JSON" \
  --arg loop_engine "$LOOP_ENGINE" \
  --arg pi "$PI" \
  --arg cursor_extension "$CURSOR_EXTENSION_PATH" \
  --arg claude_bridge_extension "$CLAUDE_BRIDGE_EXTENSION_PATH" \
  --rawfile base_preamble "$PREAMBLE_PATH" \
  --slurpfile output_schema "$OUTPUT_SCHEMA_PATH" '
    def author_count: if has("required_authors") then .required_authors else 1 end;
    .review_policies[$slot] as $policies
    | [
        $policies[] as $policy
        | range(0; ($policy | author_count)) as $author_index
        | $roster[$author_index] as $member
        | {
            command: $pi,
            args: [
              "--print", "--no-skills", "--no-extensions",
              "-e", $cursor_extension,
              "-e", $claude_bridge_extension,
              "--tools", "read,grep,find,ls",
              "--model", $member.model
            ],
            preamble: (
              $base_preamble
              + "FROZEN REVIEW ASSIGNMENT\n"
              + "provider: research\n"
              + "slot_id: " + $slot + "\n"
              + "axis: " + $policy.id + "\n"
              + "example_prompt:\n" + $policy.example_prompt + "\n"
              + "required_author_claim: " + $member.author + "\n"
            ),
            output_schema: $output_schema[0]
          }
      ] as $workers
    | .work_slot_bindings[$slot] = {
        command: $loop_engine,
        args: (["fan-out"] + ($workers | map(["--worker", tojson]) | add))
      }
  ' "$PROFILE" >"$next_profile"
jq -e . "$next_profile" >/dev/null || {
  printf 'constructor produced invalid PROFILE JSON\n' >&2
  exit 1
}
mv "$next_profile" "$PROFILE"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
PROFILE_SHA256=$(sha256_file "$PROFILE")
jq '.work_slot_bindings' "$PROFILE" >"$bindings_file"

printf '\nResulting PROFILE bytes (%s):\n' "$PROFILE"
cat "$PROFILE"
printf '\nResulting work_slot_bindings:\n'
cat "$bindings_file"
printf '\nPROFILE SHA-256: %s\n' "$PROFILE_SHA256"
"$LOOP_ENGINE" preview-bindings "@$bindings_file"
rm -f "$bindings_file"

printf 'Confirm these exact PROFILE bytes by typing SHA-256 %s: ' "$PROFILE_SHA256" >&2
IFS= read -r CONFIRMED_PROFILE_SHA256
[ "$CONFIRMED_PROFILE_SHA256" = "$PROFILE_SHA256" ] || {
  printf 'profile confirmation did not match; refusing start\n' >&2
  exit 1
}

CURRENT_PROFILE_SHA256=$(sha256_file "$PROFILE")
[ "$CURRENT_PROFILE_SHA256" = "$PROFILE_SHA256" ] || {
  printf 'PROFILE changed after preview (expected %s, got %s); refusing start\n' \
    "$PROFILE_SHA256" "$CURRENT_PROFILE_SHA256" >&2
  exit 1
}
trap - EXIT HUP INT TERM
exec "$LOOP_ENGINE" --json --config "$PROVIDER_CONFIG" \
  start research "@$PROFILE" "$RUN_LABEL"
```

The provider preamble makes the frozen assignment authoritative, treats the later state instruction body as driver context, and directs the worker to artifacts beneath the mechanically forwarded `artifact_root`. The worker returns a judgment only. The driver still owns deterministic checks, `show`, captured-output validation and candidate triage, evidence `append`, the requested `event`, and progression. Exit 0 or mechanical key presence does not establish a valid judgment.

Opt-in authoring worker (must not pass `--no-context-files`; do not add `--tools` unless you intend to restrict tools; same pattern for `scope`):

```json
"gather": {
  "command": "pi",
  "args": ["--print", "--no-skills", "--no-extensions", "-e", "CURSOR_EXTENSION_PATH", "-e", "CLAUDE_BRIDGE_EXTENSION_PATH", "--model", "MODEL"]
}
```

Driver-performed run: omit `work_slot_bindings` or set `"work_slot_bindings": {}`. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. The engine allocates the durable directory and records that absolute path in object `initial_input` (`show` reveals it). `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Then:

```sh
loop-engine --json --config "$PROVIDER_CONFIG" \
  start research "@/tmp/research-standard.json" "my research"
```

Once the run exists, subject files live under the allocated (or caller) `artifact_root` using fixed filenames: `brief.json`, `sources.json`, `verification.json`, `report.json`. Shipped `config_version` is `research-1`.

For an installed binary, dump into an empty directory first (`research data-dump "$DATA_ROOT"`), then copy `$DATA_ROOT/crates/research-provider/data/configs/standard.json`.

## External research work

Do the primary work outside Loop Engine, then record it in the subject artifacts. Author from `crates/research-provider/data/templates/`.

1. **Scope** — write a question, not a chosen answer. Name observable acceptance, constraints, and non-goals.
2. **Gather** — search and fetch externally. Record sources with stable ids, locators, and extracts later verification can check. `brief_revision` must equal current `brief.json` revision.
3. **Verify** — author claims with cited `source_ids`, support, and a genuine challenge (or an explicit record that none was found after search). `sources_revision` must equal current `sources.json` revision. Request `verified` to clear schema and links before commissioning review.
4. **Synthesize** — write a cited conclusion that answers the brief, with `{claim_id, source_id}` pairs. Do not introduce material claims verification never checked. `verification_revision` must equal current `verification.json` revision. Request `completed` to clear schema and links before commissioning review.

## Gate map

| Event (from state) | Subject checked | Evidence gate |
|---|---|---|
| `scoped` (scope) | `brief.json` | schema only |
| `gathered` (gather) | `sources.json` | schema and `brief_revision` link |
| `verified` (verify) | `verification.json` | schema, `sources_revision` link, then `verify` (`claim-grounded`, `adversarial`) |
| `completed` (synthesize) | `report.json` | schema, `verification_revision` link, then `synthesize` (`cited-conclusion`, `scope-faithful`) |
| `revise` (gather) | — check-free | returns to scope |
| `revise` / `revise-brief` (verify) | — check-free | gather / scope |
| `revise` / `revise-sources` / `revise-brief` (synthesize) | — check-free | verify / gather / scope |

Owning-phase routes for accepted upstream defects:

| From | Event | Goes to | Use when |
|---|---|---|---|
| verify | `revise` | gather | sources-owned defect |
| verify | `revise-brief` | scope | brief-owned defect |
| synthesize | `revise` | verify | verification-owned defect |
| synthesize | `revise-sources` | gather | sources-owned defect |
| synthesize | `revise-brief` | scope | brief-owned defect |

Verification-local `verification.json` corrections stay in verify: edit, recheck, retry `verified`. Report-local `report.json` corrections stay in synthesize: edit, recheck, retry `completed`. Do not waive known defects.

## Per-gate loop

1. `show` — read `current_state`, `current_state_instructions`, frozen `initial_input` (including `work_slot_bindings`, `review_policies`, `artifact_schemas`), `work_slots`, and `work_slot_invocations` (`overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, `inner_workers`).
2. If this state is a **bound** slot, do not author the room yourself. `invoke` it and poll overlay until `succeeded` / `failed` / `overrun`. On `overrun`, run `show` immediately before re-invoking. On failure, inspect `capture_dir/summary.json` and captured stdout before stderr. Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. If **unbound**, author or revise the subject artifact. Material content changes require a revision bump — a bump retires standing verdicts for that subject; keeping the revision asserts the edit was immaterial.
3. For unbound evidence gates, request the event once before commissioning review. Schema denial means fix the artifact and retry. Evidence denial after a valid shape means schema and links cleared — do not treat that denial as a review failure. Do not append review-evidence until schema and links have cleared; a later material shape fix would bump `revision` and retire the new verdicts. For a bound slot, `invoke` and reach overlay `succeeded` before requesting the checked event; do not request that event to “probe schema” while overlay is `running`, `failed`, or `overrun`.
4. Then obtain the axis's `required_authors` count of distinct external judgments (default 1): fresh context, not the artifact's author, each judging only that axis using its `example_prompt`. Unbound: you commission those reviewers. Bound: `invoke` already ran the frozen CLI; read its output, then you still triage and append. Follow `crates/research-provider/data/reviewer-protocol.md`: triage candidates before append or mutation; append only accepted in-scope material failures or conforming passes.
5. Append one `review-evidence` record per axis judgment:

```sh
loop-engine --json append "$RUN_ID" review-evidence @verdict.json
```

```json
{
  "gate": "verify",
  "policy_id": "claim-grounded",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "verification.json",
  "subject_revision": "3",
  "config_version": "research-1"
}
```

All eight fields required; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` non-empty on `fail`; `config_version` must match the run's frozen config. Out-of-enum values make the record nonconforming and block the axis until a conforming record supersedes it.

6. Request the event. Interpret the outcome:
   - **Schema denial** (`rejected`) — artifact shape or link failed; evidence was not judged: fix shape first.
   - **Evidence denial** (`rejected`) — names unsatisfied policy axes and diagnostics for nonconforming/ignored records.
   - **Error** — invalid or inaccessible `artifact_root`, or provider failure; nothing advanced.

## Evidence rules (condensed)

- Latest conforming verdict per `(axis, subject_revision, author)` stands. Evidence is not a vote; one standing `fail` blocks even when others pass.
- Distinct-author counts use exact `(name, kind)`; the subject's author never counts toward its own review.
- Stale `subject_revision` never satisfies; wrong `config_version` counts as neither pass nor fail.
- Nonconforming records block the axis with a malformed diagnostic until a later conforming record supersedes them.
- No waivers: a material finding stands until fixed or the revision changes.
- Late findings remain actionable when they provide current evidence, violated obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked. Comprehensive-first review and scope/materiality burdens still bar drip-feeding and unrelated reopening.

## Production proof boundary

Use `scripts/research-journey.py` for repository and archive checks. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. Source mode drives separate Loop Engine processes across provider TOML, SQLite, production provider, shipped standard artifacts, deterministic denials, owning-phase revise, evidence aggregation, and terminal `end`. Packaged mode starts extracted binaries, materializes embedded data with `data-dump` into an empty `--data-root`, and runs the checked prefix from that dump. `--self-test` proves invalid packaged usage fails before mutating work roots. Synthetic pass records prove schema/evidence shape, independence, routing, and persistence only; they are not semantic review judgments.
