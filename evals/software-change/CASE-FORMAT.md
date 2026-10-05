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
