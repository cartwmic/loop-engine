# Software-change eval cases

This format is software-change-specific. The general campaign core consumes only
case IDs, versions and host paths. Cases are trusted, owner-approved code, not an
untrusted uploaded-test service. Only this small example and its intentionally
broken test fixture belong in the public checkout; real cases stay private.

## Layout (version 1)

```text
case.toml
public/request.md           deliberately vague owner request
public/repo/                starting repository files (no symlinks)
hidden/brief.md             host-only owner brief
hidden/tests/test_*.py      stdlib unittest tests
hidden/reference/          complete replacement/new files, relative to repo root
```

`case.toml` has three required fields:

```toml
id = "label-keys"
version = "1"
allowed_edit_scope = ["labels.py"]
```

IDs are stable safe identifiers. Versions are nonempty strings; change the version
when case content changes. Scope entries are repository-relative, case-sensitive
`fnmatchcase` patterns (not Git pathspecs; `*` can match `/`). Absolute paths,
traversal components, backslashes, `.git` components and symlinks are not supported.
The reference is a file overlay, not a patch or an entire second checkout. Deletion
is not supported in v1. Every reference file must match allowed scope. Trial
scorers must separately check the delivered model's edits against that scope.

## Requirements and tests

Each brief requirement starts with a unique level-two heading `## REQ-1: Title`
(or another `REQ-` ID using uppercase letters, digits and hyphens). Brief prose
states the behavior and what questions should reveal it. The simulator reads this
file only on the host; do not put it into a trial container or driver prompt.
The example's exact Unicode casefolding and `inbox` blank-label policy are hidden.

Every test method has exactly one literal `@requirement("REQ-1")` decorator,
imported with `from case_runtime import requirement`, and every brief ID has at
least one test. Use stdlib `unittest.TestCase` in `test_*.py`; no test dependency
installation is required. Tests import the submitted program from the repository
root. Test subdirectories need `__init__.py` for unittest discovery. Collection
failures, duplicate/missing tags, skips, expected failures and unexecuted tests
cannot make a reference pass. Tests must finish within 60 seconds for case-check.

The harness copies `scevals/case_runtime.py` alongside the tests. Invoke it from
the submitted repository's working directory:

```sh
python3 /opt/case-check/case_runtime.py --tests /opt/case-check/tests --output /tmp/case-results.json
```

The JSON contains `collection_error` and `tests`, each with `test_id`,
`requirement_id`, and `status` (`passed`, `failed`, `error`, `skipped`, `not_run`).
Exceptions/source/output are not included. Process exit 0 means the runner wrote
a report, **not** that tests passed; scorers must inspect every status and reject
collection/count mismatches. Requirement tags expose separate per-requirement
outcomes; T5 owns delivery-recall calculation.

## Validator and reusable interfaces

From the checkout root:

```sh
uv run --project evals evalctl case-check evals/software-change/example-case
uv run --project evals evalctl case-check --expect-invalid evals/software-change/tests/fixtures/broken-case
```

`case-check` builds the pinned T2 trial image (cached layers reused), then creates
separate mount-free containers for reference and untouched/no-op attempts. It
copies only public repository files, tests, runner and (reference attempt only)
the reference overlay; the request and brief are not needed for these checks.
Every reference test must pass; at least one executed no-op test must fail/error.
It prints per-requirement counts, `VALID`/`INVALID` and a `CASE_CHECK` JSON receipt
with pinned image identity, mounts and copy logs. Default invalid exit is 1;
`--expect-invalid` inverts validity only, never infrastructure/build failures
(which exit 2 through evalctl). `--ref`/`--tag` choose a different pinned image.
Keep stdout/receipts under ignored `captures/`, especially for private cases.

