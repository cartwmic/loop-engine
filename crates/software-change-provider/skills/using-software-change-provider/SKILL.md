---
name: using-software-change-provider
description: Use when running the software-change workflow through Loop Engine with the software-change provider — confirming work-slot bindings and models with the user before start, selecting a config profile, authoring gate artifacts, invoking bound implement workers or performing unbound rooms, appending review-evidence and accepted-findings records, and clearing checked transitions.
---

# Using the software-change provider

## Overview

`software-change` is Loop Engine's reference provider, distributed standalone with its shipped data embedded (`software-change data-dump DIR` materializes it); a repo checkout remains the development path. The live graph is the union of five draft rooms plus parent and adversarial review rooms when those policy lists are nonempty; `describe` omits empty lists. Draft ready events check schema and revision links only. Parent semantic axes live on the parent review state; adversarial axes live on the distinct adversarial-review state. Validation-report-local corrections stay in the validation draft after edit/recheck for the next checked hop; nearest `revise` from validation-review and validation-adversarial-review returns to that draft, and those states also expose `revise-implementation`. Phase-named owning routes (`revise-intent`, `revise-design`, `revise-plan`) handle upstream defects from review states. Reviewer convergence contract requires candidate triage before append or mutation, focused external reconsideration for disputed candidates, comprehensive first review, and bounded confirmation review. Quiet, progress, and thrash count per review state on the post-triage accepted-finding set. Confirmation consumes the durable set and does not search again except for fix-introduced holes. Bound workers do not use previously overlooked after that state's first comprehensive review of the subject; humans still may with full failure burden. Known accepted material defects are never waived. Adversarial output is candidate data under the YAGNI/pragmatic append bar: extra mechanism, unlisted requirements, and hypothetical-future fails are not appended. Late findings still require current evidence, violated obligation, consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); prior visibility or overlook does not waive known material defects, while comprehensive-first and scope/materiality burdens block drip-feeding or unrelated reopening.

The provider is deterministic only: it validates artifact schemas and revision links, then aggregates externally supplied review evidence. `describe` and `evaluate` never generate prompts, invoke a model, or judge findings. Bound workers, when frozen, are started by `loop-engine invoke`; you still triage outputs and append verdicts. Per-run obligations are frozen in immutable `initial_input`.

## Required companion and engine driving minimum

`using-loop-engine` (`skills/using-loop-engine/SKILL.md`) is a **required companion**. This skill does not replace it. The closed driving minimum below is what you cannot skip when this skill is loaded alone; load the companion for full engine semantics.

**Run-state commands:** `start`, `list`, `show`, `append`, `event`, `history`, `terminate`, and `invoke`.

**Other commands:** `invocation-progress`, `fan-out`, and `preview-bindings`. They are not a ninth primary. `fan-out` and `preview-bindings` do not start, advance, or record a run. `invocation-progress` opens the catalog; a query failure does not flip overlay.

**Envelopes:** `completed`, `rejected`, `error`, and `invalid-invocation`. Parse JSON even on nonzero exit. Treat only `completed` as success.

**Bound versus unbound:** a catalog slot ID present in frozen `work_slot_bindings` is bound — `invoke` it; do not perform the stored work body. An absent key is unbound — perform the stored instructions yourself, then append and request the event.

**Overlay meaning:** overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. You still triage worker output, append provider-shaped records, and request the shown event.

**Dual poll while overlay is running:** `show` for overlay, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and empty `inner_workers`. `invocation-progress` for `invocation_id`, `capture_dir`, per-step `not_started`|`running`|`reaped`, and named sidecar/session traces. Graph state is Dagu helper liveness; `reaped` means the Dagu step helper finished, not overlay success and not inner waitpid 0. True inner waitpid remains sidecar/`summary.json`; overlay remains the bound CLI process exit. `dagu status` / `dagu history` remain the underlying surface `invocation-progress` uses; they are not the driver-facing path.

