//! Shared, fail-closed binary handoff for the workspace integration suite.
//!
//! The central integration target cannot use Cargo's same-package binary
//! environment values: Cargo provides those only for binaries owned by the
//! same package.  The repository runner therefore builds the production and
//! reference-fixture binaries first and writes their exact paths to a handoff
//! file.  Resolution is deliberately strict: it never searches
//! `target/debug/deps`, accepts no
//! pre-existing fallback, and validates every path before a test can spawn it.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

pub const HANDOFF_ENV: &str = "LOOP_ENGINE_TEST_BINARY_HANDOFF";
const HANDOFF_SCHEMA_VERSION: u64 = 1;

#[derive(Debug)]
struct RequiredBinary {
    package: &'static str,
    target: &'static str,
    alias: &'static str,
}

const REQUIRED_BINARIES: &[RequiredBinary] = &[
    RequiredBinary {
        package: "bookends-check",
        target: "bookends-check",
        alias: "bookends-check",
    },
    RequiredBinary {
        package: "loop-cli",
        target: "loop-engine",
        alias: "loop-engine",
    },
    RequiredBinary {
        package: "policy-document-provider",
        target: "policy-document",
        alias: "policy-document",
    },
    RequiredBinary {
        package: "research-provider",
        target: "research",
        alias: "research",
    },
    RequiredBinary {
        package: "software-change-provider",
        target: "software-change",
        alias: "software-change",
    },
    RequiredBinary {
        package: "loop-reference-fixtures",
        target: "policy-document-provider",
        alias: "fixture:policy-document-provider",
    },
    RequiredBinary {
        package: "loop-reference-fixtures",
        target: "research-provider",
        alias: "fixture:research-provider",
    },
    RequiredBinary {
        package: "loop-reference-fixtures",
        target: "software-change-provider",
        alias: "fixture:software-change-provider",
    },
];

const FRESH_BUILD_ARGS: &[&str] = &[
    "build",
    "--locked",
    "--message-format=json",
    "-p",
    "bookends-check",
    "-p",
    "loop-cli",
    "-p",
    "policy-document-provider",
    "-p",
    "research-provider",
    "-p",
    "software-change-provider",
    "-p",
    "loop-reference-fixtures",
    "--bins",
];

#[derive(Debug)]
struct FreshBinary {
    spec: &'static RequiredBinary,
    path: PathBuf,
    sha256: String,
}

#[derive(Debug)]
struct FreshBinaries {
    _target: TempDir,
    active_target: PathBuf,
    entries: HashMap<&'static str, FreshBinary>,
}

static FRESH_BINARIES: OnceLock<Result<FreshBinaries, String>> = OnceLock::new();

/// Return the repository root containing this dedicated test package.
pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace-integration package must be under the repository root")
}

/// Return the root of one workspace package by its Cargo package name.
pub fn package_root(package: &str) -> PathBuf {
    if package == "loop-reference-fixtures" {
        repository_root().join("tests/fixtures")
    } else {
        repository_root().join("crates").join(package)
    }
}

/// Resolve a production CLI by its stable executable name.
///
/// Panicking here is intentional.  A missing or invalid handoff is a test
/// setup error, and allowing `Command` to report a later spawn failure would
/// make it possible to accidentally run a suite against an unverified path.
pub fn binary(name: &str) -> PathBuf {
    resolve_binary(name).unwrap_or_else(|error| {
        panic!("central binary resolver refused `{name}`: {error}");
    })
}

/// Resolve one of the three reference-provider fixture executables.
pub fn fixture_binary(name: &str) -> PathBuf {
    let key = format!("fixture:{name}");
    resolve_binary(&key).unwrap_or_else(|error| {
        panic!("central binary resolver refused `{key}`: {error}");
    })
}

/// Return a resolved executable as the string required by a persisted command
/// binding or a JSON request.
pub fn binary_string(name: &str) -> String {
    binary(name).to_string_lossy().into_owned()
}

