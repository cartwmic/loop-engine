use crate::config::parse_initial_input;
use loop_integrations::{author_counts, review_intake};
use serde_json::{json, Value};
use std::io::Read;

pub fn counts(input: &Value, context: &Value) -> author_counts::Resolution {
    let mut base = serde_json::Map::new();
    for (gate, entries) in input["review_policies"].as_object().into_iter().flatten() {
        for p in entries.as_array().into_iter().flatten() {
            if let Some(id) = p["id"].as_str() {
                base.insert(
                    format!("{gate}.{id}"),
                    json!(p["required_authors"].as_u64().unwrap_or(1)),
                );
            }
        }
    }
    author_counts::resolve(&Value::Object(base), context)
}

fn document(input: &Value, subject: &str) -> Result<Value, String> {
    match crate::artifacts::read_artifact(input.get("artifact_root"), subject) {
        crate::artifacts::ArtifactReadOutcome::Present(document) => Ok(document.value().clone()),
        crate::artifacts::ArtifactReadOutcome::Deny(deny) => Err(format!("{subject}: {deny:?}")),
        crate::artifacts::ArtifactReadOutcome::EvaluationError(error) => Err(error.to_string()),
    }
}

pub fn run(command: &str, args: &[String]) -> i32 {
    let result = (|| {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| e.to_string())?;
        let packet: Value = serde_json::from_str(&buffer).map_err(|e| e.to_string())?;
        let show = if command == "review-candidates" {
            &packet["show"]
        } else {
            &packet
        };
        if show["operation"] != "show"
            || show["status"] != "completed"
            || show["result"]["workflow_id"] != "research"
        {
            return Err("requires completed research full show".into());
        }
        let initial = &show["result"]["initial_input"];
        parse_initial_input(initial).map_err(|e| e.to_string())?;
        if command == "author-counts" {
            let resolution = counts(initial, &show["result"]["context"]);
            if args.is_empty() {
                return Ok(author_counts::inspect(&resolution));
            }
            if args.len() == 2 && args[0] == "--propose" {
                return author_counts::propose(
                    &resolution,
                    serde_json::from_str(&args[1]).map_err(|e| e.to_string())?,
                );
            }
            return Err("usage: author-counts [--propose JSON]".into());
        }
        if command == "commission" {
            let gate = args.first().map(String::as_str).unwrap_or("verify");
            if args.len() > 1 || !matches!(gate, "verify" | "synthesize") {
                return Err("usage: commission [verify|synthesize] < full-show.json".into());
            }
            let subject = if gate == "verify" {
                "verification.json"
            } else {
                "report.json"
            };
            let mut missing = Vec::new();
            let mut artifacts = serde_json::Map::new();
            for name in [
                "brief.json",
                "sources.json",
                "verification.json",
                "report.json",
            ] {
                match document(initial, name) {
                    Ok(value) => {
                        artifacts.insert(name.into(), value);
                    }
                    Err(reason) => missing.push(json!({"input":name,"reason":reason})),
                }
            }
            let resolution = counts(initial, &show["result"]["context"]);
            let mut policies = initial["review_policies"][gate].clone();
            for p in policies.as_array_mut().into_iter().flatten() {
                if let Some(id) = p["id"].as_str() {
                    p["required_authors"] = resolution.effective[format!("{gate}.{id}")].clone();
                }
            }
            return Ok(
                json!({"gate":gate,"subject":subject,"subject_revision":artifacts.get(subject).and_then(|v|v.get("revision")),"config_version":initial["config_version"],"review_policies":policies,"author_counts":author_counts::inspect(&resolution),"artifacts":artifacts,"context":show["result"]["context"],"missing_inputs":missing}),
            );
        }
        if !args.is_empty() {
            return Err("review-candidates accepts stdin only".into());
        }
        let subject = packet["assignment"]["subject"]
            .as_str()
            .ok_or("missing subject")?;
        let gate = packet["assignment"]["gate"]
            .as_str()
            .ok_or("missing gate")?;
        if !matches!(
            (gate, subject),
            ("verify", "verification.json") | ("synthesize", "report.json")
        ) {
            return Err("gate/subject mismatch".into());
        }
        let current = document(initial, subject)?;
        if packet["assignment"]["subject_revision"] != current["revision"] {
            return Err("assignment revision is stale".into());
        }
        let mut policies = Vec::new();
        for (gate, entries) in initial["review_policies"].as_object().into_iter().flatten() {
            for p in entries.as_array().into_iter().flatten() {
                let mut p = p.clone();
                p["gate"] = json!(gate);
                policies.push(p);
            }
        }
        let mut result = review_intake::project(
            &packet,
            "research",
            &policies,
            &json!({"config_version":initial["config_version"]}),
        )?;
        for candidate in result["candidates"].as_array_mut().into_iter().flatten() {
            candidate["data"].as_object_mut().unwrap().remove("axis");
            candidate["data"]["config_version"] = initial["config_version"].clone();
        }
        Ok(result)
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
