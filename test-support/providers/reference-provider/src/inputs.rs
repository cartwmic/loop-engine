//! Run-input validation for reference provider.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::protocol::{DiagnosticDto, InputDeclarationDto, ValidateInputsResultDto};

pub fn validate_inputs(
    declarations: &[InputDeclarationDto],
    candidate_values: &BTreeMap<String, Value>,
) -> ValidateInputsResultDto {
    let mut diagnostics = Vec::new();

    for declaration in declarations {
        let value = candidate_values.get(&declaration.id);
        if declaration.required && value.is_none() {
            diagnostics.push(DiagnosticDto {
                code: "input.required".to_string(),
                message: format!("missing required input {}", declaration.id),
                path: Some(format!("/candidate_values/{}", declaration.id)),
            });
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        if !value.is_string() {
            diagnostics.push(DiagnosticDto {
                code: "input.type".to_string(),
                message: format!("input {} must be a string", declaration.id),
                path: Some(format!("/candidate_values/{}", declaration.id)),
            });
            continue;
        }
        let Some(text) = value.as_str() else {
            continue;
        };
        if text.is_empty() && declaration.required {
            diagnostics.push(DiagnosticDto {
                code: "input.empty".to_string(),
                message: format!("input {} must not be empty", declaration.id),
                path: Some(format!("/candidate_values/{}", declaration.id)),
            });
        }
    }

    if diagnostics.is_empty() {
        ValidateInputsResultDto::Accepted {
            values: Some(candidate_values.clone()),
        }
    } else {
        ValidateInputsResultDto::Rejected { diagnostics }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DescribeGraphVariant;
    use crate::graph::build_graph;

    #[test]
    fn accepts_valid_inputs() {
        let graph = build_graph(DescribeGraphVariant::V1);
        let mut values = BTreeMap::from([(
            "artifact_root".to_string(),
            Value::String("/tmp/artifacts".to_string()),
        )]);
        let result = validate_inputs(&graph.input_declarations, &values);
        assert!(matches!(result, ValidateInputsResultDto::Accepted { .. }));

        values.insert(
            "change_id".to_string(),
            Value::String("change-1".to_string()),
        );
        let result = validate_inputs(&graph.input_declarations, &values);
        assert!(matches!(result, ValidateInputsResultDto::Accepted { .. }));
    }

    #[test]
    fn rejects_missing_required_input() {
        let graph = build_graph(DescribeGraphVariant::V1);
        let values = BTreeMap::new();
        let result = validate_inputs(&graph.input_declarations, &values);
        assert!(matches!(result, ValidateInputsResultDto::Rejected { .. }));
    }
}