**Concurrency:** `loop-engine fan-out --max-active N` omitted stays uncapped; set N is at most N worker steps. `software-change run-plan-graph --working-directory ABS --max-active N` requires the driver's existing absolute directory; omitted stays 4 ordinary plan tasks, set N is at most N ordinary plan tasks, and the summarizer still runs after those tasks.

**Lock-in-before-start:** do not call `start` until the user confirms (1) bind or not (which slot IDs), (2) exact `{command, args}` per bound slot, and (3) model identity in those frozen args (nested `--worker` / `--task-worker` count) or explicit unpinned-default acceptance. Bindings freeze and cannot be patched.

Run `loop-engine preview-bindings` on the JSON you will freeze before `start`. That report includes a `dagu` PATH check (minimum 2.14.0): ok with resolved path and version, or a warning naming the path or that PATH lookup found nothing; well-formed bindings still exit 0. `fan-out` and `software-change run-plan-graph` execute fail-close on the same condition before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator keys `dagu_home`, `dag_name`, and `run_name` (`plan-graph-<capture-dir-name>` for plan-graph). Provider packages do not ship `dagu`. Dagu is GPLv3: invoke the binary as a subprocess only; do not embed its Go API. While overlay is `running`, poll `show` for overlay and `invocation-progress` for inner graph/traces. True inner waitpid lives in the sidecar and `summary.json`; snapshot `reaped` is helper liveness. Bound review `fan-out` joins mechanically (`fan-out-join` writes `summary.json`, invokes no model). Bound worker stdin is compact location JSON (absolute `artifact_root`, plus `context` when the catalog slot lists `stdin_context_kinds`); it does not dump `instruction_body`. Live review slots list `accepted-findings` so confirmation workers receive the durable set; draft slots omit the field and stay `{artifact_root}`. Hidden stdin-exec colocates Pi sessions under each worker `capture_dir/sessions` via `PI_CODING_AGENT_SESSION_DIR` unless that variable is already inherited; do not add `--session-dir` to frozen argv, and do not switch bound Pi commands to `--mode json`. Provider contract details: `crates/software-change-provider/README.md`; frozen requirements: `crates/software-change-provider/docs/prd.md`.

## Setup

```sh
cargo build -p loop-cli -p software-change-provider
```

Register `target/debug/software-change` under alias `software-change` (absolute `command` path) in an uncommitted machine-local `providers.toml`.

