//! Inert intake shared by providers; providers supply their own frozen policies.
use crate::external_work::{assemble_items, read_content, ContentSource};
use serde_json::{json, Value};

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty {key}"))
}
fn content(v: &Value) -> Result<Value, String> {
    let source = match (v.get("text"), v.get("reference")) {
        (Some(Value::String(s)), None) => ContentSource::Inline(s.clone()),
        (None, Some(Value::String(s))) if !s.is_empty() => ContentSource::File(s.into()),
        _ => return Err("content requires exactly one text or reference".into()),
    };
    serde_json::to_value(read_content(source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn project(
    input: &Value,
    provider: &str,
    policies: &[Value],
    identity: &Value,
) -> Result<Value, String> {
    let show = &input["show"];
    if show["operation"] != "show"
        || show["status"] != "completed"
        || show["result"]["workflow_id"] != provider
    {
        return Err(format!("requires completed {provider} full show envelope"));
    }
    let assignment = &input["assignment"];
    let gate = text(assignment, "gate")?;
    text(assignment, "review_stage")?;
    text(assignment, "subject")?;
    text(assignment, "subject_revision")?;
    if input.get("self_review").is_some_and(|v| !v.is_boolean()) {
        return Err("self_review must be boolean".into());
    }
    let direct = input["self_review"] == true;
    if !matches!(
        input["author"]["kind"].as_str(),
        Some("human" | "agent" | "script")
    ) {
        return Err("invalid author.kind".into());
    }
    text(&input["author"], "name")?;
    let declared = assignment["policy_ids"]
        .as_array()
        .ok_or("policy_ids must be an array")?;
    if declared.is_empty() {
        return Err("policy_ids must be nonempty".into());
    }
    let ids: Vec<&str> = declared
        .iter()
        .map(|v| v.as_str().ok_or("policy id must be string"))
        .collect::<Result<_, _>>()?;
    let mut known = serde_json::Map::new();
    for id in &ids {
        if known.contains_key(*id) {
            return Err("duplicate policy id".into());
        }
        let p = policies
            .iter()
            .find(|p| p["gate"] == gate && p["id"] == *id)
            .ok_or_else(|| format!("unknown policy {gate}.{id}"))?;
        if direct && p["self_review"] != true {
            return Err(format!("direct self review forbidden for {id}"));
        }
        known.insert((*id).into(), p.clone());
    }
    let groups: std::collections::BTreeSet<String> = ids
        .iter()
        .map(|id| known[*id]["group"].as_str().unwrap_or(id).to_owned())
        .collect();
    if groups.len() != 1 {
        return Err("assignment combines independent review groups".into());
    }
    let group = groups.first().unwrap();
    let expected: Vec<_> = policies
        .iter()
        .filter(|p| {
            p["gate"] == gate
                && p["group"]
                    .as_str()
                    .unwrap_or(p["id"].as_str().unwrap_or(""))
                    == group
        })
        .collect();
    if expected
        .iter()
        .any(|p| !ids.contains(&p["id"].as_str().unwrap_or("")))
    {
        return Err("assignment must cover the whole review group".into());
    }
    if ids.len() > 1 {
        text(assignment, "assessment_id")?;
    }
    let request = content(&input["request"])?;
    let original = content(&input["result"])?;
    let projection = input
        .get("projection")
        .cloned()
        .or_else(|| original.get("structured").filter(|v| !v.is_null()).cloned());
    let Some(mut projection) = projection else {
        return Ok(
            json!({"request":request,"original":original,"candidates":[],"diagnostics":[{"diagnostic":"result is not JSON; driver projection required"}]}),
        );
    };
    if projection
        .get("author")
        .is_some_and(|author| author != &input["author"])
    {
        return Err("projection author contradicts intake author".into());
    }
    if projection.get("author").is_none() && projection.is_object() {
        projection["author"] = input["author"].clone();
    }
    crate::external_work::check_projection(&original["structured"], &projection, "items")?;
    let items = projection["items"]
        .as_array()
        .ok_or("projection.items must be an array")?;
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    let mut metadata = json!({"gate":gate,"author":input["author"],"subject":assignment["subject"],"subject_revision":assignment["subject_revision"]}).as_object().unwrap().clone();
    for (key, value) in identity.as_object().into_iter().flatten() {
        metadata.insert(key.clone(), value.clone());
    }
    for item in assemble_items(items, &metadata) {
        let Some(mut candidate) = item.candidate else {
            diagnostics.push(json!({"index":item.index,"diagnostics":item.diagnostics}));
            continue;
        };
        let id = candidate["policy_id"].as_str().unwrap_or("");
        let valid = ids.contains(&id)
            && matches!(candidate["result"].as_str(), Some("pass" | "fail"))
            && candidate["findings"].is_string()
            && (candidate["result"] != "fail"
                || candidate["findings"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()));
        if !valid {
            diagnostics
                .push(json!({"index":item.index,"diagnostic":"malformed or unknown policy item"}));
            continue;
        }
        let assessment = assignment["assessment_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{gate}.{}.{id}",
                    assignment["subject_revision"].as_str().unwrap()
                )
            });
        candidate["assessment_id"] = json!(assessment);
        candidate["external_review"] =
            json!({"request":request,"original":original,"direct":direct});
        candidates.push(json!({"kind":"review-evidence","assessment_id":assessment,"self_review":direct,"data":candidate}));
    }
    Ok(
        json!({"request":request,"original":original,"projection":projection,"candidates":candidates,"diagnostics":diagnostics}),
    )
}

/// Carry only explicit earlier immutable review records, retaining reviewer identity.
pub fn carry(
    context: &[loop_core::ContextRecord],
    subject: &str,
    revision: &str,
    version_field: &str,
    version: &str,
    policy_document: bool,
) -> Vec<loop_core::ContextRecord> {
    let mut result = context.to_vec();
    for record in context
        .iter()
        .filter(|r| r.kind == "evidence-applicability")
    {
        let d = &record.data;
        let origins: Vec<_> = context
            .iter()
            .filter(|r| r.id.as_str() == d["origin"]["id"].as_str().unwrap_or(""))
            .collect();
        let source = origins.first().copied();
        let valid = d.as_object().is_some_and(|o| {
            o.len() == 4
                && ["origin", "target", "attesting_driver", "reason"]
                    .iter()
                    .all(|k| o.contains_key(*k))
        }) && d["origin"].as_object().is_some_and(|o| o.len() == 2)
            && d["target"].as_object().is_some_and(|o| o.len() == 2)
            && origins.len() == 1
            && source.is_some_and(|r| r.kind == "review-evidence" && r.sequence < record.sequence)
            && d["origin"]["kind"] == "context-record"
            && d["origin"].get("assignment_id").is_none()
            && d["target"]["subject"] == subject
            && d["target"]["revision"] == revision
            && d["reason"].as_str().is_some_and(|s| !s.trim().is_empty())
            && d["attesting_driver"]["name"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
            && matches!(
                d["attesting_driver"]["kind"].as_str(),
                Some("human" | "agent" | "script")
            );
        let Some(source) = source else {
            continue;
        };
        let subject_field = if policy_document {
            "target_id"
        } else {
            "subject"
        };
        let revision_field = if policy_document {
            "target_sha256"
        } else {
            "subject_revision"
        };
        if source.data[subject_field] != subject || source.data[version_field] != version {
            continue;
        }
        let mut carried = record.clone();
        carried.kind = "review-evidence".into();
        carried.data = source.data.clone();
        if valid {
            carried.data[revision_field] = json!(revision);
        } else {
            carried.data["result"] = json!("invalid-applicability");
            carried.data[revision_field] = json!(revision);
        }
        result.push(carried);
    }
    result.sort_by_key(|r| r.sequence);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_retain_exchange_for_each_provider_and_direct_review() {
        for provider in ["policy-document", "research"] {
            for direct in [false, true] {
                let packet = json!({
                    "show":{"operation":"show","status":"completed","result":{"workflow_id":provider}},
                    "assignment":{"gate":"review","review_stage":"ordinary","subject":"target",
                                  "subject_revision":"v1","policy_ids":["quality"]},
                    "author":{"name":"reviewer","kind":"human"},"self_review":direct,
                    "request":{"text":"Instructions actually followed.\n"},
                    "result":{"text":"Original judgment: passes.\n"},
                    "projection":{"items":[{"policy_id":"quality","result":"pass","findings":""}]}
                });
                let result = project(
                    &packet,
                    provider,
                    &[json!({"gate":"review","id":"quality","self_review":true})],
                    &json!({}),
                )
                .unwrap();
                let exchange = &result["candidates"][0]["data"]["external_review"];
                assert_eq!(exchange["request"]["original"], packet["request"]["text"]);
                assert_eq!(exchange["original"]["original"], packet["result"]["text"]);
                assert_eq!(exchange["direct"], direct);
            }
        }
    }
}
