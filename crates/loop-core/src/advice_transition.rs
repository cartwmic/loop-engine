//! Generic opt-in closure for driver advice before normal workflow departures.
//!
//! Core checks durable identity, visit and target linkage only. Whether an
//! occasion should trigger, and whether a recorded answer is sound, remain
//! trusted driver judgments.

use crate::{
    AdviceRequest, AdviceResponse, ContextRecord, EventId, RunId, StateId, Transition, Workflow,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const ADVICE_ANSWER_RECORD_KIND: &str = "advice-answer";

/// Genuine externally supplied typed advice, attributed without execution claims.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdviceAnswer {
    pub occasion: String,
    pub request: AdviceRequest,
    pub response: AdviceResponse,
    pub author: AdviceAuthor,
    pub original: AdviceOriginal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceAuthor {
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceOriginal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

impl ExternalAdviceAnswer {
    pub fn validate(&self) -> Result<(), String> {
        self.request.validate()?;
        self.request.validate_response(&self.response)?;
        if self.occasion != self.request.occasion {
            return Err("supplied advice occasion must match the actual request".into());
        }
        if self.author.name.trim().is_empty() || self.author.kind.trim().is_empty() {
            return Err("supplied advice needs a nonempty author name and kind".into());
        }
        if ![
            self.original.text.as_deref(),
            self.original.reference.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
        {
            return Err(
                "supplied advice needs genuine original text or an accessible reference".into(),
            );
        }
        Ok(())
    }
}

pub const ADVICE_DEPARTURES_INPUT_KEY: &str = "advice_departures";
pub const ADVICE_OCCASION_RECORD_KIND: &str = "advice-occasion";
pub const ADVICE_DISPOSITION_RECORD_KIND: &str = "advice-disposition";
pub const ADVICE_APPLICABILITY_RECORD_KIND: &str = "advice-applicability";

/// Versioned map of source-state/event departures whose occasions the driver
/// records on each visit. The map is frozen in run initial input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceDepartureMap {
    pub version: u32,
    pub occasions: Vec<AdviceDeparture>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceDeparture {
    pub state: StateId,
    pub event: EventId,
    pub occasion_id: String,
}

/// Owner-attested exception for specifically named unanswered due occasions.
/// It never carries provider-evaluation or bound-slot waiver authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceExceptionAttestation {
    pub state_visit: u64,
    pub owner: String,
    pub reason: String,
    pub occasion_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OccasionRecord {
    state_visit: u64,
    source_state: StateId,
    event: EventId,
    occasion_id: String,
    target: Value,
    triggered: bool,
    #[serde(default)]
    trigger_source_ids: Vec<String>,
    #[serde(default)]
    response_ids: Vec<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DispositionRecord {
    response_id: String,
    answer_id: String,
    occasion_id: String,
    state_visit: u64,
    target: Value,
    disposition: Disposition,
    reason: String,
    #[serde(default)]
    applicability_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Disposition {
    Accept,
    Partial,
    Reject,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicabilityRecord {
    response_id: String,
    occasion_id: String,
    state_visit: u64,
    target: Value,
    attesting_driver: String,
    reason: String,
}

#[derive(Clone, Debug)]
struct SuccessfulAdvice {
    request: AdviceRequest,
    answer_ids: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct ValidDisposition {
    state_visit: u64,
    response_id: String,
    answer_id: String,
    occasion_id: String,
    target: Value,
    applicability_id: Option<String>,
    _disposition: Disposition,
}

#[derive(Clone, Debug)]
struct ValidApplicability {
    record_id: String,
    response_id: String,
    occasion_id: String,
    target: Value,
}

/// Validate the frozen map, current-visit driver declarations, successful
/// advice answers and dispositions. Returns the exact unanswered due occasion
/// IDs, which only the separate scoped owner exception may excuse.
pub(crate) fn unanswered_due_occasions(
    initial_input: &Value,
    run_id: &RunId,
    workflow: &Workflow,
    current_state: &StateId,
    transition: &Transition,
    state_visit: u64,
    context: &[ContextRecord],
) -> Result<Option<Vec<String>>, String> {
    let Some(map_value) = initial_input
        .as_object()
        .and_then(|input| input.get(ADVICE_DEPARTURES_INPUT_KEY))
    else {
        return Ok(None);
    };
    let map = serde_json::from_value::<AdviceDepartureMap>(map_value.clone())
        .map_err(|error| format!("frozen advice departure map is invalid: {error}"))?;
    validate_map(&map, workflow)?;

    let relevant: Vec<_> = map
        .occasions
        .iter()
        .filter(|occasion| occasion.state == *current_state && occasion.event == transition.event)
        .collect();

    let mut records_by_occasion: BTreeMap<String, Vec<OccasionRecord>> = BTreeMap::new();
    for record in context
        .iter()
        .filter(|record| record.kind == ADVICE_OCCASION_RECORD_KIND)
    {
        let Ok(occasion) = serde_json::from_value::<OccasionRecord>(record.data.clone()) else {
            continue;
        };
        if occasion.state_visit != state_visit
            || occasion.source_state != *current_state
            || occasion.event != transition.event
        {
            continue;
        }
        if !relevant
            .iter()
            .any(|expected| expected.occasion_id == occasion.occasion_id)
        {
            return Err(format!(
                "advice occasion record `{}` names an occasion not mapped for this departure",
                record.id
            ));
        }
        validate_occasion_record(&occasion, record, context)?;
        records_by_occasion
            .entry(occasion.occasion_id.clone())
            .or_default()
            .push(occasion);
    }

    let successful = load_successful_advice(run_id, context)?;
    let applicability = load_applicability(state_visit, context, &successful);
    let dispositions = load_dispositions(state_visit, context, &successful, &applicability);
    ensure_every_answer_dispositioned(&successful, &dispositions)?;

    let mut due = Vec::new();
    for expected in relevant {
        let Some(records) = records_by_occasion.get(&expected.occasion_id) else {
            return Err(format!(
                "advice occasion `{}` has no driver record for state visit {state_visit}",
                expected.occasion_id
            ));
        };
        if records.len() != 1 {
            return Err(format!(
                "advice occasion `{}` has {} driver records for state visit {state_visit}; expected exactly one",
                expected.occasion_id,
                records.len()
            ));
        }
        let occasion = &records[0];
        if !occasion.triggered {
            continue;
        }

        // A driver may explicitly disposition fresh advice after an immutable
        // occasion recorded a failed attempt. Reuse that existing declaration;
        // do not rewrite the occasion or infer a response from unrelated history.
        let mut response_ids: Vec<_> = occasion.response_ids.iter().map(String::as_str).collect();
        for disposition in dispositions.iter().filter(|disposition| {
            disposition.state_visit == state_visit
                && disposition.occasion_id == expected.occasion_id
                && disposition.target == occasion.target
        }) {
            let response_id = disposition.response_id.as_str();
            if !response_ids.contains(&response_id) {
                response_ids.push(response_id);
            }
        }
        let mut answered_here = false;
        for response_id in response_ids {
            let Some(context_response) = context.iter().find(|row| row.id.as_str() == response_id)
            else {
                return Err(format!(
                    "advice occasion `{}` references missing response `{response_id}`",
                    expected.occasion_id
                ));
            };
            if context_response.kind != ADVICE_ANSWER_RECORD_KIND {
                return Err(format!(
                    "advice occasion `{}` response `{response_id}` is not externally supplied advice",
                    expected.occasion_id
                ));
            }
            let Some(answer) = successful.get(response_id) else {
                // A retained timeout, failed process, or malformed answer is
                // an unanswered occasion, not a successful response.
                continue;
            };
            if answer.request.occasion != expected.occasion_id {
                return Err(format!(
                    "advice response `{response_id}` names occasion `{}` instead of `{}`",
                    answer.request.occasion, expected.occasion_id
                ));
            }
            let response_target_matches = answer.request.target == occasion.target;
            let app_id = applicability
                .iter()
                .find(|app| {
                    app.response_id == response_id
                        && app.occasion_id == expected.occasion_id
                        && app.target == occasion.target
                })
                .map(|app| app.record_id.as_str());
            if !response_target_matches && app_id.is_none() {
                continue;
            }

            let all_answers_disposed_for_target = answer.answer_ids.iter().all(|answer_id| {
                dispositions.iter().any(|disposition| {
                    disposition.response_id == response_id
                        && disposition.answer_id == *answer_id
                        && disposition.occasion_id == expected.occasion_id
                        && disposition.target == occasion.target
                        && (response_target_matches
                            || disposition.applicability_id.as_deref() == app_id)
                })
            });
            if all_answers_disposed_for_target {
                answered_here = true;
                break;
            }
            // A successful answer without a reasoned disposition is a hard
            // closure failure and cannot be waived by the owner exception.
            return Err(format!(
                "advice response `{response_id}` has an answer without a reasoned disposition for target of occasion `{}`",
                expected.occasion_id
            ));
        }
        if !answered_here {
            due.push(expected.occasion_id.clone());
        }
    }

    Ok(Some(due))
}

pub(crate) fn validate_advice_exception(
    exception: &AdviceExceptionAttestation,
    current_visit: u64,
    due_occasion_ids: &[String],
) -> Result<(), String> {
    if exception.state_visit != current_visit {
        return Err("advice exception must name the current observed state visit".to_owned());
    }
    if exception.owner.trim().is_empty() || exception.reason.trim().is_empty() {
        return Err("advice exception owner and reason must be nonempty".to_owned());
    }
    if exception.occasion_ids.is_empty() {
        return Err("advice exception must name at least one due occasion".to_owned());
    }
    let supplied: BTreeSet<_> = exception.occasion_ids.iter().collect();
    if supplied.len() != exception.occasion_ids.len() {
        return Err("advice exception occasion IDs must be unique".to_owned());
    }
    let due: BTreeSet<_> = due_occasion_ids.iter().collect();
    if supplied != due {
        return Err(format!(
            "advice exception must name exactly the unanswered due occasions {due:?}"
        ));
    }
    Ok(())
}

fn validate_map(map: &AdviceDepartureMap, workflow: &Workflow) -> Result<(), String> {
    if map.version != 1 {
        return Err("advice departure map version must be 1".to_owned());
    }
    if map.occasions.is_empty() {
        return Err("advice departure map must contain at least one occasion".to_owned());
    }
    let mut ids = BTreeSet::new();
    for occasion in &map.occasions {
        if occasion.occasion_id.trim().is_empty() || !ids.insert(&occasion.occasion_id) {
            return Err("advice departure occasion IDs must be nonempty and unique".to_owned());
        }
        let state_exists = workflow
            .states
            .iter()
            .any(|state| state.id == occasion.state);
        let matching_transition = workflow
            .transitions
            .iter()
            .any(|edge| edge.source == occasion.state && edge.event == occasion.event);
        if !state_exists || !matching_transition {
            return Err(format!(
                "advice occasion `{}` must name a stored source state/event edge",
                occasion.occasion_id
            ));
        }
    }
    Ok(())
}

fn validate_occasion_record(
    occasion: &OccasionRecord,
    record: &ContextRecord,
    context: &[ContextRecord],
) -> Result<(), String> {
    if !occasion.target.is_object() {
        return Err(format!(
            "advice occasion record `{}` target must be a JSON object",
            record.id
        ));
    }
    let unique_ids = |ids: &[String]| {
        !ids.iter().any(|id| id.trim().is_empty())
            && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
    };
    if occasion.triggered {
        if occasion.trigger_source_ids.is_empty() || !unique_ids(&occasion.trigger_source_ids) {
            return Err(format!(
                "triggered advice occasion `{}` needs unique nonempty trigger source IDs",
                occasion.occasion_id
            ));
        }
        if occasion
            .reason
            .as_ref()
            .is_some_and(|reason| !reason.trim().is_empty())
        {
            return Err(format!(
                "triggered advice occasion `{}` must not carry a non-trigger reason",
                occasion.occasion_id
            ));
        }
        for source_id in &occasion.trigger_source_ids {
            if source_id == record.id.as_str()
                || !context.iter().any(|source| source.id.as_str() == source_id)
            {
                return Err(format!(
                    "triggered advice occasion `{}` references missing/recursive source `{source_id}`",
                    occasion.occasion_id
                ));
            }
        }
        if !unique_ids(&occasion.response_ids) {
            return Err(format!(
                "triggered advice occasion `{}` response IDs must be unique nonempty strings",
                occasion.occasion_id
            ));
        }
    } else if occasion
        .reason
        .as_deref()
        .is_none_or(|reason| reason.trim().is_empty())
        || !occasion.trigger_source_ids.is_empty()
        || !occasion.response_ids.is_empty()
    {
        return Err(format!(
            "not-triggered advice occasion `{}` needs a reason and no trigger/response IDs",
            occasion.occasion_id
        ));
    }
    Ok(())
}

fn load_successful_advice(
    _run_id: &RunId,
    context: &[ContextRecord],
) -> Result<BTreeMap<String, SuccessfulAdvice>, String> {
    let mut successful = BTreeMap::new();
    for record in context
        .iter()
        .filter(|record| record.kind == ADVICE_ANSWER_RECORD_KIND)
    {
        let supplied = serde_json::from_value::<ExternalAdviceAnswer>(record.data.clone())
            .map_err(|error| format!("supplied advice `{}` is invalid: {error}", record.id))?;
        supplied
            .validate()
            .map_err(|error| format!("supplied advice `{}` is invalid: {error}", record.id))?;
        successful.insert(
            record.id.to_string(),
            SuccessfulAdvice {
                answer_ids: supplied.response.answers.keys().cloned().collect(),
                request: supplied.request,
            },
        );
    }
    Ok(successful)
}

fn load_applicability(
    state_visit: u64,
    context: &[ContextRecord],
    successful: &BTreeMap<String, SuccessfulAdvice>,
) -> Vec<ValidApplicability> {
    context
        .iter()
        .filter(|record| record.kind == ADVICE_APPLICABILITY_RECORD_KIND)
        .filter_map(|record| {
            let app = serde_json::from_value::<ApplicabilityRecord>(record.data.clone()).ok()?;
            let response = successful.get(&app.response_id)?;
            if app.state_visit > state_visit
                || app.attesting_driver.trim().is_empty()
                || app.reason.trim().is_empty()
                || !app.target.is_object()
                || app.target == response.request.target
                || app.occasion_id != response.request.occasion
            {
                return None;
            }
            Some(ValidApplicability {
                record_id: record.id.to_string(),
                response_id: app.response_id,
                occasion_id: app.occasion_id,
                target: app.target,
            })
        })
        .collect()
}

fn load_dispositions(
    state_visit: u64,
    context: &[ContextRecord],
    successful: &BTreeMap<String, SuccessfulAdvice>,
    applicability: &[ValidApplicability],
) -> Vec<ValidDisposition> {
    context
        .iter()
        .filter(|record| record.kind == ADVICE_DISPOSITION_RECORD_KIND)
        .filter_map(|record| {
            let disposition =
                serde_json::from_value::<DispositionRecord>(record.data.clone()).ok()?;
            let response = successful.get(&disposition.response_id)?;
            if disposition.state_visit > state_visit
                || disposition.reason.trim().is_empty()
                || !disposition.target.is_object()
                || disposition.occasion_id != response.request.occasion
                || !response.answer_ids.contains(&disposition.answer_id)
            {
                return None;
            }
            if disposition.target != response.request.target {
                let app_id = disposition.applicability_id.as_deref()?;
                let valid_application = applicability.iter().any(|app| {
                    app.record_id == app_id
                        && app.response_id == disposition.response_id
                        && app.occasion_id == disposition.occasion_id
                        && app.target == disposition.target
                });
                if !valid_application {
                    return None;
                }
            } else if disposition.applicability_id.is_some() {
                return None;
            }
            Some(ValidDisposition {
                state_visit: disposition.state_visit,
                response_id: disposition.response_id,
                answer_id: disposition.answer_id,
                occasion_id: disposition.occasion_id,
                target: disposition.target,
                applicability_id: disposition.applicability_id,
                _disposition: disposition.disposition,
            })
        })
        .collect()
}

fn ensure_every_answer_dispositioned(
    successful: &BTreeMap<String, SuccessfulAdvice>,
    dispositions: &[ValidDisposition],
) -> Result<(), String> {
    for (response_id, advice) in successful {
        for answer_id in &advice.answer_ids {
            if !dispositions.iter().any(|disposition| {
                disposition.response_id == *response_id && disposition.answer_id == *answer_id
            }) {
                return Err(format!(
                    "successful advice answer `{response_id}/{answer_id}` has no valid reasoned disposition"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SemanticSequence, Timestamp};
    use serde_json::json;

    fn test_context(id: &str, kind: &str, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(1),
        )
    }

    fn workflow() -> Workflow {
        Workflow::new(
            "advice",
            "work",
            vec![
                crate::State::new("work", "Work", "", false),
                crate::State::new("done", "Done", "", true),
            ],
            vec![
                Transition::check_free("work", "revise", "work"),
                Transition::checked("work", "finish", "done"),
            ],
        )
    }

    #[test]
    fn supplied_answer_requires_typed_values_and_retained_original() {
        let value = json!({
            "occasion":"review",
            "request":{"version":crate::ADVICE_PROTOCOL_VERSION,"state":{"admissibility":{"bounded_judgment":true,"evidence_sufficient":true}},"target":{"revision":"r1"},"occasion":"review","questions":{"q":{"type":"noul","instructions":"Assess evidence","proposition":"Adequate"}}},
            "response":{"answers":{"q":{"type":"noul","noul":0.7}}},
            "author":{"name":"Advisor","kind":"human"},
            "original":{"text":"Probability 0.7"}
        });
        let supplied: ExternalAdviceAnswer = serde_json::from_value(value.clone()).unwrap();
        assert!(supplied.validate().is_ok());
        let mut missing = value.clone();
        missing["response"]["answers"]["q"]
            .as_object_mut()
            .unwrap()
            .remove("noul");
        assert!(serde_json::from_value::<ExternalAdviceAnswer>(missing).is_err());
        let mut missing_original = value;
        missing_original["original"] = json!({});
        assert!(
            serde_json::from_value::<ExternalAdviceAnswer>(missing_original)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn absent_map_disables_advice_closure_for_historical_runs() {
        let result = unanswered_due_occasions(
            &json!({}),
            &"run".into(),
            &workflow(),
            &"work".into(),
            &Transition::check_free("work", "revise", "work"),
            4,
            &[],
        )
        .expect("historical runs remain ungated");
        assert_eq!(result, None);
    }

    #[test]
    fn map_is_validated_against_frozen_state_and_event_identity() {
        let map = AdviceDepartureMap {
            version: 1,
            occasions: vec![AdviceDeparture {
                state: "work".into(),
                event: "revise".into(),
                occasion_id: "review".into(),
            }],
        };
        assert!(validate_map(&map, &workflow()).is_ok());
        assert_eq!(map.occasions[0].state.as_str(), "work");
        assert_eq!(map.occasions[0].event.as_str(), "revise");
    }

    #[test]
    fn exception_must_exactly_name_due_occurrences() {
        let exception = AdviceExceptionAttestation {
            state_visit: 4,
            owner: "owner".into(),
            reason: "advisor unavailable".into(),
            occasion_ids: vec!["review".into()],
        };
        assert!(validate_advice_exception(&exception, 4, &["review".into()]).is_ok());
        assert!(validate_advice_exception(&exception, 5, &["review".into()]).is_err());
        assert!(validate_advice_exception(&exception, 4, &["other".into()]).is_err());
    }

    #[test]
    fn not_triggered_is_an_explicit_trusted_driver_claim() {
        let occurrence = OccasionRecord {
            state_visit: 4,
            source_state: "work".into(),
            event: "revise".into(),
            occasion_id: "review".into(),
            target: json!({"revision":"r1"}),
            triggered: false,
            trigger_source_ids: Vec::new(),
            response_ids: Vec::new(),
            reason: Some("No due decision on this visit".into()),
        };
        let record = test_context("occasion", ADVICE_OCCASION_RECORD_KIND, json!({}));
        assert!(validate_occasion_record(&occurrence, &record, &[]).is_ok());
        // The claim is intentionally not checked against hidden semantic facts.
    }
}