Pick a profile from `crates/software-change-provider/data/configs/` — `minimal.json` (validation-review only, no adversarial lists), `standard.json` (intent-review, design-review, validation-review plus 1:1 adversarial counterparts), or `high-rigor.json` (all parent review gates plus 1:1 adversarial counterparts; two distinct reviewers on design-review and validation-review parent axes). Copy it to a run-specific file. Shipped profiles omit `work_slot_bindings`. Do not `start` that copy until the user has approved the work-slot policy below. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority. The engine allocates the durable directory and records that absolute path in object `initial_input` (`show` reveals it). `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers.

## Work-slot policy (confirm before start)

Cataloged slots: `intent-draft`, `intent-review`, `intent-adversarial-review`, `design-draft`, `design-review`, `design-adversarial-review`, `plan-draft`, `plan-review`, `plan-adversarial-review`, `implement`, `implementation-review`, `implementation-adversarial-review`, `validation-draft`, `validation-review`, `validation-adversarial-review`. End has no slot. Bindings are sparse and freeze at `start`. A binding whose slot is not in the snapshotted catalog fails at `start`.

Shipped profiles omit `work_slot_bindings` (or `{}`). Every slot stays driver-performed until the caller opts in. A review slot with no configured policy axes must not be bound. Copying a profile is not model lock-in. Skill and constructor do not emit draft bindings. `start` still accepts a hand-written draft binding.

Before `start`, show the resulting per-run profile, its exact `work_slot_bindings`, the expanded `preview-bindings` report, and the profile SHA-256. Wait for the caller to confirm all three:

1. **Bound slots:** which sparse slot IDs, if any.
2. **Exact command and args:** including every nested worker.
3. **Models:** every model-bearing CLI has its model frozen in argv, or the caller explicitly accepts an unpinned default.

### Deterministic review-binding constructor

Do not hand-author one generic worker. The executable constructor below accepts the same per-run `PROFILE` that will be frozen, one live review `SLOT_ID`, and an ordered caller-confirmed `ROSTER` JSON file. `ROSTER` must be a non-empty array of exact `{ "author": "...", "model": "..." }` objects with pairwise-distinct, non-empty author labels and non-empty models. Supported slots are every live review slot: `intent-review`, `validation-review`, the other parent review slots, and all adversarial slots. Skill and constructor do not emit draft bindings (`intent-draft`, `design-draft`, `plan-draft`, `implement`, `validation-draft`). `start` still accepts a hand-written draft binding. Use the same roster file for parent and adversarial slots; attacker identity has no disjoint-author floor and there is no second roster file. Bind parent and adversarial as separate slots; same-slot mixed parent and adversarial fan-out is not the enabled path.

Set every path explicitly. `DATA_ROOT` is either the checkout root or a root populated by `software-change data-dump`. `ENGINE` and `PI_COMMAND` become frozen command bytes. Review workers keep `--no-skills --no-extensions`, load only the two explicit extensions, include `--tools read,grep,find,ls`, and do not pass `--no-context-files`.

```sh
set -eu
: "${PROFILE:?path to the per-run profile being frozen}"
: "${SLOT_ID:?live review slot id}"
: "${ROSTER:?path to ordered caller-confirmed roster JSON}"
: "${DATA_ROOT:?checkout root or data-dump root}"
: "${ENGINE:?absolute loop-engine command}"
: "${PI_COMMAND:?absolute pi command}"
: "${CURSOR_EXTENSION_PATH:?absolute cursor-provider extension path}"
: "${CLAUDE_BRIDGE_EXTENSION_PATH:?absolute claude-bridge extension path}"

case "$SLOT_ID" in
  intent-review|intent-adversarial-review|design-review|design-adversarial-review|plan-review|plan-adversarial-review|implementation-review|implementation-adversarial-review|validation-review|validation-adversarial-review) ;;
  intent-draft|design-draft|plan-draft|implement|validation-draft)
    printf 'constructor does not emit draft bindings: %s\n' "$SLOT_ID" >&2; exit 1 ;;
  *) printf 'unsupported review SLOT_ID: %s\n' "$SLOT_ID" >&2; exit 1 ;;
esac

PREAMBLE_FILE="$DATA_ROOT/crates/software-change-provider/data/review-worker-preamble.txt"
SCHEMA_FILE="$DATA_ROOT/crates/software-change-provider/data/review-worker-output-schema.json"
for required_file in "$PROFILE" "$ROSTER" "$PREAMBLE_FILE" "$SCHEMA_FILE"; do
  test -f "$required_file" || { printf 'missing required file: %s\n' "$required_file" >&2; exit 1; }
done

