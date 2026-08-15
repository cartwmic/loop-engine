mod config;
mod document;
mod embedded_data;
mod evidence;
mod policy;
mod protocol;
mod workflow;

use loop_core::TransitionKind;
use protocol::{DescribeRequest, EvaluateRequest};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::path::Path;

fn main() {
    std::process::exit(run());
}
fn run() -> i32 {
    let mut args = std::env::args_os();
    let _ = args.next();
    if let Some(command) = args.next() {
        if command == "data-dump" {
            let Some(dest) = args.next() else {
                eprintln!("usage: policy-document data-dump DIR");
                return 2;
            };
            if args.next().is_some() {
                eprintln!("data-dump accepts one destination");
                return 2;
            }
            return match embedded_data::dump(Path::new(&dest)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("data-dump failed: {e}");
                    1
                }
            };
        }
        eprintln!("unsupported command `{}`", command.to_string_lossy());
        return 2;
    }
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        return protocol_error(format!("could not read request: {e}"));
    }
    let value: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => return protocol_error(format!("malformed JSON request: {e}")),
    };
    match value.get("operation").and_then(Value::as_str) {
        Some("describe") => describe(value),
        Some("evaluate") => evaluate(value),
        Some(other) => protocol_error(format!("unsupported provider operation `{other}`")),
        None => protocol_error("request must contain string operation".into()),
    }
}
fn describe(value: Value) -> i32 {
    match serde_json::from_value::<DescribeRequest>(value) {
        Ok(req) if req.operation == "describe" => write_json(&workflow::workflow()),
        Ok(_) => protocol_error("describe request has wrong operation".into()),
        Err(e) => protocol_error(format!("invalid describe request: {e}")),
    }
}
fn evaluate(value: Value) -> i32 {
    let request = match serde_json::from_value::<EvaluateRequest>(value) {
        Ok(req) if req.operation == "evaluate" => req,
        Ok(_) => return protocol_error("evaluate request has wrong operation".into()),
        Err(e) => return protocol_error(format!("invalid evaluate request: {e}")),
    };
    match evaluation_response(&request) {
        Ok(response) => write_json(&response),
        Err(error) => evaluation_error(&error),
    }
}
fn evaluation_response(request: &EvaluateRequest) -> Result<Value, String> {
    if request.workflow != workflow::workflow() {
        return Ok(protocol::unsupported());
    }
    let checked = request.transition.kind == TransitionKind::Checked;
    let route = (
        request.transition.source.as_str(),
        request.transition.event.as_str(),
        request.transition.target.as_str(),
        checked,
    );
    if !matches!(
        route,
        ("deterministic-review", "passed", "semantic-review", true)
            | ("semantic-review", "passed", "end", true)
            | ("prepare", "ready", "deterministic-review", false)
            | ("deterministic-review", "revise", "prepare", false)
            | ("semantic-review", "revise", "prepare", false)
    ) {
        return Ok(protocol::unsupported());
    }
    if !checked {
        return Ok(protocol::allow());
    }
    let _ = &request.prior_evaluations;
    let config = config::InitialInput::parse(&request.initial_input)?;
    let _ = &config.mode;
    let snapshot = document::Snapshot::read(&config.target)?;
    let findings = policy::evaluate(&snapshot, &config.deterministic_policies);
    if !findings.is_empty() {
        return Ok(protocol::deny(
            "policy-document-nonconforming",
            "deterministic policy violations",
            json!({"phase":"deterministic","violations":findings}),
        ));
    }
    if request.transition.source.as_str() == "semantic-review" {
        let (satisfied, details) = evidence::evaluate(&request.context, &config, &snapshot);
        if !satisfied {
            return Ok(protocol::deny(
                "policy-document-review-incomplete",
                "semantic review evidence incomplete",
                json!({"phase":"semantic","details":details,"target_sha256":snapshot.sha256}),
            ));
        }
    }
    Ok(protocol::allow())
}

