# Bookends v1 required-CI collection grammar (frozen)

Eligibility is fail-closed from the named required jobs in `bookends.toml`.
The checker does not execute CI and does not interpret arbitrary workflow
languages. A citation counts only when its tracked, non-skipped file belongs
to a collection established by one of those jobs.

The adopting repository uses two closed command forms:

```text
cargo test --workspace
python3 <repo-relative-script>
```

The command is one GitHub Actions `run:` scalar, split on whitespace. The
form must match exactly. Shell wrappers, extra flags, multiple commands,
package or target filters, `python3 -m`, and other runner forms are unparsed
and do not establish eligibility. A `uses:` step never establishes a
collection.

`cargo test --workspace` collects the default Rust test targets of every
workspace package visible in the current tracked tree. The checker reads only
the small amount of Cargo target metadata needed to exclude targets declared
with `test = false`; it does not run Cargo or implement a general manifest
interpreter. Unit tests, binary tests, and default integration tests count.
A skipped-only target does not count.

`python3 <repo-relative-script>` collects exactly that one tracked script.
The path must be relative to the repository root, must not contain `..`, and
must not be an option or URL.

A `run:` with an effective GitHub Actions `working-directory` other than the
repository root is unparsed. Workflow, job, and step defaults are considered;
`working-directory: .` is the repository root. This prevents a command in a
nested checkout directory from being credited as the root-workspace or
root-script collection.

A required job must exist and contain at least one parsed command. A declared
class's pathspec surface must also resolve to at least one tracked file. Job
existence, pathspec matching, and a citation alone are not enough: the same
required job must establish a collection that includes the cited file.