TMP_PROFILE=$(mktemp "$(dirname "$PROFILE")/.software-change-profile.XXXXXX")
trap 'rm -f "$TMP_PROFILE"' EXIT HUP INT TERM
jq \
  --arg slot "$SLOT_ID" \
  --arg engine "$ENGINE" \
  --arg pi "$PI_COMMAND" \
  --arg cursor "$CURSOR_EXTENSION_PATH" \
  --arg bridge "$CLAUDE_BRIDGE_EXTENSION_PATH" \
  --rawfile base_preamble "$PREAMBLE_FILE" \
  --slurpfile output_schema "$SCHEMA_FILE" \
  --slurpfile roster "$ROSTER" '
  def required_author_count:
    if has("required_authors") then .required_authors else 1 end;
  . as $profile
  | ($roster[0]) as $entries
  | if (($entries | type) != "array" or ($entries | length) == 0) then
      error("ROSTER must be a non-empty array")
    elif (all($entries[]; type == "object") | not) then
      error("every ROSTER entry must be an object")
    elif (all($entries[]; ((keys | sort) == ["author", "model"])) | not) then
      error("every ROSTER entry must contain exactly author and model")
    elif (all($entries[]; ((.author | type) == "string" and (.author | length) > 0 and (.model | type) == "string" and (.model | length) > 0)) | not) then
      error("ROSTER author and model values must be non-empty strings")
    elif (($entries | map(.author) | unique | length) != ($entries | length)) then
      error("ROSTER author labels must be pairwise distinct")
    elif (($output_schema | length) != 1 or $output_schema[0] != {"required":["axis","author","result","findings"]}) then
      error("provider review-worker output schema is missing or unsupported")
    elif (($profile.work_slot_bindings // {} | type) != "object") then
      error("PROFILE work_slot_bindings must be absent or an object")
    elif (
      $slot == "intent-draft" or $slot == "design-draft" or $slot == "plan-draft"
      or $slot == "implement" or $slot == "validation-draft"
    ) then
      error("constructor does not emit draft bindings")
    elif (([
        "intent-review","intent-adversarial-review",
        "design-review","design-adversarial-review",
        "plan-review","plan-adversarial-review",
        "implementation-review","implementation-adversarial-review",
        "validation-review","validation-adversarial-review"
      ] | index($slot)) == null) then
      error("unsupported review slot")
    else . end
  | ($profile.review_policies[$slot]) as $policies
  | if (($policies | type) != "array" or ($policies | length) == 0) then
      error("selected review slot has an unsupported or empty policy list")
    elif (all($policies[]; (type == "object" and (.id | type) == "string" and (.id | length) > 0)) | not) then
      error("every selected policy must have a non-empty id")
    elif (all($policies[]; ((.example_prompt | type) == "string" and (.example_prompt | length) > 0)) | not) then
      error("every selected policy must have a non-empty example_prompt")
    elif (all($policies[]; ((required_author_count | type) == "number" and (required_author_count | floor) == required_author_count and required_author_count > 0)) | not) then
      error("required_authors must normalize to a positive integer")
    elif (([$policies[] | required_author_count] | max) > ($entries | length)) then
      error("ROSTER has too few entries for selected required_authors")
    else . end
  | [
      $policies[] as $policy
      | range(0; ($policy | required_author_count)) as $roster_index
      | $entries[$roster_index] as $entry
      | {
          command: $pi,
          args: [
            "--print", "--no-skills", "--no-extensions",
            "-e", $cursor, "-e", $bridge,
            "--tools", "read,grep,find,ls",
            "--model", $entry.model
          ],
          preamble: (
            $base_preamble
            + "FROZEN REVIEW ASSIGNMENT\n"
            + "provider: software-change\n"
            + "slot_id: " + $slot + "\n"
            + "axis: " + $policy.id + "\n"
            + "example_prompt:\n" + $policy.example_prompt + "\n"
            + "required_author_claim: " + $entry.author
          ),
          output_schema: $output_schema[0]
        }
    ] as $workers
  | (reduce $workers[] as $worker
      (["fan-out"]; . + ["--worker", ($worker | tojson)])) as $fan_out_args
  | .work_slot_bindings = (.work_slot_bindings // {})
  | .work_slot_bindings[$slot] = {command: $engine, args: $fan_out_args}
' "$PROFILE" >"$TMP_PROFILE"
jq -e . "$TMP_PROFILE" >/dev/null
mv "$TMP_PROFILE" "$PROFILE"
trap - EXIT HUP INT TERM

profile_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
PROFILE_SHA256=$(profile_sha256 "$PROFILE")
printf '%s\n' 'Exact resulting PROFILE bytes:'
cat "$PROFILE"
printf '\n'
printf '%s\n' 'Resulting work_slot_bindings:'
jq .work_slot_bindings "$PROFILE"
printf 'PROFILE_SHA256=%s\n' "$PROFILE_SHA256"

BINDINGS_PREVIEW=$(mktemp "${TMPDIR:-/tmp}/software-change-bindings.XXXXXX")
trap 'rm -f "$BINDINGS_PREVIEW"' EXIT HUP INT TERM
jq -e .work_slot_bindings "$PROFILE" >"$BINDINGS_PREVIEW"
"$ENGINE" preview-bindings "@$BINDINGS_PREVIEW"
rm -f "$BINDINGS_PREVIEW"
trap - EXIT HUP INT TERM
printf 'Confirm this resulting profile, bindings, models, and SHA-256 before start.\n'
```

The constructor rewrites `PROFILE` atomically before hash/preview. It preserves any other sparse bindings already present, but there is no merge or edit after preview and confirmation. Worker order is profile policy order, then the first normalized `required_authors // 1` roster entries in roster order. Each nested worker freezes the exact provider preamble value, output declaration, profile axis/prompt, required author claim, and model argv. The engine later inserts the compact location JSON between this frozen preamble and the separator: `{artifact_root}` for draft slots, or `{artifact_root, context}` for live review slots whose catalog lists `accepted-findings`.

After the caller confirms, copy the displayed hash into `CONFIRMED_PROFILE_SHA256`. Immediately before `start`, fail if the same profile's bytes changed, then start that unchanged file:

```sh
set -eu
: "${CONFIRMED_PROFILE_SHA256:?exact SHA-256 confirmed by the caller}"
: "${PROVIDER_CONFIG:?absolute provider TOML path}"
: "${LABEL:?run label}"
if command -v sha256sum >/dev/null 2>&1; then
  CURRENT_PROFILE_SHA256=$(sha256sum "$PROFILE" | awk '{print $1}')
else
  CURRENT_PROFILE_SHA256=$(shasum -a 256 "$PROFILE" | awk '{print $1}')
fi
test "$CURRENT_PROFILE_SHA256" = "$CONFIRMED_PROFILE_SHA256" || {
  printf 'PROFILE changed after confirmation: expected %s, got %s\n' \
    "$CONFIRMED_PROFILE_SHA256" "$CURRENT_PROFILE_SHA256" >&2
  exit 1
}
"$ENGINE" --json --config "$PROVIDER_CONFIG" \
  start software-change "@$PROFILE" "$LABEL"
```

Driver-performed run: omit `work_slot_bindings` or set `"work_slot_bindings": {}`. Do not bind a review slot whose configured policy-axis list is empty. The constructor does not emit draft bindings; a hand-written draft binding is still accepted by `start`.

The implement binding remains a separate opt-in `run-plan-graph --working-directory ABS --task-worker` pattern; it is not produced by the review constructor, must freeze one existing absolute directory selected and maintained by the driver, must not pass `--no-context-files`, and must freeze its model before start. Omitted, relative, nonexistent, and non-directory values are rejected before workers; the same graph-level cwd reaches every plan task and summarizer. Git is not required, and the provider does not create, discover, select, reuse, merge, clean, manage, or suggest worktrees. Optional `--max-active N` may live in that frozen argv (omitted stays 4 ordinary plan tasks; set N is at most N ordinary plan tasks; the summarizer still runs after those tasks). Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only.

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--working-directory",
    "/absolute/path/to/driver-selected-checkout",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--model\",\"MODEL\"]}"
  ]
}
```

Once the run exists, subject files live under the allocated (or caller) `artifact_root` using fixed filenames: `intent.json`, `design.json`, `plan.json`, `implementation-report.json`, `validation-report.json`.

## Gate map

Draft ready/passed events check schema and revision links only. Live review `approved` or `passed` rechecks the subject, aggregates that gate's `review-evidence`, and requires a well-formed current-revision `accepted-findings` record. The live last hop into `end` is `passed`; earlier live review hops are `approved`.

| Event (from state) | Subject checked | Evidence gate |
|---|---|---|
| `intent-ready` (explore) | `intent.json` | schema/links only |
| `approved` / `passed` (`intent-review`) | `intent.json` | `intent-review` |
| `approved` / `passed` (`intent-adversarial-review`) | `intent.json` | `intent-adversarial-review` |
| `design-ready` (design) | `design.json` | schema/links only |
| `approved` / `passed` (`design-review`) | `design.json` | `design-review` |
| `approved` / `passed` (`design-adversarial-review`) | `design.json` | `design-adversarial-review` |
| `plan-ready` (plan) | `plan.json` | schema/links only |
| `approved` / `passed` (`plan-review`) | `plan.json` | `plan-review` |
| `approved` / `passed` (`plan-adversarial-review`) | `plan.json` | `plan-adversarial-review` |
| `implementation-ready` (implement) | `implementation-report.json` | schema/links only |
| `approved` / `passed` (`implementation-review`) | `implementation-report.json` | `implementation-review` |
| `approved` / `passed` (`implementation-adversarial-review`) | `implementation-report.json` | `implementation-adversarial-review` |
| `validation-ready` / `passed` (validation) | `validation-report.json` | schema/links only |
| `approved` / `passed` (`validation-review`) | `validation-report.json` | `validation-review` |
| `passed` (`validation-adversarial-review`) | `validation-report.json` | `validation-adversarial-review` |
| `revise` (any review state) | — check-free | — |

## Per-gate loop

1. `show` — read `current_state`, `current_state_instructions`, frozen `initial_input` (including `work_slot_bindings`, `review_policies`, `artifact_schemas`), `work_slots`, and `work_slot_invocations` (`overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, `inner_workers`).
2. If this state is a **bound** slot, do not author the room yourself. `invoke` it (`loop-engine --json --timeout-ms N invoke RUN_ID SLOT_ID`; raise `N` above the 30s default), then poll `show` until overlay `succeeded` / `failed` / `overrun`. While overlay is `running`, also poll `loop-engine invocation-progress RUN_ID` for inner graph/traces (`inner_workers` stays empty on `show`). On `overrun`, run `show` immediately before re-invoking. On failure, inspect `capture_dir/summary.json` and per-worker stdout before stderr. Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. For bound `implement`, `software-change run-plan-graph --working-directory ABS` runs a local Dagu `type:graph` from `plan.json` using one driver-selected existing absolute directory for every task and the summarizer (omitted, relative, nonexistent, and non-directory values fail before workers; Git and worktree management are outside the provider). Omitted `--max-active` is 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. It requires the mandatory `summarizer` to write a fresh `implementation-report.json` after those tasks before overlay `succeeded`. Graph state on `invocation-progress` is Dagu helper liveness; `reaped` is not overlay success and not inner waitpid 0. Ordinary task stdin is compact `artifact_root` JSON plus that task's plan object only; it does not dump `instruction_body`. Hidden `stdin-exec --exit-mode propagate` is the exclusive file-to-stdin helper for those task steps. For a bound review slot frozen to `fan-out`, read `capture_dir` (`summary.json` and per-worker stdout/stderr) and poll `invocation-progress` while overlay is `running`. Omitted `fan-out --max-active` stays uncapped; `--max-active N` is at most N worker steps. Overlay `succeeded` means the fan-out facade exited 0, not that the review passed.
3. If this state is **unbound**, author or revise the subject artifact in `artifact_root` using its template from `crates/software-change-provider/data/templates/`. Material content changes require a revision bump — a bump retires all standing verdicts for that subject by design; keeping the revision asserts the edit was immaterial.
4. For evidence gates, obtain the axis's configured `required_authors` count of distinct external judgments (default 1; high-rigor design-review and validation-review parent axes require 2; adversarial axes require 1): fresh context, not the artifact's author, each judging only that axis using its `example_prompt`. Follow `crates/software-change-provider/data/reviewer-protocol.md`. Unbound: you commission those reviewers. Bound review: read the captures, then you still triage and append; `fan-out` does not write records. Match worker count to `required_authors` when you freeze review `--worker` args. Adversarial output is candidate data; extra mechanism, unlisted requirements, and hypothetical-future fails are not appended.
5. Append one record per accepted axis judgment — `kind` is `review-evidence`, `data` is the eight-field object:

