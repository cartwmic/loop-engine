# Agent usage

## Public surface

Primary commands: `start`, `list`, `show`, `append`, `event`, `history`, `terminate`. Additional commands: targeted `read`, terminal `explore`, passive `monitor`. Monitoring observes durable state, not external execution. Providers use `describe` and `evaluate` transport.

Use an explicit catalog `--config`, inspect unintended LOOP/XDG/home redirects, and keep artifacts outside the checkout. Never use a user catalog for tests. Load CLI `--help` for exact arguments. Mutations require a fresh action/full observation; compact/status views do not arm them.
## External-driver loop

The engine stores and evaluates workflow state; it never executes agents, advisors or proof commands. The driver may use any agents or harness.

1. Inspect action/full `show` and provider `commission`. Early commissions return `missing_inputs`; supply those inputs before treating a commission as ready.
2. Do the work externally. Reviewers return judgments; the driver runs deterministic checks and owns progression.
3. Submit external judgments through `review-candidates`, external `command_results` through `prepare-validation`, and accepted context through engine `append`.
4. Read action/full `show`, then send the checked `event`; re-read after each transition. A successful process exit alone does not prove the deliverable.

Preserve review groups, self-review declarations, author counts and amendments. Declare evidence applicability explicitly; check ordinary review clearance before challenge review. Respect declared source scopes and current source identity. Record residual risks honestly and cover every applicable sufficiency axis; zero axes do not require invented reviews. Reuse evidence only while its declared applicability remains true. Typed advice answers are externally authored context of kind `advice-answer`.
## Provider tools

Use `describe`/`evaluate`, `checkpoint`, `commission`, `review-candidates`, `prepare-validation`, `source-identity`, `author-counts`, and `data-dump`. `setup` edits policy only with `--set`, `--add-axis`, `--remove-axis`; use explain/drift to inspect policy changes. Tools prepare or evaluate data, not external execution. Keep frozen run obligations and original runtime; no active-run migration is authorized.

## Workspace validation

From the checkout root, the final-proof owner runs `scripts/baseline.sh` plus the union of boundary commands in checkout-root `AGENTS.md`. Workers use `cargo test -p <crate> <filter>` for focused unit tests or `python3 scripts/run-central-tests.py --filter NAME` for focused central integration tests. Inspect active catalogs before rebuilding frozen provider paths; retain exact argv, source identity and outcomes. Missing required proof blocks completion.

Draft or audit README/AGENTS through policy-document under the root guide's document procedure; target edits require fresh digest-bound evidence. Release generation uses only `python3 scripts/generate-release.py` and read-only `python3 scripts/generate-release.py --check`, followed by the independent strict graph assertions in the root release procedure. Git, dispatch and publication remain separately owner-authorized.
