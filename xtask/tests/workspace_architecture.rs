use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package, TargetKind};
use tempfile::TempDir;

const CORE: &str = "loop-engine-core";
const INTEGRATIONS: &str = "loop-engine-integrations";
const CLI: &str = "loop-engine-cli";
const NON_SHIPPING_PACKAGE: &str = "xtask";

fn fixture_manifest(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/workspace-architecture")
        .join(name)
        .join("Cargo.toml")
}

fn load_metadata(manifest_path: &Path) -> Metadata {
    MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .unwrap_or_else(|error| {
            panic!(
                "Cargo metadata must load for {}: {error}",
                manifest_path.display()
            )
        })
}

fn workspace_package<'a>(metadata: &'a Metadata, name: &str) -> &'a Package {
    metadata
        .workspace_packages()
        .into_iter()
        .find(|package| package.name == name)
        .unwrap_or_else(|| panic!("workspace must contain `{name}`"))
}

fn has_normal_dependency(package: &Package, dependency_name: &str) -> bool {
    package.dependencies.iter().any(|dependency| {
        dependency.kind == DependencyKind::Normal && dependency.name == dependency_name
    })
}

fn validate_inward_product_architecture(metadata: &Metadata) -> Result<(), String> {
    let mut violations = Vec::new();

    let core = workspace_package(metadata, CORE);
    if has_normal_dependency(core, INTEGRATIONS) {
        violations.push(format!(
            "`{CORE}` must not have a normal dependency on `{INTEGRATIONS}`"
        ));
    }
    if has_normal_dependency(core, CLI) {
        violations.push(format!(
            "`{CORE}` must not have a normal dependency on `{CLI}`"
        ));
    }

    let integrations = workspace_package(metadata, INTEGRATIONS);
    if has_normal_dependency(integrations, CLI) {
        violations.push(format!(
            "`{INTEGRATIONS}` must not have a normal dependency on `{CLI}`"
        ));
    }

    let mut shipping_binary_targets = metadata
        .workspace_packages()
        .into_iter()
        .filter(|package| package.name != NON_SHIPPING_PACKAGE)
        .flat_map(|package| {
            package
                .targets
                .iter()
                .filter(|target| target.kind.contains(&TargetKind::Bin))
                .map(move |target| (package.name.as_str(), target.name.as_str()))
        })
        .collect::<Vec<_>>();
    shipping_binary_targets.sort_unstable();
    if shipping_binary_targets != vec![(CLI, "loop-engine")] {
        violations.push(format!(
            "CLI must expose the sole product binary target; found {shipping_binary_targets:?}"
        ));
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations.join("\n"))
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture destination");
    for entry in fs::read_dir(source).expect("read fixture source") {
        let entry = entry.expect("read fixture entry");
        let destination_entry = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture type").is_dir() {
            copy_tree(&entry.path(), &destination_entry);
        } else {
            let bytes = fs::read(entry.path()).expect("read fixture file");
            fs::write(destination_entry, bytes).expect("materialize fixture file");
        }
    }
}

#[test]
fn copy_tree_materializes_read_only_files_as_writable_scratch_files() {
    let temporary = TempDir::new().expect("create temporary directory");
    let source_root = temporary.path().join("source");
    let destination_root = temporary.path().join("destination");
    let source_file = source_root.join("nested/fixture.txt");
    fs::create_dir_all(source_file.parent().expect("source parent"))
        .expect("create source directory");
    fs::write(&source_file, b"immutable source").expect("write source fixture");

    let original_permissions = fs::metadata(&source_file)
        .expect("source metadata")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_readonly(true);
    fs::set_permissions(&source_file, read_only_permissions).expect("make source read-only");

    copy_tree(&source_root, &destination_root);

    let destination_file = destination_root.join("nested/fixture.txt");
    assert_eq!(
        fs::read(&destination_file).expect("read destination fixture"),
        b"immutable source"
    );
    assert!(
        !fs::metadata(&destination_file)
            .expect("destination metadata")
            .permissions()
            .readonly()
    );
    fs::write(&destination_file, b"mutated destination").expect("mutate destination fixture");
    assert_eq!(
        fs::read(&source_file).expect("read source after destination mutation"),
        b"immutable source"
    );
    assert!(
        fs::metadata(&source_file)
            .expect("source metadata after copy")
            .permissions()
            .readonly()
    );

    fs::set_permissions(source_file, original_permissions).expect("restore source permissions");
}

fn copied_allowed_fixture() -> TempDir {
    let temp = TempDir::new().expect("fixture tempdir");
    let manifest = fixture_manifest("allowed");
    let source = manifest.parent().expect("allowed fixture root");
    copy_tree(source, temp.path());
    temp
}

fn replace_fixture_manifest_text(fixture: &TempDir, crate_name: &str, old: &str, new: &str) {
    let manifest_path = fixture
        .path()
        .join("crates")
        .join(crate_name)
        .join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("read fixture manifest");
    assert!(
        manifest.contains(old),
        "fixture manifest {} must contain mutation target `{old}`",
        manifest_path.display()
    );
    fs::write(&manifest_path, manifest.replacen(old, new, 1)).expect("mutate fixture manifest");
}

