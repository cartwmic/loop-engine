//! Inert external review intake. No catalog access or worker execution.
use loop_integrations::external_work::{assemble_items, read_content, ContentSource};
use serde_json::{json, Value};
use std::io::{self, Read, Write};

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty `{key}`"))
}

fn content(value: &Value) -> Result<Value, String> {
    let source = match (value.get("text"), value.get("reference")) {
        (Some(Value::String(text)), None) => ContentSource::Inline(text.clone()),
        (None, Some(Value::String(path))) if !path.trim().is_empty() => {
            ContentSource::File(path.into())
        }
        _ => return Err("content requires exactly one text or reference".into()),
    };
    serde_json::to_value(read_content(source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn project(input: &Value) -> Result<Value, String> {
    let show = &input["show"];
    if show["operation"] != "show"
        || show["status"] != "completed"
        || show["result"]["workflow_id"] != "software-change"
    {
        return Err("requires completed software-change full show envelope".into());
    }
    let initial = &show["result"]["initial_input"];
    let assignment = &input["assignment"];
    let gate = text(assignment, "gate")?;
    let stage = text(assignment, "stage")?;
    let group = text(assignment, "group")?;
    let subject = text(assignment, "subject")?;
    let revision = text(assignment, "subject_revision")?;
    let config = text(initial, "config_version")?;
    let author = &assignment["author"];
    text(author, "name")?;
    if !["human", "agent", "script"].contains(&text(author, "kind")?) {
        return Err("invalid author kind".into());
    }
    let original = content(&input["original"])?;
    let request = content(&input["request"])?;
    let projection = input.get("projection").unwrap_or(&original["structured"]);
    let mut attributed_projection = projection.clone();
    if attributed_projection.get("author").is_none() && attributed_projection.is_object() {
        attributed_projection["author"] = author.clone();
    }
    loop_integrations::external_work::check_projection(
        &original["structured"],
        &attributed_projection,
        "judgments",
        gate,
    )?;
    let judgments = projection["judgments"]
        .as_array()
        .ok_or("original is not a JSON review batch; supply projection.judgments")?;
    if projection
        .get("author")
        .is_some_and(|claimed| claimed != author)
    {
        return Err("batch author contradicts assignment author".into());
    }
    let assessment = projection.get("assessment_id").cloned();
    let policy = &initial["review_policies"][gate];
    let axes = policy
        .as_array()
        .ok_or("assignment gate has no review policy")?;
    let root = show["result"]
        .get("artifact_root")
        .or_else(|| initial.get("artifact_root"))
        .and_then(Value::as_str);
    let subject_value = root
        .and_then(|root| std::fs::read(std::path::Path::new(root).join(subject)).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let direct = subject_value
        .as_ref()
        .is_some_and(|value| value["author"] == *author);
    let known = json!({"gate":gate,"review_stage":stage,"group":group,"author":author,
        "subject":subject,"subject_revision":revision,"config_version":config,
        "review_contract_version":3});
    let mut items = assemble_items(judgments, known.as_object().unwrap());
    let mut records = Vec::new();
    for item in &mut items {
        let Some(mut data) = item.candidate.take() else {
            continue;
        };
        let validation = (|| -> Result<(), String> {
            let axis = text(&data, "axis")?;
            if data.get("policy_id").is_some_and(|id| id != axis) {
                return Err("policy_id contradicts projected axis".into());
            }
            let axis_policy = axes
                .iter()
                .find(|p| p["id"] == axis && p["review_stage"] == stage)
                .ok_or("axis is not configured for assignment stage")?;
            if axis_policy["group"] != group {
                return Err("axis does not belong to assignment group".into());
            }
            if direct && axis == "intent-sufficient" && gate == "intent-review" {
                return Err("cold intent-sufficient reconstruction cannot be self-review".into());
            }
            if direct && axis_policy["self_review"] != true {
                return Err("self review is not allowed for this axis".into());
            }
            if data.get("reuse").is_some() {
                if data.as_object().is_none_or(|row| {
                    row.keys().any(|key| {
                        !known.as_object().unwrap().contains_key(key)
                            && key != "axis"
                            && key != "reuse"
                    })
                }) {
                    return Err("reuse row must contain only axis and reuse".into());
                }
                let mut location = show["result"].clone();
                location["artifact_root"] = json!(root);
                if let Some(controls) = input.get("controls") {
                    location["controls"] = controls.clone();
                }
                return crate::review_batch::validate_reuse_row(
                    &data, &location, gate, axis, author, config, subject, revision, stage, true,
                    false,
                );
            }
            if !["pass", "fail"].contains(&text(&data, "result")?) {
                return Err("result must be pass or fail".into());
            }
            if data["findings"].as_str().is_none() {
                return Err("findings must be a string".into());
            }
            if data["result"] == "fail"
                && data["findings"]
                    .as_str()
                    .is_some_and(|s| s.trim().is_empty())
            {
                return Err("failing judgment requires substantive findings".into());
            }
            text(&data["grounds"], "reason")?;
            let evidence = data["grounds"]["evidence"]
                .as_array()
                .ok_or("grounds.evidence must be an array")?;
            if evidence.is_empty() {
                return Err("grounds.evidence must not be empty".into());
            }
            for reference in evidence {
                text(reference, "locator")?;
            }
            Ok(())
        })();
        if let Err(error) = validation {
            item.diagnostics.push(error);
            continue;
        }
        if let Some(id) = data.get("reuse").and_then(Value::as_str) {
            let source = show["result"]["context"]
                .as_array()
                .unwrap()
                .iter()
                .find(|record| record["id"] == id)
                .unwrap();
            let candidate =
                json!({"kind":"evidence-applicability","data":source["data"],"reuse_record_id":id});
            records.push(candidate.clone());
            item.candidate = Some(candidate);
            continue;
        }
        let axis = data["axis"].clone();
        data["policy_id"] = axis;
        if data["policy_id"] == "intent-sufficient" {
            data["baseline"] = subject_value
                .as_ref()
                .and_then(|v| v.get("baseline"))
                .cloned()
                .unwrap_or(Value::Null);
        }
        if let Some(id) = &assessment {
            if id.as_str().is_none_or(|s| s.trim().is_empty()) {
                item.diagnostics
                    .push("assessment_id must be nonempty".into());
                continue;
            }
            data["assessment_id"] = id.clone();
        }
        data["external_review"] = json!({"request":request,"original":original,"direct":direct});
        if let Some(formatter) = input.get("formatter") {
            text(formatter, "name")?;
            if !["human", "agent", "script"].contains(&text(formatter, "kind")?) {
                return Err("invalid formatter kind".into());
            }
            data["external_review"]["formatter"] = formatter.clone();
        }
        let candidate = json!({"kind":"review-evidence","data":data});
        records.push(candidate.clone());
        item.candidate = Some(candidate);
    }
    Ok(json!({"schema_version":"3","records":records,"items":items}))
}

pub fn run_from_stdin() -> i32 {
    let mut bytes = String::new();
    let result = io::stdin()
        .read_to_string(&mut bytes)
        .map_err(|e| e.to_string())
        .and_then(|_| serde_json::from_str(&bytes).map_err(|e| e.to_string()))
        .and_then(|input| project(&input));
    match result {
        Ok(document) => match serde_json::to_writer(io::stdout().lock(), &document)
            .map_err(io::Error::other)
            .and_then(|_| io::stdout().flush())
        {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("review-candidates: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("review-candidates: {error}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"show":{"operation":"show","status":"completed","result":{"workflow_id":"software-change","initial_input":{"config_version":"14","review_policies":{"intent-review":[{"id":"intent-sufficient","review_stage":"aggregate","group":"intent-sufficient","self_review":true}]}},"context":[]}},"assignment":{"gate":"intent-review","stage":"aggregate","group":"intent-sufficient","author":{"name":"reviewer","kind":"agent"},"subject":"intent.json","subject_revision":"1"},"request":{"text":"review instructions"},"original":{"text":"Prose judgment"},"projection":{"judgments":[{"axis":"intent-sufficient","result":"pass","findings":"","grounds":{"reason":"adequate","evidence":[{"locator":"intent.json#goal"}]}},false,{"axis":"intent-sufficient","result":"pass"}]}})
    }
    #[test]
    fn malformed_siblings_preserve_prose_and_valid_candidate() {
        let output = project(&input()).unwrap();
        assert_eq!(output["records"].as_array().unwrap().len(), 1);
        assert_eq!(
            output["records"][0]["data"]["external_review"]["original"]["original"],
            "Prose judgment"
        );
        assert!(output["items"][1]["candidate"].is_null());
        assert!(output["items"][2]["candidate"].is_null());
    }
    #[test]
    fn invalid_reuse_preserves_fresh_sibling() {
        let mut input = input();
        input["projection"]["judgments"][1] = json!({"axis":"intent-sufficient","reuse":"missing"});
        let output = project(&input).unwrap();
        assert_eq!(output["records"].as_array().unwrap().len(), 1);
        assert!(output["items"][1]["candidate"].is_null());
        assert!(output["items"][1]["diagnostics"]
            .to_string()
            .contains("not an authorized captured applicability record"));
    }
    #[test]
    fn valid_reuse_projects_applicability_without_new_judgment() {
        use loop_core::{ContextRecord, SemanticSequence, Timestamp};
        let mut input = input();
        let artifacts = std::env::temp_dir().join(format!(
            "review-candidates-reuse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&artifacts).unwrap();
        std::fs::write(
            artifacts.join("intent.json"),
            r#"{"revision":"1","goal":"Inspected goal"}"#,
        )
        .unwrap();
        input["show"]["result"]["initial_input"]["artifact_root"] = json!(artifacts);
        input["projection"]["judgments"][0]["grounds"]["evidence"][0]["locator"] =
            json!("intent.json#/goal");
        let original = project(&input).unwrap()["records"][0]["data"].clone();
        let carry = json!({"origin":{"kind":"context-record","id":"original"},
            "target":{"subject":"intent.json","revision":"1","checkpoint":null},
            "attesting_driver":{"name":"driver","kind":"agent"},"reason":"Inspected unchanged scope"});
        let record = |id, kind, sequence, data| {
            ContextRecord::new(
                id,
                kind,
                data,
                SemanticSequence::new(sequence),
                Timestamp::from_unix_millis(0),
            )
        };
        input["show"]["result"]["context"] = json!([
            record("original", "review-evidence", 1, original),
            record("carry", "evidence-applicability", 2, carry.clone())
        ]);
        input["projection"]["judgments"] = json!([{"axis":"intent-sufficient","reuse":"carry"}]);
        let output = project(&input).unwrap();
        assert_eq!(output["records"].as_array().unwrap().len(), 1, "{output}");
        assert_eq!(output["records"][0]["kind"], "evidence-applicability");
        assert_eq!(output["records"][0]["reuse_record_id"], "carry");
        assert_eq!(output["records"][0]["data"], carry);
        for (field, value) in [
            ("/show/result/context/1/kind", json!("review-evidence")),
            ("/show/result/context/1/data/origin/id", json!("missing")),
            (
                "/show/result/context/1/data/target/revision",
                json!("stale"),
            ),
            ("/show/result/context/0/data/gate", json!("design-review")),
            (
                "/show/result/context/0/data/author/name",
                json!("wrong-author"),
            ),
            ("/show/result/context/0/sequence", json!(3)),
        ] {
            let mut invalid = input.clone();
            *invalid.pointer_mut(field).unwrap() = value;
            let refused = project(&invalid).unwrap();
            assert!(
                refused["records"].as_array().unwrap().is_empty(),
                "{field}: {refused}"
            );
            assert!(!refused["items"][0]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        input["controls"] = json!({"force_fresh":true});
        let refused = project(&input).unwrap();
        assert!(refused["records"].as_array().unwrap().is_empty());
        assert!(refused["items"][0]["diagnostics"]
            .to_string()
            .contains("force-fresh"));
        std::fs::remove_dir_all(artifacts).unwrap();
    }
    #[test]
    fn fenced_json_derives_projection() {
        let mut input = input();
        input["original"] = json!({"text":format!("```json\n{}\n```", input["projection"])});
        input.as_object_mut().unwrap().remove("projection");
        assert_eq!(
            project(&input).unwrap()["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn materialized_narrative_assignments_project_all_shipped_stages_and_groups() {
        for profile in [
            include_str!("../data/configs/minimal.json"),
            include_str!("../data/configs/standard.json"),
            include_str!("../data/configs/high-rigor.json"),
        ] {
            let mut profile: Value = serde_json::from_str(profile).unwrap();
            for phase in ["design", "plan", "validation"] {
                for (pass, suffix) in [("ordinary", "review"), ("challenge", "adversarial-review")]
                {
                    let gate = format!("{phase}-narrative-{suffix}");
                    let mut axes = profile["extra"]["narrative_review"][pass].clone();
                    for axis in axes.as_array_mut().unwrap() {
                        axis["group"] =
                            json!(axis["group"].as_str().unwrap().replace("{phase}", phase));
                    }
                    profile["review_policies"][&gate] = axes.clone();
                    for axis in axes.as_array().unwrap() {
                        let mut packet = input();
                        packet["show"]["result"]["initial_input"] = profile.clone();
                        packet["assignment"]["gate"] = json!(gate);
                        packet["assignment"]["stage"] = axis["review_stage"].clone();
                        packet["assignment"]["group"] = axis["group"].clone();
                        packet["assignment"]["subject"] = json!(format!("{phase}-narrative.json"));
                        packet["assignment"]["subject_revision"] = json!("3/1");
                        packet["projection"] = json!({"assessment_id":"genuine-return", "judgments":[{
                            "axis":axis["id"],"result":"pass","findings":"",
                            "grounds":{"reason":"exact approved sources agree","evidence":[{
                                "locator":format!("{phase}-narrative.md#L1")}]}}]});
                        packet["original"] = json!({"text":packet["projection"].to_string()});
                        let output = project(&packet).unwrap();
                        assert_eq!(output["records"].as_array().unwrap().len(), 1, "{output}");
                        assert_eq!(output["items"][0]["diagnostics"], json!([]));
                        let record = &output["records"][0]["data"];
                        assert_eq!(record["subject_revision"], "3/1");
                        assert_eq!(record["review_stage"], axis["review_stage"]);
                        assert_eq!(record["group"], axis["group"]);
                        assert_eq!(record["policy_id"], axis["id"]);
                        assert_eq!(
                            record["external_review"]["original"]["original"],
                            packet["original"]["text"]
                        );
                        packet["assignment"]["group"] = json!("wrong-group");
                        let output = project(&packet).unwrap();
                        assert!(output["records"].as_array().unwrap().is_empty());
                        assert!(output["items"][0]["diagnostics"]
                            .to_string()
                            .contains("assignment group"));
                    }
                }
            }
        }
    }
}
