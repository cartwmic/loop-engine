//! Public-binary proof that starting a run with an id already in the catalog is
//! refused, naming the id, and leaves the existing run unchanged.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Catalog {
    _root: tempfile::TempDir,
    home: PathBuf,
    database: PathBuf,
    config: PathBuf,
}

impl Catalog {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("catalog root");
        let home = root.path().join("home");
        fs::create_dir_all(&home).expect("home");
        let config = root.path().join("providers.toml");
        let provider = workspace_integration::fixture_binary("model-binding-provider");
        fs::write(
            &config,
            format!(
                "[providers.model-binding]\ncommand = {}\nargs = []\n",
                Value::String(provider.to_string_lossy().into_owned())
            ),
        )
        .expect("provider catalog");
        Self {
            database: root.path().join("run.sqlite"),
            home,
            config,
            _root: root,
        }
    }

    /// The parsed JSON reply and the process exit code.
    fn cli(&self, parts: &[&str]) -> (Value, i32) {
        let output = Command::new(workspace_integration::binary("loop-engine"))
            .current_dir(self._root.path())
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("LOOP_ENGINE_HOME", self.home.join("loop-engine"))
            .env_remove("LOOP_ENGINE_DATABASE")
            .env_remove("LOOP_ENGINE_PROVIDER_CONFIG")
            .env_remove("LOOP_ENGINE_MONITOR_DIR")
            .arg("--json")
            .arg("--database")
            .arg(&self.database)
            .arg("--config")
            .arg(&self.config)
            .args(parts)
            .output()
            .expect("run loop-engine");
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{parts:?}: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (value, output.status.code().expect("exit code"))
    }

    fn start(&self, run: &str) -> (Value, i32) {
        self.cli(&["start", "--id", run, "model-binding", "{}", run])
    }

    fn full(&self, run: &str) -> Value {
        let (shown, _) = self.cli(&["show", "--view", "full", run]);
        assert_eq!(shown["status"], "completed", "{shown}");
        shown
    }
}

fn assert_refused_as_duplicate(reply: &Value, code: i32, run: &str) {
    assert_eq!(reply["status"], "rejected", "{reply}");
    assert_eq!(reply["code"], "run-already-exists", "{reply}");
    assert!(
        reply["message"].as_str().unwrap_or("").contains(run),
        "message does not name the id: {reply}"
    );
    assert_eq!(code, 10, "{reply}");
    let text = reply.to_string();
    assert!(!text.contains("persistence-failure"), "{reply}");
    assert!(!text.contains("UNIQUE"), "{reply}");
}

#[test]
fn refuses_a_second_start_with_the_same_id_and_names_it() {
    let catalog = Catalog::new();
    let (first, _) = catalog.start("dup-run");
    assert_eq!(first["status"], "completed", "{first}");

    let (second, code) = catalog.start("dup-run");
    assert_refused_as_duplicate(&second, code, "dup-run");
}

#[test]
fn a_refused_start_leaves_the_existing_run_unchanged() {
    let catalog = Catalog::new();
    let (first, _) = catalog.start("dup-run");
    assert_eq!(first["status"], "completed", "{first}");
    let mut before = catalog.full("dup-run");

    let (second, code) = catalog.start("dup-run");
    assert_refused_as_duplicate(&second, code, "dup-run");

    let mut after = catalog.full("dup-run");
    // `show` records when it observed the run; nothing else may differ.
    before["result"]
        .as_object_mut()
        .expect("result object")
        .remove("observed_at");
    after["result"]
        .as_object_mut()
        .expect("result object")
        .remove("observed_at");
    assert_eq!(before, after);

    let (other, _) = catalog.start("other-run");
    assert_eq!(other["status"], "completed", "{other}");
}