/// Resolve a handoff selected by the supported runner's environment.
pub fn resolve_binary(name: &str) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(HANDOFF_ENV) {
        return resolve_binary_from_handoff(name, &PathBuf::from(path));
    }

    let spec = required_binary(name).ok_or_else(|| {
        format!("no fresh-build binary is declared for `{name}`; refusing fallback lookup")
    })?;
    let result = FRESH_BINARIES.get_or_init(resolve_fresh_binaries);
    match result {
        Ok(binaries) => {
            let entry = binaries.entries.get(name).ok_or_else(|| {
                format!("fresh binary build has no entry for `{name}`; refusing fallback lookup")
            })?;
            validate_fresh_binary(spec, entry, &binaries.active_target)
        }
        Err(error) => Err(error.clone()),
    }
}

fn required_binary(name: &str) -> Option<&'static RequiredBinary> {
    REQUIRED_BINARIES.iter().find(|spec| spec.alias == name)
}

fn resolve_fresh_binaries() -> Result<FreshBinaries, String> {
    let target = tempfile::Builder::new()
        .prefix("loop-engine-workspace-binaries-")
        .tempdir()
        .map_err(|error| format!("could not create fresh Cargo target directory: {error}"))?;
    let active_target = target.path().canonicalize().map_err(|error| {
        format!(
            "could not canonicalize fresh Cargo target directory {}: {error}",
            target.path().display()
        )
    })?;
    let binaries = build_fresh_binaries(Path::new("cargo"), &active_target)?;
    Ok(FreshBinaries {
        _target: target,
        active_target,
        entries: binaries,
    })
}

fn build_fresh_binaries(
    cargo: &Path,
    active_target: &Path,
) -> Result<HashMap<&'static str, FreshBinary>, String> {
    let active_target = active_target.canonicalize().map_err(|error| {
        format!(
            "fresh Cargo target directory {} is unavailable: {error}",
            active_target.display()
        )
    })?;
    if !active_target.is_dir() {
        return Err(format!(
            "fresh Cargo target directory {} is not a directory",
            active_target.display()
        ));
    }
    if fs::read_dir(&active_target)
        .map_err(|error| {
            format!(
                "could not inspect fresh Cargo target directory {}: {error}",
                active_target.display()
            )
        })?
        .next()
        .is_some()
    {
        return Err(format!(
            "fresh Cargo target directory {} is not empty; refusing stale artifacts",
            active_target.display()
        ));
    }

    let repository = repository_root();
    let output = Command::new(cargo)
        .args(FRESH_BUILD_ARGS)
        .current_dir(&repository)
        .env("CARGO_TARGET_DIR", &active_target)
        .env_remove(HANDOFF_ENV)
        .output()
        .map_err(|error| format!("could not run fresh Cargo binary build: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            format!(
                "fresh Cargo binary build failed with exit status {}",
                output.status
            )
        } else {
            format!(
                "fresh Cargo binary build failed with exit status {}: {detail}",
                output.status
            )
        });
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("fresh Cargo binary build emitted non-UTF-8 output: {error}"))?;
    let artifacts = fresh_binary_artifacts(&stdout)?;
    let mut entries = HashMap::new();
    for spec in REQUIRED_BINARIES {
        let artifact = artifacts.get(&(spec.package, spec.target)).ok_or_else(|| {
            format!(
                "fresh Cargo binary build did not produce required {}/{}",
                spec.package, spec.target
            )
        })?;
        let path = validate_fresh_artifact(spec, artifact, &active_target)?;
        let sha256 = hex_digest(&path)?;
        entries.insert(spec.alias, FreshBinary { spec, path, sha256 });
    }
    Ok(entries)
}

#[derive(Debug)]
struct FreshArtifact {
    executable: PathBuf,
}

