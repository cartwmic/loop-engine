# Agent instructions for policy-document-provider

## Scope

This file covers work in this crate: the `policy-document` binary, shipped profiles and guidance under `data/`, embedded `data-dump` contents, and `skills/using-policy-document-provider/`. Engine CLI behavior, workspace-wide checks, and the other reference provider are documented at the repository root.

Do not treat this crate as a document editor, reviewer, or model caller. It reads target bytes, applies run-frozen deterministic policies, and aggregates caller-supplied semantic verdicts.

## Authority

Use this crate's [README.md](README.md) as the provider-contract summary and [skills/using-policy-document-provider/SKILL.md](skills/using-policy-document-provider/SKILL.md) to drive a run. Engine product requirements for this workflow are PRD section 11 in the repository `docs/PRD.md`. Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and CLI envelopes.

Choose the shipped profile for the target before starting:

| Target | Copy | Keep `target.id` | Keep `profile_version` |
|---|---|---|---|
| README | `data/readme.json` | `README.md` | `readme-2` |
| AGENTS | `data/agents.json` | `AGENTS.md` | `agents-2` |

Change those identities only when intentionally authoring a custom profile. Choose `mode: draft` for an authoring/revision request and `mode: audit` for assessment of an existing target. Mode stays frozen through later corrections within that run. Deterministic and semantic policies live in immutable initial input; do not bake document-specific policy into provider code.

Providers author worker-facing role and output content; the engine only transports and mechanically enforces it. Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression. Exit 0 does not establish a valid deliverable.

Simple-first/YAGNI/KISS apply to this provider under the product-wide engine direction: added complexity needs a meaningful current reason and inadequate simpler alternative in the ordinary design, not a new justification framework. Existing implementation/protocol/dependency choices have no presumption of preservation. Software-change finding, steering, grouped-review and criterion features belong in `crates/software-change-provider`. Keep this crate focused on target bytes, frozen document policies and caller-supplied semantic verdicts.

Before using amend-binding, invoke controls, overrun recovery, cancel-invocation or event override, load repository `docs/agent-usage.md` and `skills/using-loop-engine/SKILL.md` for their execution and cleanup rules. Normal provider evidence rules below remain unchanged; an engine override does not establish document conformance.

## Workflow

```sh
cargo test -p policy-document-provider
cargo fmt --all -- --check
```

Crate tests are not a substitute for the public-boundary journeys. After any crate change, run the locked build and both source journeys against shipped profile bytes from the repository root:

```sh
cargo build --locked -p loop-cli -p policy-document-provider
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

Immediately before production `start`, follow the skill's [Setup](skills/using-policy-document-provider/SKILL.md#setup) for the provider TOML, per-run profile, binding confirmation and exact `--json --config` start form. Use the target/profile and mode rules above and an absolute UTF-8 `target.path`.

Drive production `start` of this provider with `loop-engine`. When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority.

Topology is `prepare → deterministic-review → semantic-review → end`. `ready` and both `revise` events are check-free. Both `passed` events are checked; final semantic approval reruns deterministic checks against current bytes before evidence aggregation. The provider never edits the target.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use `..` in those links; the resolver treats parent segments as escapes. Refer to repository-root files in prose instead.

`policy-document` accepts `data-dump DIR` on argv. It does not implement `--help` or `--version`; other argv is an error. Describe and evaluate remain one JSON request on stdin.

Dump refuses to overwrite any destination entry, including dangling symlinks. On write failure, rollback removes only files created by that invocation.

Before appending semantic evidence, follow the skill's [Run loop](skills/using-policy-document-provider/SKILL.md#run-loop) and [Evidence record](skills/using-policy-document-provider/SKILL.md#evidence-record) for the exact command and eight-field shape. Append one `review-evidence` record per semantic axis, bound to exact `target_id`, lowercase SHA-256 of current target bytes, and frozen `profile_version`. Serialize `append` and `event`. Any target byte change invalidates prior evidence: `revise`, rerun deterministic review, recompute the digest, and commission fresh verdicts. Reviewer identity and verdicts are caller claims, not signatures.

## Completion and Handoff

Crate work is complete when tests and both source journeys pass, shipped data and the skill still match runtime behavior, and README/AGENTS.md in this crate remain accurate. These crate-local checks supplement the repository-root AGENTS.md workspace completion gate.

Handoff the files changed, commands run, digest, run ID and database path used if a policy-document run was used. Resume from `show` on that same catalog. Retain the residuals: the provider cannot lock target bytes across an engine commit, and synthetic journey passes do not prove semantic verdict quality.
