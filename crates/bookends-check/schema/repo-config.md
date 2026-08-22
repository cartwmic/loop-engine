# Bookends v1 repo config (frozen)

An enabling repository commits `bookends.toml` at the repository root. Presence
of that file is the enablement signal for the bookends layer in that
repository. Keys below are frozen. Unknown keys, unknown tables, and unknown
coverage-class table names are invalid.

## Required keys

```toml
prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**", "scripts/**", "crates/**/src/**"]
required_ci_jobs = ["baseline-and-source-journey"]
```

- `prd` is a repo-relative markdown path to the living git PRD. It is not an
  absolute path, not a URL, and must not contain parent-directory segments.
- `[classes.e2e_journey]` is required. It maps to the PRD coverage token
  `e2e/journey`.
- `pathspecs` is a git pathspec array. Each entry is a nonempty string. The
  array is nonempty. Surface-liveness is evaluated against these pathspecs:
  each declared class's discovery surface must resolve to at least one
  tracked file, or the check is red. For `e2e/journey`, tracked files under
  `crates/**/src/**` are discovery inputs only: citations in them, including
  comments inside internal Rust unit-test modules, are never eligible public
  journey coverage.
- `required_ci_jobs` is a workflow job-id array. Each entry is a nonempty
  string naming a GitHub Actions job `id` that exists in the tree. The array
  is nonempty. Eligibility uses those named jobs' `run:` commands against
  the runner grammar; job existence alone is not eligibility.

## Optional contract class

`[classes.contract]` is present only when the contract class is declared. It
uses the same keys as `[classes.e2e_journey]`:

```toml
[classes.contract]
pathspecs = ["contracts/**"]
required_ci_jobs = ["baseline-and-source-journey"]
```

Omitting `[classes.contract]` means the class is undeclared and unenforced.
A live PRD record must not list `contract` coverage unless this table is
present. v1 does not invent an OpenAPI-style contract surface so the class
has somewhere to point.

No other `[classes.*]` table is valid.

## What this file does not name

`bookends.toml` does not list proof files, test names, or citation tokens.
The PRD declares IDs and coverage classes. Proof files cite
`bookends:LE-<n>`. Discovery is the class pathspecs plus named-job command
collection.
