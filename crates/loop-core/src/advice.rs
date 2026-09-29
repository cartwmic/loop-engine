//! Provider-neutral typed advice request and response contracts.
//!
//! Advice remains separate from run-state decisions and bound work slots. The
//! caller supplies all state, target and question meaning; core validates only
//! the closed transport shape and typed answer arithmetic.

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const ADVICE_COMMAND_INPUT_KEY: &str = "advice_command";
pub const ADVICE_CAPTURE_KIND: &str = "advice-attempt";
pub const ADVICE_PROTOCOL_VERSION: u32 = 1;
const PROBABILITY_TOLERANCE: f64 = 1e-9;

/// Frozen optional external command and positive per-call bounds stored in a
/// run's initial input under [`ADVICE_COMMAND_INPUT_KEY`]. There is no
/// provider default or lifetime call quota.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceCommandConfig {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub max_request_bytes: u64,
    pub max_response_bytes: u64,
}

impl AdviceCommandConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.command.trim().is_empty() || self.command.contains('\0') {
            return Err("advice command must be a non-empty executable name".to_owned());
        }
        if self.args.iter().any(|argument| argument.contains('\0')) {
            return Err("advice argv must not contain NUL bytes".to_owned());
        }
        if self.timeout_ms == 0 {
            return Err("advice timeout_ms must be positive".to_owned());
        }
        if self.max_request_bytes == 0 || self.max_response_bytes == 0 {
            return Err("advice request and response byte limits must be positive".to_owned());
        }
        Ok(())
    }
}

/// Explicit caller assessment, not an engine judgment of semantic sufficiency.
/// All questions in a batch must meet both conditions before any backend runs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceAdmissibility {
    pub bounded_judgment: bool,
    pub evidence_sufficient: bool,
}

impl AdviceAdmissibility {
    pub fn validate(&self) -> Result<(), String> {
        if !self.bounded_judgment {
            return Err("advice requires bounded judgments, not investigation, planning or multi-step reasoning; keep that work with the driver".to_owned());
        }
        if !self.evidence_sufficient {
            return Err("advice requires sufficient supplied evidence; obtain missing facts before calling an advisor".to_owned());
        }
        Ok(())
    }
}

/// Closed caller-supplied advice request. `state` and `target` are structured
/// JSON objects; the engine does not derive either from run state or show.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceRequest {
    pub version: u32,
    pub state: Value,
    pub target: Value,
    pub occasion: String,
    pub questions: BTreeMap<String, AdviceQuestion>,
}

impl AdviceRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value = parse_json_rejecting_duplicate_keys(bytes)?;
        let request = serde_json::from_value::<Self>(value)
            .map_err(|error| format!("invalid advice request: {error}"))?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != ADVICE_PROTOCOL_VERSION {
            return Err(format!(
                "advice request version must be {ADVICE_PROTOCOL_VERSION}"
            ));
        }
        if !self.state.is_object() || !self.target.is_object() {
            return Err("advice state and target must be JSON objects".to_owned());
        }
        let admissibility: AdviceAdmissibility = serde_json::from_value(
            self.state
                .get("admissibility")
                .cloned()
                .ok_or("advice state requires the caller's admissibility assessment")?,
        )
        .map_err(|error| format!("invalid advice admissibility: {error}"))?;
        admissibility.validate()?;
        if self.occasion.trim().is_empty() {
            return Err("advice occasion must be a non-empty string".to_owned());
        }
        if self.questions.is_empty() {
            return Err("advice questions must contain at least one named question".to_owned());
        }
        for (id, question) in &self.questions {
            if id.trim().is_empty() {
                return Err("advice question IDs must be non-empty".to_owned());
            }
            question.validate(id)?;
        }
        Ok(())
    }

    pub fn validate_response(&self, response: &AdviceResponse) -> Result<(), String> {
        response.validate_for(self)
    }
}

/// Typed question forms are intentionally independent and named by their map
/// key. No answer controls a workflow transition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum AdviceQuestion {
    Choice {
        instructions: String,
        /// Choice key -> supplied rubric/criteria. The key set is the exact
        /// admissible answer and probability-distribution domain.
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: String,
        /// Ordered levels; response legend entries must match descriptions by
        /// their zero-based string index.
        levels: Vec<AdviceScoreLevel>,
    },
    #[serde(rename = "noul")]
    Noul {
        instructions: String,
        proposition: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        true_criteria: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        false_criteria: Option<Value>,
    },
}

