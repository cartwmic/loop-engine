//! The provider is a checker: it decides whether a checked transition is allowed.
use crate::model::*;
use crate::protocol::{allow, deny, unsupported, EvaluateRequest};
use crate::workflow::workflow;
use loop_core::ContextRecord;
use loop_integrations::owner_checkpoint::{self, Actor, Status};
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

fn current_intent(
    records: &[ContextRecord],
    input: &InitialInput,
) -> Result<(String, Intent), Value> {
    let record = latest(records, "intent")
        .ok_or_else(|| refuse("intent-missing", "append an intent record first", json!({})))?;
    let invalid = |error: String| {
        refuse(
            "intent-invalid",
            "the latest intent record is invalid",
            json!({"record": record.id.as_str(), "error": error}),
        )
    };
    let intent = Intent::parse(&record.data).map_err(invalid)?;
    intent
        .check_old(input.old_vs_new.is_some())
        .map_err(invalid)?;
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
    if request.workflow != workflow(input.review, input.old_vs_new.is_some()) {
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
        ("explore", "intent-ready") => intent_ready(records, &input),
        ("intent-approval", event) => approval(records, &input, event),
        ("plan", "planned") => plan(records, &input),
        ("verify", "verified") => verification(records, &input, tree),
        ("review", "reviewed") => review(records, &input, tree),
        _ => return Ok(unsupported()),
    };
    Ok(gate.map_or_else(|denial| denial, |()| allow()))
}

/// Profile b: the first well-formed intent-check, every finding disposed, and a fix means a newer intent.
fn intent_ready(records: &[ContextRecord], input: &InitialInput) -> Gate {
    let (id, _) = current_intent(records, input)?;
    if !input.review {
        return Ok(());
    }
    let incomplete =
        |problem: String| refuse("intent-check-incomplete", &problem, json!({"intent": id}));
    let checks = records
        .iter()
        .filter(|r| r.kind == "intent-check")
        .collect();
    let none = "run lean-change intent-packet, give it to a fresh reviewer and append their intent-check record";
    let (check_id, check) =
        first_well_formed(checks, none, |r| intent_check_form(r, records)).map_err(incomplete)?;
    if disposed(records, check_id, &check.findings).map_err(incomplete)? && check.intent == id {
        return Err(incomplete(
            "a finding was fixed, so append a new intent record that fixes it".into(),
        ));
    }
    Ok(())
}

fn approval(records: &[ContextRecord], input: &InitialInput, event: &str) -> Gate {
    let (id, intent) = current_intent(records, input)?;
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

fn plan(records: &[ContextRecord], input: &InitialInput) -> Gate {
    let (id, _) = current_intent(records, input)?;
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
    let (id, intent) = current_intent(records, input)?;
    // A later intent record, or a later hard flag, must not inherit an earlier approval or plan.
    require_approved(records, &id)?;
    if intent.hard {
        plan(records, input)?;
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
                    Some(r) => {
                        if let Some(problem) = criterion.old.as_ref().and_then(|expectation| old_problem(expectation, r)) {
                            problems.push(json!({"criterion": criterion.id, "problem": problem}));
                        }
                    }
                }
            }
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(refuse("verification-incomplete", "every criterion check must pass on the current tree; run lean-change verify and append its output", json!({"intent": id, "problems": problems})))
}

/// What the record's old runs must show for the intent's expectation. The record's `old` must exist and name that
/// expectation too; its runs are judged against the intent's expectation, not the record's. None when all of that
/// holds; otherwise the problem names the criterion's old runs.
fn old_problem(expectation: &OldExpectation, current: &CheckResult) -> Option<String> {
    let expect = expectation.expect;
    let Some(old) = current.old.as_ref() else {
        return Some(format!(
            "old (expect {}): missing: the record has no old runs",
            expect.name()
        ));
    };
    let named = (old.expect != expect).then(|| {
        format!(
            "the record names expect {} instead of {}",
            old.expect.name(),
            expect.name()
        )
    });
    let judged = run_problem(expect, &old.runs, current);
    let details: Vec<String> = [named, judged].into_iter().flatten().collect();
    (!details.is_empty()).then(|| format!("old (expect {}): {}", expect.name(), details.join("; ")))
}

