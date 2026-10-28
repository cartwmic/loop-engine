//! Public-binary proof of the engine's model-binding library and view through the
//! reference model-binding fixture provider: frozen per-role bindings, declared
//! sessions, refusals, owner amendments, stale-mark eligibility and visibility.
//! Declarations are scripted; nothing here runs or verifies a real model.

use serde_json::{json, Value};
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

    fn path(&self, name: &str) -> PathBuf {
        self._root.path().join(name)
    }

    fn cli(&self, parts: &[&str]) -> Value {
        let mut command = Command::new(workspace_integration::binary("loop-engine"));
        command
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
            .args(parts);
        let output = command.output().expect("run loop-engine");
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{parts:?}: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn start(&self, run: &str, input: &Value) -> Value {
        let profile = self.path(&format!("{run}-profile.json"));
        fs::write(&profile, input.to_string()).expect("profile");
        let started = self.cli(&[
            "start",
            "--id",
            run,
            "model-binding",
            &format!("@{}", profile.display()),
            run,
        ]);
        // Later edits to the profile file never reach the frozen run input.
        fs::write(&profile, json!({"model_bindings":{"roles":{}}}).to_string())
            .expect("edit profile");
        started
    }

    fn full(&self, run: &str) -> Value {
        let shown = self.cli(&["show", "--view", "full", run]);
        assert_eq!(shown["status"], "completed", "{shown}");
        shown["result"].clone()
    }

    /// The fixture's commission projection over the current full show.
    fn commission(&self, run: &str) -> Value {
        use std::io::Write;
        let shown = self.cli(&["show", "--view", "full", run]);
        let mut child = Command::new(workspace_integration::fixture_binary(
            "model-binding-provider",
        ))
        .arg("commission")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run commission");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(shown.to_string().as_bytes())
            .expect("commission input");
        let output = child.wait_with_output().expect("commission output");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("commission json")
    }

    /// Commission and the checked event give the same answer; returns the event response.
    fn approval_agrees(&self, run: &str, allowed: bool) -> Value {
        let prepared = self.commission(run)["approval"].clone();
        assert_eq!(
            prepared["result"],
            if allowed { "allow" } else { "deny" },
            "{prepared}"
        );
        if allowed {
            self.moved(run, "approved", "done");
            return prepared;
        }
        let denied = self.refused(run, || self.event(run, "approved"), &["review-incomplete"]);
        assert!(
            denied
                .to_string()
                .contains(&prepared["details"].to_string()),
            "commission {prepared} and event {denied} disagree"
        );
        denied
    }

    fn append(&self, run: &str, id: &str, kind: &str, data: &Value) -> Value {
        self.full(run);
        self.cli(&["append", run, kind, &data.to_string(), "--record-id", id])
    }

    fn stored(&self, run: &str, id: &str, kind: &str, data: Value) {
        let value = self.append(run, id, kind, &data);
        assert_eq!(value["status"], "completed", "{id}: {value}");
    }

    fn event(&self, run: &str, event: &str) -> Value {
        self.full(run);
        self.cli(&["event", run, event])
    }

    fn moved(&self, run: &str, event: &str, to: &str) {
        let value = self.event(run, event);
        assert_eq!(value["status"], "completed", "{event}: {value}");
        assert_eq!(self.full(run)["current_state"], to);
    }

    /// The attempt is refused, names every needle, and leaves context and state unchanged.
    fn refused(&self, run: &str, attempt: impl FnOnce() -> Value, needles: &[&str]) -> Value {
        let before = self.full(run);
        let value = attempt();
        assert_ne!(value["status"], "completed", "{value}");
        for needle in needles {
            assert!(
                value.to_string().contains(needle),
                "missing {needle:?}: {value}"
            );
        }
        let after = self.full(run);
        assert_eq!(after["context"], before["context"]);
        assert_eq!(after["current_state"], before["current_state"]);
        value
    }
}

fn binding(model: &str, thinking: &str) -> Value {
    json!({"model": model, "thinking": thinking})
}

fn review(name: &str, revision: &str, session: Option<Value>) -> Value {
    let mut data = json!({"reviewer":{"name":name,"kind":"agent"},"subject_revision":revision,
        "axis":"correctness","result":"pass","findings":[]});
    if let Some(session) = session {
        data["session"] = session;
    }
    data
}

fn declaration(revision: &str, session: Value) -> Value {
    json!({"role":"author","subject":"draft","subject_revision":revision,"session":session})
}