T4/T5 can import `scevals.cases.load_case(Path) -> EvalCase`: fields `path`, `id`,
`version`, `allowed_edit_scope`, `requirement_ids`, `test_requirements` (one ID per
test); properties `repository`, `request`, `brief`, `tests`, `reference` are host
paths. Loading parses TOML/AST/brief headings without executing case code on the
host. `check_results(case, reference, noop)` performs the validator's result
checks. `run_hidden_tests` creates its own disposable Docker container and is
**only for case-check**: campaign scoring must copy/run tests in the delivered
trial's terminal sandbox instead, and enforce the terminal-state gate itself.
Case-check deliberately bypasses that workflow gate because it validates a case,
not a model trial. It does not prove a complete software-change workflow.

## Optional reader

Some requirements are about what a short text promises (an error message, a line of a document), where a word
list in a test is brittle. A case can instead declare a **reader**: a model fills a small form for each such text,
the harness votes the readings, and the tests grade the form against what the program really does. A case with no
`[reader]` table behaves exactly as described above.

### Declaration

```toml
[reader]
model     = "anthropic/claude-haiku-5-5"      # an Inspect model id; part of the case, so changing it is a version bump
guide     = "hidden/reader/guide.md"          # the system prompt; host only
shapes    = "hidden/reader/form-shapes.json"  # the shape of each kind's form; host only
collector = "collect_texts.py"                # a file name in hidden/tests; runs in the scoring sandbox
saved     = "hidden/reader/saved-forms.json"  # forms for case-check; may not exist until --save-forms writes it
```

All five keys are required. `load_case` reads them as data and refuses a path outside the case, a missing
guide, shapes or collector file, a collector name with a slash, or shapes that do not parse. It does not require
`saved` to exist. Everything under `hidden/reader/` stays on the host and is never copied into a container.

### The shapes file

A JSON object with one entry per **kind**. A leaf is `{"one_of": [...]}` (one of these strings) or
`{"set_of": [...]}` (any subset, a list of strings). Any other object nests more entries.

```json
{"message": {"offers": {"set_of": ["ask", "grant", "no_key", "other"]}},
 "bullet":  {"ask": {"loads": {"one_of": ["yes", "no", "not_said", "mixed"]}}}}
```

A reading is valid only if it has exactly the shape's keys and allowed values.

### The collector

The harness runs `python3 <tests dir>/<collector>` in the scoring sandbox after the trial has stopped, with the
delivered repository as the working directory (the same one the tests get). It prints one JSON object and nothing
else:

```json
{"texts": [{"id": "grant-message", "kind": "message", "text": "…"},
           {"id": "contract-bullet", "kind": "bullet", "text": null}]}
```

`id` is unique, `kind` is a key of the shapes file and `text` is a string or `null`. The collector must not
raise: a missing text, a refusal that printed nothing or a crashed tool gives `null`, which fails the tests that
need that form but is never an infrastructure failure. A collector that exits non-zero, prints anything but this
shape or names an unknown kind is a case bug: an infrastructure failure in a campaign, `INVALID` in case-check.
A collector may normalize what it prints (for example, replace a scratch path) so the same delivery gives the same
text on every run.

`case-check` also scans the hidden tests, collector included, for names the case never states. The reader's own
keys (`texts`, `kind`, `forms`, `form`) are not counted. A name the collector prints that the case's request, brief
or repository does not state (a text `id` such as `grant-message`) goes in `interface_allowlist` in `case.toml`.

### Reading and voting

For each non-null text the harness calls the pinned model three times (the guide as the system prompt, a JSON
object `{"kind", "text"}` as the user message; temperature 0.7, 8192 tokens, a 120 s timeout and two retries per
call, at most four calls at once). Inspect drops the temperature for models that refuse it, such as Haiku 5.5. Prompt
caching is off, because the Anthropic proxy refuses Inspect's cache headers. Each reply is parsed (the last fenced
JSON block in it, or if there is none, the last JSON object), validated against the shape and voted leaf by leaf: a `one_of` leaf needs a strict majority, a
`set_of` leaf votes per element, at least two readings must parse, and any leaf with no majority leaves the text
unreadable.