/// The judgment of the old runs for the intent's expectation: None when they show it, otherwise what they show.
fn run_problem(expect: Expect, runs: &[OldRun], current: &CheckResult) -> Option<String> {
    let problem = |detail: String| Some(detail);
    match expect {
        Expect::NotApplicable => None,
        Expect::Fails => match runs {
            [] => problem("missing: no old runs are recorded; two are needed".into()),
            [first, second] if first.is_real_failure() && second.is_real_failure() => {
                if first.exit_code == second.exit_code {
                    None
                } else {
                    problem(format!(
                        "the two old runs disagree (exit {:?} and {:?}), so the check is flaky",
                        first.exit_code, second.exit_code
                    ))
                }
            }
            [first, second] => problem(format!(
                "old run 1 {} and old run 2 {}; a real failure exits other than 0, 126 or 127 without timeout",
                describe_run(first),
                describe_run(second)
            )),
            _ => problem(format!("{} old runs are recorded; two are needed", runs.len())),
        },
        Expect::Absent => match runs {
            [] => problem("missing: no old run is recorded".into()),
            _ => runs
                .iter()
                .position(|run| run.exit_code == Some(0) && !run.timed_out)
                .and_then(|index| {
                    problem(format!(
                        "old run {} exited 0, so the behavior is present on the old tree",
                        index + 1
                    ))
                }),
        },
        Expect::Same => match runs {
            [] => problem("missing: no old run is recorded; one is needed".into()),
            [run] if run.exit_code == Some(0) && !run.timed_out => {
                if run.output_tail == current.output_tail {
                    None
                } else {
                    problem("the old output differs from the current output".into())
                }
            }
            [run] => problem(format!(
                "the old run {}; a refactor needs exit 0 on the old tree",
                describe_run(run)
            )),
            _ => problem(format!("{} old runs are recorded; one is needed", runs.len())),
        },
    }
}

fn describe_run(run: &OldRun) -> String {
    match (run.timed_out, run.exit_code) {
        (true, _) => "timed out".into(),
        (false, Some(code)) => format!("exited {code}"),
        (false, None) => {
            "had no exit code (it could not start or was killed by a signal; see its output tail)"
                .into()
        }
    }
}

fn review_form(record: &ContextRecord, intent: &Intent) -> Result<Review, String> {
    let review: Review = serde_json::from_value(record.data.clone())
        .map_err(|e| format!("the review record is invalid: {e}"))?;
    let (who, fresh) = (&review.reviewer, review.fresh_context);
    form(
        &QUESTIONS,
        intent,
        (who, fresh),
        &review.answers,
        &review.findings,
    )?;
    Ok(review)
}

fn intent_check_form(
    record: &ContextRecord,
    records: &[ContextRecord],
) -> Result<IntentCheck, String> {
    let check: IntentCheck = serde_json::from_value(record.data.clone())
        .map_err(|e| format!("the intent-check record is invalid: {e}"))?;
    // Independence is judged against the author of the intent that was checked.
    let checked = records
        .iter()
        .find(|r| r.kind == "intent" && r.id.as_str() == check.intent)
        .ok_or("the intent-check must name an intent record")?;
    let intent = Intent::parse(&checked.data)
        .map_err(|e| format!("the checked intent record is invalid: {e}"))?;
    let (who, fresh) = (&check.reviewer, check.fresh_context);
    form(
        &INTENT_QUESTIONS,
        &intent,
        (who, fresh),
        &check.answers,
        &check.findings,
    )?;
    Ok(check)
}

