use std::collections::BTreeMap;

use crate::protocol::{DiagnosticDto, ValidateInputsPayloadDto, ValidateInputsResultDto};
use crate::scenarios::Scenario;

pub fn validate(scenario: Scenario, payload: ValidateInputsPayloadDto) -> ValidateInputsResultDto {
    match scenario {
        Scenario::InputRequiredAccepted => ValidateInputsResultDto::Accepted {
            values: Some(payload.candidate_values),
        },
        Scenario::InputOptionalAccepted => {
            let mut values = payload.candidate_values;
            for declaration in &payload.declarations {
                if !declaration.required && !values.contains_key(&declaration.id) {
                    values.insert(declaration.id.clone(), serde_json::Value::Null);
                }
            }
            ValidateInputsResultDto::Accepted {
                values: Some(values),
            }
        }
        Scenario::InputRequiredRejected => ValidateInputsResultDto::Rejected {
            diagnostics: vec![DiagnosticDto {
                code: "input.missing".into(),
                message: "Required input 'ticket' is missing.".into(),
                path: Some("/candidate_values/ticket".into()),
            }],
        },
        Scenario::InputInvalidRejected => ValidateInputsResultDto::Rejected {
            diagnostics: vec![DiagnosticDto {
                code: "input.invalid".into(),
                message: "Input 'ticket' has invalid type.".into(),
                path: Some("/candidate_values/ticket".into()),
            }],
        },
        Scenario::InputEvaluationError => ValidateInputsResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation".into(),
                message: "Scenario-controlled validation evaluation error.".into(),
                path: None,
            }],
        },
        _ => ValidateInputsResultDto::Accepted {
            values: Some(BTreeMap::new()),
        },
    }
}
