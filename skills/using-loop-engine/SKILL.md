---
name: using-loop-engine
description: Use when starting, inspecting, resuming, or advancing a durable Loop Engine run while an external driver performs work and supplies evidence.
---

# Using Loop Engine

## Overview

The engine stores workflow state and requests deterministic provider describe/evaluate
transport. It never executes agents, advisors or proof commands. One logical driver
owns mutations; external workers return work and judgments, not progression authority.
Load the active provider skill as well as this engine procedure.

## Deterministic setup

Use the normal user catalog for production unless the owner explicitly requests
isolation in this session. Never use a user catalog for tests. Put test databases,
artifacts and retained external outputs in driver-owned directories outside the checkout.
Do not infer isolation permission from an earlier run or inherited environment.

Inspect flags, profile artifact_root, LOOP_ variables, XDG_DATA_HOME, HOME and
USERPROFILE before start. Record the actual catalog path; unset unintended redirects
only for this operation, preserving intentional home configuration.
Catalog precedence is explicit --database, then the first nonempty value of
LOOP_ENGINE_DATABASE, LOOP_ENGINE_DATABASE_PATH, LOOP_DATABASE, LOOP_DATABASE_PATH,
LOOP_DB_PATH, LOOP_ENGINE_DB, LOOP_DB; then LOOP_ENGINE_HOME/loop.db,
LOOP_HOME/loop.db, XDG_DATA_HOME/loop-engine/loop.db, or
$HOME/.local/share/loop-engine/loop.db (USERPROFILE if HOME is absent).
Tilde expansion follows path selection.

Pass explicit --config with machine-local provider TOML, using an absolute command:

```toml
[providers.software-change]
command = "/absolute/path/software-change"
args = []
```

Use --json for workflow envelopes. Parse stdout even on nonzero exit; only completed
is success. rejected means the checked request did not advance; error and
invalid-invocation are not provider approval.
Global --timeout-ms defaults to 30000 per provider describe/evaluate call. Raise it
for a slow provider, not for externally executed work: driver timeouts are separate.

## Exact profile confirmation

Before start, display the exact per-run profile bytes and SHA-256 to the user.
Confirm profile identity/version, live gates, stages/groups, axes, author floors,
self-review policy, advice and Bookends state, operating boundaries and work policy.
Copying a shipped profile is not confirmation. Rehash immediately before start;
any byte change requires renewed confirmation. Never substitute a default afterward.

Confirm a separate external role-to-model manifest, commands, write ownership,
serial budget, proof owner and escalation owner. Verify the exact model with the
chosen harness and pass it explicitly; stop rather than silently substituting.
The profile hash does not cover this external fleet. No executor is frozen in engine
state. Later profile edits do not change a started run's frozen obligations.

## Commands

```sh
loop-engine --json --config /absolute/providers.toml start software-change @/absolute/profile.json "change"
loop-engine --json list
loop-engine --json show RUN --view full
loop-engine --json append --record-id RECORD RUN KIND @/absolute/data.json
loop-engine --json event RUN EVENT
loop-engine --json history RUN
loop-engine --json terminate RUN
```

start returns result.run.id. Initial input and append data accept inline JSON,
@FILE or stdin (-). Append stores context, not semantic approval or a transition.
Use the same resolved database for every operation; pass --database explicitly when
isolation is authorized. terminate changes workflow state, never cancels external work.

The coordinating assistant must post a concise source-backed owner update in the active Pi conversation after a meaningful observed change, before the next wait, inspect, or owner-help decision, and stay quiet for unchanged observations.

## Choose an observation

- Action show (default) reveals current instructions/events and arms mutation.
- Full show includes frozen input and complete context; use it for provider helpers.
- Status show and --compact do not arm. Neither do monitor, list, history or read.
- After every transition, read action/full again before append, event or terminate.

```sh
loop-engine --json show RUN --view action
loop-engine --json show RUN --view status
loop-engine monitor --run RUN --database /absolute/loop.db --json --attention-seconds 300
loop-engine read RUN --kind history --cursor 1 --limit 20
loop-engine explore RUN
```

Monitor is passive JSONL durable-state observation: external execution is unknown.
It does not approve, retry or cancel work. Stop the observer without stopping workers.
After a meaningful observed change, post a concise source-backed owner update naming
what changed and the needed action/decision before another wait or inspection.
Use targeted read for bounded history/delta or retained historical output; missing
execution information is unknown, not success. Explore is a terminal inspection UI.

## Canonical loop

1. Observe action/full; read current instructions and artifact locators.
2. Commission through the provider; missing_inputs means supply inputs, not launch-ready.
3. Perform work externally under confirmed ownership. Preserve request, original return,
   source identity and genuine command results outside the checkout.
4. Intake review returns through provider review-candidates; inspect candidates and
   diagnostics, triage findings, then append accepted provider-shaped context.
5. Observe and request the shown checked event. Correct the named cause on denial;
   do not fabricate evidence or use an exception as ordinary clearance.
6. Repeat until the actual terminal state; report residuals and pending owner decisions.

## Execution recovery minimum

The external driver owns launch, timeout, cancellation and verified cleanup. Before
retry, inspect partial edits and retained output, verify no overlapping writer remains,
and decide whether to restore or incorporate work. Never signal remembered PIDs or
claim workflow termination stopped a process. Preserve failed attempts honestly.

## Evidence and handoff

Keep genuine authors, review groups and self-review declarations. Effective author
counts include explicit owner-attested amendments; never silently lower a floor.
Reuse only with explicit evidence-applicability and a reason tied to current subject,
revision/source identity. Typed external advice is appended as advice-answer; its
reasoned disposition does not replace review, proof or a checked event.

At completion, return run/catalog identity, terminal observation, artifact and proof
locators, actual review decisions, residual risks and outstanding Git/owner handoff.
Process exit, schema validity and semantic acceptance are separate claims.
