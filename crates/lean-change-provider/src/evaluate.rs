//! The provider is a checker: it decides whether a checked transition is allowed.
use crate::model::*;
use crate::protocol::{allow, deny, unsupported, EvaluateRequest};
use crate::workflow::workflow;
use loop_core::ContextRecord;
use loop_integrations::owner_checkpoint::{self, Status};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Port: the identity of the repository tree right now. The adapter supplies the real one.
pub type TreeIdentity<'a> = &'a dyn Fn(&InitialInput) -> Result<String, String>;

type Gate = Result<(), Value>;

fn refuse(code: &str, message: &str, details: Value) -> Value {
    deny(code, message, details)
}

/// Latest record of a kind by semantic sequence.
fn latest<'a>(records: &'a [ContextRecord], kind: &str) -> Option<&'a ContextRecord> {
    records
        .iter()
        .filter(|r| r.kind == kind)
        .max_by_key(|r| r.sequence)
}

fn current_intent(records: &[ContextRecord]) -> Result<(String, Intent), Value> {
    let record = latest(records, "intent")
        .ok_or_else(|| refuse("intent-missing", "append an intent record first", json!({})))?;
    let intent = Intent::parse(&record.data).map_err(|e| {
        refuse(
            "intent-invalid",
            "the latest intent record is invalid",
            json!({"record": record.id.as_str(), "error": e}),
        )
    })?;
    Ok((record.id.as_str().to_owned(), intent))
}

fn parse_latest<T: serde::de::DeserializeOwned>(
    records: &[ContextRecord],
    kind: &str,
) -> Result<Option<(String, T)>, Value> {
    match latest(records, kind) {
        None => Ok(None),
        Some(r) => serde_json::from_value(r.data.clone())
            .map(|v| Some((r.id.as_str().to_owned(), v)))
            .map_err(|e| {
                refuse(
                    &format!("{kind}-invalid"),
                    &format!("the latest {kind} record is invalid"),
                    json!({"record": r.id.as_str(), "error": e.to_string()}),
                )
            }),
    }
}

pub fn evaluate(request: &EvaluateRequest, tree: TreeIdentity) -> Result<Value, String> {
    let input = InitialInput::parse(&request.initial_input)?;
    if request.workflow != workflow(input.review) {
        return Ok(unsupported());
    }
    let t = &request.transition;
    if !request.workflow.transitions.contains(t) {
        return Ok(unsupported());
    }
    if !t.kind.is_checked() {
        return Ok(allow());
    }
    let records = &request.context;
    let gate = match (t.source.as_str(), t.event.as_str()) {
        ("explore", "intent-ready") => current_intent(records).map(|_| ()),
        ("intent-approval", event) => approval(records, event),
        ("plan", "planned") => plan(records),
        ("verify", "verified") => verification(records, &input, tree),
        ("review", "reviewed") => review(records, &input, tree),
        _ => return Ok(unsupported()),
    };
    Ok(gate.map_or_else(|denial| denial, |()| allow()))
}

fn approval(records: &[ContextRecord], event: &str) -> Gate {
    let (id, intent) = current_intent(records)?;
    require_approved(records, &id)?;
    if (event == "approved-hard") != intent.hard {
        return Err(refuse(
            "hard-mismatch",
            "request approved when hard=false and approved-hard when hard=true",
            json!({"hard": intent.hard}),
        ));
    }
    Ok(())
}

/// The owner approved exactly this intent record.
fn require_approved(records: &[ContextRecord], id: &str) -> Gate {
    let evaluation = owner_checkpoint::evaluate(records, "intent", "intent", id, &["intent"]);
    match evaluation.status {
        Status::Approved => {}
        Status::Rejected { reason, .. } => {
            return Err(refuse(
                "intent-rejected",
                "the owner rejected this intent; request revise",
                json!({"intent": id, "reason": reason}),
            ));
        }
        Status::Pending => {
            let ignored: Vec<_> = evaluation
                .diagnostics
                .iter()
                .map(|d| json!({"record": d.record_id, "error": d.message}))
                .collect();
            return Err(refuse(
                "owner-decision-missing",
                "no well-formed owner decision with their verbatim words for this intent record",
                json!({"intent": id, "ignored": ignored}),
            ));
        }
    }
    Ok(())
}

fn plan(records: &[ContextRecord]) -> Gate {
    let (id, _) = current_intent(records)?;
    match parse_latest::<Plan>(records, "plan")? {
        Some((_, plan)) if plan.intent == id && !plan.text.trim().is_empty() => Ok(()),
        _ => Err(refuse(
            "plan-missing",
            "append a plan record for the current intent with nonempty text",
            json!({"intent": id}),
        )),
    }
}

/// Whether the verified transition would be allowed now; the review packet reuses it.
pub fn verification_ready(
    records: &[ContextRecord],
    input: &InitialInput,
    tree: TreeIdentity,
) -> Result<(), Value> {
    verification(records, input, tree)
}

