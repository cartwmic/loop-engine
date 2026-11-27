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
required_ci_jobs = ["tests"]
```

- `prd` is a repo-relative markdown path to the living git PRD. It is not an
  absolute path, not a URL, and must not contain parent-directory segments.
- `[classes.e2e_journey]` is required. It maps to the PRD coverage token
  `e2e/journey`.
- `pathspecs` is a git pathspec array. Each entry is a nonempty string. The
  array is nonempty. Surface-liveness is evaluated against these pathspecs:
  each declared class's discovery surface must resolve to at least one
  tracked file, or the check is red. Pathspecs are the approved proof locations; they do not
  make every file under a directory a collected test. Documentation/data files
  and paths containing `doc`, `docs`, `documentation`, `example`,
  `examples`, `fixture`, `fixtures`, `generated`, or `vendor` are ignored as
  proof surfaces. For `e2e/journey`,
  tracked files under `crates/**/src/**` are discovery inputs only: citations
  in them, including comments inside internal Rust unit-test modules, are
  never eligible public journey coverage.
- `required_ci_jobs` is a workflow job-id array. Each entry is a nonempty
  string naming a GitHub Actions job `id` that exists in the tree. The array
  is nonempty. Eligibility uses those named jobs' `run:` commands against
  the runner grammar; job existence alone is not eligibility.

## Publication checking

The ordinary checker reads the current working tree. The pre-push/CI
publication mode instead receives Git's four-field ref-update stream and reads
all config, PRD, workflow, and proof bytes from immutable commit trees. For each
update it checks commits reachable from the new object but not the old object;
a zero old object means all reachable history, and every commit is compared with
each recorded parent. A pre-adoption commit without the enabling config is not
an enabled snapshot; once a PRD has been adopted, removing its config or PRD is
a failure.

Missing history is fetched from the supplied source remote with ordinary
non-pruning fetches that do not update local branches. A failed fetch,
interrupted or bounded walk, or changed local HEAD/index/worktree is incomplete
and cannot produce `GREEN`. Publication invocations retain a write-once range
and completeness receipt outside the repository.

Every `check` or `candidate` run also appends one JSON line
at `$XDG_STATE_HOME/bookends-check/log.jsonl` (else
`~/.local/state/bookends-check/log.jsonl`), whatever `--receipt-root` says.
`--help`, `--version` and usage errors write no line. Fields:
`ts` (UTC), `action` (`publication`, `check` or `candidate`), `outcome`
(`green`, `red`, `bypass`, `refused` when a receipt cannot be written, or
`error`), `duration_ms`, `id`, and for replay `repo`, `remote`, `refs`,
`commit`, `candidate`, `findings`, `finding_text`, `commits_checked`,
`complete`, `bypass_class`, `receipt` and `error`. The `id` is also the
`run_id` in the receipt. A failed log write is reported on stderr and never
changes the verdict. Bypass reasons are not logged.

Two fields name what a `check` or `candidate` run looked at, because those runs
write no receipt: `commit` is the commit `HEAD` named when a `check` run read
the work tree (`null` for other actions, and for a directory with no commit;
the work tree may also hold uncommitted changes), and `candidate` is the path
given to a `candidate` run (`-` for standard input; `null` for other actions).
For a `check` or `candidate` run, `finding_text` lists every finding the run
printed, in full and in order, so its length equals `findings`; it is empty for
a green run. For a `publication` run it is always empty: the receipt holds the
finding text in `diagnostics`.

## Optional contract class

`[classes.contract]` is present only when the contract class is declared. It
uses the same keys as `[classes.e2e_journey]`:

```toml
[classes.contract]
pathspecs = ["contracts/**"]
required_ci_jobs = ["tests"]
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
