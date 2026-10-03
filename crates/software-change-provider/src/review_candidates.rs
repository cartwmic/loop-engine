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
            if direct && axis_policy["self_review"] != true {
                return Err("self review is not allowed for this axis".into());
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
        let axis = data["axis"].clone();
        data["policy_id"] = axis;
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
}