fn fresh_binary_artifacts(
    stdout: &str,
) -> Result<HashMap<(&'static str, &'static str), FreshArtifact>, String> {
    let mut artifacts = HashMap::new();
    for (line_number, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!(
                "fresh Cargo binary build emitted invalid JSON at line {}: {error}",
                line_number + 1
            )
        })?;
        if value.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let target = value.get("target").and_then(Value::as_object);
        let Some(target) = target else {
            continue;
        };
        let kind = target.get("kind").and_then(Value::as_array);
        if kind != Some(&vec![Value::String("bin".to_owned())]) {
            continue;
        }
        let manifest = value
            .get("manifest_path")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let target_name = target.get("name").and_then(Value::as_str);
        let Some(manifest) = manifest else {
            continue;
        };
        let Some(target_name) = target_name else {
            continue;
        };
        let Some(spec) = REQUIRED_BINARIES
            .iter()
            .find(|spec| spec.target == target_name && manifest_matches(spec.package, &manifest))
        else {
            continue;
        };
        if value.get("fresh").and_then(Value::as_bool) != Some(false) {
            return Err(format!(
                "fresh Cargo binary build marked required {}/{} as fresh; refusing stale output",
                spec.package, spec.target
            ));
        }
        let executable = value
            .get("executable")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                format!(
                    "fresh Cargo binary build omitted executable for required {}/{}",
                    spec.package, spec.target
                )
            })?;
        if artifacts
            .insert((spec.package, spec.target), FreshArtifact { executable })
            .is_some()
        {
            return Err(format!(
                "fresh Cargo binary build emitted duplicate required {}/{} artifacts",
                spec.package, spec.target
            ));
        }
    }
    Ok(artifacts)
}

fn manifest_matches(package: &str, manifest: &Path) -> bool {
    let expected = package_root(package).join("Cargo.toml");
    match (expected.canonicalize(), manifest.canonicalize()) {
        (Ok(expected), Ok(manifest)) => expected == manifest,
        _ => false,
    }
}

fn validate_fresh_artifact(
    spec: &'static RequiredBinary,
    artifact: &FreshArtifact,
    active_target: &Path,
) -> Result<PathBuf, String> {
    let active_target = active_target.canonicalize().map_err(|error| {
        format!(
            "fresh Cargo target {} is unavailable: {error}",
            active_target.display()
        )
    })?;
    let debug = active_target.join("debug");
    let direct = debug.join(format!("{}{EXE_SUFFIX}", spec.target));
    let canonical_debug = debug.canonicalize().map_err(|error| {
        format!(
            "fresh Cargo output directory {} is unavailable for {}/{}: {error}",
            debug.display(),
            spec.package,
            spec.target
        )
    })?;
    if canonical_debug != debug {
        return Err(format!(
            "fresh Cargo output directory {} is outside the direct target layout",
            debug.display()
        ));
    }
    let canonical_direct = direct.canonicalize().map_err(|error| {
        format!(
            "fresh Cargo build did not produce direct {}/{} output {}: {error}",
            spec.package,
            spec.target,
            direct.display()
        )
    })?;
    if canonical_direct != direct {
        return Err(format!(
            "fresh Cargo direct {}/{} output {} is a symlink; refusing fallback",
            spec.package,
            spec.target,
            direct.display()
        ));
    }
    if !canonical_direct.starts_with(&active_target)
        || canonical_direct
            .strip_prefix(&active_target)
            .map_err(|_| "fresh Cargo output escaped active target".to_owned())?
            .components()
            .any(|component| component.as_os_str() == "deps")
    {
        return Err(format!(
            "fresh Cargo direct {}/{} output {} is outside active target or under deps",
            spec.package,
            spec.target,
            canonical_direct.display()
        ));
    }
    let executable = artifact.executable.canonicalize().map_err(|error| {
        format!(
            "fresh Cargo artifact path for {}/{} {} is unavailable: {error}",
            spec.package,
            spec.target,
            artifact.executable.display()
        )
    })?;
    if executable != canonical_direct {
        return Err(format!(
            "fresh Cargo artifact path for {}/{} is not direct output {} (got {})",
            spec.package,
            spec.target,
            canonical_direct.display(),
            executable.display()
        ));
    }
    if !is_executable(&canonical_direct) {
        return Err(format!(
            "fresh Cargo direct {}/{} output {} is not executable",
            spec.package,
            spec.target,
            canonical_direct.display()
        ));
    }
    Ok(canonical_direct)
}

