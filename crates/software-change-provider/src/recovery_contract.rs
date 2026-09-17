//! Explicit execution boundary. Historical profiles remain readable by core,
//! but only declared v3 profiles execute with this provider.
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CriterionPolicy {
    pub required_authors: NonZeroUsize,
    pub goal_required_authors: NonZeroUsize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryContract {
    pub contract_version: u64,
    pub criterion_policy: CriterionPolicy,
}

impl RecoveryContract {
    pub(crate) fn from_input(value: &serde_json::Value) -> Result<Self, String> {
        Self::parse(serde_json::json!({
            "contract_version": value["contract_version"],
            "criterion_policy": value["criterion_policy"]
        }))
    }

    pub(crate) fn parse(value: serde_json::Value) -> Result<Self, String> {
        match value["contract_version"].as_u64() {
            Some(2 | 3) => {}
            _ => {
                return Err("unsupported software-change semantic contract; use the fixed original provider for old-profile runs".into())
            }
        }
        serde_json::from_value(value).map_err(|e| format!("invalid criterion_policy: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recovery_contract_fixture_loader_checks_bounded_schemas() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../data/recovery-contract-v2.json")).unwrap();
        RecoveryContract::parse(fixtures["declaration"].clone()).unwrap();
        for (name, schema) in fixtures["schemas"].as_object().unwrap() {
            let schema = crate::schema::validate_schema(schema).unwrap();
            assert!(
                schema.evaluate(&fixtures["examples"][name]).is_valid(),
                "{name}"
            );
            assert!(!schema.evaluate(&json!({})).is_valid(), "{name}");
            let mut extra = fixtures["examples"][name].clone();
            extra["unexpected"] = json!("rejected");
            assert!(!schema.evaluate(&extra).is_valid(), "{name}");
        }
    }

    #[test]
    fn recovery_contract_requires_explicit_supported_version_and_independent_positive_policy() {
        for version in [2, 3] {
            let value = json!({"contract_version":version,"criterion_policy":{
                "required_authors":1,"goal_required_authors":1}});
            assert_eq!(
                serde_json::to_value(RecoveryContract::parse(value.clone()).unwrap()).unwrap(),
                value
            );
        }
        for invalid in [
            json!({}),
            json!({"contract_version":1,"criterion_policy":{
            "required_authors":1,"goal_required_authors":1}}),
            json!({"contract_version":2,"criterion_policy":{
                "required_authors":0,"goal_required_authors":1}}),
            json!({"contract_version":2,"criterion_policy":{
                "required_authors":1,"goal_required_authors":1,"batch":true}}),
        ] {
            assert!(RecoveryContract::parse(invalid).is_err());
        }
    }
}