impl AdviceQuestion {
    fn validate(&self, id: &str) -> Result<(), String> {
        match self {
            Self::Choice {
                instructions,
                criteria,
            } => {
                require_text(instructions, &format!("question `{id}` instructions"))?;
                if criteria.len() < 2 || criteria.keys().any(|key| key.trim().is_empty()) {
                    return Err(format!(
                        "Choice question `{id}` needs at least two non-empty criteria keys"
                    ));
                }
            }
            Self::Score {
                instructions,
                levels,
            } => {
                require_text(instructions, &format!("question `{id}` instructions"))?;
                if levels.len() < 2 {
                    return Err(format!(
                        "Score question `{id}` needs at least two ordered levels"
                    ));
                }
                for (index, level) in levels.iter().enumerate() {
                    require_text(
                        &level.description,
                        &format!("Score question `{id}` level {index} description"),
                    )?;
                    if level.criteria.is_null() {
                        return Err(format!(
                            "Score question `{id}` level {index} criteria must be supplied"
                        ));
                    }
                }
            }
            Self::Noul {
                instructions,
                proposition,
                ..
            } => {
                require_text(instructions, &format!("question `{id}` instructions"))?;
                require_text(proposition, &format!("Noul question `{id}` proposition"))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceScoreLevel {
    pub description: String,
    pub criteria: Value,
}

/// Exact typed response envelope. Unknown envelope fields, duplicate JSON
/// keys, missing answers, extra answers and mismatched answer types refuse.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceResponse {
    pub answers: BTreeMap<String, AdviceAnswer>,
}

impl AdviceResponse {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value = parse_json_rejecting_duplicate_keys(bytes)?;
        serde_json::from_value::<Self>(value)
            .map_err(|error| format!("invalid advice response: {error}"))
    }

    pub fn validate_for(&self, request: &AdviceRequest) -> Result<(), String> {
        let expected: BTreeSet<_> = request.questions.keys().cloned().collect();
        let actual: BTreeSet<_> = self.answers.keys().cloned().collect();
        if actual != expected {
            let missing: Vec<_> = expected.difference(&actual).cloned().collect();
            let extra: Vec<_> = actual.difference(&expected).cloned().collect();
            return Err(format!(
                "advice answer IDs must exactly match request questions (missing: {missing:?}; extra: {extra:?})"
            ));
        }

        for (id, question) in &request.questions {
            let answer = self
                .answers
                .get(id)
                .expect("exact answer keys were checked");
            validate_answer(id, question, answer)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum AdviceAnswer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_string"
        )]
        rationale: Option<String>,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, String>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_string"
        )]
        rationale: Option<String>,
    },
    #[serde(rename = "noul")]
    Noul {
        noul: f64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_string"
        )]
        rationale: Option<String>,
    },
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