/// The first well-formed record is the one that counts: a later one cannot replace it and drop its findings.
fn first_well_formed<'a, T>(
    mut candidates: Vec<&'a ContextRecord>,
    none: &str,
    parse: impl Fn(&ContextRecord) -> Result<T, String>,
) -> Result<(&'a str, T), String> {
    candidates.sort_by_key(|r| r.sequence);
    let mut problem = none.to_owned();
    for record in candidates {
        match parse(record) {
            Ok(parsed) => return Ok((record.id.as_str(), parsed)),
            Err(p) => problem = p,
        }
    }
    Err(problem)
}

/// Structure shared by the review and the intent check: independence, fresh context,
/// one noted answer per question, complete findings.
fn form(
    questions: &[(&str, &str)],
    intent: &Intent,
    (reviewer, fresh_context): (&Actor, bool),
    answers: &[Answer],
    findings: &[Finding],
) -> Result<(), String> {
    if reviewer.name.trim().is_empty()
        || reviewer
            .name
            .trim()
            .eq_ignore_ascii_case(intent.author.name.trim())
    {
        return Err("the reviewer must be someone other than the intent author".into());
    }
    if !fresh_context {
        return Err("the review must be done in a fresh context".into());
    }
    let mut asked = BTreeSet::new();
    for answer in answers {
        if !questions.iter().any(|(q, _)| *q == answer.question)
            || !asked.insert(answer.question.as_str())
        {
            return Err(format!("unknown or repeated question {}", answer.question));
        }
        if answer.note.trim().is_empty() {
            return Err(format!("answer to {} needs a note", answer.question));
        }
    }
    if let Some((q, _)) = questions.iter().find(|(q, _)| !asked.contains(q)) {
        return Err(format!("question {q} is unanswered"));
    }
    let mut ids = BTreeSet::new();
    for finding in findings {
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
    for answer in answers.iter().filter(|a| a.verdict == Verdict::Fail) {
        if !findings.iter().any(|f| f.question == answer.question) {
            return Err(format!(
                "question {} failed but has no finding",
                answer.question
            ));
        }
    }
    Ok(())
}

/// Every finding needs its latest disposition with a reason; true when any was fixed.
fn disposed(
    records: &[ContextRecord],
    record_id: &str,
    findings: &[Finding],
) -> Result<bool, String> {
    let mut fixed = false;
    for finding in findings {
        let disposition = records
            .iter()
            .filter(|r| r.kind == "finding-disposition")
            .filter_map(|r| {
                serde_json::from_value::<Disposition>(r.data.clone())
                    .ok()
                    .map(|d| (r.sequence, d))
            })
            .filter(|(_, d)| d.review == record_id && d.finding == finding.id)
            .max_by_key(|(sequence, _)| *sequence)
            .map(|(_, d)| d);
        match disposition {
            Some(d) if !d.reason.trim().is_empty() => {
                fixed |= d.disposition == DispositionKind::Fixed
            }
            _ => {
                return Err(format!(
                    "finding {} needs a disposition (fixed or rejected) with a reason",
                    finding.id
                ))
            }
        }
    }
    Ok(fixed)
}

fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {
    verification(records, input, tree)?;
    let (intent_id, intent) = current_intent(records, input)?;
    let incomplete =
        |problem: String| refuse("review-incomplete", &problem, json!({"intent": intent_id}));
    let reviews = records
        .iter()
        .filter(|r| r.kind == "review" && r.data["intent"] == intent_id.as_str())
        .collect();
    let none = "append a review record for the current intent";
    let (review_id, review) =
        first_well_formed(reviews, none, |r| review_form(r, &intent)).map_err(incomplete)?;
    let fixed = disposed(records, review_id, &review.findings).map_err(incomplete)?;
    let now = tree(input).map_err(|e| refuse("tree-unreadable", &e, json!({})))?;
    if review.tree != now && !fixed {
        return Err(incomplete(
            "the tree changed after the review and no finding was fixed".into(),
        ));
    }
    Ok(())
}