fn amendment(changes: Value, driver_session: Option<Value>) -> Value {
    let mut data = json!({"actor":{"name":"Michael","kind":"human"},"authority":{"kind":"owner"},
        "words":"Use the cheaper reviewer from here on.","changes":changes});
    if let Some(session) = driver_session {
        data["driver_session"] = session;
    }
    data
}

#[test]
// bookends:LE-195 — per-run bindings are validated at start, frozen with the input, unchanged by profile edits.
// bookends:LE-200 — status and full views state never-bound runs explicitly; unbound runs behave as before.
fn model_bindings_profile_identity_and_unbound_runs_public_binary() {
    let catalog = Catalog::new();
    let author = binding("anthropic/claude-sonnet", "high");
    let driver = binding("openai/gpt", "medium");
    for (input, needle) in [
        (
            json!({"model_bindings":{"roles":{"author":{"model":" ","thinking":"high"}}}}),
            "author",
        ),
        (
            json!({"model_bindings":{"roles":{"tester":author}}}),
            "unknown role",
        ),
        (
            json!({"model_bindings":{"roles":{"driver":driver}}}),
            "driver_session is required",
        ),
        (
            json!({"model_bindings":{"roles":{"driver":driver},"driver_session":binding("openai/gpt","low")}}),
            "thinking level only",
        ),
    ] {
        let refused = catalog.start("refused", &input);
        assert_ne!(refused["status"], "completed", "{refused}");
        assert!(
            refused.to_string().contains(needle),
            "missing {needle:?}: {refused}"
        );
        assert!(catalog
            .cli(&["list"])
            .to_string()
            .find("\"refused\"")
            .is_none());
    }
    let bound = json!({"model_bindings":{"roles":{"author":author,"driver":driver},"driver_session":driver}});
    assert_eq!(catalog.start("bound", &bound)["status"], "completed");
    let full = catalog.full("bound");
    assert_eq!(
        full["initial_input"]["model_bindings"],
        bound["model_bindings"]
    );
    assert_eq!(full["model_bindings"]["state"], "bound");
    let roles = full["model_bindings"]["roles"].as_array().unwrap();
    let author_row = roles.iter().find(|row| row["role"] == "author").unwrap();
    assert_eq!(author_row["original"], author);
    assert_eq!(author_row["in_force"], author);
    assert!(roles
        .iter()
        .any(|row| row["role"] == "ordinary-reviewer" && row["original"].is_null()));
    let status = catalog.cli(&["show", "--view", "status", "bound"]);
    assert_eq!(
        status["result"]["model_bindings"], full["model_bindings"],
        "{status}"
    );

    assert_eq!(catalog.start("plain", &json!({}))["status"], "completed");
    for view in ["status", "full"] {
        let shown = catalog.cli(&["show", "--view", view, "plain"]);
        assert_eq!(
            shown["result"]["model_bindings"]["state"], "never-bound",
            "{shown}"
        );
        assert_eq!(
            shown["result"]["model_bindings"]["statement"],
            loop_core::NEVER_BOUND_STATEMENT
        );
    }
    catalog.stored("plain", "d1", "draft", json!({"revision":"r1"}));
    catalog.moved("plain", "drafted", "review");
    catalog.stored("plain", "rev-1", "review", review("Reviewer", "r1", None));
    catalog.moved("plain", "approved", "done");
}

#[test]
// bookends:LE-196 bookends:LE-197 — bound author and reviewer work needs an exactly equal declared session; refusals name role, expected, actual and the differing part, and store nothing.
fn model_bindings_declared_sessions_and_refusals_public_binary() {
    let catalog = Catalog::new();
    let author = binding("anthropic/claude-sonnet", "high");
    let reviewer = binding("openai/gpt", "xhigh");
    let run = "declared";
    assert_eq!(
        catalog.start(
            run,
            &json!({"model_bindings":{"roles":{"author":author,"ordinary-reviewer":reviewer}}})
        )["status"],
        "completed"
    );
    catalog.stored(run, "d1", "draft", json!({"revision":"r1"}));
    let missing = catalog.refused(
        run,
        || catalog.event(run, "drafted"),
        &["author", "statement missing"],
    );
    let history = catalog.cli(&["history", run]).to_string();
    assert!(
        history.contains("denied") || history.contains("deny"),
        "denial recorded: {history}"
    );
    assert!(missing.to_string().contains("claude-sonnet"), "{missing}");
    catalog.stored(
        run,
        "decl-1",
        "session-declaration",
        declaration("r1", binding("anthropic/claude-sonnet", "low")),
    );
    catalog.refused(
        run,
        || catalog.event(run, "drafted"),
        &["author", "thinking level only"],
    );
    catalog.stored(
        run,
        "decl-2",
        "session-declaration",
        declaration("r1", author.clone()),
    );
    catalog.moved(run, "drafted", "review");
    for (id, session, needle) in [
        ("rev-missing", None, "statement missing"),
        (
            "rev-model",
            Some(binding("openai/gpt-mini", "xhigh")),
            "model only",
        ),
        (
            "rev-both",
            Some(binding("openai/gpt-mini", "low")),
            "model and thinking level",
        ),
        (
            "rev-alias",
            Some(binding("OpenAI/GPT", "xhigh")),
            "model only",
        ),
    ] {
        let data = review("Reviewer", "r1", session);
        let value = catalog.refused(
            run,
            || catalog.append(run, id, "review", &data),
            &["ordinary-reviewer", needle, id],
        );
        assert_eq!(value["code"], "record-refused", "{value}");
    }
    catalog.stored(
        run,
        "rev-1",
        "review",
        review("Reviewer", "r1", Some(reviewer)),
    );
    catalog.moved(run, "approved", "done");
}

