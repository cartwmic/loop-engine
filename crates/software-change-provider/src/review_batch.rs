//! Provider-owned batch checks. Engine captures remain opaque, immutable sources.
use loop_core::ContextRecord;
use serde_json::Value;
use std::{fs, path::Path};

pub(crate) fn validate_schema(schema: &Value, value: &Value) -> Result<(), String> {
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(schema)
        .map_err(|e| format!("invalid review contract: {e}"))?;
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Read the frozen contract and delivered location/controls. New projected
/// captures verify against the independently retained full routed context;
/// unmarked captures keep their original stdin context. No driver inventory.
pub(crate) fn captured_commission(
    capture: &Path,
    assignment: &str,
) -> Result<(Value, Value), String> {
    let capture = fs::canonicalize(capture).map_err(|e| e.to_string())?;
    let spec: Value = serde_json::from_slice(
        &fs::read(capture.join("fan-out-spec.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let matches: Vec<_> = spec["workers"]
        .as_array()
        .ok_or("missing captured workers")?
        .iter()
        .filter(|w| w["assignment_id"] == assignment)
        .collect();
    if matches.len() != 1 {
        return Err("captured assignment is missing or ambiguous".into());
    }
    let worker = matches[0];
    let stdin = fs::canonicalize(
        worker["stdin_path"]
            .as_str()
            .ok_or("missing captured stdin")?,
    )
    .map_err(|e| e.to_string())?;
    if !stdin.starts_with(&capture) {
        return Err("captured stdin escapes capture directory".into());
    }
    let bytes = fs::read_to_string(stdin).map_err(|e| e.to_string())?;
    let location = bytes
        .strip_suffix("---\n\n")
        .unwrap_or(&bytes)
        .trim_end()
        .lines()
        .next_back()
        .ok_or("missing compact location")?;
    let mut location: Value =
        serde_json::from_str(location).map_err(|e| format!("invalid captured location: {e}"))?;
    if !location["artifact_root"].is_string() {
        return Err("captured location has no artifact_root".into());
    }
    if let Some(format) = spec.get("capture_format") {
        if format != "bound-context-projection-v1" {
            return Err("unsupported captured context format".into());
        }
        let snapshot = worker
            .get("routed_inputs")
            .filter(|value| value.is_array())
            .ok_or("missing or malformed full captured context snapshot")?;
        serde_json::from_value::<Vec<ContextRecord>>(snapshot.clone())
            .map_err(|e| format!("invalid full captured context snapshot: {e}"))?;
        location["context"] = snapshot.clone();
    }
    Ok((worker["full_output_schema"].clone(), location))
}

/// Complete batches only. Return rows in frozen assignment order, independent
/// of the order in the worker's JSON. Reuse is a reference, never a fresh verdict.
#[allow(dead_code)]
pub(crate) fn rows<'a>(
    schema: &Value,
    value: &'a Value,
    location: &Value,
    gate: &str,
    subject: &str,
    revision: &str,
) -> Result<Vec<&'a Value>, String> {
    rows_for_stage(
        schema,
        value,
        location,
        gate,
        subject,
        revision,
        "aggregate",
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rows_for_stage<'a>(
    schema: &Value,
    value: &'a Value,
    location: &Value,
    gate: &str,
    subject: &str,
    revision: &str,
    review_stage: &str,
    require_stage: bool,
) -> Result<Vec<&'a Value>, String> {
    rows_for_stage_with_options(
        schema,
        value,
        location,
        gate,
        subject,
        revision,
        review_stage,
        require_stage,
        false,
    )
}

/// Validate a batch while optionally requiring a fresh aggregate row. The
/// latter is the high-rigor multi-stage rule: individual applicability may
/// carry unaffected axes, but an aggregate judgment must be produced again
/// after a correction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rows_for_stage_with_options<'a>(
    schema: &Value,
    value: &'a Value,
    _location: &Value,
    _gate: &str,
    _subject: &str,
    _revision: &str,
    review_stage: &str,
    require_stage: bool,
    _require_fresh_aggregate: bool,
) -> Result<Vec<&'a Value>, String> {
    validate_schema(schema, value)?;
    let schema_stage = schema
        .pointer("/properties/review_stage/const")
        .and_then(Value::as_str);
    let value_stage = value.get("review_stage").and_then(Value::as_str);
    if schema_stage.is_some() && value_stage.is_none() {
        return Err("review output is missing frozen review_stage".to_owned());
    }
    let declared_stage = schema_stage.or(value_stage);
    if require_stage && declared_stage.is_none() {
        return Err("individual review batch is missing frozen review_stage".to_owned());
    }
    if let Some(declared_stage) = declared_stage {
        if declared_stage != review_stage {
            return Err(format!(
                "review batch stage `{declared_stage}` does not match expected `{review_stage}`"
            ));
        }
    }
    let axes = schema
        .pointer("/properties/judgments/allOf")
        .and_then(Value::as_array)
        .ok_or("batch contract is missing frozen assigned axes")?;
    value.get("author").ok_or("batch has no author")?;
    let judgments = value["judgments"]
        .as_array()
        .ok_or("batch has no judgments")?;
    if axes.len() != judgments.len() {
        return Err("batch axis coverage is incomplete".into());
    }
    let mut ordered = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for assigned in axes {
        let axis = assigned
            .pointer("/contains/properties/axis/const")
            .and_then(Value::as_str)
            .ok_or("invalid assigned axis")?;
        if !seen.insert(axis) {
            return Err("duplicate frozen axis".into());
        }
        let matches: Vec<_> = judgments.iter().filter(|row| row["axis"] == axis).collect();
        if matches.len() != 1 {
            return Err(format!("axis `{axis}` must appear exactly once"));
        }
        let row = matches[0];
        ordered.push(row);
    }
    Ok(ordered)
}