fn validate_fresh_binary(
    spec: &'static RequiredBinary,
    entry: &FreshBinary,
    active_target: &Path,
) -> Result<PathBuf, String> {
    if entry.spec.package != spec.package || entry.spec.target != spec.target {
        return Err(format!(
            "cached fresh binary identity mismatch for `{}`",
            spec.alias
        ));
    }
    let artifact = FreshArtifact {
        executable: entry.path.clone(),
    };
    let path = validate_fresh_artifact(spec, &artifact, active_target)?;
    let actual_sha256 = hex_digest(&path)?;
    if actual_sha256 != entry.sha256 {
        return Err(format!(
            "cached fresh binary `{}` digest mismatch: expected {}, got {}",
            spec.alias, entry.sha256, actual_sha256
        ));
    }
    Ok(path)
}

const EXE_SUFFIX: &str = std::env::consts::EXE_SUFFIX;

/// Resolve a named entry from an explicit handoff file.
///
/// This function is public so the central package's black-box contract tests
/// can exercise the same path validation used by every subprocess test.
pub fn resolve_binary_from_handoff(name: &str, handoff_path: &Path) -> Result<PathBuf, String> {
    let bytes = fs::read(handoff_path).map_err(|error| {
        format!(
            "could not read binary handoff {}: {error}",
            handoff_path.display()
        )
    })?;
    let handoff: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "binary handoff {} is not valid JSON: {error}",
            handoff_path.display()
        )
    })?;

    let object = handoff
        .as_object()
        .ok_or_else(|| "binary handoff must be a JSON object".to_owned())?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(HANDOFF_SCHEMA_VERSION) {
        return Err(format!(
            "binary handoff schema_version must be {HANDOFF_SCHEMA_VERSION}"
        ));
    }
    let active_target = absolute_path(object, "active_target")?;
    let active_target = active_target.canonicalize().map_err(|error| {
        format!(
            "binary handoff active_target {} is unavailable: {error}",
            active_target.display()
        )
    })?;
    if !active_target.is_dir() {
        return Err(format!(
            "binary handoff active_target {} is not a directory",
            active_target.display()
        ));
    }

    let entries = object
        .get("binaries")
        .and_then(Value::as_object)
        .ok_or_else(|| "binary handoff binaries must be an object".to_owned())?;
    let entry = entries.get(name).ok_or_else(|| {
        format!("binary handoff has no entry for `{name}`; no fallback search is permitted")
    })?;
    let entry_object = entry
        .as_object()
        .ok_or_else(|| format!("binary handoff entry `{name}` must be an object"))?;
    let target = entry_object
        .get("target")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("binary handoff entry `{name}` lacks a target name"))?;
    let path = absolute_path(entry_object, "path")?;
    let canonical_path = path.canonicalize().map_err(|error| {
        format!(
            "binary handoff entry `{name}` path {} is unavailable: {error}",
            path.display()
        )
    })?;
    if !canonical_path.is_file() {
        return Err(format!(
            "binary handoff entry `{name}` path {} is not a file",
            path.display()
        ));
    }
    if !canonical_path.starts_with(&active_target) {
        return Err(format!(
            "binary handoff entry `{name}` path {} is outside active target {}",
            canonical_path.display(),
            active_target.display()
        ));
    }
    let relative_path = canonical_path
        .strip_prefix(&active_target)
        .map_err(|_| format!("binary handoff entry `{name}` path escaped active target"))?;
    if relative_path
        .components()
        .any(|component| component.as_os_str() == "deps")
    {
        return Err(format!(
            "binary handoff entry `{name}` path {} is under target/deps; hashed/deps fallback selection is rejected",
            canonical_path.display()
        ));
    }
    let file_name = canonical_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("binary handoff entry `{name}` path has no UTF-8 file name"))?;
    if file_name != target && file_name != format!("{target}.exe") {
        return Err(format!(
            "binary handoff entry `{name}` path {} is not the direct `{target}` build output; hashed/deps fallback selection is rejected",
            canonical_path.display()
        ));
    }
    if !is_executable(&canonical_path) {
        return Err(format!(
            "binary handoff entry `{name}` path {} is not executable",
            canonical_path.display()
        ));
    }
    let expected_sha256 = entry_object
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| format!("binary handoff entry `{name}` lacks a valid sha256"))?;
    let actual_sha256 = hex_digest(&canonical_path)?;
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "binary handoff entry `{name}` digest mismatch: expected {expected_sha256}, got {actual_sha256}"
        ));
    }
    Ok(canonical_path)
}

