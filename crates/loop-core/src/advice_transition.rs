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
pub const ADVICE_POLICY_CHANGE_RECORD_KIND: &str = "advice-policy-change";
pub const ADVICE_DISPOSITION_RECORD_KIND: &str = "advice-disposition";
pub const ADVICE_APPLICABILITY_RECORD_KIND: &str = "advice-applicability";

/// Versioned map of source-state/event departures whose occasions the driver
/// records on each visit. The original map is frozen in run initial input;
/// owner changes supply complete replacement maps without rewriting it.
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

/// Trusted owner attribution and a complete replacement policy for this visit.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdvicePolicyChange {
    pub actor: AdviceAuthor,
    pub reason: String,
    pub state_visit: u64,
    pub advice_departures: AdviceDepartureMap,
}

/// One policy projection shared by admission, closure and public show.
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveAdvicePolicy {
    pub frozen: Option<AdviceDepartureMap>,
    pub change_record_ids: Vec<String>,
    pub effective: Option<AdviceDepartureMap>,
    pub enabled: bool,
}

/// Resolve in durable order. An empty replacement map disables new advice,
/// but does not discharge successful answers received earlier.
pub fn effective_advice_policy(
    initial_input: &Value,
    workflow: &Workflow,
    context: &[ContextRecord],
) -> Result<EffectiveAdvicePolicy, String> {
    let frozen = initial_input
        .get(ADVICE_DEPARTURES_INPUT_KEY)
        .map(|value| {
            serde_json::from_value::<AdviceDepartureMap>(value.clone())
                .map_err(|error| format!("frozen advice departure map is invalid: {error}"))
        })
        .transpose()?;
    if let Some(map) = &frozen {
        validate_map(map, workflow)?;
        if map.occasions.is_empty() {
            return Err("frozen advice departure map must contain at least one occasion".into());
        }
    }
    let mut effective = frozen.clone();
    let mut change_record_ids = Vec::new();
    let mut ordered: Vec<_> = context.iter().collect();
    ordered.sort_by_key(|record| record.sequence);
    for record in ordered
        .into_iter()
        .filter(|record| record.kind == ADVICE_POLICY_CHANGE_RECORD_KIND)
    {
        let change = parse_policy_change(&record.data, workflow)
            .map_err(|error| format!("advice policy change `{}` is invalid: {error}", record.id))?;
        effective = Some(change.advice_departures);
        change_record_ids.push(record.id.to_string());
    }
    let enabled = effective
        .as_ref()
        .is_some_and(|map| !map.occasions.is_empty());
    Ok(EffectiveAdvicePolicy {
        frozen,
        change_record_ids,
        effective,
        enabled,
    })
}

fn parse_policy_change(data: &Value, workflow: &Workflow) -> Result<AdvicePolicyChange, String> {
    let change: AdvicePolicyChange =
        serde_json::from_value(data.clone()).map_err(|error| format!("invalid shape: {error}"))?;
    if change.actor.name.trim().is_empty() || change.actor.kind.trim().is_empty() {
        return Err("actor name and kind must be nonempty".into());
    }
    if change.reason.trim().is_empty() {
        return Err("reason must be nonempty".into());
    }
    validate_map(&change.advice_departures, workflow)?;
    Ok(change)
}

