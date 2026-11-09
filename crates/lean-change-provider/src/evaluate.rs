//! The provider is a checker: it decides whether a checked transition is allowed.
use crate::model::*;
use crate::protocol::{allow, deny, unsupported, EvaluateRequest};
use crate::workflow::workflow;
use loop_core::ContextRecord;
use loop_integrations::owner_checkpoint::{
    self, Actor, Decision, Delegation, OwnerDecision, Rewrite, RewriteField, Status,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

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
    let schema = check.findings_schema;
    if disposed(records, check_id, schema, &check.findings).map_err(incomplete)?
        && check.intent == id
    {
        return Err(incomplete(
            "a finding was fixed, so append a new intent record that fixes it".into(),
        ));
    }
    Ok(())
}

fn approval(records: &[ContextRecord], input: &InitialInput, event: &str) -> Gate {
    let (id, intent) = current_intent(records, input)?;
    require_approved(records, &id, &intent)?;
    if (event == "approved-hard") != intent.hard {
        return Err(refuse(
            "hard-mismatch",
            "request approved when hard=false and approved-hard when hard=true",
            json!({"hard": intent.hard}),
        ));
    }
    Ok(())
}

/// The owner approved exactly this intent record. A delegated approval must also cover the intent's criteria and
/// keep the outcome of the last validly approved intent before it.
fn require_approved(records: &[ContextRecord], id: &str, intent: &Intent) -> Gate {
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
    // Only an approval gets here, and only an approval can carry a delegation.
    if let Some(Delegation {
        covers, rewrites, ..
    }) = evaluation
        .approved
        .as_ref()
        .and_then(|approval| approval.delegation.as_ref())
    {
        covers_intent(id, intent, covers)?;
        keeps_outcome(records, id, intent, rewrites)?;
    }
    Ok(())
}

/// The criteria a delegation misses, and the ids it names that the intent lacks. Both empty means it covers the
/// intent. `covers` is a set, so its order and repeats do not matter.
fn coverage_gap<'a>(covers: &'a [String], intent: &'a Intent) -> (Vec<&'a str>, Vec<&'a str>) {
    let covered: BTreeSet<&str> = covers.iter().map(String::as_str).collect();
    let criteria: BTreeSet<&str> = intent.criteria.iter().map(|c| c.id.as_str()).collect();
    (
        criteria.difference(&covered).copied().collect(),
        covered.difference(&criteria).copied().collect(),
    )
}

/// Check 1: a delegation must name exactly the criteria of the intent it approves.
fn covers_intent(id: &str, intent: &Intent, covers: &[String]) -> Gate {
    let (missing, extra) = coverage_gap(covers, intent);
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    Err(refuse(
        "delegation-not-covering",
        "a delegated approval must name exactly the criteria of this intent",
        json!({"intent": id, "missing": missing, "extra": extra}),
    ))
}

/// Check 2: the approving decision, when delegated, must keep the outcome of the baseline before it. The goal, the
/// constraints and the set of criterion ids are the outcome. A criterion's wording may change only as the
/// delegation's `rewrites` declare it.
fn keeps_outcome(
    records: &[ContextRecord],
    id: &str,
    intent: &Intent,
    rewrites: &[Rewrite],
) -> Gate {
    let approvals = intent_approvals(records);
    // The approving decision is the latest well-formed intent decision, so it is the last approval listed.
    let earlier = approvals
        .split_last()
        .map_or(&[][..], |(_, earlier)| earlier);
    let Some((approved_revision, baseline)) = last_valid_approval(records, earlier) else {
        return Ok(());
    };
    match judge(&baseline, intent, rewrites) {
        Outcome::Keeps => Ok(()),
        Outcome::Changed(differs) => Err(refuse(
            "delegation-changes-approved-outcome",
            "a delegated approval may not change the goal, constraints or criteria the owner approved",
            json!({"intent": id, "approved_revision": approved_revision, "differs": differs}),
        )),
        Outcome::RewriteMismatch(problems) => Err(refuse(
            "delegation-rewrite-mismatch",
            "a delegated approval may reword a criterion statement only when its rewrites name that criterion with the exact text before and after and a reason, and no other",
            json!({"intent": id, "approved_revision": approved_revision, "undeclared": problems.undeclared,
                "not_changed": problems.not_changed, "duplicate": problems.duplicate, "wrong": problems.wrong}),
        )),
    }
}

/// The latest intent whose approval is valid. An approval in the owner's own words is valid. A delegated one is valid
/// only if it covers its intent and keeps the outcome of the baseline before it (with its rewrites declared), so a
/// refused delegation never becomes the baseline for a later one.
fn last_valid_approval(
    records: &[ContextRecord],
    approvals: &[OwnerDecision],
) -> Option<(String, Intent)> {
    let mut baseline: Option<(String, Intent)> = None;
    for approval in approvals {
        let Some(intent) = intent_record(records, &approval.subject_revision) else {
            continue;
        };
        let valid = match &approval.delegation {
            None => true,
            Some(Delegation {
                covers, rewrites, ..
            }) => {
                let (missing, extra) = coverage_gap(covers, &intent);
                missing.is_empty()
                    && extra.is_empty()
                    && baseline.as_ref().is_none_or(|(_, before)| {
                        judge(before, &intent, rewrites) == Outcome::Keeps
                    })
            }
        };
        if valid {
            baseline = Some((approval.subject_revision.clone(), intent));
        }
    }
    baseline
}