```sh
loop-engine --json append "$RUN_ID" review-evidence @verdict.json
```

```json
{
  "gate": "design-review",
  "policy_id": "intent-faithful",
  "result": "pass",
  "findings": "",
  "author": {"name": "reviewer-sol", "kind": "agent"},
  "subject": "design.json",
  "subject_revision": "3",
  "config_version": "standard-6"
}
```

All eight fields required; `result` is exactly `pass` or `fail`; `author.kind` is exactly `human`, `agent`, or `script`; `findings` non-empty on `fail`; `config_version` must match the run's frozen config. Out-of-enum values make the record nonconforming and block the axis until a conforming record supersedes it.

After triage, emit the loop-engine append envelope for `accepted-findings` from the shipped template. Neither the provider nor the engine writes the kind. Empty `findings` is well-formed. Optional `author` may be present and is not counted.

```sh
TEMPLATE="$DATA_ROOT/crates/software-change-provider/data/templates/accepted-findings.json"
jq \
  --arg gate "$GATE" \
  --arg subject "$SUBJECT" \
  --arg rev "$SUBJECT_REVISION" \
  --argjson findings "$FINDINGS_JSON" \
  '.data.gate=$gate | .data.subject=$subject | .data.subject_revision=$rev | .data.findings=$findings' \
  "$TEMPLATE" >accepted-findings.envelope.json
KIND=$(jq -r .kind accepted-findings.envelope.json)
loop-engine --json append "$RUN_ID" "$KIND" "$(jq -c .data accepted-findings.envelope.json)"
```