fn validate_answer(
    id: &str,
    question: &AdviceQuestion,
    answer: &AdviceAnswer,
) -> Result<(), String> {
    match (question, answer) {
        (
            AdviceQuestion::Choice { criteria, .. },
            AdviceAnswer::Choice {
                choice,
                probabilities,
                confidence,
                ..
            },
        ) => {
            if !criteria.contains_key(choice) {
                return Err(format!(
                    "Choice answer `{id}` selected an unknown key `{choice}`"
                ));
            }
            validate_probabilities(
                id,
                "Choice",
                criteria.keys().map(String::as_str),
                probabilities,
            )?;
            validate_confidence(id, *confidence)?;
        }
        (
            AdviceQuestion::Score { levels, .. },
            AdviceAnswer::Score {
                score,
                legend,
                probabilities,
                confidence,
                ..
            },
        ) => {
            let indexes: Vec<String> = (0..levels.len()).map(|index| index.to_string()).collect();
            let expected_legend: BTreeMap<String, String> = levels
                .iter()
                .enumerate()
                .map(|(index, level)| (index.to_string(), level.description.clone()))
                .collect();
            if legend != &expected_legend {
                return Err(format!(
                    "Score answer `{id}` legend must exactly match the ordered request levels"
                ));
            }
            validate_probabilities(
                id,
                "Score",
                indexes.iter().map(String::as_str),
                probabilities,
            )?;
            validate_confidence(id, *confidence)?;
            if !score.is_finite() {
                return Err(format!("Score answer `{id}` value must be finite"));
            }
            let weighted_value: f64 = probabilities
                .iter()
                .map(|(index, probability)| {
                    index.parse::<usize>().unwrap_or_default() as f64 * probability
                })
                .sum();
            if (score - weighted_value).abs() > PROBABILITY_TOLERANCE {
                return Err(format!(
                    "Score answer `{id}` value {score} does not match weighted ordered-level value {weighted_value}"
                ));
            }
        }
        (AdviceQuestion::Noul { .. }, AdviceAnswer::Noul { noul, .. }) => {
            if !noul.is_finite() || !(0.0..=1.0).contains(noul) {
                return Err(format!(
                    "Noul answer `{id}` probability must be finite and in [0,1]"
                ));
            }
        }
        _ => {
            return Err(format!(
                "advice answer `{id}` type does not match its question"
            ))
        }
    }
    Ok(())
}

fn validate_probabilities<'a>(
    id: &str,
    kind: &str,
    expected_keys: impl Iterator<Item = &'a str>,
    probabilities: &BTreeMap<String, f64>,
) -> Result<(), String> {
    let expected: BTreeSet<_> = expected_keys.map(str::to_owned).collect();
    let actual: BTreeSet<_> = probabilities.keys().cloned().collect();
    if expected != actual {
        return Err(format!(
            "{kind} answer `{id}` probabilities must cover exactly the supplied keys"
        ));
    }
    let mut total = 0.0;
    for probability in probabilities.values() {
        if !probability.is_finite() || !(0.0..=1.0).contains(probability) {
            return Err(format!(
                "{kind} answer `{id}` probabilities must be finite and in [0,1]"
            ));
        }
        total += probability;
    }
    if (total - 1.0).abs() > PROBABILITY_TOLERANCE {
        return Err(format!(
            "{kind} answer `{id}` probabilities must be normalized to 1"
        ));
    }
    Ok(())
}

fn validate_confidence(id: &str, confidence: f64) -> Result<(), String> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(format!(
            "Choice/Score answer `{id}` confidence must be finite and in [0,1]"
        ));
    }
    Ok(())
}

fn require_text(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must be non-empty"))
    } else {
        Ok(())
    }
}

/// Parse JSON while preserving the one-object/one-key meaning of the closed
/// protocol. `serde_json::Value` alone would silently keep the last duplicate
/// key, so duplicate detection happens before typed deserialization.
pub fn parse_json_rejecting_duplicate_keys(bytes: &[u8]) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = NoDuplicateValue::deserialize(&mut deserializer)
        .map_err(|error| format!("invalid JSON: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("invalid JSON: {error}"))?;
    Ok(value.0)
}

struct NoDuplicateValue(Value);

impl<'de> Deserialize<'de> for NoDuplicateValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateValueVisitor)
    }
}

struct NoDuplicateValueVisitor;

