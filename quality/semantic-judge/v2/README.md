# Semantic judge v2 executable protocol

This directory defines language-neutral JSON-over-stdio contract used by validation runner. Judge implementation may use any language or model provider.

## Process contract

Runner starts configured program directly, with fixed configured argv, candidate-root-contained cwd, typed inherited/set/unset environment, and one canonical UTF-8 JSON request on stdin. Program writes exactly one UTF-8 JSON response to stdout. Diagnostic text belongs on stderr. Unknown, duplicate, or missing fields fail validation.

`schema_version` is `2`. Every response must exactly echo `request_kind`, `axis_id`, `base_revision`, `candidate_revision`, and `candidate_tree` from request. Status is `pass`, `block`, `indeterminate`, or `unavailable`. `unavailable` has no citations; every other status has at least one citation.

Citation `kind` selects supplied authority:

- `rubric`: `reference` equals current rubric ID;
- `candidate`: `reference` is `diff` or one supplied resulting-file path;
- `deterministic_evidence`: `reference` is supplied prerequisite/check ID;
- `axis_result`: coherence only; `reference` is one supplied focused-axis ID.

`detail` names exact rule, line, observation, or conflict. Citations outside supplied request fail closed.

## Request kinds

- `axis`: one focused rubric, shared exact revision/tree binding, exact diff bytes, resulting changed-file content, and deterministic evidence. `axis_results` is empty.
- `coherence`: coherence rubric plus four normalized focused results. Runs only after all focused invocations finish. It may add blocker but cannot alter focused statuses.
- `correction`: same logical input plus exact invalid response and validation error. `original_request_kind` is `axis` or `coherence`.

Malformed successful output receives exactly one `correction` invocation. Both attempts share original timeout budget and same writable scratch root. Timeout, spawn failure, nonzero exit, output-limit failure, cancellation, or malformed correction output normalize to `unavailable` without another child.

## Scheduling and authority

Exactly four focused axes come only from `quality/manifest.toml`. Runner executes them concurrently in manifest order for normalized output. Each gets only own rubric and distinct candidate-external writable scratch root. Coherence gets another distinct root and runs last.

After every semantic child runner verifies candidate source bytes, paths, modes, symlinks, and sealed permissions against bound tree. Mutation cancels and awaits active sibling groups, suppresses later correction/coherence children, and synthesizes complete `unavailable` records. Coherence mutation also blocks.

Rust runner mechanically derives `pass` only when all four focused statuses and coherence are `pass`. Every `block`, `indeterminate`, or `unavailable` yields `semantic_block`; judge never chooses gate decision or approval eligibility.

Schemas are [`request.schema.json`](request.schema.json) and [`response.schema.json`](response.schema.json).