fn verification(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {
    let (id, intent) = current_intent(records)?;
    // A later intent record, or a later hard flag, must not inherit an earlier approval or plan.
    require_approved(records, &id)?;
    if intent.hard {
        plan(records)?;
    }
    let mut problems = Vec::new();
    let recorded = parse_latest::<Verification>(records, "verification")?;
    match recorded.filter(|(_, v)| v.intent == id) {
        None => problems.push(json!({"problem": "no verification record for this intent"})),
        Some((_, v)) => {
            let now = tree(input).map_err(|e| refuse("tree-unreadable", &e, json!({})))?;
            if v.tree != now {
                problems.push(json!({"problem": "stale: the tree changed since the verification", "verified_tree": v.tree, "current_tree": now}));
            }
            for criterion in &intent.criteria {
                match v.results.iter().find(|r| r.criterion == criterion.id) {
                    None => problems.push(json!({"criterion": criterion.id, "problem": "missing: no check result"})),
                    Some(r) if r.check_sha256 != criterion.check.sha256() => problems.push(json!({"criterion": criterion.id, "problem": "the check is not the approved one"})),
                    Some(r) if r.timed_out || r.exit_code != Some(0) => problems.push(json!({"criterion": criterion.id, "problem": "the check failed", "exit_code": r.exit_code, "timed_out": r.timed_out, "output_tail": r.output_tail})),
                    Some(_) => {}
                }
            }
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(refuse("verification-incomplete", "every criterion check must pass on the current tree; run lean-change verify and append its output", json!({"intent": id, "problems": problems})))
}

/// Structure of one review: independence, fresh context, six noted answers, complete findings.
fn review_form(record: &ContextRecord, intent: &Intent) -> Result<Review, String> {
    let review: Review = serde_json::from_value(record.data.clone())
        .map_err(|e| format!("the review record is invalid: {e}"))?;
    if review.reviewer.name.trim().is_empty()
        || review
            .reviewer
            .name
            .trim()
            .eq_ignore_ascii_case(intent.author.name.trim())
    {
        return Err("the reviewer must be someone other than the intent author".into());
    }
    if !review.fresh_context {
        return Err("the review must be done in a fresh context".into());
    }
    let mut asked = BTreeSet::new();
    for answer in &review.answers {
        if !QUESTIONS.iter().any(|(q, _)| *q == answer.question)
            || !asked.insert(answer.question.as_str())
        {
            return Err(format!("unknown or repeated question {}", answer.question));
        }
        if answer.note.trim().is_empty() {
            return Err(format!("answer to {} needs a note", answer.question));
        }
    }
    if let Some((q, _)) = QUESTIONS.iter().find(|(q, _)| !asked.contains(q)) {
        return Err(format!("question {q} is unanswered"));
    }
    let mut ids = BTreeSet::new();
    for finding in &review.findings {
        let fields = [
            &finding.id,
            &finding.text,
            &finding.evidence,
            &finding.consequence,
        ];
        if fields.iter().any(|f| f.trim().is_empty())
            || !asked.contains(finding.question.as_str())
            || !ids.insert(finding.id.as_str())
        {
            return Err(format!(
                "finding {} needs a unique id, a known question, text, evidence and consequence",
                finding.id
            ));
        }
    }
    for answer in review.answers.iter().filter(|a| a.verdict == Verdict::Fail) {
        if !review
            .findings
            .iter()
            .any(|f| f.question == answer.question)
        {
            return Err(format!(
                "question {} failed but has no finding",
                answer.question
            ));
        }
    }
    Ok(review)
}

fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {
    verification(records, input, tree)?;
    let (intent_id, intent) = current_intent(records)?;
    let incomplete =
        |problem: String| refuse("review-incomplete", &problem, json!({"intent": intent_id}));
    let mut reviews: Vec<&ContextRecord> = records
        .iter()
        .filter(|r| r.kind == "review" && r.data["intent"] == intent_id.as_str())
        .collect();
    reviews.sort_by_key(|r| r.sequence);
    // The first well-formed review is the review: a later one cannot replace it and drop its findings.
    let mut problem = "append a review record for the current intent".to_owned();
    let mut chosen = None;
    for record in reviews {
        match review_form(record, &intent) {
            Ok(review) => {
                chosen = Some((record.id.as_str(), review));
                break;
            }
            Err(p) => problem = p,
        }
    }
    let Some((review_id, review)) = chosen else {
        return Err(incomplete(problem));
    };
    let mut fixed = false;
    for finding in &review.findings {
        let disposition = records
            .iter()
            .filter(|r| r.kind == "finding-disposition")
            .filter_map(|r| {
                serde_json::from_value::<Disposition>(r.data.clone())
                    .ok()
                    .map(|d| (r.sequence, d))
            })
            .filter(|(_, d)| d.review == review_id && d.finding == finding.id)
            .max_by_key(|(sequence, _)| *sequence)
            .map(|(_, d)| d);
        match disposition {
            Some(d) if !d.reason.trim().is_empty() => {
                fixed |= d.disposition == DispositionKind::Fixed
            }
            _ => {
                return Err(incomplete(format!(
                    "finding {} needs a disposition (fixed or rejected) with a reason",
                    finding.id
                )))
            }
        }
    }
    let now = tree(input).map_err(|e| refuse("tree-unreadable", &e, json!({})))?;
    if review.tree != now && !fixed {
        return Err(incomplete(
            "the tree changed after the review and no finding was fixed".into(),
        ));
    }
    Ok(())
}