fn write_json<T: Serialize>(value: &T) -> i32 {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match serde_json::to_writer(&mut out, value) {
        Ok(()) => match out.flush() {
            Ok(()) => 0,
            Err(e) => protocol_error(format!("could not flush response: {e}")),
        },
        Err(e) => protocol_error(format!("could not write response: {e}")),
    }
}
fn protocol_error(message: String) -> i32 {
    eprintln!("{message}");
    2
}
fn evaluation_error(message: &str) -> i32 {
    eprintln!("{message}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{ContextRecord, SemanticSequence, Timestamp, Transition};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn target(content: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "policy-provider-main-{}-{}.md",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, content).unwrap();
        path
    }

    fn input(path: &Path) -> Value {
        json!({"schema_version":1,"profile_version":"test-1","mode":"draft","target":{"id":"doc","path":path},"deterministic_policies":[{"id":"title","type":"any-heading","level":1}],"semantic_policies":[{"id":"quality","description":"quality","example_prompt":"review quality"}]})
    }

    fn request(
        path: &Path,
        transition: Transition,
        context: Vec<ContextRecord>,
    ) -> EvaluateRequest {
        EvaluateRequest {
            operation: "evaluate".into(),
            workflow: workflow::workflow(),
            initial_input: input(path),
            context,
            transition,
            prior_evaluations: Vec::new(),
        }
    }

    fn evidence(path: &Path) -> ContextRecord {
        let digest = document::Snapshot::read(&config::Target {
            id: "doc".into(),
            path: path.display().to_string(),
        })
        .unwrap()
        .sha256;
        ContextRecord::new(
            "evidence",
            "review-evidence",
            json!({"gate":"semantic-review","policy_id":"quality","result":"pass","findings":"","author":{"name":"reviewer","kind":"agent"},"target_id":"doc","target_sha256":digest,"profile_version":"test-1"}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(1),
        )
    }

    #[test]
    fn unsupported_workflow_and_route_are_explicit() {
        let path = target("# Title\n");
        let mut wrong = request(
            &path,
            Transition::checked("other", "passed", "end"),
            Vec::new(),
        );
        assert_eq!(
            evaluation_response(&wrong).unwrap()["result"],
            "unsupported"
        );
        wrong.workflow = loop_core::Workflow::new(
            "other",
            "end",
            vec![loop_core::State::new("end", "End", "done", true)],
            vec![],
        );
        assert_eq!(
            evaluation_response(&wrong).unwrap()["result"],
            "unsupported"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn check_free_routes_allow_without_reading_target() {
        let missing = Path::new("/definitely/missing/policy-document.md");
        for transition in [
            Transition::check_free("prepare", "ready", "deterministic-review"),
            Transition::check_free("deterministic-review", "revise", "prepare"),
            Transition::check_free("semantic-review", "revise", "prepare"),
        ] {
            assert_eq!(
                evaluation_response(&request(missing, transition, Vec::new())).unwrap()["result"],
                "allow"
            );
        }
    }

    #[test]
    fn deterministic_and_final_routes_reject_fenced_or_empty_h1() {
        for content in ["```md\n# Fake\n```\n", "# ###\n"] {
            let path = target(content);
            for transition in [
                Transition::checked("deterministic-review", "passed", "semantic-review"),
                Transition::checked("semantic-review", "passed", "end"),
            ] {
                let response =
                    evaluation_response(&request(&path, transition, Vec::new())).unwrap();
                assert_eq!(
                    response["feedback"]["code"],
                    "policy-document-nonconforming"
                );
                assert_eq!(response["feedback"]["details"]["phase"], "deterministic");
            }
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn semantic_finalization_rechecks_same_current_snapshot() {
        let path = target("# Title\n");
        let context = vec![evidence(&path)];
        let final_route = Transition::checked("semantic-review", "passed", "end");
        assert_eq!(
            evaluation_response(&request(&path, final_route.clone(), context.clone())).unwrap()
                ["result"],
            "allow"
        );
        fs::write(&path, "no heading\n").unwrap();
        let response = evaluation_response(&request(&path, final_route, context)).unwrap();
        assert_eq!(
            response["feedback"]["code"],
            "policy-document-nonconforming"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn target_read_and_utf8_failures_are_evaluation_errors() {
        let missing = Path::new("/definitely/missing/policy-document.md");
        let route = Transition::checked("deterministic-review", "passed", "semantic-review");
        assert!(
            evaluation_response(&request(missing, route.clone(), Vec::new()))
                .unwrap_err()
                .contains("target `")
        );

        let invalid_path = target("valid");
        fs::write(&invalid_path, [0xff]).unwrap();
        assert_eq!(
            evaluation_response(&request(&invalid_path, route, Vec::new())).unwrap_err(),
            "target is not valid UTF-8"
        );
        fs::remove_file(invalid_path).unwrap();
    }

    #[test]
    fn bad_config_is_evaluation_error() {
        let path = Path::new("/definitely/missing/policy-document.md");
        let mut invalid = request(
            path,
            Transition::checked("deterministic-review", "passed", "semantic-review"),
            Vec::new(),
        );
        invalid.initial_input["unknown"] = json!(true);
        assert!(evaluation_response(&invalid)
            .unwrap_err()
            .contains("unknown field"));
    }

    #[test]
    fn checked_passed_evaluate_parses_composed_object_with_artifact_root() {
        let path = target("# Title\n");
        let mut req = request(
            &path,
            Transition::checked("deterministic-review", "passed", "semantic-review"),
            Vec::new(),
        );
        req.initial_input["artifact_root"] = json!("/tmp/unused");
        let response = evaluation_response(&req).unwrap_or_else(|error| {
            panic!("composed object with artifact_root must parse; got evaluation error: {error}")
        });
        let result = response["result"].as_str().unwrap_or("");
        assert!(
            result == "allow" || result == "deny",
            "expected allow or policy deny, got {response}"
        );
        assert_ne!(
            response["feedback"]["code"], "invalid-initial-input",
            "{response}"
        );
        fs::remove_file(path).unwrap();
    }
}