impl<'de> de::Visitor<'de> for NoDuplicateValueVisitor {
    type Value = NoDuplicateValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(NoDuplicateValue)
            .ok_or_else(|| E::custom("JSON number is not finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value)))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<NoDuplicateValue>()? {
            values.push(value.0);
        }
        Ok(NoDuplicateValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key `{key}`")));
            }
            let value = object.next_value::<NoDuplicateValue>()?;
            values.insert(key, value.0);
        }
        Ok(NoDuplicateValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> AdviceRequest {
        serde_json::from_value(json!({
            "version": 1,
            "state": {"fact": "present", "admissibility": {"bounded_judgment":true,"evidence_sufficient":true}},
            "target": {"revision": "r1"},
            "occasion": "test",
            "questions": {
                "choice": {"type":"choice", "instructions":"Pick", "criteria":{"yes":"Yes", "no":"No"}},
                "score": {"type":"score", "instructions":"Rate", "levels":[
                    {"description":"low", "criteria":"low"},
                    {"description":"high", "criteria":"high"}
                ]},
                "noul": {"type":"noul", "instructions":"Is it true?", "proposition":"fact is true"}
            }
        })).unwrap()
    }

    #[test]
    fn refuses_missing_or_negative_admissibility_before_advice() {
        let original = serde_json::to_value(request()).unwrap();
        let mut missing = original.clone();
        missing["state"]
            .as_object_mut()
            .unwrap()
            .remove("admissibility");
        assert!(AdviceRequest::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
        for field in ["bounded_judgment", "evidence_sufficient"] {
            let mut value = original.clone();
            value["state"]["admissibility"][field] = json!(false);
            assert!(AdviceRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    fn response() -> AdviceResponse {
        serde_json::from_value(json!({"answers": {
            "choice": {"type":"choice", "choice":"yes", "probabilities":{"yes":0.75,"no":0.25}, "confidence":0.8},
            "score": {"type":"score", "score":0.25, "legend":{"0":"low","1":"high"}, "probabilities":{"0":0.75,"1":0.25}, "confidence":0.7, "rationale":"weighted"},
            "noul": {"type":"noul", "noul":0.6}
        }})).unwrap()
    }

    #[test]
    fn validates_choice_score_and_noul_without_required_noul_confidence_or_rationale() {
        let request = request();
        request.validate().unwrap();
        let response = response();
        request.validate_response(&response).unwrap();
        assert!(matches!(
            response.answers.get("noul"),
            Some(AdviceAnswer::Noul { .. })
        ));
        assert!(matches!(
            response.answers.get("choice"),
            Some(AdviceAnswer::Choice {
                rationale: None,
                ..
            })
        ));
        assert!(matches!(
            response.answers.get("score"),
            Some(AdviceAnswer::Score {
                rationale: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn rejects_duplicate_unknown_missing_wrong_type_and_fabricated_probability_data() {
        let request = request();
        for (body, expected) in [
            (
                r#"{"answers":{"choice":{"type":"choice","choice":"yes","probabilities":{"yes":0.5,"no":0.5},"confidence":0.8}},"answers":{}}"#,
                "duplicate JSON key",
            ),
            (r#"{"answers":{},"extra":true}"#, "invalid advice response"),
            (
                r#"{"answers":{"unexpected":{"type":"noul","noul":0.5}}}"#,
                "exactly match",
            ),
        ] {
            match AdviceResponse::parse(body.as_bytes()) {
                Ok(response) => {
                    let error = request.validate_response(&response).unwrap_err();
                    assert!(error.contains(expected), "{error}");
                }
                Err(error) => assert!(error.contains(expected), "{error}"),
            }
        }

        let mut probability_response = response();
        if let Some(AdviceAnswer::Choice { probabilities, .. }) =
            probability_response.answers.get_mut("choice")
        {
            probabilities.insert("no".into(), 0.2);
        }
        assert!(request
            .validate_response(&probability_response)
            .unwrap_err()
            .contains("normalized"));

        let mut noul_response = response();
        if let Some(AdviceAnswer::Noul { noul, .. }) = noul_response.answers.get_mut("noul") {
            *noul = 1.1;
        }
        assert!(request
            .validate_response(&noul_response)
            .unwrap_err()
            .contains("[0,1]"));

        let mut score_response = response();
        if let Some(AdviceAnswer::Score { score, .. }) = score_response.answers.get_mut("score") {
            *score = 1.0;
        }
        assert!(request
            .validate_response(&score_response)
            .unwrap_err()
            .contains("weighted ordered-level"));
    }

    #[test]
    fn config_requires_positive_bounds_and_rejects_unknown_fields() {
        let config: AdviceCommandConfig = serde_json::from_value(json!({
            "command":"script.py", "args":[], "timeout_ms":1,
            "max_request_bytes":1, "max_response_bytes":1
        }))
        .unwrap();
        config.validate().unwrap();
        let mut config = config;
        config.timeout_ms = 0;
        assert!(config.validate().unwrap_err().contains("positive"));
        assert!(serde_json::from_value::<AdviceCommandConfig>(json!({
            "command":"script.py", "args":[], "timeout_ms":1,
            "max_request_bytes":1, "max_response_bytes":1, "calls":5
        }))
        .is_err());
    }
}