#[test]
// bookends:LE-198 — owner amendments add, change and remove bindings at their own position; invalid amendments are refused with the run unchanged.
// bookends:LE-199 — unmarked amendments keep earlier evidence eligible; a stale mark makes earlier reviews ineligible and names amendment, role, author, assessment and axis.
// bookends:LE-200 — views show original, in-force and ordered amendment history with the owner's words.
fn model_bindings_owner_amendments_and_eligibility_public_binary() {
    let catalog = Catalog::new();
    let first = binding("openai/gpt", "xhigh");
    let second = binding("openai/gpt-mini", "medium");
    let driver = binding("anthropic/claude-opus", "high");
    let run = "amended";
    assert_eq!(catalog.start(run, &json!({}))["status"], "completed");
    let refusals = [
        (
            {
                let mut data = amendment(
                    json!([{"role":"ordinary-reviewer","old":null,"new":first}]),
                    None,
                );
                data["actor"]["kind"] = json!("agent");
                data
            },
            "actor-not-human",
        ),
        (
            {
                let mut data = amendment(
                    json!([{"role":"ordinary-reviewer","old":null,"new":first}]),
                    None,
                );
                data["authority"] = json!({"kind":"standing-delegation"});
                data
            },
            "authority-not-owner",
        ),
        (
            {
                let mut data = amendment(
                    json!([{"role":"ordinary-reviewer","old":null,"new":first}]),
                    None,
                );
                data["words"] = json!("  ");
                data
            },
            "words-blank",
        ),
        (
            amendment(json!([{"role":"tester","old":null,"new":first}]), None),
            "unknown-role",
        ),
        (
            amendment(
                json!([{"role":"ordinary-reviewer","old":second,"new":first}]),
                None,
            ),
            "old-binding",
        ),
        (
            amendment(
                json!([{"role":"ordinary-reviewer","old":null,"new":{"model":"","thinking":"x"}}]),
                None,
            ),
            "new-binding",
        ),
        (
            amendment(json!([{"role":"driver","old":null,"new":driver}]), None),
            "driver-declaration",
        ),
        (
            amendment(
                json!([{"role":"author","old":null,"new":first,"mark_stale":true}]),
                None,
            ),
            "stale",
        ),
    ];
    for (index, (data, needle)) in refusals.iter().enumerate() {
        let id = format!("bad-{index}");
        let value = catalog.refused(
            run,
            || catalog.append(run, &id, "model-binding-amendment", data),
            &[needle],
        );
        assert_eq!(value["code"], "record-refused", "{value}");
    }
    assert_eq!(catalog.full(run)["model_bindings"]["state"], "never-bound");
    catalog.stored(run, "add", "model-binding-amendment",
        amendment(json!([{"role":"ordinary-reviewer","old":null,"new":first},{"role":"driver","old":null,"new":driver}]), Some(driver.clone())));
    catalog.stored(run, "d1", "draft", json!({"revision":"r1"}));
    catalog.moved(run, "drafted", "review");
    catalog.stored(
        run,
        "rev-first",
        "review",
        review("First reviewer", "r1", Some(first.clone())),
    );
    // An unmarked change keeps the earlier review eligible; new returns need the new binding.
    catalog.stored(
        run,
        "change",
        "model-binding-amendment",
        amendment(
            json!([{"role":"ordinary-reviewer","old":first,"new":second}]),
            Some(driver.clone()),
        ),
    );
    catalog.refused(
        run,
        || {
            catalog.append(
                run,
                "rev-old",
                "review",
                &review("Late reviewer", "r1", Some(first.clone())),
            )
        },
        &["ordinary-reviewer", "model and thinking level"],
    );
    let view = catalog.full(run)["model_bindings"].clone();
    let row = view["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["role"] == "ordinary-reviewer")
        .unwrap()
        .clone();
    assert!(
        row["original"].is_null() && row["in_force"] == second,
        "{view}"
    );
    let history = view["history"].as_array().unwrap();
    assert_eq!(
        history
            .iter()
            .map(|entry| entry["record_id"].clone())
            .collect::<Vec<_>>(),
        [json!("add"), json!("change")]
    );
    assert_eq!(
        history[1]["words"],
        "Use the cheaper reviewer from here on."
    );
    assert_eq!(history[1]["actor"]["kind"], "human");
    catalog.moved(run, "revise", "draft");
    catalog.moved(run, "drafted", "review");
    // A stale mark makes the earlier review ineligible for decisions requested after it.
    catalog.stored(
        run,
        "stale",
        "model-binding-amendment",
        amendment(
            json!([{"role":"ordinary-reviewer","old":second,"new":null,"mark_stale":true}]),
            Some(driver.clone()),
        ),
    );
    let denied = catalog.refused(
        run,
        || catalog.event(run, "approved"),
        &[
            "stale",
            "rev-first",
            "First reviewer",
            "ordinary-reviewer",
            "correctness",
        ],
    );
    assert!(denied.to_string().contains("\"stale\""), "{denied}");
    // The removed binding needs no declaration; a fresh review after the mark is eligible.
    catalog.stored(
        run,
        "rev-after",
        "review",
        review("Fresh reviewer", "r1", None),
    );
    catalog.moved(run, "approved", "done");
    let finished = catalog.full(run);
    let context = finished["context"].as_array().unwrap();
    assert!(
        context.iter().any(|record| record["id"] == "rev-first"),
        "original records stay"
    );
    assert_eq!(
        finished["model_bindings"]["history"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

fn carry(origin: &str, revision: &str) -> Value {
    json!({"origin":{"kind":"context-record","id":origin},"target":{"subject":"draft","revision":revision},
        "attesting_driver":{"name":"driver","kind":"agent"},"reason":"Only wording outside the reviewed claim changed."})
}

#[test]
// bookends:LE-199 — an unmarked change keeps an earlier review eligible directly and when carried by explicit evidence-applicability; a stale mark makes it uncarryable to a new target and the refusal names amendment, role, author, assessment and axis.
fn model_bindings_amendment_eligibility_and_carry_public_binary() {
    let catalog = Catalog::new();
    let first = binding("openai/gpt", "xhigh");
    let second = binding("openai/gpt-mini", "medium");
    let driver = binding("anthropic/claude-opus", "high");
    let add = || {
        amendment(
            json!([{"role":"ordinary-reviewer","old":null,"new":first},{"role":"driver","old":null,"new":driver}]),
            Some(driver.clone()),
        )
    };
    let reviewed = |run: &str| {
        assert_eq!(catalog.start(run, &json!({}))["status"], "completed");
        catalog.stored(run, "add", "model-binding-amendment", add());
        catalog.stored(run, "d1", "draft", json!({"revision":"r1"}));
        catalog.moved(run, "drafted", "review");
        catalog.stored(
            run,
            "rev-first",
            "review",
            review("First reviewer", "r1", Some(first.clone())),
        );
    };
    let revised = |run: &str| {
        catalog.moved(run, "revise", "draft");
        catalog.stored(run, "d2", "draft", json!({"revision":"r2"}));
        catalog.moved(run, "drafted", "review");
    };
    let change = amendment(
        json!([{"role":"ordinary-reviewer","old":first,"new":second}]),
        Some(driver.clone()),
    );

    // Unmarked change: the review made under the old binding still approves.
    reviewed("unmarked");
    catalog.stored(
        "unmarked",
        "change",
        "model-binding-amendment",
        change.clone(),
    );
    catalog.moved("unmarked", "approved", "done");

    // Unmarked change: the old review, carried to a new draft, still approves.
    reviewed("carried");
    revised("carried");
    catalog.stored(
        "carried",
        "change",
        "model-binding-amendment",
        change.clone(),
    );
    catalog.stored(
        "carried",
        "carry",
        "evidence-applicability",
        carry("rev-first", "r2"),
    );
    catalog.moved("carried", "approved", "done");
    let context = catalog.full("carried")["context"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        context
            .iter()
            .filter(|record| record["kind"] == "review")
            .count(),
        1,
        "no review was rerun"
    );

    // Stale mark: the old review cannot be carried to the new draft.
    reviewed("stale-carry");
    revised("stale-carry");
    catalog.stored(
        "stale-carry",
        "stale",
        "model-binding-amendment",
        amendment(
            json!([{"role":"ordinary-reviewer","old":first,"new":null,"mark_stale":true}]),
            Some(driver.clone()),
        ),
    );
    catalog.stored(
        "stale-carry",
        "carry",
        "evidence-applicability",
        carry("rev-first", "r2"),
    );
    let denied = catalog.approval_agrees("stale-carry", false);
    for needle in [
        "stale",
        "rev-first",
        "First reviewer",
        "ordinary-reviewer",
        "correctness",
    ] {
        assert!(
            denied.to_string().contains(needle),
            "missing {needle}: {denied}"
        );
    }
    assert!(denied.to_string().contains("\"stale\""), "{denied}");
    // The earlier decision and every original record stay as they were.
    let shown = catalog.full("stale-carry");
    let history = shown["evaluation_history"].as_array().unwrap();
    assert!(
        history
            .iter()
            .filter(|entry| entry.to_string().contains("drafted"))
            .count()
            >= 2,
        "{shown}"
    );
    assert!(shown["context"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record["id"] == "rev-first" && record["data"]["session"] == first));
    catalog.stored(
        "stale-carry",
        "rev-fresh",
        "review",
        review("Fresh reviewer", "r2", None),
    );
    catalog.approval_agrees("stale-carry", true);
}

#[test]
// bookends:LE-199 — group coverage counts returns made under different bindings, author independence is unchanged, and commission and the checked event agree.
// bookends:LE-200 — the owner approval view prepared by commission shows the same bindings and history as status and full.
fn model_bindings_mixed_binding_group_and_owner_view_public_binary() {
    let catalog = Catalog::new();
    let first = binding("openai/gpt", "xhigh");
    let second = binding("openai/gpt-mini", "medium");
    let driver = binding("anthropic/claude-opus", "high");
    let run = "grouped";
    assert_eq!(
        catalog.start(run, &json!({"required_authors": 2}))["status"],
        "completed"
    );
    let never = catalog.commission(run)["owner_approval_view"]["model_bindings"].clone();
    assert_eq!(never, catalog.full(run)["model_bindings"]);
    assert_eq!(never["state"], "never-bound", "{never}");
    catalog.stored(run, "add", "model-binding-amendment",
        amendment(json!([{"role":"ordinary-reviewer","old":null,"new":first},{"role":"driver","old":null,"new":driver}]), Some(driver.clone())));
    catalog.stored(run, "d1", "draft", json!({"revision":"r1"}));
    catalog.moved(run, "drafted", "review");
    catalog.stored(
        run,
        "rev-a",
        "review",
        review("Reviewer A", "r1", Some(first.clone())),
    );
    catalog.stored(
        run,
        "change",
        "model-binding-amendment",
        amendment(
            json!([{"role":"ordinary-reviewer","old":first,"new":second}]),
            Some(driver.clone()),
        ),
    );
    // The same author again, under the new binding, still counts once.
    catalog.stored(
        run,
        "rev-a2",
        "review",
        review("Reviewer A", "r1", Some(second.clone())),
    );
    let short = catalog.approval_agrees(run, false);
    assert!(short.to_string().contains("Reviewer A"), "{short}");
    catalog.stored(
        run,
        "rev-b",
        "review",
        review("Reviewer B", "r1", Some(second.clone())),
    );
    let view = catalog.commission(run)["owner_approval_view"]["model_bindings"].clone();
    let status = catalog.cli(&["show", "--view", "status", run]);
    assert_eq!(view, catalog.full(run)["model_bindings"]);
    assert_eq!(view, status["result"]["model_bindings"], "{status}");
    let row = view["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["role"] == "ordinary-reviewer")
        .unwrap()
        .clone();
    assert!(
        row["original"].is_null() && row["in_force"] == second,
        "{view}"
    );
    assert_eq!(view["history"].as_array().unwrap().len(), 2, "{view}");
    let allowed = catalog.approval_agrees(run, true);
    assert_eq!(
        allowed["details"]["eligible_authors"],
        json!(["Reviewer A", "Reviewer B"]),
        "{allowed}"
    );
}
