# Validation manifest v2

`manifest.schema.json` freezes tracked `quality/manifest.toml` schema version 2. Runtime parsing is implemented by `xtask::config` and rejects unknown TOML fields, enum values, placeholders, duplicate identifiers, non-positive bounds, invalid environment names, and repository-path escapes.

Configuration declares direct executable plus ordered argv values. It never declares shell strings, project command identifiers, runner enums, or plugin types. Supported phases are `pre-commit` and `publication`; supported scopes are `repository` and `changed-files`.

Only these placeholders are valid in executable, argv, cwd, and environment values:

- `{git_directory}`
- `{candidate_root}`
- `{scratch_root}`
- `{cache_root}`
- `{target_root}`
- `{base_revision}`
- `{candidate_revision}`
- `{candidate_tree}`

Configured cwd values must start at `{candidate_root}` and remain beneath it. Runtime code must additionally prove expanded cwd exists beneath exact materialized candidate root.

`xtask::config::compute_binding` is sole binding-digest API. It hashes exact manifest bytes retained by parsed document and exact candidate rubric bytes, returning rubric digests in repository-relative path order. Deterministic-only parsing permits absent semantic configuration. Publication and advisory callers must parse with required semantic configuration.

Tracked `quality/manifest.toml` is sole project-policy registry. It runs, in order, exact-object diff checking, Rust formatting/checking/Clippy/tests/docs, both Rust provider suites, Go 1.26.5 reference-provider tests through `mise`, and cargo-deny 0.20.2 for both deterministic phases. Default environment removes `RUSTUP_TOOLCHAIN`, redirects writable build/cache/temp output outside candidate source, and disables `mise` auto-install. `xtask/tests/manifest_policy.rs` is exact project-policy golden, including complete runner-input closure; generic candidate/runner code contains no project path registry.
