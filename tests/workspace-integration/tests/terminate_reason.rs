//! Public-binary proof that `terminate RUN [REASON]` stores an optional reason on
//! the run's terminated history entry and that `show` displays it.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const REASON: &str = "superseded by another run";

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

    fn run(&self, json: bool, parts: &[&str]) -> Output {
        let mut command = Command::new(workspace_integration::binary("loop-engine"));
        command
            .current_dir(self._root.path())
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("LOOP_ENGINE_HOME", self.home.join("loop-engine"))
            .env_remove("LOOP_ENGINE_DATABASE")
            .env_remove("LOOP_ENGINE_PROVIDER_CONFIG")
            .env_remove("LOOP_ENGINE_MONITOR_DIR");
        if json {
            command.arg("--json");
        }
        command
            .arg("--database")
            .arg(&self.database)
            .arg("--config")
            .arg(&self.config)
            .args(parts);
        command.output().expect("run loop-engine")
    }

    fn cli(&self, parts: &[&str]) -> Value {
        let output = self.run(true, parts);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{parts:?}: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn text(&self, parts: &[&str]) -> String {
        let output = self.run(false, parts);
        assert!(output.status.success(), "{parts:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Start a run and `show` it, which arms mutation, then terminate it.
    fn terminated(&self, run: &str, reason: Option<&str>) {
        let started = self.cli(&["start", "--id", run, "model-binding", "{}", run]);
        assert_eq!(started["status"], "completed", "{started}");
        let shown = self.cli(&["show", run]);
        assert_eq!(shown["status"], "completed", "{shown}");
        let mut parts = vec!["terminate", run];
        parts.extend(reason);
        let terminated = self.cli(&parts);
        assert_eq!(terminated["status"], "completed", "{terminated}");
    }

    /// The `action` of the run's single terminated history entry.
    fn terminated_action(&self, run: &str) -> Value {
        let history = self.cli(&["history", run]);
        history["result"]
            .as_array()
            .expect("history result")
            .iter()
            .map(|entry| entry["action"].clone())
            .find(|action| action["kind"] == "terminated")
            .unwrap_or_else(|| panic!("no terminated entry: {history}"))
    }
}

#[test]
fn terminate_stores_the_reason_in_history() {
    let catalog = Catalog::new();
    catalog.terminated("run", Some(REASON));

    let action = catalog.terminated_action("run");
    assert_eq!(action, json!({"kind":"terminated","reason":REASON}));
}

#[test]
fn show_displays_the_termination_reason() {
    let catalog = Catalog::new();
    catalog.terminated("run", Some(REASON));

    // The field is `termination_reason` on the show result in every view.
    for view_args in [
        vec!["show", "run"],
        vec!["show", "--view", "status", "run"],
        vec!["show", "--view", "full", "run"],
    ] {
        let shown = catalog.cli(&view_args);
        assert_eq!(shown["status"], "completed", "{view_args:?}: {shown}");
        assert_eq!(
            shown["result"]["termination_reason"], REASON,
            "{view_args:?}: {shown}"
        );
    }

    let text = catalog.text(&["show", "run"]);
    let line = text
        .lines()
        .find(|line| line.contains("terminated:"))
        .unwrap_or_else(|| panic!("no `terminated:` line in {text}"));
    assert!(line.contains(REASON), "{line}");
}

#[test]
fn terminate_without_a_reason_is_unchanged() {
    let catalog = Catalog::new();
    catalog.terminated("run", None);

    assert_eq!(
        catalog.terminated_action("run"),
        json!({"kind":"terminated"})
    );

    for view_args in [
        vec!["show", "run"],
        vec!["show", "--view", "status", "run"],
        vec!["show", "--view", "full", "run"],
    ] {
        let shown = catalog.cli(&view_args);
        assert_eq!(shown["status"], "completed", "{view_args:?}: {shown}");
        assert!(
            shown["result"]["termination_reason"].is_null(),
            "{view_args:?}: {shown}"
        );
    }
    let text = catalog.text(&["show", "run"]);
    assert!(!text.contains("terminated:"), "{text}");
}

#[test]
fn terminate_help_shows_the_optional_reason() {
    let catalog = Catalog::new();
    let help = catalog.text(&["terminate", "--help"]);
    assert!(help.contains("terminate <run-id> [reason]"), "{help}");
}

#[test]
fn terminate_refuses_a_third_argument() {
    let catalog = Catalog::new();
    let started = catalog.cli(&["start", "--id", "run", "model-binding", "{}", "run"]);
    assert_eq!(started["status"], "completed", "{started}");
    let output = catalog.run(true, &["terminate", "run", REASON, "extra"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}
