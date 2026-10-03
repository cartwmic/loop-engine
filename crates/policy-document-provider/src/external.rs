use crate::{config::InitialInput, document::Snapshot};
use loop_integrations::{author_counts, review_intake};
use serde_json::{json, Value};
use std::io::Read;

pub fn counts(input: &Value, context: &Value) -> author_counts::Resolution {
    let mut base = serde_json::Map::new();
    for p in input["semantic_policies"].as_array().into_iter().flatten() {
        if let Some(id) = p["id"].as_str() {
            base.insert(
                format!("semantic-review.{id}"),
                json!(p["required_authors"].as_u64().unwrap_or(1)),
            );
        }
    }
    author_counts::resolve(&Value::Object(base), context)
}

pub fn run(command: &str, args: &[String]) -> i32 {
    let result = (|| {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| e.to_string())?;
        let packet: Value = serde_json::from_str(&buffer).map_err(|e| e.to_string())?;
        if command == "author-counts" {
            if packet["operation"] != "show" || packet["status"] != "completed" {
                return Err("requires completed full show".into());
            }
            let input = &packet["result"]["initial_input"];
            InitialInput::parse(input)?;
            let resolution = counts(input, &packet["result"]["context"]);
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
        if !args.is_empty() {
            return Err("review-candidates accepts stdin only".into());
        }
        let initial = &packet["show"]["result"]["initial_input"];
        let config = InitialInput::parse(initial)?;
        let snapshot = Snapshot::read(&config.target)?;
        if packet["assignment"]["subject"] != snapshot.sha256
            || packet["assignment"]["subject_revision"] != snapshot.sha256
        {
            return Err("assignment target or digest is stale".into());
        }
        let policies: Vec<Value> = config.semantic_policies.iter().map(|p| json!({"gate":"semantic-review","id":p.id,"group":p.group,"self_review":p.self_review})).collect();
        let mut result = review_intake::project(
            &packet,
            "policy-document",
            &policies,
            &json!({"target_id":snapshot.target_id,"target_sha256":snapshot.sha256,"profile_version":config.profile_version}),
        )?;
        for candidate in result["candidates"].as_array_mut().into_iter().flatten() {
            let data = candidate["data"].as_object_mut().unwrap();
            data.remove("axis");
            data.remove("subject");
            data.remove("subject_revision");
            data.insert("target_id".into(), json!(snapshot.target_id));
            data.insert("target_sha256".into(), json!(snapshot.sha256));
            data.insert("profile_version".into(), json!(config.profile_version));
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