fn add_fixture_workspace_member(fixture: &TempDir, crate_name: &str) {
    let root_manifest_path = fixture.path().join("Cargo.toml");
    let root_manifest = fs::read_to_string(&root_manifest_path).expect("read root manifest");
    let cli_member = "    \"crates/loop-engine-cli\",\n";
    assert!(
        root_manifest.contains(cli_member),
        "allowed fixture must contain CLI workspace member"
    );
    fs::write(
        root_manifest_path,
        root_manifest.replacen(
            cli_member,
            &format!("{cli_member}    \"crates/{crate_name}\",\n"),
            1,
        ),
    )
    .expect("add package to fixture workspace");
}

#[test]
fn current_workspace_has_inward_dependencies_and_one_product_binary() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
    validate_inward_product_architecture(&load_metadata(&manifest))
        .expect("current workspace must satisfy final metadata architecture policy");
}

#[test]
fn metadata_policy_accepts_inward_fixture() {
    validate_inward_product_architecture(&load_metadata(&fixture_manifest("allowed")))
        .expect("inward fixture must pass metadata architecture policy");
}

#[test]
fn metadata_policy_rejects_outward_core_dependency_fixture() {
    let error =
        validate_inward_product_architecture(&load_metadata(&fixture_manifest("outward-core")))
            .expect_err("outward core dependency must fail policy");
    assert_eq!(
        error,
        format!("`{CORE}` must not have a normal dependency on `{INTEGRATIONS}`")
    );
}

#[test]
fn metadata_policy_rejects_core_dependency_on_cli() {
    let fixture = copied_allowed_fixture();
    replace_fixture_manifest_text(
        &fixture,
        CLI,
        "\n[dependencies]\nloop-engine-core = { path = \"../loop-engine-core\" }\nloop-engine-integrations = { path = \"../loop-engine-integrations\" }\n",
        "\n",
    );
    replace_fixture_manifest_text(
        &fixture,
        CORE,
        "edition.workspace = true\n",
        "edition.workspace = true\n\n[dependencies]\nloop-engine-cli = { path = \"../loop-engine-cli\" }\n",
    );

    let error =
        validate_inward_product_architecture(&load_metadata(&fixture.path().join("Cargo.toml")))
            .expect_err("core dependency on CLI must fail policy");
    assert_eq!(
        error,
        format!("`{CORE}` must not have a normal dependency on `{CLI}`")
    );
}

#[test]
fn metadata_policy_rejects_integrations_dependency_on_cli() {
    let fixture = copied_allowed_fixture();
    replace_fixture_manifest_text(
        &fixture,
        CLI,
        "loop-engine-integrations = { path = \"../loop-engine-integrations\" }\n",
        "",
    );
    replace_fixture_manifest_text(
        &fixture,
        INTEGRATIONS,
        "loop-engine-core = { path = \"../loop-engine-core\" }\n",
        "loop-engine-core = { path = \"../loop-engine-core\" }\nloop-engine-cli = { path = \"../loop-engine-cli\" }\n",
    );

    let error =
        validate_inward_product_architecture(&load_metadata(&fixture.path().join("Cargo.toml")))
            .expect_err("integrations dependency on CLI must fail policy");
    assert_eq!(
        error,
        format!("`{INTEGRATIONS}` must not have a normal dependency on `{CLI}`")
    );
}

#[test]
fn metadata_policy_rejects_binary_outside_cli() {
    let fixture = copied_allowed_fixture();
    let extra_main = fixture
        .path()
        .join("crates/loop-engine-integrations/src/main.rs");
    fs::write(extra_main, "fn main() {}\n").expect("write extra product binary");

    let error =
        validate_inward_product_architecture(&load_metadata(&fixture.path().join("Cargo.toml")))
            .expect_err("product binary outside CLI must fail policy");
    assert!(error.contains("sole product binary"));
}

#[test]
fn metadata_policy_rejects_binary_in_non_prefixed_workspace_package() {
    let fixture = copied_allowed_fixture();
    add_fixture_workspace_member(&fixture, "engine-admin");

    let package_root = fixture.path().join("crates/engine-admin");
    fs::create_dir_all(package_root.join("src")).expect("create non-prefixed package source");
    fs::write(
        package_root.join("Cargo.toml"),
        "[package]\nname = \"engine-admin\"\nversion.workspace = true\nedition.workspace = true\n",
    )
    .expect("write non-prefixed package manifest");
    fs::write(package_root.join("src/main.rs"), "fn main() {}\n")
        .expect("write non-prefixed binary");

    let error =
        validate_inward_product_architecture(&load_metadata(&fixture.path().join("Cargo.toml")))
            .expect_err("binary in non-prefixed workspace package must fail policy");
    assert_eq!(
        error,
        "CLI must expose the sole product binary target; found [(\"engine-admin\", \"engine-admin\"), (\"loop-engine-cli\", \"loop-engine\")]"
    );
}

#[test]
fn metadata_policy_rejects_multiple_cli_binaries() {
    let fixture = copied_allowed_fixture();
    let cli_manifest = fixture.path().join("crates/loop-engine-cli/Cargo.toml");
    let mut manifest = fs::read_to_string(&cli_manifest).expect("read CLI manifest");
    manifest.push_str("\n[[bin]]\nname = \"loop-engine-admin\"\npath = \"src/admin.rs\"\n");
    fs::write(&cli_manifest, manifest).expect("write second CLI binary target");
    fs::write(
        fixture.path().join("crates/loop-engine-cli/src/admin.rs"),
        "fn main() {}\n",
    )
    .expect("write second CLI binary");

    let error =
        validate_inward_product_architecture(&load_metadata(&fixture.path().join("Cargo.toml")))
            .expect_err("multiple CLI binaries must fail policy");
    assert!(error.contains("sole product binary"));
}