The voted forms are written to `<tests dir>/forms.json` before the tests run:

```json
{"forms": {"grant-message": {"form": {"offers": ["ask"]}}, "contract-bullet": {"form": null}}}
```

Only forms are written, never texts or raw readings. A test reads its form from this file and fails when the entry
is `null`.

### Failure rules

| What happened | Campaign scoring | `case-check` |
|---|---|---|
| Collector fails or prints the wrong shape | infrastructure failure | `INVALID` |
| A text is `null` | no reading, form `null`; the tests that need it fail | same |
| A model call fails after its retries, or the model cannot be built (no API key, unknown provider) | infrastructure failure | exit 2, nothing written |
| A text has fewer than two parsed readings, or a leaf has no majority | infrastructure failure naming the text ids; the readings stay in the capture folder, but are not reused | exit 2, nothing written (`--save-forms`) |
| The reader step passes 600 s | infrastructure failure | not capped |

An infrastructure failure never counts against the trial: it is left out of the success rate. The campaign does not retry it by itself.

### Cost, metrics and rescoring

The reader's calls are not a workflow role, so they are reported and never counted against a campaign's dollar
stop. `hidden_tests.reader` in a trial's metrics holds the model, a settings hash, the number of texts, calls and
tokens, failed readings and the leaves that split (`<id>:<leaf>`); it holds no text.

The readings of each text (the text, the raw replies, the form and the split leaves) are kept in the trial's capture folder as `reader-<key>.json`, where the key hashes the
kind, the text and the settings (model, temperature, tokens, readings, guide and shapes). A rescore reuses a file
whose key matches, so it gets the same forms with no model call; any other key reads again. Writes are atomic. A file that
holds a valid reading is never replaced; one that cannot be parsed, or that holds no form, is. A trial with no capture folder, or a folder that cannot be written, keeps no cache and scoring goes on.

### `case-check` and saved forms

`case-check` runs the collector in the reference and the no-op containers and looks each text up in `saved` by its
SHA-256. It makes no model call. It is `INVALID` ("saved readings are stale; run case-check --save-forms") when the
saved file is missing, its settings hash differs from today's, or any text's hash or entry differs.
`case-check --save-forms` reads each non-null text with the model, rewrites `saved` and then checks as usual.

```json
{"settings_sha256": "…",
 "scenarios": {"reference": {"grant-message": {"text_sha256": "…", "form": {"offers": ["ask"]}}},
               "noop": {"grant-message": {"text_sha256": null, "form": null}}}}
```

### `evalctl read-texts`

A case's own reader check (many texts, graded against labels) calls the same reader through the harness:

```sh
uv run --project evals evalctl read-texts CASE --in texts.json --out readings.json
```

Input is the collector's shape, with a unique string `id` and a nonempty string `text` for every entry (`null` is
refused: this command reads, it does not collect). Output is, in input order:

```json
{"settings_sha256": "…", "model": "anthropic/claude-haiku-5-5",
 "results": [{"id": "…", "kind": "…", "form": {"offers": ["ask"]},
              "readings": [{"offers": ["ask"]}, {"offers": ["ask"]}, null],
              "failed": 1, "split": [],
              "usage": {"calls": 3, "input_tokens": 1830, "output_tokens": 210}}]}
```

`form` is `null` when the text has no majority, which is data here and not a failure. `readings` always has three
entries (`null` for one that did not parse). `split` lists the leaves the parsed readings disagreed on. Exit
codes: `0` written; `2` the case has no `[reader]`, the input is not this shape, an id repeats or a kind is
unknown; `3` a model call failed after its retries and nothing is written. The command always calls the model; it
uses no cache and no saved forms.

The model is reached through Inspect, so the `anthropic` package must be installed (it is pinned in
`evals/pyproject.toml`) and `ANTHROPIC_API_KEY` set.
