# Validation manifest v2

[`manifest.schema.json`](manifest.schema.json) freezes tracked [`quality/manifest.toml`](../../manifest.toml) schema version 2. Runtime parsing is implemented by `xtask::config` and rejects unknown TOML fields, enum values, placeholders, duplicate identifiers, non-positive bounds, invalid environment names, and repository-path escapes.

Configuration declares direct executable plus ordered argv values. It never declares shell strings, project command identifiers, runner enums, or plugins. Supported phases are `pre-commit` and `publication`; supported scopes are `repository` and `changed-files`.

Only these placeholders are valid in executable, argv, cwd, and environment values:

- `{git_directory}`
- `{candidate_root}`
- `{scratch_root}`
- `{cache_root}`
- `{target_root}`
- `{base_revision}`
- `{candidate_revision}`
- `{candidate_tree}`

Configured cwd values must start at `{candidate_root}` and remain beneath it. Runtime additionally proves expanded cwd exists beneath exact read-only materialized candidate root. Writable scratch, cache, and target roots remain outside candidate source.

`xtask::config::compute_binding` is sole binding-digest API. It hashes exact manifest bytes retained by parsed document and exact candidate rubric bytes, returning rubric digests in repository-relative path order. Deterministic-only parsing permits absent semantic configuration. Publication and advisory callers require complete semantic configuration.

Tracked [`quality/manifest.toml`](../../manifest.toml) is sole project-policy registry. It runs, in order, exact-object diff checking, Rust formatting/checking/Clippy/tests/docs, both Rust provider suites, Go 1.26.5 reference-provider tests through `mise`, and cargo-deny 0.20.2 for both deterministic phases. Default environment removes `RUSTUP_TOOLCHAIN`, redirects writable build/cache/temp output outside candidate source, and sets both `MISE_AUTO_INSTALL=false` and `MISE_AUTO_INSTALL_DISABLE_TOOLS=go`. Missing or version-mismatched prerequisites fail with install hints and are never installed by validation.

Owner entry points:

```bash
cargo xtask validate --staged
cargo xtask validate --semantic --base <base-revision> --candidate HEAD
cargo xtask validate --publication --updates-stdin
cargo xtask validate --publication --ci-event <path>
```

Staged mode validates exact index tree. Advisory mode requires candidate resolve to current `HEAD`, accepts any base revision, runs complete `publication` deterministic phase before semantic review, writes evaluation only, and ignores approvals. Publication modes produce one aggregate attempt. Exact hook, evidence, approval, and retry behavior is documented in [`docs/development-policy.md`](../../../docs/development-policy.md).

`xtask/tests/manifest_policy.rs::final_manifest_is_exact_project_policy_registry` is exact project-policy golden, including complete runner-input closure. Generic candidate/runner code contains no project path registry.