/// Validate one structurally valid reuse row independently. Callers retain
/// fresh siblings when this reference is invalid; the row itself contributes
/// no judgment or carry until this check succeeds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_reuse_row(
    row: &Value,
    location: &Value,
    gate: &str,
    axis: &str,
    author: &Value,
    config_version: &str,
    subject: &str,
    revision: &str,
    review_stage: &str,
    require_stage: bool,
    require_fresh_aggregate: bool,
) -> Result<(), String> {
    let Some(reuse) = row.get("reuse") else {
        return Ok(());
    };
    if require_fresh_aggregate && review_stage == "aggregate" {
        return Err(
            "high-rigor aggregate review requires fresh judgments; carried rows are not allowed"
                .into(),
        );
    }
    if location
        .pointer("/controls/force_fresh")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err("force-fresh commission cannot use carried rows".into());
    }
    let context: Vec<ContextRecord> = serde_json::from_value(
        location
            .get("context")
            .cloned()
            .unwrap_or(serde_json::json!([])),
    )
    .map_err(|e| format!("invalid captured context: {e}"))?;
    crate::evidence::validate_batch_reuse_for_stage(
        &context,
        reuse.as_str().ok_or("invalid reuse ID")?,
        gate,
        axis,
        author,
        subject,
        revision,
        location.get("artifact_root"),
        review_stage,
        require_stage,
        Some(config_version),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        let mut schema: Value =
            serde_json::from_str(include_str!("../data/review-worker-output-schema.json")).unwrap();
        schema["properties"]["author"]["const"] = json!({"name":"reviewer","kind":"agent"});
        let rows = &mut schema["properties"]["judgments"];
        rows["minItems"] = json!(2);
        rows["maxItems"] = json!(2);
        for row in rows["items"]["oneOf"].as_array_mut().unwrap() {
            row["properties"]["axis"]["enum"] = json!(["alpha", "beta"]);
        }
        rows["allOf"] = json!(["alpha", "beta"].map(|axis| json!({"contains": {
            "type":"object", "required":["axis"], "properties":{"axis":{"const":axis}}
        }})));
        schema
    }

    #[test]
    fn review_output_contract_v2_requires_grounded_fresh_rows_but_allows_reuse() {
        let mut schema: Value =
            serde_json::from_str(include_str!("../data/review-worker-output-schema-v2.json"))
                .unwrap();
        schema["properties"]["review_stage"]["const"] = json!("aggregate");
        schema["properties"]["author"]["const"] = json!({"name":"reviewer","kind":"agent"});
        schema["properties"]["judgments"]["minItems"] = json!(1);
        schema["properties"]["judgments"]["maxItems"] = json!(1);
        for branch in schema["properties"]["judgments"]["items"]["oneOf"]
            .as_array_mut()
            .unwrap()
        {
            if let Some(axis) = branch.pointer_mut("/properties/axis") {
                axis["enum"] = json!(["alpha"]);
            }
        }
        schema["properties"]["judgments"]["allOf"] =
            json!([{"contains":{"properties":{"axis":{"const":"alpha"}}}}]);
        let good = json!({
            "review_contract_version":2,"review_stage":"aggregate",
            "author":{"name":"reviewer","kind":"agent"},
            "judgments":[{"axis":"alpha","result":"pass","findings":"",
                "grounds":{"reason":"Inspected the cited outcome.","evidence":[{"locator":"intent.json#/outcome","sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}]}}]
        });
        assert!(validate_schema(&schema, &good).is_ok());
        let mut missing = good.clone();
        missing["judgments"][0]
            .as_object_mut()
            .unwrap()
            .remove("grounds");
        assert!(validate_schema(&schema, &missing).is_err());
        let carried = json!({
            "review_contract_version":2,"review_stage":"aggregate",
            "author":{"name":"reviewer","kind":"agent"},
            "judgments":[{"axis":"alpha","reuse":"applicability-1"}]
        });
        assert!(validate_schema(&schema, &carried).is_ok());
    }

    #[test]
    fn recovery_batch_full_schema_preserves_distinct_verdicts_and_exact_coverage() {
        let schema = schema();
        let output = json!({"review_stage":"aggregate","author":{"name":"reviewer","kind":"agent"},"judgments":[
            {"axis":"beta","result":"fail","findings":"material"},
            {"axis":"alpha","result":"pass","findings":""}
        ]});
        let location = json!({"artifact_root":"/not-read-for-fresh-batch"});
        let ordered = rows(
            &schema,
            &output,
            &location,
            "intent-review",
            "intent.json",
            "1",
        )
        .unwrap();
        assert_eq!(ordered[0]["axis"], "alpha");
        assert_eq!(ordered[1]["result"], "fail");
        for bad in [
            json!({"author":{"name":"wrong","kind":"agent"},"judgments": output["judgments"]}),
            json!({"author": output["author"],"judgments":[output["judgments"][0]]}),
            json!({"author": output["author"],"judgments":[output["judgments"][0],output["judgments"][0]]}),
            json!({"author": output["author"],"judgments":[output["judgments"][0],{"axis":"unknown","result":"pass","findings":""}]}),
            json!({"author": output["author"],"judgments":[output["judgments"][0],{"axis":"alpha","result":"pass","findings":"not empty"}]}),
            json!({"author": output["author"],"judgments":[output["judgments"][0],{"axis":"alpha","result":"fail","findings":""}]}),
        ] {
            assert!(rows(
                &schema,
                &bad,
                &location,
                "intent-review",
                "intent.json",
                "1"
            )
            .is_err());
        }
    }

    #[test]
    fn recovery_batch_historical_checkpoint_identity_is_not_rebound_to_current_files() {
        use loop_core::{SemanticSequence, Timestamp};
        let record = |id, kind, n, data| {
            ContextRecord::new(
                id,
                kind,
                data,
                SemanticSequence::new(n),
                Timestamp::from_unix_millis(0),
            )
        };
        let context = vec![
            record(
                "original",
                "review-evidence",
                1,
                json!({"gate":"implementation-review", "policy_id":"beta",
                "author":{"name":"reviewer","kind":"agent"},"result":"fail","findings":"original failure",
                "subject":"implementation-report.json","subject_revision":"1","config_version":"fixture"}),
            ),
            record(
                "carry",
                "evidence-applicability",
                2,
                json!({"origin":{"kind":"context-record","id":"original"},
                "target":{"subject":"implementation-report.json","revision":"2","checkpoint":{"phase":"implementation","report_revision":"2"}},
                "attesting_driver":{"name":"driver","kind":"agent"},"reason":"unaffected source"}),
            ),
        ];
        let output = json!({"review_stage":"aggregate","author":{"name":"reviewer","kind":"agent"},"judgments":[
            {"axis":"alpha","result":"pass","findings":""},{"axis":"beta","reuse":"carry"}]});
        let mut location =
            json!({"artifact_root":"/no-longer-current-checkpoint", "context":context});
        assert!(rows(
            &schema(),
            &output,
            &location,
            "implementation-review",
            "implementation-report.json",
            "2"
        )
        .is_ok());
        let ordered = rows_for_stage_with_options(
            &schema(),
            &output,
            &location,
            "implementation-review",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            true,
        )
        .expect("invalid carry does not erase the fresh sibling");
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0]["axis"], "alpha");

        let mut ambiguous_context = context.clone();
        ambiguous_context.push(context[1].clone());
        let mut ambiguous_location = location.clone();
        ambiguous_location["context"] = serde_json::to_value(ambiguous_context).unwrap();
        let ambiguous_rows = rows_for_stage_with_options(
            &schema(),
            &output,
            &ambiguous_location,
            "implementation-review",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            false,
        )
        .expect("ambiguous applicability is diagnosed per row after batch shape passes");
        assert_eq!(ambiguous_rows.len(), 2);
        assert_eq!(ambiguous_rows[0]["axis"], "alpha");
        assert!(validate_reuse_row(
            &ambiguous_rows[1],
            &ambiguous_location,
            "implementation-review",
            "beta",
            &output["author"],
            "fixture",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            false,
        )
        .unwrap_err()
        .contains("not an authorized captured applicability"));

        let mut late_location = location.clone();
        late_location["context"] = json!([context[0].clone()]);
        assert!(validate_reuse_row(
            ordered[1],
            &late_location,
            "implementation-review",
            "beta",
            &output["author"],
            "fixture",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            false,
        )
        .unwrap_err()
        .contains("not an authorized captured applicability"));
        assert!(validate_reuse_row(
            ordered[1],
            &location,
            "implementation-review",
            "beta",
            &output["author"],
            "fixture",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            true,
        )
        .unwrap_err()
        .contains("aggregate review requires fresh"));
        let stale_rows = rows(
            &schema(),
            &output,
            &location,
            "implementation-review",
            "implementation-report.json",
            "3",
        )
        .expect("stale reuse remains isolated to its row");
        assert!(validate_reuse_row(
            stale_rows[1],
            &location,
            "implementation-review",
            "beta",
            &output["author"],
            "fixture",
            "implementation-report.json",
            "3",
            "aggregate",
            false,
            false,
        )
        .unwrap_err()
        .contains("stale"));
        location["controls"] = json!({"force_fresh":true});
        let ordered = rows(
            &schema(),
            &output,
            &location,
            "implementation-review",
            "implementation-report.json",
            "2",
        )
        .expect("force-fresh carry is a row diagnostic");
        assert!(validate_reuse_row(
            ordered[1],
            &location,
            "implementation-review",
            "beta",
            &output["author"],
            "fixture",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            false,
        )
        .unwrap_err()
        .contains("force-fresh"));
        location.as_object_mut().unwrap().remove("controls");
        assert!(validate_reuse_row(
            ordered[1],
            &location,
            "implementation-review",
            "beta",
            &output["author"],
            "other-config",
            "implementation-report.json",
            "2",
            "aggregate",
            false,
            false,
        )
        .unwrap_err()
        .contains("stale config version"));
    }
}