fn hex_digest(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not hash binary handoff path {}: {error}",
            path.display()
        )
    })?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn absolute_path(object: &serde_json::Map<String, Value>, key: &str) -> Result<PathBuf, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("binary handoff `{key}` must be a non-empty absolute path"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!(
            "binary handoff `{key}` must be an absolute path: {value}"
        ));
    }
    Ok(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

    fn handoff(root: &Path, path: &Path, target: &str, include_entry: bool) -> PathBuf {
        let mut binaries = serde_json::Map::new();
        if include_entry {
            binaries.insert(
                "loop-engine".to_owned(),
                json!({
                    "package": "loop-cli",
                    "target": target,
                    "path": path,
                    "sha256": hex_digest(path).expect("candidate digest"),
                }),
            );
        }
        let handoff = root.join("handoff.json");
        fs::write(
            &handoff,
            serde_json::to_vec(&json!({
                "schema_version": HANDOFF_SCHEMA_VERSION,
                "repository_root": root,
                "active_target": root.join("target"),
                "binaries": binaries,
            }))
            .expect("serialize handoff"),
        )
        .expect("write handoff");
        handoff
    }

    fn active_root() -> (TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let active = root.path().join("target/debug");
        fs::create_dir_all(&active).expect("active target");
        (root, active)
    }

    #[cfg(unix)]
    fn fake_cargo(root: &Path, mode: &str) -> PathBuf {
        let script = root.join("cargo");
        let mut body = String::from("#!/bin/sh\nset -eu\n");
        body.push_str(
            "if [ -n \"${FAKE_CARGO_COUNT:-}\" ]; then\n  count=0\n  if [ -f \"$FAKE_CARGO_COUNT\" ]; then count=$(cat \"$FAKE_CARGO_COUNT\"); fi\n  printf '%s' $((count + 1)) > \"$FAKE_CARGO_COUNT\"\nfi\n",
        );
        if mode == "failed" {
            body.push_str("exit 19\n");
        } else {
            body.push_str("mkdir -p \"$CARGO_TARGET_DIR/debug\"\n");
            for spec in REQUIRED_BINARIES {
                let (executable, create_outside) = match mode {
                    "deps" => (
                        format!("$CARGO_TARGET_DIR/debug/deps/{}-deadbeef", spec.target),
                        false,
                    ),
                    "outside" => (
                        format!("$CARGO_TARGET_DIR/../outside-{}", spec.target),
                        true,
                    ),
                    _ => (format!("$CARGO_TARGET_DIR/debug/{}", spec.target), false),
                };
                if create_outside {
                    body.push_str("mkdir -p \"$CARGO_TARGET_DIR/../outside\"\n");
                }
                if mode == "deps" {
                    body.push_str("mkdir -p \"$CARGO_TARGET_DIR/debug/deps\"\n");
                }
                let executable_suffix = if create_outside {
                    format!("$CARGO_TARGET_DIR/../outside-{}", spec.target)
                } else {
                    executable
                };
                if mode != "missing" || spec.target != "loop-engine" {
                    body.push_str(&format!(
                        "printf '%s\\n' '#!/bin/sh\\nexit 0' > \"{executable_suffix}\"\nchmod +x \"{executable_suffix}\"\n"
                    ));
                }
                let executable_json = match mode {
                    "deps" => format!(
                        "'\"$CARGO_TARGET_DIR\"'/debug/deps/{}-deadbeef",
                        spec.target
                    ),
                    "outside" => format!("'\"$CARGO_TARGET_DIR\"'/../outside-{}", spec.target),
                    _ => format!("'\"$CARGO_TARGET_DIR\"'/debug/{}", spec.target),
                };
                let manifest = package_root(spec.package)
                    .join("Cargo.toml")
                    .canonicalize()
                    .expect("manifest");
                let manifest = serde_json::to_string(&manifest.to_string_lossy().to_string())
                    .expect("manifest JSON");
                let fresh = if mode == "stale" { "true" } else { "false" };
                body.push_str(&format!(
                    "printf '%s\\n' '{{\"reason\":\"compiler-artifact\",\"manifest_path\":{manifest},\"target\":{{\"kind\":[\"bin\"],\"name\":\"{}\"}},\"executable\":\"{executable_json}\",\"fresh\":{fresh}}}'\n",
                    spec.target
                ));
            }
        }
        fs::write(&script, body).expect("fake cargo");
        let mut permissions = fs::metadata(&script)
            .expect("fake cargo metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("fake cargo permissions");
        script
    }

    #[cfg(unix)]
    fn fake_build(mode: &str) -> Result<HashMap<&'static str, FreshBinary>, String> {
        let root = tempfile::tempdir().expect("fake build root");
        let target = root.path().join("target");
        fs::create_dir(&target).expect("fake target");
        let cargo = fake_cargo(root.path(), mode);
        build_fresh_binaries(&cargo, &target)
    }

    #[cfg(unix)]
    #[test]
    fn unset_handoff_builds_once_and_resolves_every_required_direct_binary() {
        let _guard = ENVIRONMENT_LOCK.lock().expect("environment lock");
        let root = tempfile::tempdir().expect("fake cargo root");
        let fake = fake_cargo(root.path(), "success");
        let count = root.path().join("build-count");
        let original_handoff = env::var_os(HANDOFF_ENV);
        let original_path = env::var_os("PATH");
        env::remove_var(HANDOFF_ENV);
        let mut path = fake
            .parent()
            .expect("fake cargo parent")
            .to_string_lossy()
            .into_owned();
        if let Some(original_path) = original_path.as_ref() {
            path.push(':');
            path.push_str(&original_path.to_string_lossy());
        }
        env::set_var("PATH", path);
        env::set_var("FAKE_CARGO_COUNT", &count);
        let resolved = REQUIRED_BINARIES
            .iter()
            .map(|spec| resolve_binary(spec.alias).map(|path| (spec.alias, path)))
            .collect::<Result<Vec<_>, _>>();
        match original_handoff {
            Some(value) => env::set_var(HANDOFF_ENV, value),
            None => env::remove_var(HANDOFF_ENV),
        }
        match original_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
        env::remove_var("FAKE_CARGO_COUNT");

        let resolved = resolved.expect("fresh fallback");
        assert_eq!(fs::read_to_string(count).expect("build count"), "1");
        assert_eq!(resolved.len(), REQUIRED_BINARIES.len());
        let debug = resolved
            .first()
            .and_then(|(_, path)| path.parent())
            .expect("debug directory")
            .to_path_buf();
        for (spec, (alias, path)) in REQUIRED_BINARIES.iter().zip(resolved.iter()) {
            assert_eq!(*alias, spec.alias);
            assert_eq!(path, &path.canonicalize().expect("canonical binary"));
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some(spec.target)
            );
            assert_eq!(path.parent(), Some(debug.as_path()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn fresh_build_rejects_missing_direct_output() {
        let error = fake_build("missing").expect_err("missing output must fail closed");
        assert!(
            error.contains("direct") || error.contains("required"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fresh_build_rejects_hashed_deps_only_output() {
        let error = fake_build("deps").expect_err("deps output must fail closed");
        assert!(
            error.contains("direct") || error.contains("deps"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fresh_build_rejects_outside_target_output() {
        let error = fake_build("outside").expect_err("outside output must fail closed");
        assert!(
            error.contains("direct") || error.contains("outside"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fresh_build_rejects_stale_artifact_output() {
        let error = fake_build("stale").expect_err("stale output must fail closed");
        assert!(
            error.contains("fresh") || error.contains("stale"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fresh_build_rejects_failed_build() {
        let error = fake_build("failed").expect_err("failed build must fail closed");
        assert!(error.contains("build failed"), "{error}");
    }

    #[test]
    fn missing_handoff_file_is_rejected_before_any_binary_lookup() {
        let root = tempfile::tempdir().expect("tempdir");
        let missing = root.path().join("missing-handoff.json");
        let error = resolve_binary_from_handoff("loop-engine", &missing).expect_err("must reject");
        assert!(error.contains("could not read binary handoff"), "{error}");
        assert!(!root.path().join("executed").exists());
    }

    #[test]
    fn non_executable_path_is_rejected_before_spawn() {
        let (root, active) = active_root();
        let candidate = active.join("loop-engine");
        fs::write(&candidate, b"not executable").expect("candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&candidate, fs::Permissions::from_mode(0o644))
                .expect("permissions");
        }
        let handoff = handoff(root.path(), &candidate, "loop-engine", true);
        let error = resolve_binary_from_handoff("loop-engine", &handoff).expect_err("must reject");
        assert!(error.contains("not executable"), "{error}");
    }

    #[test]
    fn stale_direct_candidate_digest_is_rejected_before_spawn() {
        let (root, active) = active_root();
        let candidate = active.join("loop-engine");
        fs::write(&candidate, b"#!/bin/sh\nexit 0\n").expect("candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
                .expect("permissions");
        }
        let handoff = handoff(root.path(), &candidate, "loop-engine", true);
        let mut value: Value =
            serde_json::from_slice(&fs::read(&handoff).expect("handoff")).expect("handoff JSON");
        value["binaries"]["loop-engine"]["sha256"] = Value::String("0".repeat(64));
        fs::write(
            &handoff,
            serde_json::to_vec(&value).expect("serialize handoff"),
        )
        .expect("rewrite handoff");
        let error = resolve_binary_from_handoff("loop-engine", &handoff).expect_err("must reject");
        assert!(error.contains("digest mismatch"), "{error}");
    }

    #[test]
    fn path_outside_active_target_is_rejected_before_spawn() {
        let (root, active) = active_root();
        let outside = root.path().join("outside/loop-engine");
        fs::create_dir_all(outside.parent().expect("outside parent")).expect("outside");
        fs::write(&outside, b"#!/bin/sh\nexit 0\n").expect("candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).expect("permissions");
        }
        let handoff = handoff(root.path(), &outside, "loop-engine", true);
        let error = resolve_binary_from_handoff("loop-engine", &handoff).expect_err("must reject");
        assert!(error.contains("outside active target"), "{error}");
        assert!(active.ends_with("target/debug"));
    }

    #[test]
    fn stale_hashed_candidate_is_never_selected_as_a_fallback() {
        let (root, active) = active_root();
        let deps = active.join("deps");
        fs::create_dir_all(&deps).expect("deps");
        let stale = deps.join("loop-engine-deadbeef");
        fs::write(&stale, b"#!/bin/sh\ntouch executed\n").expect("stale candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stale, fs::Permissions::from_mode(0o755)).expect("permissions");
        }
        let handoff = handoff(root.path(), &stale, "loop-engine", false);
        let error = resolve_binary_from_handoff("loop-engine", &handoff).expect_err("must reject");
        assert!(
            error.contains("no entry") || error.contains("fallback"),
            "{error}"
        );
        assert!(!root.path().join("executed").exists());
    }

    #[test]
    fn direct_name_under_deps_is_rejected_before_spawn() {
        let (root, active) = active_root();
        let deps = active.join("deps");
        fs::create_dir_all(&deps).expect("deps");
        let candidate = deps.join("loop-engine");
        fs::write(&candidate, b"#!/bin/sh\ntouch executed\n").expect("candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
                .expect("permissions");
        }
        let handoff = handoff(root.path(), &candidate, "loop-engine", true);
        let error = resolve_binary_from_handoff("loop-engine", &handoff).expect_err("must reject");
        assert!(error.contains("under target/deps"), "{error}");
        assert!(!root.path().join("executed").exists());
    }
}