/// Every well-formed approval of the owner's intent decisions, oldest first.
fn intent_approvals(records: &[ContextRecord]) -> Vec<OwnerDecision> {
    let mut owner: Vec<&ContextRecord> = records
        .iter()
        .filter(|r| r.kind == "owner-decision")
        .collect();
    owner.sort_by_key(|r| r.sequence);
    owner
        .into_iter()
        .filter_map(|r| owner_checkpoint::parse(&r.data, &["intent"]).ok())
        .filter(|d| {
            d.checkpoint == "intent" && d.subject == "intent" && d.decision == Decision::Approve
        })
        .collect()
}

/// The intent record with this id, when it parses.
fn intent_record(records: &[ContextRecord], id: &str) -> Option<Intent> {
    let record = records
        .iter()
        .find(|r| r.kind == "intent" && r.id.as_str() == id)?;
    Intent::parse(&record.data).ok()
}

/// How a delegated approval's intent compares with the baseline before it.
#[derive(Debug, PartialEq)]
enum Outcome {
    Keeps,
    /// The goal, the constraints or the criterion ids changed, in a fixed order.
    Changed(Vec<&'static str>),
    RewriteMismatch(RewriteProblems),
}

#[derive(Debug, Default, PartialEq)]
struct RewriteProblems {
    /// Criteria whose statement changed with no rewrite entry.
    undeclared: Vec<String>,
    /// Entries naming a criterion and field that did not change, or a criterion the intent lacks.
    not_changed: Vec<Value>,
    /// Criterion and field named by more than one entry.
    duplicate: Vec<Value>,
    /// Entries whose before, after or reason is wrong.
    wrong: Vec<Value>,
}

/// A criterion's statement and check, the two things a rewrite can name.
fn field_value(intent: &Intent, id: &str, field: RewriteField) -> Option<Value> {
    let criterion = intent.criteria.iter().find(|c| c.id == id)?;
    Some(match field {
        RewriteField::Statement => json!(criterion.statement),
        RewriteField::Check => json!(criterion.check),
    })
}

/// The goal, the constraints and the criterion ids are what the owner approved. A statement may change only as
/// `rewrites` declare. A check change needs no declaration (the checks, their timeouts, the old expectations and the
/// hard flag are the agents' choices), but a declared check rewrite must match. Both sides come from the intents.
fn judge(before: &Intent, after: &Intent, rewrites: &[Rewrite]) -> Outcome {
    let ids = |intent: &Intent| -> BTreeSet<String> {
        intent.criteria.iter().map(|c| c.id.clone()).collect()
    };
    let mut differs = Vec::new();
    if before.goal != after.goal {
        differs.push("goal");
    }
    if before.constraints != after.constraints {
        differs.push("constraints");
    }
    if ids(before) != ids(after) {
        differs.push("criteria");
    }
    if !differs.is_empty() {
        return Outcome::Changed(differs);
    }
    let mut problems = RewriteProblems::default();
    let mut declared: BTreeMap<(&str, RewriteField), Vec<&Rewrite>> = BTreeMap::new();
    for rewrite in rewrites {
        declared
            .entry((&rewrite.criterion, rewrite.field))
            .or_default()
            .push(rewrite);
    }
    let name = |id: &str, field: RewriteField| json!({"criterion": id, "field": field});
    for ((id, field), entries) in &declared {
        if entries.len() > 1 {
            problems.duplicate.push(name(id, *field));
            continue;
        }
        let (was, now) = (
            field_value(before, id, *field),
            field_value(after, id, *field),
        );
        if was == now {
            problems.not_changed.push(name(id, *field));
            continue;
        }
        let rewrite = entries[0];
        for (what, expected, given) in [
            ("before", &was, &rewrite.before),
            ("after", &now, &rewrite.after),
        ] {
            if expected.as_ref() != Some(given) {
                problems.wrong.push(json!({"criterion": id, "field": field, "wrong": what, "expected": expected, "given": given}));
            }
        }
        if rewrite.reason.trim().is_empty() {
            problems.wrong.push(
                json!({"criterion": id, "field": field, "wrong": "reason", "problem": "blank"}),
            );
        }
    }
    for criterion in &after.criteria {
        let id = criterion.id.as_str();
        let changed = field_value(before, id, RewriteField::Statement)
            != field_value(after, id, RewriteField::Statement);
        if changed && !declared.contains_key(&(id, RewriteField::Statement)) {
            problems.undeclared.push(id.to_owned());
        }
    }
    if problems == RewriteProblems::default() {
        Outcome::Keeps
    } else {
        Outcome::RewriteMismatch(problems)
    }
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
    require_approved(records, &id, &intent)?;
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
pub fn old_problem(expectation: &OldExpectation, current: &CheckResult) -> Option<String> {
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
        review_questions(review.findings_schema),
        intent,
        (who, fresh),
        review.findings_schema,
        review.head.is_some() || review.brief_sha256.is_some(),
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
        check.findings_schema,
        check.head.is_some() || check.brief_sha256.is_some(),
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

/// The contract's reject codes for a malformed finding, in its check order. A blank id is checked over all findings
/// before a duplicate one, and so on: the first check that fails over any finding gives the code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindingReject {
    BlankId,
    DuplicateId,
    BlankText,
    BadSeverity,
}

impl FindingReject {
    pub fn code(self) -> &'static str {
        match self {
            FindingReject::BlankId => "blank-id",
            FindingReject::DuplicateId => "duplicate-id",
            FindingReject::BlankText => "blank-text",
            FindingReject::BadSeverity => "bad-severity",
        }
    }
}

/// The per-finding checks of `findings_schema: 1`. Severity that is unknown fails the record's parse, so here it is
/// only missing. A blocking finding that lacks a field is not rejected: `Finding::effective` demotes it.
pub fn check_findings(findings: &[Finding]) -> Result<(), FindingReject> {
    if findings.iter().any(|f| f.id.trim().is_empty()) {
        return Err(FindingReject::BlankId);
    }
    let mut ids = BTreeSet::new();
    if !findings.iter().all(|f| ids.insert(f.id.as_str())) {
        return Err(FindingReject::DuplicateId);
    }
    if findings.iter().any(|f| f.text.trim().is_empty()) {
        return Err(FindingReject::BlankText);
    }
    if findings.iter().any(|f| f.severity.is_none()) {
        return Err(FindingReject::BadSeverity);
    }
    Ok(())
}

/// Structure shared by the review and the intent check: independence, fresh context,
/// one noted answer per question, complete findings.
fn form(
    questions: &[(&str, &str)],
    intent: &Intent,
    (reviewer, fresh_context): (&Actor, bool),
    findings_schema: Option<u8>,
    has_envelope_fields: bool,
    answers: &[Answer],
    findings: &[Finding],
) -> Result<(), String> {
    if findings_schema.is_some_and(|schema| schema != 1) {
        return Err("findings_schema must be 1 or absent".into());
    }
    // `head` and `brief_sha256` are the findings contract's optional envelope fields. They are accepted and nothing
    // reads them.
    if has_envelope_fields && findings_schema.is_none() {
        return Err("head and brief_sha256 need findings_schema: 1".into());
    }
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
    if findings_schema.is_some() {
        // Notes need only an id, a known question and text; a blocking finding that lacks a field is demoted on read.
        check_findings(findings).map_err(|reject| match reject {
            FindingReject::BlankId | FindingReject::DuplicateId => {
                "findings need a unique, nonblank id".to_owned()
            }
            FindingReject::BlankText => "every finding needs text".to_owned(),
            FindingReject::BadSeverity => {
                "every finding needs a severity: blocking or note".to_owned()
            }
        })?;
        if let Some(finding) = findings
            .iter()
            .find(|f| !asked.contains(f.question.as_str()))
        {
            return Err(format!("finding {} needs a known question", finding.id));
        }
    } else {
        let blank = |field: &Option<String>| field.as_deref().is_none_or(|f| f.trim().is_empty());
        let mut ids = BTreeSet::new();
        for finding in findings {
            let v1_only = finding.severity.is_some()
                || finding.obligation.is_some()
                || finding.component.is_some();
            if v1_only
                || finding.id.trim().is_empty()
                || finding.text.trim().is_empty()
                || blank(&finding.evidence)
                || blank(&finding.consequence)
                || !asked.contains(finding.question.as_str())
                || !ids.insert(finding.id.as_str())
            {
                return Err(format!(
                    "finding {} needs a unique id, a known question, text, evidence and consequence (severity, obligation and component need findings_schema: 1)",
                    finding.id
                ));
            }
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

/// Every effectively-blocking finding needs its latest disposition with a reason; true when any was fixed. A note
/// needs none.
fn disposed(
    records: &[ContextRecord],
    record_id: &str,
    findings_schema: Option<u8>,
    findings: &[Finding],
) -> Result<bool, String> {
    let mut fixed = false;
    for finding in findings
        .iter()
        .filter(|f| f.effective(findings_schema) == Effective::Blocking)
    {
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
    let schema = review.findings_schema;
    let fixed = disposed(records, review_id, schema, &review.findings).map_err(incomplete)?;
    let now = tree(input).map_err(|e| refuse("tree-unreadable", &e, json!({})))?;
    // A v1 review with no effectively-blocking finding has nothing to fix, so a later edit needs only
    // re-verification. A v0 review gates as before: a later edit needs a fixed finding.
    let needs_fix = schema.is_none()
        || review
            .findings
            .iter()
            .any(|f| f.effective(schema) == Effective::Blocking);
    if review.tree != now && needs_fix && !fixed {
        return Err(incomplete(
            "the tree changed after the review and no finding was fixed".into(),
        ));
    }
    Ok(())
}