Well-formed `data` is `{gate, subject, subject_revision, findings}` where `findings` is an array of `{policy_id, statement}` objects with nonempty strings.

6. Request the event. Interpret the outcome:
   - **Schema denial** (`rejected`) — artifact shape or link failed; evidence was not judged: fix shape first.
   - **Evidence denial** (`rejected`) — names unsatisfied policy axes and diagnostics for nonconforming/ignored records.
   - **Error** — invalid or inaccessible `artifact_root`, or provider failure; nothing advanced.

## Evidence rules (condensed)

- Latest conforming verdict per `(axis, subject_revision, author)` stands. Evidence is not a vote; one standing `fail` blocks even when others pass.
- Distinct-author counts use exact `(name, kind)`; the subject's author never counts toward its own review.
- Stale `subject_revision` never satisfies; wrong `config_version` counts as neither pass nor fail.
- Nonconforming records block the axis with a malformed diagnostic until a later conforming record supersedes them.
- No waivers: a material finding stands until fixed or the revision changes. Known accepted material defects are never waived.
- After triage, a well-formed current-revision `accepted-findings` record is required before live-review `approved` or `passed`. evaluate does not judge contents, quiet/progress/thrash, or provenance.
- Confirmation consumes the durable accepted-finding set and does not search again except for fix-introduced holes. Bound workers do not use previously overlooked after that state's first comprehensive review of the subject; humans still may with full failure burden.
- Late findings remain actionable when they provide current evidence, violated obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked; timing, prior visibility, or reviewer overlook does not waive materiality. Comprehensive-first review and scope/materiality burdens still bar drip-feeding and unrelated reopening.

## Production proof boundary

Use `scripts/software-change-journey.py` for repository and archive checks. Those journey commands are harness examples, distinct from the production start; do not copy isolation flags from them into production start. Source `full` mode drives separate Loop Engine processes across provider TOML, SQLite, production provider, shipped high-rigor artifacts, deterministic denials, evidence aggregation, and terminal state. After the high-rigor run reaches `end`, it starts a second run from shipped `minimal.json` and walks the stitched hops (empty review lists omitted, last-hop `passed`). Packaged `checked-prefix` mode starts extracted binaries, materializes embedded data with `data-dump`, and runs one checked transition from that dump. Synthetic pass records prove schema/evidence shape, independence, routing, aggregation, and persistence only; they are not semantic review judgments.
