# Agent instructions for policy-document-provider

## Scope

This file covers work in this crate: the `policy-document` binary, shipped profiles and guidance under `data/`, embedded `data-dump` contents, and `skills/using-policy-document-provider/`. Engine CLI behavior, workspace-wide checks, and the other reference provider are documented at the repository root.

Do not treat this crate as a document editor, reviewer, or model caller. It reads target bytes, applies run-frozen deterministic policies, and aggregates caller-supplied semantic verdicts.

## Authority

Use this crate's [README.md](README.md) as the provider-contract summary and [skills/using-policy-document-provider/SKILL.md](skills/using-policy-document-provider/SKILL.md) to drive a run. Engine product requirements for this workflow are PRD section 11 in the repository `docs/PRD.md`. Repository-root `AGENTS.md` and `docs/agent-usage.md` govern checkout-wide operation and CLI envelopes.

Shipped profiles `readme-1` and `agents-1` are product. Keep `target.id` as `README.md` or `AGENTS.md` and keep `profile_version` unless you are intentionally authoring a custom profile. Deterministic and semantic policies live in immutable initial input; do not bake document-specific policy into provider code.

## Workflow

```sh
cargo test -p policy-document-provider
cargo fmt --all -- --check
```

After provider protocol, profile, evidence, or skill-contract changes, also run both source journeys against shipped profile bytes from the repository root:

```sh
for mode in draft audit; do
  python3 scripts/policy-document-journey.py \
    --engine target/debug/loop-engine \
    --provider target/debug/policy-document \
    --profile crates/policy-document-provider/data/readme.json \
    --mode "$mode"
done
```

Register `target/debug/policy-document` under exact alias `policy-document` with an absolute command path in uncommitted provider TOML. Copy a profile, set `mode` to `draft` or `audit`, and replace `target.path` with an absolute UTF-8 file path.

Topology is `prepare → deterministic-review → semantic-review → end`. `ready` and both `revise` events are check-free. Both `passed` events are checked; final semantic approval reruns deterministic checks against current bytes before evidence aggregation. The provider never edits the target.

Local markdown links in this crate's documents must resolve under this crate directory. Do not use `..` in those links; the resolver treats parent segments as escapes. Refer to repository-root files in prose instead.

`policy-document` accepts `data-dump DIR` on argv. It does not implement `--help` or `--version`; other argv is an error. Describe and evaluate remain one JSON request on stdin.

Dump refuses to overwrite any destination entry, including dangling symlinks. On write failure, rollback removes only files created by that invocation.

Append one `review-evidence` record per semantic axis, bound to exact `target_id`, lowercase SHA-256 of current target bytes, and frozen `profile_version`. Serialize `append` and `event`. Any target byte change invalidates prior evidence: `revise`, rerun deterministic review, recompute the digest, and commission fresh verdicts. Reviewer identity and verdicts are caller claims, not signatures.

## Completion and Handoff

Crate work is complete when tests (and journeys, when required) pass, shipped data and the skill still match runtime behavior, and README/AGENTS.md in this crate remain accurate.

Handoff the files changed, commands run, digest and run ID if a policy-document run was used, and any residual: the provider cannot lock target bytes across an engine commit, and synthetic journey passes do not prove semantic verdict quality.