/// Engine-owned admission extension point, independent of provider admission.
/// `context` contains only earlier durable records, in durable order.
pub fn validate_advice_record(
    record_id: &str,
    kind: &str,
    data: &Value,
    run: &crate::Run,
    context: &[ContextRecord],
) -> Result<(), String> {
    let check = || -> Result<(), String> {
        if kind == ADVICE_ANSWER_RECORD_KIND {
            let answer: ExternalAdviceAnswer = serde_json::from_value(data.clone())
                .map_err(|error| format!("invalid answer shape: {error}"))?;
            answer.validate()?;
        } else if kind == ADVICE_POLICY_CHANGE_RECORD_KIND {
            let change = parse_policy_change(data, &run.workflow)?;
            if change.state_visit != run.control_revision.as_u64() {
                return Err("state_visit must name the current observed state visit".into());
            }
        } else if kind == ADVICE_OCCASION_RECORD_KIND {
            let policy = effective_advice_policy(&run.initial_input, &run.workflow, context)?;
            if !policy.enabled {
                return Ok(()); // No effective policy: preserve opaque storage.
            }
            let occasion: OccasionRecord = serde_json::from_value(data.clone())
                .map_err(|error| format!("invalid occasion shape: {error}"))?;
            if occasion.state_visit != run.control_revision.as_u64()
                || occasion.source_state != run.current_state
            {
                return Err(
                    "source_state/state_visit must name the current observed state and visit"
                        .into(),
                );
            }
            if !policy
                .effective
                .as_ref()
                .unwrap()
                .occasions
                .iter()
                .any(|expected| {
                    expected.state == occasion.source_state
                        && expected.event == occasion.event
                        && expected.occasion_id == occasion.occasion_id
                })
            {
                return Err(
                    "occasion_id/source_state/event is not mapped in the effective advice policy"
                        .into(),
                );
            }
            validate_occasion_record(&occasion, record_id, context)?;
            let latest = context
                .iter()
                .filter(|record| record.kind == ADVICE_OCCASION_RECORD_KIND)
                .filter_map(|record| {
                    serde_json::from_value::<OccasionRecord>(record.data.clone())
                        .ok()
                        .map(|previous| (record, previous))
                })
                .filter(|(_, previous)| same_occasion(previous, &occasion))
                .max_by_key(|(record, _)| record.sequence);
            validate_occasion_revision(&occasion, latest.map(|(record, _)| record.id.as_str()))?;
        }
        Ok(())
    };
    check().map_err(|error| format!("advice record `{record_id}` refused: {error}"))
}

fn same_occasion(left: &OccasionRecord, right: &OccasionRecord) -> bool {
    left.state_visit == right.state_visit
        && left.source_state == right.source_state
        && left.event == right.event
        && left.occasion_id == right.occasion_id
}

