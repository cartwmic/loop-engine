use software_change_provider::embedded_data::FILES;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("provider crate must be nested under repository root")
        .to_path_buf()
}

fn collect_files(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(current).expect("read data directory") {
        let entry = entry.expect("read data directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("file must be beneath collection root")
                .to_string_lossy()
                .replace('\\', "/");
            let previous = files.insert(relative.clone(), fs::read(&path).expect("read data file"));
            assert!(previous.is_none(), "duplicate collected path: {relative}");
        }
    }
}

fn on_disk_data_files() -> BTreeMap<String, Vec<u8>> {
    let root = repo_root();
    let data = root.join("crates/software-change-provider/data");
    let mut files = BTreeMap::new();
    collect_files(&root, &data, &mut files);
    files
}

fn embedded_data_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for file in FILES {
        let previous = files.insert(file.path.to_owned(), file.bytes.to_vec());
        assert!(previous.is_none(), "duplicate embedded path: {}", file.path);
    }
    files
}

fn temporary_path(label: &str) -> PathBuf {
    let index = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "software-change-provider-embedded-{label}-{}-{index}",
        std::process::id()
    ))
}

fn dump(destination: &Path) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_software-change"));
    command.arg("data-dump").arg(destination);
    super::bounded_process::run(&mut command, "software-change data-dump")
        .expect("data-dump process should spawn")
}

#[test]
fn embedded_manifest_exactly_matches_on_disk_data_tree() {
    assert_eq!(embedded_data_files(), on_disk_data_files());
}

#[test]
fn data_dump_matches_tree_and_refuses_existing_targets_without_writing() {
    let root = temporary_path("dump");
    fs::create_dir_all(&root).expect("create temporary root");
    let destination = root.join("materialized");

    let first = dump(&destination);
    assert!(
        first.status.success(),
        "first dump failed: {:?}",
        first.stderr
    );

    let mut dumped = BTreeMap::new();
    collect_files(&destination, &destination, &mut dumped);
    let expected = embedded_data_files();
    assert_eq!(dumped, expected);

    let existing_target =
        destination.join("crates/software-change-provider/data/configs/standard.json");
    let sentinel = b"sentinel that must survive";
    fs::write(&existing_target, sentinel).expect("replace one target with sentinel");
    let before = collect_all_bytes(&destination);

    let second = dump(&destination);
    assert!(
        !second.status.success(),
        "second dump unexpectedly succeeded"
    );
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("refusing to overwrite existing target"),
        "unexpected second-dump stderr: {:?}",
        second.stderr
    );
    assert_eq!(fs::read(&existing_target).expect("read sentinel"), sentinel);
    assert_eq!(collect_all_bytes(&destination), before);

    fs::remove_dir_all(root).expect("remove temporary root");
}

fn collect_all_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files);
    files
}