fn validate_occasion_revision(
    occasion: &OccasionRecord,
    latest: Option<&str>,
) -> Result<(), String> {
    match (latest, occasion.revises.as_deref()) {
        (None, None) if occasion.revision_reason.is_none() => Ok(()),
        (Some(id), Some(revises)) if id == revises
            && occasion.revision_reason.as_deref().is_some_and(|reason| !reason.trim().is_empty()) => Ok(()),
        (Some(id), _) => Err(format!("existing advice occasion record `{id}` requires revises naming that latest record and a nonblank revision_reason")),
        (None, _) => Err("revises/revision_reason requires an earlier record for the same occasion, state, visit and event".into()),
    }
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
    #[serde(default)]
    revises: Option<String>,
    #[serde(default)]
    revision_reason: Option<String>,
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
    let policy = effective_advice_policy(initial_input, workflow, context)?;
    let Some(map) = policy.effective else {
        return Ok(None);
    };
    let relevant: Vec<_> = map
        .occasions
        .iter()
        .filter(|occasion| occasion.state == *current_state && occasion.event == transition.event)
        .collect();

    let mut records_by_occasion: BTreeMap<String, (&ContextRecord, OccasionRecord)> =
        BTreeMap::new();
    let mut ordered = context.to_vec();
    ordered.sort_by_key(|record| record.sequence);
    for (index, record) in ordered
        .iter()
        .enumerate()
        .filter(|(_, record)| record.kind == ADVICE_OCCASION_RECORD_KIND)
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
        // Removed requirements are historical, not malformed/unanswered.
        if !relevant
            .iter()
            .any(|expected| expected.occasion_id == occasion.occasion_id)
        {
            continue;
        }
        validate_occasion_record(&occasion, record.id.as_str(), &ordered[..index])?;
        let latest = records_by_occasion
            .get(&occasion.occasion_id)
            .map(|(record, _)| record.id.as_str());
        validate_occasion_revision(&occasion, latest)?;
        records_by_occasion.insert(occasion.occasion_id.clone(), (record, occasion));
    }

    let successful = load_successful_advice(run_id, context)?;
    let applicability = load_applicability(state_visit, context, &successful);
    let dispositions = load_dispositions(state_visit, context, &successful, &applicability);
    ensure_every_answer_dispositioned(&successful, &dispositions)?;

    let mut due = Vec::new();
    for expected in relevant {
        let Some((_, occasion)) = records_by_occasion.get(&expected.occasion_id) else {
            return Err(format!(
                "advice occasion `{}` has no driver record for state visit {state_visit}",
                expected.occasion_id
            ));
        };
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
    record_id: &str,
    context: &[ContextRecord],
) -> Result<(), String> {
    if !occasion.target.is_object() {
        return Err(format!(
            "advice occasion record `{}` target must be a JSON object",
            record_id
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
            if source_id == record_id
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
    for response_id in &occasion.response_ids {
        let response = context
            .iter()
            .find(|record| record.id.as_str() == response_id)
            .ok_or_else(|| {
                format!("response_ids references missing earlier response `{response_id}`")
            })?;
        if response.kind != ADVICE_ANSWER_RECORD_KIND {
            return Err(format!(
                "response_ids `{response_id}` is not externally supplied advice"
            ));
        }
        let supplied: ExternalAdviceAnswer = serde_json::from_value(response.data.clone())
            .map_err(|error| format!("response_ids `{response_id}` is invalid: {error}"))?;
        supplied.validate()?;
        if supplied.occasion != occasion.occasion_id {
            return Err(format!(
                "response_ids `{response_id}` names another occasion"
            ));
        }
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
        let current = run(json!({})); // Answers must be safe even before advice is enabled.
        assert!(
            validate_advice_record("answer", ADVICE_ANSWER_RECORD_KIND, &value, &current, &[])
                .is_ok()
        );
        let mut missing = value.clone();
        missing["response"]["answers"]["q"]
            .as_object_mut()
            .unwrap()
            .remove("noul");
        assert!(validate_advice_record(
            "answer",
            ADVICE_ANSWER_RECORD_KIND,
            &missing,
            &current,
            &[]
        )
        .unwrap_err()
        .contains("advice record `answer` refused"));
        assert!(serde_json::from_value::<ExternalAdviceAnswer>(missing).is_err());
        let mut missing_original = value;
        missing_original["original"] = json!({});
        assert!(validate_advice_record(
            "answer",
            ADVICE_ANSWER_RECORD_KIND,
            &missing_original,
            &current,
            &[]
        )
        .unwrap_err()
        .contains("genuine original"));
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
            revises: None,
            revision_reason: None,
        };
        let record = test_context("occasion", ADVICE_OCCASION_RECORD_KIND, json!({}));
        assert!(validate_occasion_record(&occurrence, record.id.as_str(), &[]).is_ok());
        // The claim is intentionally not checked against hidden semantic facts.
    }
    fn run(initial_input: Value) -> crate::Run {
        crate::Run::new(
            "run",
            None,
            workflow(),
            crate::ProviderAssociation::new(json!({})),
            initial_input,
            "work",
            crate::Lifecycle::Active,
            4_u64.into(),
            10_u64.into(),
            Timestamp::from_unix_millis(1),
        )
    }

    fn policy(ids: &[&str]) -> Value {
        json!({"version":1,"occasions":ids.iter().map(|id|
            json!({"state":"work","event":"finish","occasion_id":id})).collect::<Vec<_>>()})
    }

    fn change(id: &str, ids: &[&str]) -> ContextRecord {
        test_context(
            id,
            ADVICE_POLICY_CHANGE_RECORD_KIND,
            json!({
                "actor":{"name":"owner","kind":"human"},"reason":"Owner changed requirements",
                "state_visit":4,"advice_departures":policy(ids)
            }),
        )
    }

    fn occasion(id: &str, occasion: &str, triggered: bool) -> ContextRecord {
        test_context(
            id,
            ADVICE_OCCASION_RECORD_KIND,
            json!({
                "source_state":"work","state_visit":4,"event":"finish","occasion_id":occasion,
                "target":{"revision":"r1"},"triggered":triggered,
                "trigger_source_ids":if triggered {vec!["source"]} else {vec![]},
                "response_ids":[],"reason":if triggered {None} else {Some("No admissible question")}
            }),
        )
    }

    fn closure(initial: &Value, context: &[ContextRecord]) -> Result<Option<Vec<String>>, String> {
        unanswered_due_occasions(
            initial,
            &"run".into(),
            &workflow(),
            &"work".into(),
            &Transition::checked("work", "finish", "done"),
            4,
            context,
        )
    }

    #[test]
    fn advice_admission_refuses_duplicates_and_allows_explicit_bidirectional_revisions() {
        let initial = json!({"advice_departures":policy(&["review"])});
        let current = run(initial.clone());
        let source = test_context("source", "note", json!({}));
        let original = occasion("original", "review", false);
        let mut context = vec![source, original.clone()];
        let mut revised = occasion("revised", "review", true);
        let check = |row: &ContextRecord, context: &[ContextRecord]| {
            validate_advice_record(row.id.as_str(), &row.kind, &row.data, &current, context)
        };
        assert!(check(&original, &context[..1]).is_ok());
        assert!(check(&revised, &context).unwrap_err().contains("original"));
        revised.data["revises"] = json!("original");
        assert!(check(&revised, &context)
            .unwrap_err()
            .contains("revision_reason"));
        revised.data["revision_reason"] = json!("New candidates arrived");
        assert!(check(&revised, &context).is_ok());
        revised.sequence = 2_u64.into();
        context.push(revised);
        assert_eq!(
            closure(&initial, &context).unwrap(),
            Some(vec!["review".into()])
        );
        let mut back = occasion("back", "review", false);
        back.data["revises"] = json!("original");
        back.data["revision_reason"] = json!("The claim no longer applies");
        assert!(check(&back, &context).unwrap_err().contains("revised"));
        back.data["revises"] = json!("revised");
        assert!(check(&back, &context).is_ok());
        back.sequence = 3_u64.into();
        context.push(back);
        assert_eq!(closure(&initial, &context).unwrap(), Some(vec![]));
        assert_eq!(
            context[1].data, original.data,
            "original history is unchanged"
        );
    }

    #[test]
    fn advice_admission_checks_shape_visit_edges_and_earlier_references() {
        let current = run(json!({"advice_departures":policy(&["review"])}));
        let source = test_context("source", "note", json!({}));
        for (field, value, expected) in [
            ("state_visit", json!(3), "state_visit"),
            ("source_state", json!("done"), "source_state"),
            ("event", json!("missing"), "not mapped"),
            ("occasion_id", json!("other"), "not mapped"),
            ("target", json!(null), "target"),
            ("trigger_source_ids", json!(["missing"]), "missing"),
            ("response_ids", json!(["missing"]), "response_ids"),
            ("response_ids", json!(["source"]), "not externally supplied"),
            ("extra", json!(true), "shape"),
        ] {
            let mut row = occasion("candidate", "review", true);
            row.data[field] = value;
            let error = validate_advice_record(
                "candidate",
                &row.kind,
                &row.data,
                &current,
                std::slice::from_ref(&source),
            )
            .unwrap_err();
            assert!(
                error.contains("candidate") && error.contains(expected),
                "{field}: {error}"
            );
        }
        let row = occasion("candidate", "review", true);
        assert!(
            validate_advice_record("candidate", &row.kind, &row.data, &run(json!({})), &[]).is_ok(),
            "off policy preserves opaque occasion storage"
        );
    }

    #[test]
    fn policy_changes_apply_immediately_to_pending_departure_and_keep_frozen_history() {
        let initial = json!({"advice_departures":policy(&["old"])});
        let source = test_context("source", "note", json!({}));
        let old = occasion("old-declaration", "old", true);
        let mut replace = change("replace", &["new"]);
        replace.sequence = 3_u64.into();
        let mut context = vec![source, old, replace];
        assert!(closure(&initial, &context)
            .unwrap_err()
            .contains("`new` has no driver record"));
        let mut new = occasion("new-declaration", "new", false);
        new.sequence = 4_u64.into();
        context.push(new);
        assert_eq!(
            closure(&initial, &context).unwrap(),
            Some(vec![]),
            "old due advice is inactive"
        );
        let mut off = change("off", &[]);
        off.sequence = 5_u64.into();
        context.push(off);
        let effective = effective_advice_policy(&initial, &workflow(), &context).unwrap();
        assert!(!effective.enabled);
        assert_eq!(effective.frozen.unwrap().occasions[0].occasion_id, "old");
        assert_eq!(effective.change_record_ids, ["replace", "off"]);
        assert_eq!(closure(&initial, &context).unwrap(), Some(vec![]));
        let on = change("on", &["new"]);
        assert!(closure(&json!({}), std::slice::from_ref(&on))
            .unwrap_err()
            .contains("no driver record"));
    }

    #[test]
    fn owner_change_validates_attribution_reason_visit_and_complete_map() {
        let current = run(json!({}));
        let valid = change("owner-change", &["review"]);
        for (pointer, value) in [
            ("/actor/name", json!(" ")),
            ("/actor/kind", json!("")),
            ("/reason", json!("")),
            ("/state_visit", json!(5)),
            ("/advice_departures/version", json!(2)),
            ("/advice_departures/occasions/0/event", json!("missing")),
        ] {
            let mut row = valid.clone();
            *row.data.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_advice_record(row.id.as_str(), &row.kind, &row.data, &current, &[])
                    .is_err(),
                "{pointer}"
            );
        }
        let off = change("off", &[]);
        assert!(
            validate_advice_record(off.id.as_str(), &off.kind, &off.data, &current, &[]).is_ok()
        );
    }

    #[test]
    fn successful_answers_keep_independent_disposition_debt_after_removal_disable_and_revision() {
        let initial = json!({"advice_departures":policy(&["review"])});
        let answer = test_context(
            "answer",
            ADVICE_ANSWER_RECORD_KIND,
            json!({
                "occasion":"review",
                "request":{"version":crate::ADVICE_PROTOCOL_VERSION,"state":{"admissibility":{"bounded_judgment":true,"evidence_sufficient":true}},"target":{"revision":"r1"},"occasion":"review","questions":{"q":{"type":"noul","instructions":"Assess evidence","proposition":"Adequate"}}},
                "response":{"answers":{"q":{"type":"noul","noul":0.7}}},
                "author":{"name":"Advisor","kind":"human"},"original":{"text":"Probability 0.7"}
            }),
        );
        let declaration = occasion("declaration", "review", false);
        for replacement in [change("off", &[]), change("remove", &["other"])] {
            let mut replacement = replacement;
            replacement.sequence = 3_u64.into();
            let mut context = vec![answer.clone(), declaration.clone(), replacement];
            assert!(closure(&initial, &context)
                .unwrap_err()
                .contains("answer/q"));
            context.push(test_context("disposition", ADVICE_DISPOSITION_RECORD_KIND, json!({
                "response_id":"answer","answer_id":"q","occasion_id":"review","state_visit":4,
                "target":{"revision":"r1"},"disposition":"reject","reason":"Driver rejects on supplied evidence"
            })));
            if context[2].id.as_str() == "off" {
                assert_eq!(closure(&initial, &context).unwrap(), Some(vec![]));
            } else {
                assert!(closure(&initial, &context)
                    .unwrap_err()
                    .contains("`other` has no driver record"));
            }
        }
        assert!(
            closure(&initial, &[answer, declaration])
                .unwrap_err()
                .contains("answer/q"),
            "not-triggered declaration cannot erase received answer debt"
        );
    }
}
