//! Unit tests: every gate has a test that fails if the gate is removed.
use lean_change_provider::evaluate::evaluate as evaluate_with;
use lean_change_provider::model::*;
use lean_change_provider::protocol::EvaluateRequest;
use lean_change_provider::workflow::workflow;

fn evaluate(request: &EvaluateRequest) -> Result<Value, String> {
    evaluate_with(request, &crate::run::tree_identity)
}
use loop_core::{ContextRecord, SemanticSequence, Timestamp, Transition};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lean-change-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "t@example.invalid"]);
    git(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "one").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "base"]);
    dir
}

const PRINCIPLES: &str = "1. Compose.";

fn input(repo: &Path, review: bool) -> Value {
    let mut v = json!({"schema_version":1,"profile_version":"t-1","repo":repo.display().to_string(),"base":"HEAD","review":review});
    if review {
        v["principles"] =
            json!({"source":"p.md","sha256":sha256_hex(PRINCIPLES.as_bytes()),"text":PRINCIPLES});
    }
    v
}

fn record(id: &str, kind: &str, sequence: u64, data: Value) -> ContextRecord {
    ContextRecord::new(
        id,
        kind,
        data,
        SemanticSequence::new(sequence),
        Timestamp::from_unix_millis(0),
    )
}

fn intent(hard: bool) -> Value {
    json!({"author":{"name":"driver","kind":"agent"},"goal":"g","constraints":["c"],"hard":hard,
        "criteria":[{"id":"c1","statement":"s1","check":{"argv":["true"],"timeout_ms":1000}},
                    {"id":"c2","statement":"s2","check":{"argv":["true"],"timeout_ms":1000}}]})
}

fn decision(text: &str, revision: &str) -> Value {
    json!({"checkpoint":"intent","subject":"intent","subject_revision":revision,"decision":"approve","text":text,"actor":{"name":"owner","kind":"human"}})
}

fn tree_of(repo: &Path) -> String {
    loop_integrations::proof_metadata::repository_proof_identity(repo).unwrap()
}

fn verification(repo: &Path, intent_value: &Value) -> Value {
    let results: Vec<Value> = intent_value["criteria"].as_array().unwrap().iter().map(|c| {
        let check: Check = serde_json::from_value(c["check"].clone()).unwrap();
        json!({"criterion":c["id"],"check_sha256":check.sha256(),"exit_code":0,"timed_out":false,"output_tail":""})
    }).collect();
    json!({"intent":"intent-1","tree":tree_of(repo),"results":results})
}

/// The six answers of a record without `findings_schema`.
fn answers() -> Vec<Value> {
    review_questions(None)
        .iter()
        .map(|(q, _)| json!({"question":q,"verdict":"pass","note":"ok"}))
        .collect()
}

fn review(repo: &Path) -> Value {
    json!({"intent":"intent-1","tree":tree_of(repo),"reviewer":{"name":"reviewer","kind":"agent"},"fresh_context":true,"answers":answers(),"findings":[]})
}

fn finding(id: &str, question: &str) -> Value {
    json!({"id":id,"question":question,"text":"t","evidence":"e","consequence":"c"})
}

fn ask(
    repo: &Path,
    review_on: bool,
    source: &str,
    event: &str,
    context: Vec<ContextRecord>,
) -> Value {
    ask_with(input(repo, review_on), source, event, context)
}

/// Asks the gate about an initial input as given, so an old-vs-new profile can be asked too.
fn ask_with(initial_input: Value, source: &str, event: &str, context: Vec<ContextRecord>) -> Value {
    let review = initial_input["review"].as_bool().unwrap();
    let wf = workflow(review, initial_input.get("old_vs_new").is_some());
    let transition = wf
        .transitions
        .iter()
        .find(|t| t.source.as_str() == source && t.event.as_str() == event)
        .cloned()
        .unwrap();
    let request = EvaluateRequest {
        operation: "evaluate".into(),
        workflow: wf,
        initial_input,
        context,
        transition,
        prior_evaluations: Vec::new(),
    };
    evaluate(&request).unwrap()
}

fn code(response: &Value) -> String {
    assert_eq!(response["result"], "deny", "{response}");
    response["feedback"]["code"].as_str().unwrap().to_owned()
}

fn problems(response: &Value) -> String {
    response["feedback"]["details"]["problems"].to_string()
}

fn allowed(response: &Value) {
    assert_eq!(response["result"], "allow", "{response}");
}

#[test]
fn check_free_edges_are_allowed_and_foreign_requests_unsupported() {
    let r = repo();
    allowed(&ask(&r, true, "implement", "ready", vec![]));
    let wf = workflow(true, false);
    let request = |workflow, transition| EvaluateRequest {
        operation: "evaluate".into(),
        workflow,
        initial_input: input(&r, true),
        context: vec![],
        transition,
        prior_evaluations: vec![],
    };
    let unknown = Transition::checked("explore", "bogus", "end");
    assert_eq!(
        evaluate(&request(wf.clone(), unknown)).unwrap()["result"],
        "unsupported"
    );
    // A run frozen as C is not evaluated as B.
    let t = wf.transitions[0].clone();
    assert_eq!(
        evaluate(&request(workflow(false, false), t)).unwrap()["result"],
        "unsupported"
    );
}

#[test]
fn intent_gate_needs_a_valid_intent_with_a_check_per_criterion() {
    let r = repo();
    assert_eq!(
        code(&ask(&r, true, "explore", "intent-ready", vec![])),
        "intent-missing"
    );
    let mut bad = intent(false);
    bad["criteria"][0]["check"]["argv"] = json!([]);
    assert_eq!(
        code(&ask(
            &r,
            true,
            "explore",
            "intent-ready",
            vec![record("intent-1", "intent", 1, bad)]
        )),
        "intent-invalid"
    );
    let mut dup = intent(false);
    dup["criteria"][1]["id"] = json!("c1");
    assert_eq!(
        code(&ask(
            &r,
            true,
            "explore",
            "intent-ready",
            vec![record("intent-1", "intent", 1, dup)]
        )),
        "intent-invalid"
    );
    // Profile c needs no intent check.
    allowed(&ask(
        &r,
        false,
        "explore",
        "intent-ready",
        vec![record("intent-1", "intent", 1, intent(false))],
    ));
}

fn check_value() -> Value {
    let answers: Vec<Value> = INTENT_QUESTIONS
        .iter()
        .map(|(q, _)| json!({"question":q,"verdict":"pass","note":"settled"}))
        .collect();
    json!({"intent":"intent-1","reviewer":{"name":"reviewer","kind":"agent"},"fresh_context":true,"answers":answers,"findings":[]})
}

/// A check whose question "use" failed with one finding f1.
fn failed_check() -> Value {
    let mut v = check_value();
    v["answers"][0]["verdict"] = json!("fail");
    v["findings"] = json!([finding("f1", "use")]);
    v
}

fn check_ctx(check: Value, extra: Vec<ContextRecord>) -> Vec<ContextRecord> {
    let mut ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("chk1", "intent-check", 2, check),
    ];
    ctx.extend(extra);
    ctx
}

fn check_answer(r: &Path, ctx: Vec<ContextRecord>) -> Value {
    ask(r, true, "explore", "intent-ready", ctx)
}

fn check_denial(r: &Path, ctx: Vec<ContextRecord>) -> String {
    let response = check_answer(r, ctx);
    assert_eq!(code(&response), "intent-check-incomplete", "{response}");
    response["feedback"]["message"].as_str().unwrap().to_owned()
}

fn check_disposition(finding: &str, kind: &str, reason: &str, sequence: u64) -> ContextRecord {
    record(
        &format!("cd-{sequence}"),
        "finding-disposition",
        sequence,
        json!({"review":"chk1","finding":finding,"disposition":kind,"reason":reason}),
    )
}

#[test]
fn intent_ready_needs_an_independent_complete_disposed_intent_check() {
    let r = repo();
    // No check, or only an intent: the driver is told what to run.
    let message = check_denial(&r, vec![record("intent-1", "intent", 1, intent(false))]);
    assert!(message.contains("intent-packet"), "{message}");
    allowed(&check_answer(&r, check_ctx(check_value(), vec![])));
    let bad = |mutate: fn(&mut Value)| {
        let mut v = check_value();
        mutate(&mut v);
        check_denial(&r, check_ctx(v, vec![]))
    };
    assert!(bad(|v| v["reviewer"]["name"] = json!("Driver")).contains("someone other"));
    assert!(bad(|v| v["fresh_context"] = json!(false)).contains("fresh context"));
    assert!(bad(|v| v["intent"] = json!("intent-9")).contains("must name an intent record"));
    assert!(bad(|v| v["answers"][1]["question"] = json!("scope")).contains("unknown or repeated"));
    assert!(bad(|v| v["answers"][1]["question"] = json!("use")).contains("unknown or repeated"));
    assert!(bad(|v| {
        v["answers"].as_array_mut().unwrap().pop();
    })
    .contains("unanswered"));
    assert!(bad(|v| v["answers"][0]["note"] = json!(" ")).contains("needs a note"));
    assert!(bad(|v| v["answers"][3]["verdict"] = json!("fail")).contains("has no finding"));
    assert!(bad(|v| v["findings"] = json!([finding("f1", "scope")])).contains("unique id"));
    assert!(bad(|v| {
        v["findings"] = json!([finding("f1", "use")]);
        v["findings"][0]["evidence"] = json!("");
    })
    .contains("unique id"));
    // Every finding is disposed with a reason; a disposition for another record does not count.
    let failed = || check_ctx(failed_check(), vec![]);
    assert!(check_denial(&r, failed()).contains("f1 needs a disposition"));
    let wrong = record(
        "d-rev",
        "finding-disposition",
        3,
        json!({"review":"rev1","finding":"f1","disposition":"rejected","reason":"x"}),
    );
    assert!(
        check_denial(&r, check_ctx(failed_check(), vec![wrong])).contains("f1 needs a disposition")
    );
    let blank = check_disposition("f1", "rejected", " ", 3);
    assert!(
        check_denial(&r, check_ctx(failed_check(), vec![blank])).contains("f1 needs a disposition")
    );
    allowed(&check_answer(
        &r,
        check_ctx(
            failed_check(),
            vec![check_disposition(
                "f1",
                "rejected",
                "settled by the goal",
                3,
            )],
        ),
    ));
    // A fix needs a newer intent record than the one checked.
    let fixed = check_disposition("f1", "fixed", "named the term", 3);
    assert!(
        check_denial(&r, check_ctx(failed_check(), vec![fixed.clone()]))
            .contains("new intent record")
    );
    let newer = record("intent-2", "intent", 4, intent(false));
    allowed(&check_answer(
        &r,
        check_ctx(failed_check(), vec![fixed, newer]),
    ));
    // The first well-formed check is the check: a later clean one cannot drop its findings.
    let mut ctx = failed();
    ctx.push(record("chk2", "intent-check", 3, check_value()));
    assert!(check_denial(&r, ctx).contains("f1 needs a disposition"));
    // Order in the context does not matter, only the sequence.
    let mut ctx = failed();
    ctx.push(record("chk2", "intent-check", 3, check_value()));
    ctx.reverse();
    assert!(check_denial(&r, ctx).contains("f1 needs a disposition"));
    // Independence is judged against the author of the intent that was checked, not the current one.
    let by = |name: &str| {
        let mut v = intent(false);
        v["author"]["name"] = json!(name);
        v
    };
    let checked_by = |reviewer: &str, mut value: Value| {
        value["reviewer"]["name"] = json!(reviewer);
        value
    };
    let ctx = vec![
        record("intent-1", "intent", 1, by("alice")),
        record("chk1", "intent-check", 2, checked_by("bob", failed_check())),
        record("intent-2", "intent", 3, by("bob")),
        record(
            "chk2",
            "intent-check",
            4,
            checked_by("carol", check_value()),
        ),
    ];
    assert!(check_denial(&r, ctx).contains("f1 needs a disposition"));
    let ctx = vec![
        record("intent-1", "intent", 1, by("alice")),
        record(
            "chk1",
            "intent-check",
            2,
            checked_by("alice", check_value()),
        ),
        record("intent-2", "intent", 3, by("bob")),
    ];
    assert!(check_denial(&r, ctx).contains("someone other"));
    // An earlier malformed check does not count.
    let mut ctx = vec![record("intent-1", "intent", 1, intent(false))];
    let mut broken = check_value();
    broken["fresh_context"] = json!(false);
    ctx.push(record("chk0", "intent-check", 2, broken));
    ctx.push(record("chk1", "intent-check", 3, check_value()));
    allowed(&check_answer(&r, ctx));
}

#[test]
fn approval_needs_the_owners_verbatim_words_for_this_intent_record() {
    let r = repo();
    let base = |decisions: Vec<ContextRecord>, hard: bool, event: &str| {
        let mut ctx = vec![record("intent-1", "intent", 1, intent(hard))];
        ctx.extend(decisions);
        ask(&r, true, "intent-approval", event, ctx)
    };
    assert_eq!(
        code(&base(vec![], false, "approved")),
        "owner-decision-missing"
    );
    // Blank words are not the owner's words.
    assert_eq!(
        code(&base(
            vec![record("d", "owner-decision", 2, decision("  ", "intent-1"))],
            false,
            "approved"
        )),
        "owner-decision-missing"
    );
    // An approval of another intent record does not carry over.
    assert_eq!(
        code(&base(
            vec![record(
                "d",
                "owner-decision",
                2,
                decision("yes", "intent-0")
            )],
            false,
            "approved"
        )),
        "owner-decision-missing"
    );
    let good = vec![record(
        "d",
        "owner-decision",
        2,
        decision("Yes, build it.", "intent-1"),
    )];
    allowed(&base(good.clone(), false, "approved"));
    assert_eq!(
        code(&base(good.clone(), false, "approved-hard")),
        "hard-mismatch"
    );
    assert_eq!(code(&base(good.clone(), true, "approved")), "hard-mismatch");
    allowed(&base(good, true, "approved-hard"));
    // A revised intent needs a new approval.
    let revised = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d", "owner-decision", 2, decision("yes", "intent-1")),
        record("intent-2", "intent", 3, intent(false)),
    ];
    assert_eq!(
        code(&ask(&r, true, "intent-approval", "approved", revised)),
        "owner-decision-missing"
    );
    let mut rejected = decision("no", "intent-1");
    rejected["decision"] = json!("reject");
    rejected["target"] = json!("intent");
    rejected["reason"] = json!("too wide");
    assert_eq!(
        code(&base(
            vec![record("d", "owner-decision", 2, rejected)],
            false,
            "approved"
        )),
        "intent-rejected"
    );
}

#[test]
fn plan_gate_needs_a_plan_for_the_current_intent() {
    let r = repo();
    let ctx = |plan: Option<Value>| {
        let mut v = vec![record("intent-1", "intent", 1, intent(true))];
        v.extend(plan.map(|p| record("p", "plan", 2, p)));
        v
    };
    assert_eq!(
        code(&ask(&r, true, "plan", "planned", ctx(None))),
        "plan-missing"
    );
    assert_eq!(
        code(&ask(
            &r,
            true,
            "plan",
            "planned",
            ctx(Some(json!({"intent":"intent-1","text":" "})))
        )),
        "plan-missing"
    );
    assert_eq!(
        code(&ask(
            &r,
            true,
            "plan",
            "planned",
            ctx(Some(json!({"intent":"intent-0","text":"x"})))
        )),
        "plan-missing"
    );
    allowed(&ask(
        &r,
        true,
        "plan",
        "planned",
        ctx(Some(json!({"intent":"intent-1","text":"do x"}))),
    ));
}

fn verify_ctx(r: &Path, mutate: impl Fn(&mut Value)) -> Vec<ContextRecord> {
    let i = intent(false);
    let mut v = verification(r, &i);
    mutate(&mut v);
    vec![
        record("intent-1", "intent", 1, i),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("v1", "verification", 3, v),
    ]
}

#[test]
fn verification_gate_binds_every_criterion_check_to_the_current_tree() {
    let r = repo();
    // No verification at all.
    let only_intent = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
    ];
    assert_eq!(
        code(&ask(&r, false, "verify", "verified", only_intent)),
        "verification-incomplete"
    );
    // A criterion without a result.
    let missing = ask(
        &r,
        false,
        "verify",
        "verified",
        verify_ctx(&r, |v| {
            v["results"].as_array_mut().unwrap().pop();
        }),
    );
    assert_eq!(code(&missing), "verification-incomplete");
    assert!(problems(&missing).contains("missing"));
    // A failed check.
    let failed = ask(
        &r,
        false,
        "verify",
        "verified",
        verify_ctx(&r, |v| v["results"][0]["exit_code"] = json!(1)),
    );
    assert!(problems(&failed).contains("failed"));
    let timed = ask(
        &r,
        false,
        "verify",
        "verified",
        verify_ctx(&r, |v| {
            v["results"][0]["exit_code"] = json!(null);
            v["results"][0]["timed_out"] = json!(true);
        }),
    );
    assert!(problems(&timed).contains("failed"));
    // A result for a different check than the approved one.
    let changed = ask(
        &r,
        false,
        "verify",
        "verified",
        verify_ctx(&r, |v| v["results"][0]["check_sha256"] = json!("deadbeef")),
    );
    assert!(problems(&changed).contains("not the approved one"));
    // Verification of another intent record.
    let other = ask(
        &r,
        false,
        "verify",
        "verified",
        verify_ctx(&r, |v| v["intent"] = json!("intent-0")),
    );
    assert_eq!(code(&other), "verification-incomplete");
    // Fresh verification passes; any later edit makes it stale.
    let ctx = verify_ctx(&r, |_| {});
    allowed(&ask(&r, false, "verify", "verified", ctx.clone()));
    std::fs::write(r.join("a.txt"), "two").unwrap();
    let stale = ask(&r, false, "verify", "verified", ctx);
    assert_eq!(code(&stale), "verification-incomplete");
    assert!(problems(&stale).contains("stale"));
}

fn review_ctx(r: &Path, review_value: Value, extra: Vec<ContextRecord>) -> Vec<ContextRecord> {
    let mut ctx = verify_ctx(r, |_| {});
    ctx.push(record("rev1", "review", 4, review_value));
    ctx.extend(extra);
    ctx
}

fn disposition(finding: &str, kind: &str, reason: &str, sequence: u64) -> ContextRecord {
    record(
        &format!("d-{finding}-{sequence}"),
        "finding-disposition",
        sequence,
        json!({"review":"rev1","finding":finding,"disposition":kind,"reason":reason}),
    )
}

fn review_denial(r: &Path, review_value: Value, extra: Vec<ContextRecord>) -> String {
    let response = ask(
        r,
        true,
        "review",
        "reviewed",
        review_ctx(r, review_value, extra),
    );
    assert_eq!(code(&response), "review-incomplete", "{response}");
    response["feedback"]["message"].as_str().unwrap().to_owned()
}

#[test]
fn review_gate_needs_an_independent_complete_disposed_review() {
    let r = repo();
    // Verification is checked first.
    assert_eq!(
        code(&ask(
            &r,
            true,
            "review",
            "reviewed",
            vec![
                record("intent-1", "intent", 1, intent(false)),
                record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1"))
            ]
        )),
        "verification-incomplete"
    );
    // No review record.
    let none = ask(&r, true, "review", "reviewed", verify_ctx(&r, |_| {}));
    assert_eq!(code(&none), "review-incomplete");
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, review(&r), vec![]),
    ));

    let mut same = review(&r);
    same["reviewer"]["name"] = json!(" Driver ");
    assert!(review_denial(&r, same, vec![]).contains("someone other"));
    let mut stale_ctx = review(&r);
    stale_ctx["fresh_context"] = json!(false);
    assert!(review_denial(&r, stale_ctx, vec![]).contains("fresh context"));
    let mut missing_q = review(&r);
    missing_q["answers"].as_array_mut().unwrap().pop();
    assert!(review_denial(&r, missing_q, vec![]).contains("principles is unanswered"));
    let mut dup_q = review(&r);
    dup_q["answers"][1]["question"] = json!("intent");
    assert!(review_denial(&r, dup_q, vec![]).contains("repeated"));
    let mut no_note = review(&r);
    no_note["answers"][0]["note"] = json!("");
    assert!(review_denial(&r, no_note, vec![]).contains("needs a note"));
    let mut shape = review(&r);
    shape["extra"] = json!(1);
    assert!(review_denial(&r, shape, vec![]).contains("invalid"));
    let mut other_intent = review(&r);
    other_intent["intent"] = json!("intent-0");
    let response = ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, other_intent, vec![]),
    );
    assert_eq!(code(&response), "review-incomplete");

    // A failed question needs a finding; a finding needs all fields.
    let mut failed = review(&r);
    failed["answers"][5]["verdict"] = json!("fail");
    assert!(review_denial(&r, failed.clone(), vec![]).contains("no finding"));
    let mut thin = failed.clone();
    let mut f = finding("f1", "principles");
    f["evidence"] = json!(" ");
    thin["findings"] = json!([f]);
    assert!(review_denial(&r, thin, vec![]).contains("evidence and consequence"));
    let mut unknown = failed.clone();
    unknown["findings"] = json!([finding("f1", "nonsense")]);
    assert!(review_denial(&r, unknown, vec![]).contains("known question"));
    let mut twice = failed.clone();
    twice["findings"] = json!([finding("f1", "principles"), finding("f1", "docs")]);
    assert!(review_denial(&r, twice, vec![]).contains("unique id"));

    // Every finding needs a disposition with a reason.
    let mut with_finding = failed.clone();
    with_finding["findings"] = json!([finding("f1", "principles")]);
    assert!(review_denial(&r, with_finding.clone(), vec![]).contains("needs a disposition"));
    assert!(review_denial(
        &r,
        with_finding.clone(),
        vec![disposition("f1", "rejected", " ", 4)]
    )
    .contains("needs a disposition"));
    let mut wrong_review = disposition("f1", "rejected", "why", 4);
    wrong_review.data["review"] = json!("rev0");
    assert!(
        review_denial(&r, with_finding.clone(), vec![wrong_review]).contains("needs a disposition")
    );
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(
            &r,
            with_finding.clone(),
            vec![disposition("f1", "rejected", "not in scope", 4)],
        ),
    ));
    // The latest disposition wins.
    assert!(review_denial(
        &r,
        with_finding.clone(),
        vec![
            disposition("f1", "rejected", "why", 4),
            disposition("f1", "fixed", "", 5)
        ]
    )
    .contains("needs a disposition"));
}

#[test]
fn review_gate_after_edits_needs_a_fixed_finding_and_fresh_verification() {
    let r = repo();
    let mut reviewed = review(&r);
    reviewed["answers"][5]["verdict"] = json!("fail");
    reviewed["findings"] = json!([finding("f1", "principles")]);
    // Records written before the fix describe the tree before it.
    let fixed = review_ctx(
        &r,
        reviewed.clone(),
        vec![disposition("f1", "fixed", "done", 4)],
    );
    let rejected = review_ctx(&r, reviewed, vec![disposition("f1", "rejected", "no", 4)]);
    std::fs::write(r.join("a.txt"), "fixed").unwrap();
    // The fix changes the tree: the old verification is stale until re-run.
    let stale = ask(&r, true, "review", "reviewed", fixed.clone());
    assert_eq!(code(&stale), "verification-incomplete");
    let verified = record("v2", "verification", 5, verification(&r, &intent(false)));
    let mut ctx = fixed;
    ctx.push(verified.clone());
    allowed(&ask(&r, true, "review", "reviewed", ctx));
    // The same edit with the finding rejected is an unreviewed change.
    let mut ctx = rejected;
    ctx.push(verified);
    let denied = ask(&r, true, "review", "reviewed", ctx);
    assert_eq!(code(&denied), "review-incomplete");
    assert!(denied["feedback"]["message"]
        .as_str()
        .unwrap()
        .contains("tree changed after the review"));
}

#[test]
fn initial_input_is_validated() {
    let r = repo();
    assert!(InitialInput::parse(&input(&r, true)).is_ok());
    assert!(InitialInput::parse(&input(&r, false)).is_ok());
    let mut no_principles = input(&r, true);
    no_principles.as_object_mut().unwrap().remove("principles");
    assert!(InitialInput::parse(&no_principles)
        .unwrap_err()
        .contains("requires principles"));
    let mut tampered = input(&r, true);
    tampered["principles"]["text"] = json!("changed");
    assert!(InitialInput::parse(&tampered)
        .unwrap_err()
        .contains("sha256"));
    let mut relative = input(&r, false);
    relative["repo"] = json!("rel");
    assert!(InitialInput::parse(&relative)
        .unwrap_err()
        .contains("absolute"));
    let mut extra = input(&r, false);
    extra["x"] = json!(1);
    assert!(InitialInput::parse(&extra).is_err());
}

fn show(r: &Path, review_on: bool, context: Vec<ContextRecord>) -> Value {
    show_with(input(r, review_on), context)
}

fn show_with(initial_input: Value, context: Vec<ContextRecord>) -> Value {
    json!({"operation":"show","status":"completed","result":{"workflow_id":"lean-change","initial_input":initial_input,"context":context}})
}

#[test]
fn verify_runs_each_check_and_records_the_tree_after() {
    let r = repo();
    let mut i = intent(false);
    i["criteria"][0]["check"] =
        json!({"argv":["sh","-c","echo out; test -f a.txt"],"timeout_ms":5000});
    i["criteria"][1]["check"] =
        json!({"argv":["sh","-c","echo bad >&2; exit 3"],"timeout_ms":5000});
    let v = crate::run::verify(&show(
        &r,
        false,
        vec![record("intent-1", "intent", 1, i.clone())],
    ))
    .unwrap();
    assert_eq!(v.intent, "intent-1");
    assert_eq!(v.tree, tree_of(&r));
    assert_eq!(
        (v.results[0].exit_code, v.results[0].output_tail.as_str()),
        (Some(0), "out\n")
    );
    assert_eq!(
        (v.results[1].exit_code, v.results[1].output_tail.as_str()),
        (Some(3), "bad\n")
    );
    let parsed: Intent = serde_json::from_value(i).unwrap();
    assert_eq!(v.results[1].check_sha256, parsed.criteria[1].check.sha256());
}

#[test]
fn verify_kills_a_check_at_its_timeout_and_uses_the_latest_intent() {
    let r = repo();
    let mut slow = intent(false);
    slow["criteria"][0]["check"] = json!({"argv":["sleep","5"],"timeout_ms":100});
    let first = record("intent-1", "intent", 1, intent(false));
    let second = record("intent-2", "intent", 2, slow);
    let v = crate::run::verify(&show(&r, false, vec![second, first])).unwrap();
    assert_eq!(v.intent, "intent-2");
    assert!(v.results[0].timed_out && v.results[0].exit_code.is_none());
    assert!(crate::run::verify(&show(&r, false, vec![]))
        .unwrap_err()
        .contains("no intent"));
    assert!(crate::run::verify(&json!({"operation":"show"})).is_err());
}

#[test]
fn setup_freezes_repo_base_and_principles() {
    let r = repo();
    let p = r.join("p.md");
    std::fs::write(&p, PRINCIPLES).unwrap();
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let repo_arg = r.display().to_string();
    let b = crate::packet::setup(&args(&[
        "--profile",
        "b",
        "--repo",
        &repo_arg,
        "--principles",
        &p.display().to_string(),
    ]))
    .unwrap();
    assert_eq!(b["review"], true);
    assert_eq!(b["principles"]["sha256"], sha256_hex(PRINCIPLES.as_bytes()));
    assert_eq!(b["base"].as_str().unwrap().len(), 40);
    InitialInput::parse(&b).unwrap();
    let c = crate::packet::setup(&args(&["--profile", "c", "--repo", &repo_arg])).unwrap();
    assert_eq!(c["review"], false);
    assert!(c.get("principles").is_none());
    assert!(
        crate::packet::setup(&args(&["--profile", "b", "--repo", &repo_arg]))
            .unwrap_err()
            .contains("--principles")
    );
    assert!(crate::packet::setup(&args(&["--profile", "x", "--repo", &repo_arg])).is_err());
    assert!(crate::packet::setup(&args(&["--profile", "c"])).is_err());
    assert!(
        crate::packet::setup(&args(&["--profile", "c", "--repo", "/nonexistent-lean"])).is_err()
    );
}

#[test]
fn review_packet_carries_questions_intent_principles_and_the_untracked_diff() {
    let r = repo();
    std::fs::write(r.join("new.txt"), "brand new").unwrap();
    std::fs::write(r.join("a.txt"), "changed").unwrap();
    let i = intent(false);
    let approved = vec![
        record("intent-1", "intent", 1, i.clone()),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
    ];
    // No verification yet.
    assert!(crate::packet::review_packet(&show(&r, true, approved.clone())).is_err());
    let mut ctx = approved;
    ctx.push(record("v1", "verification", 3, verification(&r, &i)));
    let packet = crate::packet::review_packet(&show(&r, true, ctx.clone())).unwrap();
    assert!(packet.contains(&tree_of(&r)) && packet.contains("- c1: exit Some(0)"));
    std::fs::write(r.join("late.txt"), "x").unwrap();
    assert!(crate::packet::review_packet(&show(&r, true, ctx.clone()))
        .unwrap_err()
        .contains("stale"));
    std::fs::remove_file(r.join("late.txt")).unwrap();
    for (id, _) in QUESTIONS {
        assert!(packet.contains(&format!("- {id}: ")), "{id}");
    }
    assert!(
        packet.contains(PRINCIPLES) && packet.contains("brand new") && packet.contains("+changed")
    );
    assert!(packet.contains("\"goal\": \"g\"") && packet.contains("intent-1"));
    // The reviewer's work does not touch the repository's own index.
    assert!(String::from_utf8(
        Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&r)
            .output()
            .unwrap()
            .stdout
    )
    .unwrap()
    .contains("?? new.txt"));
    let mut ctx = vec![record("intent-1", "intent", 1, i.clone())];
    ctx.push(record(
        "d-ok",
        "owner-decision",
        2,
        decision("Yes.", "intent-1"),
    ));
    ctx.push(record("v1", "verification", 3, verification(&r, &i)));
    assert!(crate::packet::review_packet(&show(&r, false, ctx)).is_err());
}

#[test]
fn downstream_gates_need_the_approval_and_plan_of_the_current_intent() {
    let r = repo();
    // Approved as not hard, then replaced by a different, hard, unapproved intent.
    let mut ctx = verify_ctx(&r, |_| {});
    let mut hard = intent(true);
    hard["criteria"][0]["check"]["argv"] = json!(["false"]);
    ctx.push(record("intent-2", "intent", 4, hard.clone()));
    let mut v = verification(&r, &hard);
    v["intent"] = json!("intent-2");
    ctx.push(record("v2", "verification", 5, v.clone()));
    assert_eq!(
        code(&ask(&r, false, "verify", "verified", ctx.clone())),
        "owner-decision-missing"
    );
    // Approved but hard: it still needs its plan.
    ctx.push(record(
        "d2",
        "owner-decision",
        6,
        decision("Yes.", "intent-2"),
    ));
    assert_eq!(
        code(&ask(&r, false, "verify", "verified", ctx.clone())),
        "plan-missing"
    );
    ctx.push(record(
        "p",
        "plan",
        7,
        json!({"intent":"intent-2","text":"do it"}),
    ));
    allowed(&ask(&r, false, "verify", "verified", ctx.clone()));
    // A later rejection of the same revision withdraws the approval.
    let mut rejected = decision("no", "intent-2");
    rejected["decision"] = json!("reject");
    rejected["target"] = json!("intent");
    rejected["reason"] = json!("changed my mind");
    ctx.push(record("d3", "owner-decision", 8, rejected));
    assert_eq!(
        code(&ask(&r, false, "verify", "verified", ctx)),
        "intent-rejected"
    );
}

#[test]
fn a_later_clean_review_cannot_erase_the_first_reviews_findings() {
    let r = repo();
    let mut first = review(&r);
    first["answers"][5]["verdict"] = json!("fail");
    first["findings"] = json!([finding("f1", "principles")]);
    let mut ctx = review_ctx(&r, first, vec![]);
    ctx.push(record("rev2", "review", 5, review(&r)));
    let mut shuffled = ctx.clone();
    shuffled.reverse();
    assert_eq!(
        code(&ask(&r, true, "review", "reviewed", shuffled)),
        "review-incomplete"
    );
    let denied = ask(&r, true, "review", "reviewed", ctx.clone());
    assert_eq!(code(&denied), "review-incomplete");
    assert!(denied["feedback"]["message"]
        .as_str()
        .unwrap()
        .contains("needs a disposition"));
    // Disposing the first review's finding satisfies the gate.
    ctx.push(disposition("f1", "rejected", "out of scope", 6));
    allowed(&ask(&r, true, "review", "reviewed", ctx));
}

#[test]
fn verify_refuses_checks_that_edit_what_an_earlier_check_relied_on() {
    let r = repo();
    // A later check keeps rewriting the tree: no result can be bound to it.
    let mut i = intent(false);
    i["criteria"][1]["check"] =
        json!({"argv":["sh","-c","date +%s%N > stamp.txt"],"timeout_ms":5000});
    let err =
        crate::run::verify(&show(&r, false, vec![record("intent-1", "intent", 1, i)])).unwrap_err();
    assert!(err.contains("change the repository"), "{err}");
    // A lone check that edits what it just checked must not pass either: the settled pass shows it failing.
    let mut lone = intent(false);
    lone["criteria"] = json!([{"id":"c1","statement":"s","check":{"argv":["sh","-c","grep -q one a.txt && printf two > a.txt"],"timeout_ms":5000}}]);
    let v = crate::run::verify(&show(
        &r,
        false,
        vec![record("intent-1", "intent", 1, lone)],
    ))
    .unwrap();
    assert_eq!(v.results[0].exit_code, Some(1));
    std::fs::write(r.join("a.txt"), "one").unwrap();
    // A later check that breaks an earlier one is reported by the second pass, as a failure.
    let mut j = intent(false);
    j["criteria"][0]["check"] = json!({"argv":["sh","-c","grep -q one a.txt"],"timeout_ms":5000});
    j["criteria"][1]["check"] = json!({"argv":["sh","-c","echo two > a.txt"],"timeout_ms":5000});
    let v = crate::run::verify(&show(&r, false, vec![record("intent-1", "intent", 1, j)])).unwrap();
    assert_eq!(v.results[0].exit_code, Some(1));
    // Output a check leaves behind once is fine: the second pass settles.
    let mut cache = intent(false);
    cache["criteria"][0]["check"] = json!({"argv":["sh","-c","touch cache.tmp"],"timeout_ms":5000});
    let v = crate::run::verify(&show(
        &r,
        false,
        vec![record("intent-1", "intent", 1, cache)],
    ))
    .unwrap();
    assert_eq!(v.tree, tree_of(&r));
}

#[test]
fn verify_ends_a_timed_out_shell_and_its_children_and_bounds_the_output() {
    let r = repo();
    let mut i = intent(false);
    i["criteria"][0]["check"] =
        json!({"argv":["sh","-c","(sleep 1; echo late > late.txt) & sleep 30"],"timeout_ms":200});
    i["criteria"][1]["check"] =
        json!({"argv":["sh","-c","head -c 3000000 /dev/zero | tr '\\0' x"],"timeout_ms":20000});
    let v = crate::run::verify(&show(&r, false, vec![record("intent-1", "intent", 1, i)])).unwrap();
    assert!(v.results[0].timed_out);
    assert!(v.results[1].output_tail.len() <= 2000 && v.results[1].exit_code == Some(0));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        !r.join("late.txt").exists(),
        "a child outlived the timed-out check"
    );
}

#[test]
fn the_core_has_no_process_file_or_git_access() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        "evaluate.rs",
        "model.rs",
        "workflow.rs",
        "protocol.rs",
        "lib.rs",
    ] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        for forbidden in [
            "std::process",
            "std::fs",
            "proof_metadata",
            "std::env",
            "crate::run",
            "crate::packet",
        ] {
            assert!(!text.contains(forbidden), "{file} imports {forbidden}");
        }
    }
}

#[test]
fn review_packet_refuses_what_verified_would_refuse() {
    let r = repo();
    let i = intent(false);
    let base = |decision_record: bool, mutate: &dyn Fn(&mut Value)| {
        let mut ctx = vec![record("intent-1", "intent", 1, i.clone())];
        if decision_record {
            ctx.push(record(
                "d-ok",
                "owner-decision",
                2,
                decision("Yes.", "intent-1"),
            ));
        }
        let mut v = verification(&r, &i);
        mutate(&mut v);
        ctx.push(record("v1", "verification", 3, v));
        crate::packet::review_packet(&show(&r, true, ctx))
    };
    assert!(base(true, &|_| {}).is_ok());
    assert!(base(false, &|_| {}).unwrap_err().contains("owner"));
    assert!(base(true, &|v| v["results"][0]["exit_code"] = json!(1))
        .unwrap_err()
        .contains("failed"));
    assert!(base(true, &|v| {
        v["results"].as_array_mut().unwrap().pop();
    })
    .unwrap_err()
    .contains("missing"));
    assert!(
        base(true, &|v| v["results"][0]["check_sha256"] = json!("x"))
            .unwrap_err()
            .contains("approved")
    );
}

#[test]
fn the_question_lists_match_the_prd_text() {
    let prd =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PRD.md"))
            .unwrap();
    let section = |id: &str| -> String {
        let start = prd.find(&format!("### {id}:")).unwrap();
        let rest = &prd[start + 4..];
        rest[..rest.find("\n### ").unwrap_or(rest.len())].to_owned()
    };
    let intent_ids: Vec<&str> = INTENT_QUESTIONS.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        intent_ids,
        [
            "use",
            "edge",
            "control",
            "terms",
            "criteria-gap",
            "principles-gap"
        ]
    );
    let le209 = section("LE-209");
    for (id, text) in INTENT_QUESTIONS {
        assert!(le209.contains(&format!("\"{text}\" ({id})")), "{id}");
    }
    assert!(le209.contains("six fixed intent-check questions"));
    let review_ids: Vec<&str> = QUESTIONS.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        review_ids,
        [
            "intent",
            "scope",
            "edge-cases",
            "checks",
            "docs",
            "principles",
            "smaller-design"
        ]
    );
    let le207 = section("LE-207");
    assert!(le207.contains(
        "seven fixed questions (intent, scope, edge-cases, checks, docs, principles, smaller-design)"
    ));
    for (id, text) in QUESTIONS {
        assert!(le207.contains(&format!("\"{text}\" ({id})")), "{id}");
    }
    let docs = QUESTIONS.iter().find(|(id, _)| *id == "docs").unwrap().1;
    assert!(docs.ends_with(
        "If the repository keeps a living PRD, name each requirement the change adds, changes or relies on, \
         and the check that proves it; a requirement that no check proves is a finding."
    ));
    assert!(le207.contains(&format!("\"{docs}\" (docs)")));
    assert_eq!(review_questions(None).len(), 6);
    assert_eq!(review_questions(Some(1)).len(), 7);
}

#[test]
fn intent_packet_carries_the_six_questions_the_principles_and_the_draft_intent() {
    let r = repo();
    let draft = vec![record("intent-1", "intent", 1, intent(false))];
    // Needs no verification: the intent is still a draft.
    let packet = crate::packet::intent_packet(&show(&r, true, draft.clone())).unwrap();
    for (id, text) in INTENT_QUESTIONS {
        assert!(packet.contains(&format!("- {id}: {text}")), "{id}");
    }
    assert!(packet.contains("\"goal\": \"g\"") && packet.contains("(intent-1)"));
    assert!(packet.contains(&r.display().to_string()));
    // The frozen principles text and its sha256 sit between the questions and the draft intent.
    let heading = format!(
        "## Design principles (sha256 {})",
        sha256_hex(PRINCIPLES.as_bytes())
    );
    let (questions, principles, draft_at) = (
        packet.find("## Questions").unwrap(),
        packet.find(&heading).unwrap(),
        packet.find("## Draft intent").unwrap(),
    );
    assert!(questions < principles && principles < draft_at);
    assert!(packet[principles..draft_at].contains(PRINCIPLES));
    // The latest intent record is the one checked.
    let mut later = draft.clone();
    later.push(record("intent-2", "intent", 2, intent(true)));
    assert!(crate::packet::intent_packet(&show(&r, true, later))
        .unwrap()
        .contains("(intent-2)"));
    assert!(crate::packet::intent_packet(&show(&r, false, draft)).is_err());
    assert!(crate::packet::intent_packet(&show(&r, true, vec![])).is_err());
}

#[test]
fn only_b_asks_for_the_intent_check_and_c_is_unchanged() {
    // C is byte-for-byte the workflow of commit 9b3e9b8, before the intent check existed.
    let c = serde_json::to_string(&workflow(false, false)).unwrap();
    assert_eq!(
        sha256_hex(c.as_bytes()),
        "20c5f85d3365d568b27ca09029886dcd844ec154262c97685fd77e138c58dd60"
    );
    let explore = |review| workflow(review, false).states[0].instructions.clone();
    assert!(explore(true).contains("intent-packet") && explore(true).contains("intent-check"));
    assert!(!explore(false).contains("intent-check"));
    // B differs from v1 B only in the explore text.
    let strip = |mut w: serde_json::Value| {
        w["states"][0]["instructions"] = json!("");
        w
    };
    let b = strip(serde_json::to_value(workflow(true, false)).unwrap());
    assert_eq!(
        sha256_hex(b.to_string().as_bytes()),
        "9785388e978bfb46fc8bd80f6fddca0637d0e689ca7bd7b783d23643729987d5"
    );
}

#[test]
fn the_frozen_repo_is_the_worktree_root_never_a_subdirectory() {
    let r = repo();
    std::fs::create_dir(r.join("sub")).unwrap();
    std::fs::write(r.join("sub/k.txt"), "k").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "sub"]);
    let sub = r.join("sub");
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Setup normalizes a subdirectory to its worktree root.
    let c = crate::packet::setup(&args(&[
        "--profile",
        "c",
        "--repo",
        &sub.display().to_string(),
    ]))
    .unwrap();
    assert_eq!(
        c["repo"],
        std::fs::canonicalize(&r).unwrap().display().to_string()
    );
    // A hand-written input naming a subdirectory is refused. Otherwise a check reading ../a.txt
    // could pass on an already-modified a.txt whose later edit the subdirectory tree would miss.
    std::fs::write(r.join("a.txt"), "dirty").unwrap();
    let ctx = verify_ctx(&sub, |_| {});
    std::fs::write(r.join("a.txt"), "dirty and different").unwrap();
    assert_eq!(
        code(&ask(&sub, false, "verify", "verified", ctx.clone())),
        "tree-unreadable"
    );
    let parsed = InitialInput::parse(&input(&sub, false)).unwrap();
    assert!(crate::run::tree_identity(&parsed)
        .unwrap_err()
        .contains("root of its Git worktree"));
    assert!(crate::run::verify(&show(&sub, false, ctx)).is_err());
    assert!(crate::packet::setup(&args(&[
        "--profile",
        "c",
        "--repo",
        &std::env::temp_dir().display().to_string()
    ]))
    .is_err());
}

#[test]
fn review_packet_includes_a_tracked_file_that_matches_an_ignore_rule() {
    let r = repo();
    std::fs::write(r.join(".gitignore"), "*.log\n").unwrap();
    git(&r, &["add", ".gitignore"]);
    git(&r, &["commit", "-qm", "ignore logs"]);
    std::fs::write(r.join("forced.log"), "forced content").unwrap();
    git(&r, &["add", "-f", "forced.log"]);
    std::fs::write(r.join("skipped.log"), "ignored content").unwrap();
    let i = intent(false);
    let ctx = vec![
        record("intent-1", "intent", 1, i.clone()),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("v1", "verification", 3, verification(&r, &i)),
    ];
    let packet = crate::packet::review_packet(&show(&r, true, ctx)).unwrap();
    assert!(packet.contains("+forced content"), "{packet}");
    assert!(!packet.contains("ignored content"));
}

#[test]
fn review_packet_includes_tracked_edits_hidden_by_index_bits() {
    let r = repo();
    std::fs::write(r.join("b.txt"), "two").unwrap();
    git(&r, &["add", "b.txt"]);
    git(&r, &["commit", "-qm", "b"]);
    git(&r, &["update-index", "--assume-unchanged", "a.txt"]);
    git(&r, &["update-index", "--skip-worktree", "b.txt"]);
    std::fs::write(r.join("a.txt"), "assumed edit").unwrap();
    std::fs::write(r.join("b.txt"), "skipped edit").unwrap();
    let i = intent(false);
    let ctx = vec![
        record("intent-1", "intent", 1, i.clone()),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("v1", "verification", 3, verification(&r, &i)),
    ];
    let packet = crate::packet::review_packet(&show(&r, true, ctx)).unwrap();
    assert!(
        packet.contains("+assumed edit") && packet.contains("+skipped edit"),
        "{packet}"
    );
    // The repository's own index keeps its bits.
    let bits = Command::new("git")
        .args(["ls-files", "-v"])
        .current_dir(&r)
        .output()
        .unwrap()
        .stdout;
    let bits = String::from_utf8(bits).unwrap();
    assert!(
        bits.contains("h a.txt") && bits.contains("S b.txt"),
        "{bits}"
    );
}

/// The initial input setup writes now: old-vs-new with these proof paths.
fn old_vs_new_input(repo: &Path, review: bool, proof_paths: &[&str]) -> Value {
    let mut v = input(repo, review);
    v["old_vs_new"] = json!({"proof_paths": proof_paths});
    v
}

fn criterion(id: &str, old: Option<Value>) -> Value {
    let mut c = json!({"id":id,"statement":"s","check":{"argv":["true"],"timeout_ms":1000}});
    if let Some(old) = old {
        c["old"] = old;
    }
    c
}

fn intent_with(criteria: Vec<Value>) -> Value {
    let mut i = intent(false);
    i["criteria"] = json!(criteria);
    i
}

fn intent_ready_with(initial_input: Value, criteria: Vec<Value>) -> Value {
    ask_with(
        initial_input,
        "explore",
        "intent-ready",
        vec![record("intent-1", "intent", 1, intent_with(criteria))],
    )
}

/// An old run as a verification record holds it.
fn old_run_value(exit_code: Option<i32>, timed_out: bool, output_tail: &str) -> Value {
    json!({"exit_code":exit_code,"timed_out":timed_out,"output_tail":output_tail})
}

/// The intent of the old-run gate tests: one criterion per expectation, in the order f, s, a, n.
fn old_intent() -> Value {
    intent_with(vec![
        criterion("f", Some(json!({"expect":"fails"}))),
        criterion("s", Some(json!({"expect":"same"}))),
        criterion("a", Some(json!({"expect":"absent"}))),
        criterion("n", Some(json!({"expect":"none","reason":"docs only"}))),
    ])
}

/// Old results in the order of `old_intent()` that every gate test accepts; a test replaces one.
fn good_olds() -> [Value; 4] {
    [
        json!({"expect":"fails","runs":[old_run_value(Some(1),false,"boom"),old_run_value(Some(1),false,"boom")]}),
        json!({"expect":"same","runs":[old_run_value(Some(0),false,"")]}),
        json!({"expect":"absent","runs":[old_run_value(Some(127),false,"")]}),
        json!({"expect":"none","runs":[]}),
    ]
}

/// The approved `old_intent()` with a verification whose old results are `olds`, asked by `verified`.
fn old_verified_ctx(r: &Path, olds: [Value; 4]) -> Vec<ContextRecord> {
    let i = old_intent();
    let mut v = verification(r, &i);
    for (result, old) in v["results"].as_array_mut().unwrap().iter_mut().zip(olds) {
        result["old"] = old;
    }
    vec![
        record("intent-1", "intent", 1, i),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("v1", "verification", 3, v),
    ]
}

fn old_verified(r: &Path, olds: [Value; 4]) -> Value {
    ask_with(
        old_vs_new_input(r, false, &[]),
        "verify",
        "verified",
        old_verified_ctx(r, olds),
    )
}

#[test]
fn old_vs_new_intent_needs_old_on_every_criterion_and_a_reason_for_none() {
    let r = repo();
    let profile = old_vs_new_input(&r, false, &[]);
    // Every criterion needs old, even with no proof paths.
    assert_eq!(
        code(&intent_ready_with(
            profile.clone(),
            vec![criterion("c1", None)]
        )),
        "intent-invalid"
    );
    assert_eq!(
        code(&intent_ready_with(
            profile.clone(),
            vec![criterion("c1", Some(json!({"expect":"maybe"})))]
        )),
        "intent-invalid"
    );
    // none needs a nonblank reason.
    for old in [
        json!({"expect":"none"}),
        json!({"expect":"none","reason":"  "}),
    ] {
        assert_eq!(
            code(&intent_ready_with(
                profile.clone(),
                vec![criterion("c1", Some(old))]
            )),
            "intent-invalid"
        );
    }
    // A profile made by lean-change 0.31.0 refuses old.
    assert_eq!(
        code(&intent_ready_with(
            input(&r, false),
            vec![criterion("c1", Some(json!({"expect":"fails"})))]
        )),
        "intent-invalid"
    );
    // Every criterion may be none, with reasons; a 0.31.0 profile needs no old.
    allowed(&intent_ready_with(
        profile,
        vec![
            criterion("c1", Some(json!({"expect":"none","reason":"docs only"}))),
            criterion("c2", Some(json!({"expect":"none","reason":"docs only"}))),
        ],
    ));
    allowed(&intent_ready_with(
        input(&r, false),
        vec![criterion("c1", None)],
    ));
}

#[test]
fn fails_needs_two_real_failures_on_the_old_tree() {
    let r = repo();
    allowed(&old_verified(&r, good_olds()));
    let with_f = |old: Value| {
        let mut olds = good_olds();
        olds[0] = old;
        old_verified(&r, olds)
    };
    let one = with_f(json!({"expect":"fails","runs":[old_run_value(Some(1),false,"boom")]}));
    assert_eq!(code(&one), "verification-incomplete");
    assert!(problems(&one).contains("\"criterion\":\"f\""));
    assert!(problems(&one).contains("two are needed"));
    let passes = with_f(
        json!({"expect":"fails","runs":[old_run_value(Some(0),false,""),old_run_value(Some(0),false,"")]}),
    );
    assert!(problems(&passes).contains("real failure"));
    let absent_command = with_f(
        json!({"expect":"fails","runs":[old_run_value(Some(127),false,""),old_run_value(Some(127),false,"")]}),
    );
    assert!(problems(&absent_command).contains("real failure"));
    let timed_out = with_f(
        json!({"expect":"fails","runs":[old_run_value(None,true,""),old_run_value(None,true,"")]}),
    );
    assert!(problems(&timed_out).contains("timed out"));
    let flaky = with_f(
        json!({"expect":"fails","runs":[old_run_value(Some(1),false,"a"),old_run_value(Some(2),false,"b")]}),
    );
    assert!(problems(&flaky).contains("disagree"));
    // The judgment is the intent's expectation: a record relabelled none, or with no old, is refused.
    let relabelled = with_f(json!({"expect":"none","runs":[]}));
    assert!(problems(&relabelled).contains("names expect none instead of fails"));
    assert!(problems(&relabelled).contains("missing"));
    let no_old = with_f(json!(null));
    assert!(problems(&no_old).contains("missing"));
    // A run with no exit code is a command that could not start or a signal; the problem says so.
    let signalled = with_f(
        json!({"expect":"fails","runs":[old_run_value(None,false,"Killed"),old_run_value(None,false,"Killed")]}),
    );
    assert!(problems(&signalled).contains("had no exit code"));
}

#[test]
fn same_and_absent_are_judged_by_their_one_old_run_and_none_is_not_judged() {
    let r = repo();
    let with_s = |old: Value| {
        let mut olds = good_olds();
        olds[1] = old;
        old_verified(&r, olds)
    };
    let differs = with_s(json!({"expect":"same","runs":[old_run_value(Some(0),false,"old\n")]}));
    assert!(problems(&differs).contains("differs"));
    let failed = with_s(json!({"expect":"same","runs":[old_run_value(Some(1),false,"")]}));
    assert!(problems(&failed).contains("a refactor needs exit 0"));
    let with_a = |old: Value| {
        let mut olds = good_olds();
        olds[2] = old;
        old_verified(&r, olds)
    };
    // Any old result but a pass, including a command that cannot start.
    allowed(&with_a(
        json!({"expect":"absent","runs":[old_run_value(Some(1),false,"")]}),
    ));
    allowed(&with_a(
        json!({"expect":"absent","runs":[old_run_value(None,false,"could not start")]}),
    ));
    let passes = with_a(json!({"expect":"absent","runs":[old_run_value(Some(0),false,"")]}));
    assert!(problems(&passes).contains("present on the old tree"));
    let no_run = with_a(json!({"expect":"absent","runs":[]}));
    assert!(problems(&no_run).contains("missing"));
    // none is not run on the old tree, so a record's runs for it are not judged.
    let mut olds = good_olds();
    olds[3] = json!({"expect":"none","runs":[old_run_value(Some(0),false,"")]});
    allowed(&old_verified(&r, olds));
}

#[test]
fn old_record_must_name_the_intents_expectation_and_a_none_criterion_needs_its_old() {
    let r = repo();
    let with_f = |old: Value| {
        let mut olds = good_olds();
        olds[0] = old;
        old_verified(&r, olds)
    };
    // Only old.expect is changed, so the two genuine failing runs stay: the record is refused for its name.
    let as_absent = with_f(
        json!({"expect":"absent","runs":[old_run_value(Some(1),false,"boom"),old_run_value(Some(1),false,"boom")]}),
    );
    assert_eq!(code(&as_absent), "verification-incomplete");
    assert!(problems(&as_absent).contains("\"criterion\":\"f\""));
    assert!(problems(&as_absent).contains("names expect absent instead of fails"));
    // The runs are judged by the intent's expectation as well: exit 127 is no real failure for fails.
    let absent_runs = with_f(
        json!({"expect":"absent","runs":[old_run_value(Some(127),false,""),old_run_value(Some(127),false,"")]}),
    );
    assert!(problems(&absent_runs).contains("real failure"));
    // A none criterion's result with its old removed is refused for that criterion.
    let mut ctx = old_verified_ctx(&r, good_olds());
    ctx[2].data["results"][3]
        .as_object_mut()
        .unwrap()
        .remove("old");
    let no_old = ask_with(old_vs_new_input(&r, false, &[]), "verify", "verified", ctx);
    assert_eq!(code(&no_old), "verification-incomplete");
    assert!(problems(&no_old).contains("\"criterion\":\"n\""));
    assert!(problems(&no_old).contains("the record has no old runs"));
    let mut none_named_fails = good_olds();
    none_named_fails[3] = json!({"expect":"fails","runs":[]});
    assert!(problems(&old_verified(&r, none_named_fails))
        .contains("names expect fails instead of none"));
    // The genuine record is accepted.
    allowed(&old_verified(&r, good_olds()));
}

#[test]
fn setup_writes_old_vs_new_proof_paths_in_order_and_refuses_bad_ones() {
    let r = repo();
    let repo_arg = r.display().to_string();
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let with_paths = crate::packet::setup(&args(&[
        "--profile",
        "c",
        "--repo",
        &repo_arg,
        "--proof-path",
        "journeys",
        "--proof-path",
        ":(glob)**/*_test.sh",
    ]))
    .unwrap();
    assert_eq!(with_paths["profile_version"], "lean-c-2");
    assert_eq!(
        with_paths["old_vs_new"],
        json!({"proof_paths":["journeys",":(glob)**/*_test.sh"]})
    );
    InitialInput::parse(&with_paths).unwrap();
    let none = crate::packet::setup(&args(&["--profile", "c", "--repo", &repo_arg])).unwrap();
    assert_eq!(none["old_vs_new"], json!({"proof_paths":[]}));
    // Git's pathspec magic does not hide a `..` or an absolute path from the check.
    for bad in [
        "",
        " ",
        "/absolute",
        "a/../b",
        "..",
        ":",
        ":(glob)",
        ":!",
        ":(glob)../x",
        ":(top)../x",
        ":(exclude)../x",
        ":(literal)..",
        ":^../x",
        "::../x",
        ":!:../x",
        ":^:../x",
        ":!/etc",
        ":!:/etc",
        ":(top)/etc",
        ":(glob)/abs",
        ":/etc",
    ] {
        assert!(
            crate::packet::setup(&args(&[
                "--profile",
                "c",
                "--repo",
                &repo_arg,
                "--proof-path",
                bad
            ]))
            .is_err(),
            "{bad:?} is refused"
        );
    }
    for good in [":/", ":(top)"] {
        assert!(
            crate::packet::setup(&args(&[
                "--profile",
                "c",
                "--repo",
                &repo_arg,
                "--proof-path",
                good
            ]))
            .is_ok(),
            "{good:?} names the whole repository and is allowed"
        );
    }
}

#[test]
fn verify_runs_the_old_checks_on_the_base_with_the_proof_paths_laid_over() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/probe.sh"), "echo base-probe\n").unwrap();
    std::fs::write(r.join("journeys/gone.sh"), "echo gone\n").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "proof at base"]);
    // The product file, a proof file and a new proof file move on; a tracked proof file is deleted.
    std::fs::write(r.join("a.txt"), "two").unwrap();
    std::fs::write(r.join("journeys/probe.sh"), "echo current-probe\n").unwrap();
    std::fs::write(r.join("journeys/new.sh"), "echo new-proof\n").unwrap();
    std::fs::remove_file(r.join("journeys/gone.sh")).unwrap();
    let mut i = intent_with(vec![criterion("probe", Some(json!({"expect":"fails"})))]);
    i["criteria"][0]["check"] = json!({"argv":["sh","-c","cat a.txt; echo; ls journeys; sh journeys/probe.sh; sh journeys/new.sh; test \"$(cat a.txt)\" = two"],"timeout_ms":5000});
    let v = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &["journeys"]),
        vec![record("intent-1", "intent", 1, i)],
    ))
    .unwrap();
    assert_eq!(
        v.results[0].output_tail,
        "two\nnew.sh\nprobe.sh\ncurrent-probe\nnew-proof\n"
    );
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(runs.len(), 2);
    for run in runs {
        assert_eq!(run.exit_code, Some(1));
        assert_eq!(
            run.output_tail,
            "one\nnew.sh\nprobe.sh\ncurrent-probe\nnew-proof\n"
        );
    }
    // The old tree is removed after the runs.
    let prefix = format!("lean-change-old-{}-", std::process::id());
    let leftover: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .collect();
    assert!(leftover.is_empty(), "old trees left behind: {leftover:?}");
}

#[test]
fn verify_lays_over_a_proof_directory_the_base_does_not_have() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/new.sh"), "echo new-proof\n").unwrap();
    let mut i = intent_with(vec![criterion("new", Some(json!({"expect":"absent"})))]);
    i["criteria"][0]["check"] = json!({"argv":["sh","journeys/new.sh"],"timeout_ms":5000});
    let v = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &["journeys"]),
        vec![record("intent-1", "intent", 1, i)],
    ))
    .unwrap();
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, Some(0));
    assert_eq!(runs[0].output_tail, "new-proof\n");
}

#[test]
fn verify_lays_over_a_file_that_replaces_a_base_directory() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys/data")).unwrap();
    std::fs::write(r.join("journeys/data/a.txt"), "base-data\n").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "data directory at base"]);
    // The change replaces the tracked directory by a staged file of the same name.
    git(&r, &["rm", "-rq", "journeys/data"]);
    // git removes the now-empty journeys directory too.
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/data"), "new-data\n").unwrap();
    git(&r, &["add", "journeys/data"]);
    let mut i = intent_with(vec![criterion("data", Some(json!({"expect":"same"})))]);
    i["criteria"][0]["check"] = json!({"argv":["cat","journeys/data"],"timeout_ms":5000});
    let v = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &["journeys"]),
        vec![record("intent-1", "intent", 1, i)],
    ))
    .unwrap();
    assert_eq!(v.results[0].output_tail, "new-data\n");
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, Some(0));
    assert_eq!(runs[0].output_tail, "new-data\n");
}

#[test]
fn verify_lays_over_a_file_that_replaces_a_nested_base_directory() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys/data/sub/deep")).unwrap();
    std::fs::write(r.join("journeys/data/sub/deep/a.txt"), "base-data\n").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "nested data directory at base"]);
    // The change replaces the tracked directory by a staged file of the same name. The directories below it
    // are left empty and go with it.
    git(&r, &["rm", "-rq", "journeys/data"]);
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/data"), "new-data\n").unwrap();
    git(&r, &["add", "journeys/data"]);
    let mut i = intent_with(vec![criterion("data", Some(json!({"expect":"same"})))]);
    i["criteria"][0]["check"] = json!({"argv":["cat","journeys/data"],"timeout_ms":5000});
    let v = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &["journeys"]),
        vec![record("intent-1", "intent", 1, i)],
    ))
    .unwrap();
    assert_eq!(v.results[0].output_tail, "new-data\n");
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, Some(0));
    assert_eq!(runs[0].output_tail, "new-data\n");
}

#[test]
fn verify_refuses_a_file_left_in_a_directory_that_a_current_file_replaces() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys/data")).unwrap();
    std::fs::write(r.join("journeys/data/a.txt"), "base-data\n").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "data directory at base"]);
    // The glob matches only the top level of journeys, so journeys/data/a.txt is not a proof file and stays
    // in the old tree. The current file journeys/data cannot replace a directory that still holds it.
    git(&r, &["rm", "-rq", "journeys/data"]);
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/data"), "new-data\n").unwrap();
    git(&r, &["add", "journeys/data"]);
    let mut i = intent_with(vec![criterion("data", Some(json!({"expect":"same"})))]);
    i["criteria"][0]["check"] = json!({"argv":["cat","journeys/data"],"timeout_ms":5000});
    let Err(error) = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &[":(glob)journeys/*"]),
        vec![record("intent-1", "intent", 1, i)],
    )) else {
        panic!("verify lays over a file that still has a file inside its directory");
    };
    assert!(error.contains("a file remains"), "{error}");
    assert!(error.contains("journeys/data/a.txt"), "{error}");
}

#[test]
fn verify_lays_over_a_directory_that_replaces_a_base_file() {
    let r = repo();
    std::fs::create_dir_all(r.join("journeys")).unwrap();
    std::fs::write(r.join("journeys/x"), "base-x\n").unwrap();
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "file at base"]);
    // The change replaces the tracked file by a staged directory of the same name.
    git(&r, &["rm", "-q", "journeys/x"]);
    std::fs::create_dir_all(r.join("journeys/x")).unwrap();
    std::fs::write(r.join("journeys/x/y.sh"), "echo new-x\n").unwrap();
    git(&r, &["add", "journeys/x/y.sh"]);
    let mut i = intent_with(vec![criterion("x", Some(json!({"expect":"same"})))]);
    i["criteria"][0]["check"] = json!({"argv":["sh","journeys/x/y.sh"],"timeout_ms":5000});
    let v = crate::run::verify(&show_with(
        old_vs_new_input(&r, false, &["journeys"]),
        vec![record("intent-1", "intent", 1, i)],
    ))
    .unwrap();
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, Some(0));
    assert_eq!(runs[0].output_tail, "new-x\n");
}

#[test]
fn old_vs_new_describe_tells_the_driver_about_old_and_the_proof_paths() {
    for review in [true, false] {
        let explore = workflow(review, true).states[0].instructions.clone();
        for words in ["`old`", "--proof-path", "bare base", "escape hatch"] {
            assert!(explore.contains(words), "{words} missing: {explore}");
        }
        assert!(!workflow(review, false).states[0]
            .instructions
            .contains("--proof-path"));
    }
}

#[test]
fn review_packet_lists_each_criterions_old_runs_and_their_tails() {
    let r = repo();
    let ctx = old_verified_ctx(&r, good_olds());
    let packet =
        crate::packet::review_packet(&show_with(old_vs_new_input(&r, true, &["journeys"]), ctx))
            .unwrap();
    assert!(packet.contains("- f: old (expect fails): exit Some(1), Some(1)\n"));
    assert!(packet.contains("  old output tail: \"boom\"\n"));
    assert!(packet.contains("- s: old (expect same): exit Some(0)\n"));
    assert!(!packet.contains("- n: old"));
}

// ---- findings_schema 1: severity, notes and demotion ----

/// A review in the shared finding fields: all seven answers.
fn review_v1(repo: &Path) -> Value {
    let mut v = review(repo);
    v["findings_schema"] = json!(1);
    v["answers"] = QUESTIONS
        .iter()
        .map(|(q, _)| json!({"question":q,"verdict":"pass","note":"ok"}))
        .collect();
    v
}

/// A blocking finding with every field, a blocking one missing its consequence, and a note.
fn complete(id: &str, question: &str) -> Value {
    json!({"id":id,"severity":"blocking","question":question,"text":"t","obligation":"o","evidence":"e","consequence":"c","component":"packet"})
}

fn missing_consequence(id: &str, question: &str) -> Value {
    let mut f = complete(id, question);
    f.as_object_mut().unwrap().remove("consequence");
    f
}

fn note(id: &str, question: &str) -> Value {
    json!({"id":id,"severity":"note","question":question,"text":"a wish"})
}

fn fail_question(review: &mut Value, question: &str) {
    let answer = review["answers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["question"] == question)
        .unwrap();
    answer["verdict"] = json!("fail");
}

fn blocking_finding(overrides: &[(&str, Value)]) -> Finding {
    let mut f = complete("f1", "scope");
    for (key, value) in overrides {
        f[*key] = value.clone();
    }
    serde_json::from_value(f).unwrap()
}

#[test]
fn effective_is_blocking_only_with_severity_and_all_three_fields() {
    let nothing = |f: &Finding| f.effective(Some(1));
    assert_eq!(nothing(&blocking_finding(&[])), Effective::Blocking);
    let demoted = |reason: &str| Effective::Note {
        demoted_reason: Some(format!("demoted: missing {reason}")),
    };
    assert_eq!(
        nothing(&blocking_finding(&[("obligation", json!(" "))])),
        demoted("obligation")
    );
    assert_eq!(
        nothing(&blocking_finding(&[("evidence", json!(""))])),
        demoted("evidence")
    );
    assert_eq!(
        nothing(&blocking_finding(&[("consequence", json!("\n\t"))])),
        demoted("consequence")
    );
    assert_eq!(
        nothing(&blocking_finding(&[
            ("obligation", Value::Null),
            ("evidence", json!(" ")),
            ("consequence", json!(""))
        ])),
        demoted("obligation, evidence, consequence")
    );
    // The component is not part of the bar.
    assert_eq!(
        nothing(&blocking_finding(&[("component", json!(" "))])),
        Effective::Blocking
    );
    // A note is never demoted, whatever it has.
    assert_eq!(
        nothing(&blocking_finding(&[("severity", json!("note"))])),
        Effective::Note {
            demoted_reason: None
        }
    );
    // Without findings_schema every finding is blocking, as before.
    assert_eq!(
        blocking_finding(&[("consequence", json!(""))]).effective(None),
        Effective::Blocking
    );
}

#[test]
fn v1_blocking_finding_missing_a_field_is_a_note_that_needs_no_disposition() {
    let r = repo();
    let mut thin = review_v1(&r);
    fail_question(&mut thin, "scope");
    thin["findings"] = json!([missing_consequence("f1", "scope")]);
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, thin.clone(), vec![]),
    ));
    // A complete blocking finding beside it still needs one, and only it does.
    let mut mixed = thin.clone();
    mixed["findings"] = json!([
        missing_consequence("f1", "scope"),
        complete("f2", "scope"),
        note("n1", "docs")
    ]);
    let message = review_denial(&r, mixed.clone(), vec![]);
    assert!(
        message.contains("finding f2 needs a disposition"),
        "{message}"
    );
    // Disposing f2 alone is enough: the demoted finding and the note need none.
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(
            &r,
            mixed,
            vec![disposition("f2", "rejected", "not in scope", 4)],
        ),
    ));
}

#[test]
fn v1_note_with_no_disposition_lets_reviewed_pass() {
    let r = repo();
    let mut notes = review_v1(&r);
    fail_question(&mut notes, "docs");
    notes["findings"] = json!([note("n1", "docs")]);
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, notes, vec![]),
    ));
    // A failed answer still needs a finding of some severity.
    let mut none = review_v1(&r);
    fail_question(&mut none, "docs");
    assert!(review_denial(&r, none, vec![]).contains("failed but has no finding"));
}

#[test]
fn v1_review_after_a_tree_change_with_only_notes_is_accepted_once_verified() {
    let r = repo();
    let mut notes = review_v1(&r);
    fail_question(&mut notes, "docs");
    notes["findings"] = json!([note("n1", "docs"), missing_consequence("f2", "scope")]);
    let ctx = review_ctx(&r, notes, vec![]);
    std::fs::write(r.join("a.txt"), "edited after the review").unwrap();
    // The changed tree still goes through verification first.
    assert_eq!(
        code(&ask(&r, true, "review", "reviewed", ctx.clone())),
        "verification-incomplete"
    );
    let mut verified = ctx;
    verified.push(record(
        "v2",
        "verification",
        5,
        verification(&r, &intent(false)),
    ));
    allowed(&ask(&r, true, "review", "reviewed", verified));
    // A blocking finding that was rejected is still an unreviewed change.
    let mut blocking = review_v1(&r);
    fail_question(&mut blocking, "scope");
    blocking["findings"] = json!([complete("f1", "scope")]);
    let mut ctx = review_ctx(&r, blocking, vec![disposition("f1", "rejected", "no", 4)]);
    std::fs::write(r.join("a.txt"), "edited again").unwrap();
    ctx.push(record(
        "v3",
        "verification",
        6,
        verification(&r, &intent(false)),
    ));
    let denied = ask(&r, true, "review", "reviewed", ctx);
    assert_eq!(code(&denied), "review-incomplete");
    assert!(denied["feedback"]["message"]
        .as_str()
        .unwrap()
        .contains("tree changed after the review"));
}

#[test]
fn v1_form_refuses_what_the_contract_rejects() {
    let r = repo();
    let with = |findings: Value| {
        let mut v = review_v1(&r);
        fail_question(&mut v, "scope");
        v["findings"] = findings;
        v
    };
    let no_severity = {
        let mut f = complete("f1", "scope");
        f.as_object_mut().unwrap().remove("severity");
        f
    };
    assert!(review_denial(&r, with(json!([no_severity])), vec![]).contains("severity"));
    let mut blank_text = note("n1", "scope");
    blank_text["text"] = json!(" ");
    assert!(review_denial(&r, with(json!([blank_text])), vec![]).contains("needs text"));
    let mut blank_id = note("n1", "scope");
    blank_id["id"] = json!(" ");
    assert!(review_denial(&r, with(json!([blank_id])), vec![]).contains("nonblank id"));
    assert!(review_denial(
        &r,
        with(json!([note("n1", "scope"), note("n1", "docs")])),
        vec![]
    )
    .contains("unique"));
    assert!(
        review_denial(&r, with(json!([note("n1", "nonsense")])), vec![]).contains("known question")
    );
    // An unknown severity fails the parse.
    let mut odd = note("n1", "scope");
    odd["severity"] = json!("major");
    assert!(review_denial(&r, with(json!([odd])), vec![]).contains("invalid"));
    // Only schema 1 is accepted.
    let mut two = review_v1(&r);
    two["findings_schema"] = json!(2);
    assert!(review_denial(&r, two, vec![]).contains("findings_schema must be 1"));
    // A question stays required on a note.
    let mut no_question = note("n1", "scope");
    no_question.as_object_mut().unwrap().remove("question");
    assert!(review_denial(&r, with(json!([no_question])), vec![]).contains("invalid"));
}

#[test]
fn a_v1_review_answers_all_seven_questions_and_a_v0_review_the_original_six() {
    let r = repo();
    let mut six = review_v1(&r);
    six["answers"].as_array_mut().unwrap().pop();
    assert!(review_denial(&r, six, vec![]).contains("smaller-design is unanswered"));
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, review_v1(&r), vec![]),
    ));
    // A record without findings_schema is checked against the six it was written for, and gates as before.
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(&r, review(&r), vec![]),
    ));
    let mut seven = review(&r);
    seven["answers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"question":"smaller-design","verdict":"pass","note":"ok"}));
    assert!(
        review_denial(&r, seven, vec![]).contains("unknown or repeated question smaller-design")
    );
}

#[test]
fn a_v0_record_gates_exactly_as_before() {
    let r = repo();
    // Every v0 finding is blocking and needs a disposition, whatever else it says.
    let mut failed = review(&r);
    failed["answers"][5]["verdict"] = json!("fail");
    failed["findings"] = json!([finding("f1", "principles")]);
    assert!(review_denial(&r, failed.clone(), vec![]).contains("needs a disposition"));
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        review_ctx(
            &r,
            failed.clone(),
            vec![disposition("f1", "rejected", "why", 4)],
        ),
    ));
    // The shared fields need the schema: a v0 record that carries them is refused, not silently read as blocking.
    for field in ["severity", "obligation", "component"] {
        let mut odd = failed.clone();
        odd["findings"][0][field] = json!("note");
        assert!(
            review_denial(&r, odd, vec![]).contains("need findings_schema: 1"),
            "{field}"
        );
    }
    // A v0 finding still needs its evidence and consequence.
    let mut thin = failed.clone();
    thin["findings"][0]
        .as_object_mut()
        .unwrap()
        .remove("consequence");
    assert!(review_denial(&r, thin, vec![]).contains("evidence and consequence"));
    // And after a tree change a v0 review still needs a fixed finding.
    let mut ctx = review_ctx(&r, failed, vec![disposition("f1", "rejected", "no", 4)]);
    std::fs::write(r.join("a.txt"), "edited").unwrap();
    ctx.push(record(
        "v2",
        "verification",
        5,
        verification(&r, &intent(false)),
    ));
    let denied = ask(&r, true, "review", "reviewed", ctx);
    assert_eq!(code(&denied), "review-incomplete");
    // A clean v0 review (no findings) followed by an edit is still refused, as in v0.39.0: N7 is v1 only.
    let r = repo();
    let mut ctx = review_ctx(&r, review(&r), vec![]);
    std::fs::write(r.join("a.txt"), "edited").unwrap();
    ctx.push(record(
        "v2",
        "verification",
        5,
        verification(&r, &intent(false)),
    ));
    let denied = ask(&r, true, "review", "reviewed", ctx);
    assert_eq!(code(&denied), "review-incomplete");
}

#[test]
fn the_intent_check_uses_the_same_rules() {
    let r = repo();
    let mut v1 = check_value();
    v1["findings_schema"] = json!(1);
    v1["answers"][0]["verdict"] = json!("fail");
    // A note needs no disposition.
    v1["findings"] = json!([note("n1", "use")]);
    allowed(&check_answer(&r, check_ctx(v1.clone(), vec![])));
    // A blocking finding missing a field is a note too.
    v1["findings"] = json!([missing_consequence("f1", "use")]);
    allowed(&check_answer(&r, check_ctx(v1.clone(), vec![])));
    // A complete one needs a disposition.
    v1["findings"] = json!([complete("f1", "use")]);
    assert!(check_denial(&r, check_ctx(v1.clone(), vec![])).contains("needs a disposition"));
    allowed(&check_answer(
        &r,
        check_ctx(
            v1.clone(),
            vec![check_disposition("f1", "rejected", "the goal is clear", 3)],
        ),
    ));
    // The intent check keeps its six questions.
    assert_eq!(INTENT_QUESTIONS.len(), 6);
    v1["findings_schema"] = json!(2);
    assert!(check_denial(&r, check_ctx(v1, vec![])).contains("findings_schema must be 1"));
}

// ---- the contract's optional envelope fields: head and brief_sha256 ----

const ENVELOPE: [(&str, &str); 2] = [
    (
        "head",
        "77ece9d68e03b892ae95b4cfbf17828239dc0f75acf88dd65ff6460d1fa9adcf",
    ),
    (
        "brief_sha256",
        "1584e670abca44ac8ca752756a5abd90908970854f717512df80785fc87cdc56",
    ),
];

#[test]
fn a_v1_review_with_head_and_brief_sha256_gates_as_without_them() {
    let r = repo();
    let mut plain = review_v1(&r);
    plain["findings"] = json!([complete("f1", "scope")]);
    fail_question(&mut plain, "scope");
    let mut with = plain.clone();
    for (field, value) in ENVELOPE {
        with[field] = json!(value);
    }
    let ctx = |v: &Value, extra: Vec<ContextRecord>| review_ctx(&r, v.clone(), extra);
    // An undisposed blocking finding is refused the same way, with or without the fields.
    assert_eq!(
        review_denial(&r, with.clone(), vec![]),
        review_denial(&r, plain.clone(), vec![])
    );
    // Disposed, both are allowed.
    let disposed = || vec![disposition("f1", "rejected", "why", 5)];
    allowed(&ask(
        &r,
        true,
        "review",
        "reviewed",
        ctx(&plain, disposed()),
    ));
    allowed(&ask(&r, true, "review", "reviewed", ctx(&with, disposed())));
    // Neither field is compared with anything: a head that is not the tree gates the same.
    let mut odd = with.clone();
    odd["head"] = json!("not the tree");
    odd["brief_sha256"] = json!("");
    allowed(&ask(&r, true, "review", "reviewed", ctx(&odd, disposed())));
    // Each field alone is accepted too.
    for (field, value) in ENVELOPE {
        let mut one = plain.clone();
        one[field] = json!(value);
        allowed(&ask(&r, true, "review", "reviewed", ctx(&one, disposed())));
    }
    // A clean v1 review with the fields is allowed.
    let mut clean = review_v1(&r);
    for (field, value) in ENVELOPE {
        clean[field] = json!(value);
    }
    allowed(&ask(&r, true, "review", "reviewed", ctx(&clean, vec![])));
}

#[test]
fn a_v1_intent_check_with_head_and_brief_sha256_gates_as_without_them() {
    let r = repo();
    let mut plain = failed_check();
    plain["findings_schema"] = json!(1);
    plain["findings"] = json!([complete("f1", "use")]);
    let mut with = plain.clone();
    for (field, value) in ENVELOPE {
        with[field] = json!(value);
    }
    assert_eq!(
        check_denial(&r, check_ctx(with.clone(), vec![])),
        check_denial(&r, check_ctx(plain.clone(), vec![]))
    );
    let disposed = || vec![check_disposition("f1", "rejected", "the goal is clear", 3)];
    allowed(&check_answer(&r, check_ctx(plain, disposed())));
    allowed(&check_answer(&r, check_ctx(with.clone(), disposed())));
    // Not compared with anything.
    with["head"] = json!("");
    with["brief_sha256"] = json!("not a sha");
    allowed(&check_answer(&r, check_ctx(with, disposed())));
}

#[test]
fn a_v0_record_with_head_or_brief_sha256_is_refused() {
    let r = repo();
    for (field, value) in ENVELOPE {
        let mut check = check_value();
        check[field] = json!(value);
        assert!(
            check_denial(&r, check_ctx(check, vec![])).contains("need findings_schema: 1"),
            "intent-check {field}"
        );
        let mut v0 = review(&r);
        v0[field] = json!(value);
        assert!(
            review_denial(&r, v0, vec![]).contains("need findings_schema: 1"),
            "review {field}"
        );
    }
}

/// A reviewer's return as Run S saved it (findings contract v1, with `head` and `brief_sha256`), appended unedited as
/// an intent-check. Lean-change refused it before the two fields were accepted.
#[test]
fn a_saved_run_s_return_is_accepted_as_an_intent_check() {
    let bytes = include_bytes!("../testdata/run-s-intent-check-round-1.json");
    let saved: Value = serde_json::from_slice(bytes).unwrap();
    assert!(saved["head"].is_string() && saved["brief_sha256"].is_string());
    let r = repo();
    let ctx = |extra| check_ctx(saved.clone(), extra);
    // It parses: the only thing left to ask is the disposition of its three blocking findings.
    assert!(check_denial(&r, ctx(vec![])).contains("needs a disposition"));
    let disposed: Vec<ContextRecord> = ["f1", "f2", "f3"]
        .iter()
        .zip(3..)
        .map(|(id, seq)| check_disposition(id, "rejected", "the intent settles it", seq))
        .collect();
    allowed(&check_answer(&r, ctx(disposed)));
}

#[test]
fn both_packets_carry_the_bar_and_the_v1_example() {
    let r = repo();
    let i = intent(false);
    let mut ctx = vec![
        record("intent-1", "intent", 1, i.clone()),
        record("d-ok", "owner-decision", 2, decision("Yes.", "intent-1")),
    ];
    ctx.push(record("v1", "verification", 3, verification(&r, &i)));
    let review_packet = crate::packet::review_packet(&show(&r, true, ctx)).unwrap();
    let draft = vec![record("intent-1", "intent", 1, intent(false))];
    let intent_packet = crate::packet::intent_packet(&show(&r, true, draft)).unwrap();
    for packet in [&review_packet, &intent_packet] {
        for needle in [
            "\"findings_schema\": 1",
            "\"severity\": \"blocking|note\"",
            "\"obligation\"",
            "\"component\"",
            "data loss, corruption or a broken promised outcome always blocks",
            "ordinary single-user use",
            "names its component",
        ] {
            assert!(packet.contains(needle), "{needle}");
        }
    }
    assert!(review_packet.contains(
        "If the intent and the checks settle a question, pass it. Do not invent. Pass every question if nothing material remains."
    ));
    assert!(review_packet
        .contains("- smaller-design: Is there a materially smaller design that meets the intent"));
    assert!(!intent_packet.contains("smaller-design"));
}

// ---- the shared conformance fixture ----

/// The fixture is shared with the coordinator repo, byte for byte. To change it, change both copies (coordinator and
/// loop-engine) and both pins in one change set.
const FIXTURE_SHA256: &str = "ffb5f849d93f77fcf723bf396832ec47d23829ef077f44cf3497830b007183f1";

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/findings-conformance-v1.json"),
    )
    .unwrap()
}

#[test]
fn the_conformance_fixture_is_the_pinned_copy() {
    assert_eq!(sha256_hex(&fixture_bytes()), FIXTURE_SHA256);
}

/// Runs a case's findings through the per-finding checks and returns the contract's reject code, if any. The contract
/// order is blank-id, duplicate-id, blank-text, bad-severity; a severity that is not blocking or note is reported at
/// the bad-severity step, after the other checks, not at parse time.
fn reject_code(findings: &[Value]) -> Option<&'static str> {
    let parsed: Vec<Finding> = findings
        .iter()
        .map(|f| {
            let mut f = f.clone();
            if !matches!(f["severity"].as_str(), Some("blocking" | "note")) {
                f.as_object_mut().unwrap().remove("severity");
            }
            // Lean-change keeps a known question on every finding; the shared fixture has none on most.
            if f.get("question").is_none() {
                f["question"] = json!("scope");
            }
            serde_json::from_value(f).unwrap()
        })
        .collect();
    lean_change_provider::evaluate::check_findings(&parsed)
        .err()
        .map(|reject| reject.code())
}

#[test]
fn findings_conformance_v1() {
    let fixture: Value = serde_json::from_slice(&fixture_bytes()).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 26);
    let mut ran = 0;
    for case in cases {
        let (name, expected) = (case["name"].as_str().unwrap(), &case["expected"]);
        // Skipped: cases that test the return's own envelope (bad-json, bad-schema-version, blank-reviewer,
        // blank-head, bad-shape), which lean-change does not parse, and the v0 case, because lean-change reads v0.
        if expected["envelope"] == true || expected["v0_reader"] == true {
            continue;
        }
        // Not parsed as a Review (the envelope differs): only the findings are run.
        let findings = case["input"]["findings"].as_array().unwrap();
        let rejected = reject_code(findings);
        if expected["accepted"] == false {
            assert_eq!(rejected, expected["reject"].as_str(), "{name}");
            ran += 1;
            continue;
        }
        assert_eq!(rejected, None, "{name}");
        let want = expected["findings"].as_array().unwrap();
        assert_eq!(findings.len(), want.len(), "{name}");
        let mut blocking = false;
        for (given, want) in findings.iter().zip(want) {
            let mut given = given.clone();
            given["question"] = json!("scope");
            let finding: Finding = serde_json::from_value(given).unwrap();
            let got = finding.effective(Some(1));
            match want["effective"].as_str().unwrap() {
                "blocking" => assert_eq!(got, Effective::Blocking, "{name}"),
                _ => assert_eq!(
                    got,
                    Effective::Note {
                        demoted_reason: want["demoted_reason"].as_str().map(str::to_owned)
                    },
                    "{name}"
                ),
            }
            blocking |= got == Effective::Blocking;
        }
        assert_eq!(
            if blocking { "FAIL" } else { "PASS" },
            expected["verdict"].as_str().unwrap(),
            "{name}"
        );
        ran += 1;
    }
    // 26 cases; the 7 envelope cases (bad-json, both bad-schema-version cases incl. the v0 one, blank-reviewer,
    // blank-head, two bad-shape) are skipped above, so 19 run.
    assert_eq!(
        ran,
        cases
            .iter()
            .filter(|c| c["expected"]["envelope"] != true)
            .count()
    );
    assert_eq!(ran, 19);
}

/// A lead's approval of `revision` under a delegation that cites the owner's earlier words for `covers`.
fn delegated(revision: &str, covers: &[&str]) -> Value {
    json!({"checkpoint":"intent","subject":"intent","subject_revision":revision,"decision":"approve",
        "text":"The cited approval covers this intent.",
        "actor":{"name":"lead:loop-engine","kind":"agent"},
        "delegation":{"basis":"approval","source":"ask #412",
            "quote":"yes, go ahead with the plan as written","covers":covers}})
}

/// Intent-1 approved in the owner's words, then `changed` approved by `event` under a delegation for `covers`.
fn delegated_after_own_words(repo: &Path, changed: Value, covers: &[&str], event: &str) -> Value {
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, changed),
        record("d2", "owner-decision", 4, delegated("intent-2", covers)),
    ];
    ask(repo, true, "intent-approval", event, ctx)
}

#[test]
fn gate_delegated_coverage_accepts_matching_covers() {
    let r = repo();
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record(
            "d",
            "owner-decision",
            2,
            delegated("intent-1", &["c2", "c1", "c1"]),
        ),
    ];
    allowed(&ask(&r, true, "intent-approval", "approved", ctx.clone()));
    let mut verified = ctx;
    verified.push(record(
        "v1",
        "verification",
        3,
        verification(&r, &intent(false)),
    ));
    allowed(&ask(&r, false, "verify", "verified", verified));
}

#[test]
fn gate_delegated_coverage_refuses_missing_and_extra_id() {
    let r = repo();
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record(
            "d",
            "owner-decision",
            2,
            delegated("intent-1", &["c1", "c9"]),
        ),
    ];
    let refused = ask(&r, true, "intent-approval", "approved", ctx.clone());
    assert_eq!(code(&refused), "delegation-not-covering");
    assert_eq!(refused["feedback"]["details"]["intent"], json!("intent-1"));
    assert_eq!(refused["feedback"]["details"]["missing"], json!(["c2"]));
    assert_eq!(refused["feedback"]["details"]["extra"], json!(["c9"]));
    let mut verified = ctx;
    verified.push(record(
        "v1",
        "verification",
        3,
        verification(&r, &intent(false)),
    ));
    let refused = ask(&r, false, "verify", "verified", verified);
    assert_eq!(code(&refused), "delegation-not-covering");
    assert_eq!(refused["feedback"]["details"]["missing"], json!(["c2"]));
    assert_eq!(refused["feedback"]["details"]["extra"], json!(["c9"]));
}

#[test]
fn gate_delegated_coverage_own_words_unaffected() {
    let r = repo();
    let ctx = verify_ctx(&r, |_| {});
    allowed(&ask(
        &r,
        true,
        "intent-approval",
        "approved",
        ctx[..2].to_vec(),
    ));
    allowed(&ask(&r, false, "verify", "verified", ctx));
}

#[test]
fn gate_delegated_revision_refuses_changed_goal() {
    let r = repo();
    let mut changed = intent(false);
    changed["goal"] = json!("g, reworded");
    let refused = delegated_after_own_words(&r, changed, &["c1", "c2"], "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-1")
    );
    assert_eq!(refused["feedback"]["details"]["differs"], json!(["goal"]));
}

#[test]
fn gate_delegated_revision_refuses_changed_constraints() {
    let r = repo();
    let mut changed = intent(false);
    changed["constraints"] = json!(["d"]);
    let refused = delegated_after_own_words(&r, changed, &["c1", "c2"], "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["differs"],
        json!(["constraints"])
    );
}

#[test]
fn gate_delegated_revision_refuses_changed_criterion_statement() {
    let r = repo();
    let mut changed = intent(false);
    changed["criteria"][1]["statement"] = json!("s2, reworded");
    // Changed from the old expectation: a statement change with no rewrites is now `delegation-rewrite-mismatch`.
    let refused = delegated_after_own_words(&r, changed, &["c1", "c2"], "approved");
    assert_eq!(code(&refused), "delegation-rewrite-mismatch");
    assert_eq!(refused["feedback"]["details"]["undeclared"], json!(["c2"]));
}

#[test]
fn gate_delegated_revision_refuses_added_criterion() {
    let r = repo();
    let mut changed = intent(false);
    changed["criteria"]
        .as_array_mut()
        .unwrap()
        .push(criterion("c3", None));
    let refused = delegated_after_own_words(&r, changed, &["c1", "c2", "c3"], "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["differs"],
        json!(["criteria"])
    );
}

#[test]
fn gate_delegated_revision_refuses_dropped_criterion() {
    let r = repo();
    let mut changed = intent(false);
    changed["criteria"].as_array_mut().unwrap().pop();
    let refused = delegated_after_own_words(&r, changed, &["c1"], "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["differs"],
        json!(["criteria"])
    );
}

#[test]
fn gate_delegated_revision_accepts_check_only_change() {
    let r = repo();
    let base = intent(false);
    // The check's argv and timeout, and the order of the criteria, are the agents' choices.
    let mut checks = base.clone();
    checks["criteria"][0]["check"] = json!({"argv":["sh","-c","true"],"timeout_ms":5000});
    checks["criteria"][1]["check"]["timeout_ms"] = json!(2000);
    allowed(&delegated_after_own_words(
        &r,
        checks,
        &["c1", "c2"],
        "approved",
    ));
    let mut reordered = base.clone();
    reordered["criteria"] = json!([base["criteria"][1], base["criteria"][0]]);
    allowed(&delegated_after_own_words(
        &r,
        reordered,
        &["c1", "c2"],
        "approved",
    ));
    // The hard flag is also the agents' choice: the same outcome, approved as hard.
    allowed(&delegated_after_own_words(
        &r,
        intent(true),
        &["c1", "c2"],
        "approved-hard",
    ));
}

#[test]
fn gate_delegated_revision_accepts_old_expectation_change() {
    let r = repo();
    let old_intent = |expect: &str| {
        intent_with(vec![
            criterion("c1", Some(json!({"expect": expect}))),
            criterion("c2", Some(json!({"expect": "same"}))),
        ])
    };
    let ctx = vec![
        record("intent-1", "intent", 1, old_intent("fails")),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, old_intent("absent")),
        record(
            "d2",
            "owner-decision",
            4,
            delegated("intent-2", &["c1", "c2"]),
        ),
    ];
    allowed(&ask_with(
        old_vs_new_input(&r, true, &[]),
        "intent-approval",
        "approved",
        ctx,
    ));
}

#[test]
fn gate_delegated_revision_refused_at_verified_for_revision_appended_during_implement() {
    let r = repo();
    let mut changed = intent(false);
    changed["criteria"][0]["statement"] = json!("s1, reworded");
    let mut v = verification(&r, &changed);
    v["intent"] = json!("intent-2");
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record(
            "d1",
            "owner-decision",
            2,
            delegated("intent-1", &["c1", "c2"]),
        ),
        record("intent-2", "intent", 3, changed),
        record(
            "d2",
            "owner-decision",
            4,
            delegated("intent-2", &["c1", "c2"]),
        ),
        record("v2", "verification", 5, v),
    ];
    let refused = ask(&r, false, "verify", "verified", ctx);
    // Changed from the old expectation: the reworded statement has no rewrites, so `delegation-rewrite-mismatch`.
    assert_eq!(code(&refused), "delegation-rewrite-mismatch");
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-1")
    );
}

#[test]
fn gate_delegated_revision_refused_delegation_never_a_baseline() {
    let r = repo();
    let mut changed = intent(false);
    changed["goal"] = json!("g, reworded");
    let mut ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record(
            "d1",
            "owner-decision",
            2,
            delegated("intent-1", &["c1", "c2"]),
        ),
        record("intent-2", "intent", 3, changed.clone()),
        record(
            "d2",
            "owner-decision",
            4,
            delegated("intent-2", &["c1", "c2"]),
        ),
    ];
    // intent-2 changes the goal, so its delegation is refused and counts for nothing afterwards.
    let refused = ask(&r, true, "intent-approval", "approved", ctx.clone());
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    // intent-3 repeats that change. Its baseline is still intent-1, not the refused intent-2.
    ctx.push(record("intent-3", "intent", 5, changed));
    ctx.push(record(
        "d3",
        "owner-decision",
        6,
        delegated("intent-3", &["c1", "c2"]),
    ));
    let refused = ask(&r, true, "intent-approval", "approved", ctx.clone());
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-1")
    );
    // The owner's own words approve intent-3.
    ctx.push(record(
        "d4",
        "owner-decision",
        7,
        decision("Yes, that change too.", "intent-3"),
    ));
    allowed(&ask(&r, true, "intent-approval", "approved", ctx));
}

#[test]
fn gate_delegated_revision_baseline_is_latest_validly_approved() {
    let r = repo();
    let base = intent(false);
    // intent-2 changes only a check, so it keeps the outcome and becomes the baseline.
    let mut check_only = base.clone();
    check_only["criteria"][1]["check"]["timeout_ms"] = json!(2000);
    // intent-3 changes only a check of intent-2.
    let mut second_check = check_only.clone();
    second_check["criteria"][0]["check"]["argv"] = json!(["sh", "-c", "true"]);
    let mut ctx = vec![
        record("intent-1", "intent", 1, base),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, check_only),
        record(
            "d2",
            "owner-decision",
            4,
            delegated("intent-2", &["c1", "c2"]),
        ),
        record("intent-3", "intent", 5, second_check.clone()),
        record(
            "d3",
            "owner-decision",
            6,
            delegated("intent-3", &["c1", "c2"]),
        ),
    ];
    allowed(&ask(&r, true, "intent-approval", "approved", ctx.clone()));
    // A goal change is judged against intent-3, the latest approval that was valid.
    let mut goal = second_check;
    goal["goal"] = json!("g, reworded");
    ctx.push(record("intent-4", "intent", 7, goal));
    ctx.push(record(
        "d4",
        "owner-decision",
        8,
        delegated("intent-4", &["c1", "c2"]),
    ));
    let refused = ask(&r, true, "intent-approval", "approved", ctx);
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-3")
    );
}

// ---- Delegated rewrites: a statement may be reworded only as the delegation declares. ----

/// `delegated`, with `rewrites` declared.
fn delegated_rewriting(revision: &str, covers: &[&str], rewrites: Value) -> Value {
    let mut data = delegated(revision, covers);
    data["delegation"]["rewrites"] = rewrites;
    data
}

fn statement_rewrite(id: &str, before: &str, after: &str) -> Value {
    json!({"criterion": id, "before": before, "after": after, "reason": "wording only, same promise"})
}

/// `intent(false)` with c1's statement reworded.
fn reworded() -> Value {
    let mut changed = intent(false);
    changed["criteria"][0]["statement"] = json!("s1, reworded");
    changed
}

/// Intent-1 approved in the owner's words, then `changed` approved under a delegation declaring `rewrites`.
fn rewriting_after_own_words(repo: &Path, changed: Value, rewrites: Value, event: &str) -> Value {
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, changed),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting("intent-2", &["c1", "c2"], rewrites),
        ),
    ];
    ask(repo, true, "intent-approval", event, ctx)
}

fn mismatch(refused: &Value) -> &Value {
    assert_eq!(code(refused), "delegation-rewrite-mismatch", "{refused}");
    &refused["feedback"]["details"]
}

#[test]
fn gate_delegated_rewrite_accepts_declared_statement_change() {
    let r = repo();
    let rewrites = json!([statement_rewrite("c1", "s1", "s1, reworded")]);
    allowed(&rewriting_after_own_words(
        &r,
        reworded(),
        rewrites.clone(),
        "approved",
    ));
    // approved-hard: the same rewrite on a hard intent
    let mut hard = intent(true);
    hard["criteria"][0]["statement"] = json!("s1, reworded");
    allowed(&rewriting_after_own_words(
        &r,
        hard,
        rewrites.clone(),
        "approved-hard",
    ));
    // verified: the check runs again at verification
    let mut v = verification(&r, &reworded());
    v["intent"] = json!("intent-2");
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, reworded()),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting("intent-2", &["c1", "c2"], rewrites),
        ),
        record("v2", "verification", 5, v),
    ];
    allowed(&ask(&r, false, "verify", "verified", ctx));
}

#[test]
fn gate_delegated_rewrite_refused_at_verified_when_wrong() {
    let r = repo();
    let mut v = verification(&r, &reworded());
    v["intent"] = json!("intent-2");
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, reworded()),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting(
                "intent-2",
                &["c1", "c2"],
                json!([statement_rewrite("c1", "s1", "something else")]),
            ),
        ),
        record("v2", "verification", 5, v),
    ];
    let refused = ask(&r, false, "verify", "verified", ctx);
    assert_eq!(mismatch(&refused)["wrong"][0]["wrong"], json!("after"));
}

#[test]
fn gate_delegated_old_record_without_rewrites_behaves_as_before() {
    let r = repo();
    // No `rewrites` key at all: still parses, keeps the outcome, and is accepted.
    let data = delegated("intent-2", &["c1", "c2"]);
    assert!(data["delegation"].get("rewrites").is_none());
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, intent(false)),
        record("d2", "owner-decision", 4, data),
    ];
    allowed(&ask(&r, true, "intent-approval", "approved", ctx));
}

#[test]
fn gate_delegated_rewrite_refuses_goal_constraints_and_criterion_ids_even_with_rewrites() {
    let r = repo();
    let rewrites = json!([statement_rewrite("c1", "s1", "s1, reworded")]);
    let mut goal = reworded();
    goal["goal"] = json!("g, reworded");
    let refused = rewriting_after_own_words(&r, goal, rewrites.clone(), "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(refused["feedback"]["details"]["differs"], json!(["goal"]));
    let mut constraints = reworded();
    constraints["constraints"] = json!(["d"]);
    let refused = rewriting_after_own_words(&r, constraints, rewrites.clone(), "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["differs"],
        json!(["constraints"])
    );
    // Goal first, when a statement is also reworded without a rewrite.
    let mut both = reworded();
    both["goal"] = json!("g, reworded");
    let refused = rewriting_after_own_words(&r, both, json!([]), "approved");
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    let mut added = reworded();
    added["criteria"]
        .as_array_mut()
        .unwrap()
        .push(criterion("c3", None));
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, added),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting("intent-2", &["c1", "c2", "c3"], rewrites.clone()),
        ),
    ];
    let refused = ask(&r, true, "intent-approval", "approved", ctx);
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
    assert_eq!(
        refused["feedback"]["details"]["differs"],
        json!(["criteria"])
    );
    let mut dropped = reworded();
    dropped["criteria"].as_array_mut().unwrap().pop();
    let ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, dropped),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting("intent-2", &["c1"], rewrites),
        ),
    ];
    let refused = ask(&r, true, "intent-approval", "approved", ctx);
    assert_eq!(code(&refused), "delegation-changes-approved-outcome");
}

#[test]
fn gate_delegated_rewrite_refuses_statement_change_without_rewrites() {
    let r = repo();
    let refused = rewriting_after_own_words(&r, reworded(), json!([]), "approved");
    assert_eq!(mismatch(&refused)["undeclared"], json!(["c1"]));
}

#[test]
fn gate_delegated_rewrite_refuses_rewrite_for_unchanged_criterion() {
    let r = repo();
    let rewrites = json!([
        statement_rewrite("c1", "s1", "s1, reworded"),
        statement_rewrite("c2", "s2", "s2, reworded")
    ]);
    let refused = rewriting_after_own_words(&r, reworded(), rewrites, "approved");
    assert_eq!(
        mismatch(&refused)["not_changed"],
        json!([{"criterion": "c2", "field": "statement"}])
    );
    // A criterion the intent does not have counts the same.
    let refused = rewriting_after_own_words(
        &r,
        reworded(),
        json!([
            statement_rewrite("c1", "s1", "s1, reworded"),
            statement_rewrite("c9", "x", "y")
        ]),
        "approved",
    );
    assert_eq!(
        mismatch(&refused)["not_changed"],
        json!([{"criterion": "c9", "field": "statement"}])
    );
}

#[test]
fn gate_delegated_rewrite_refuses_wrong_before_after_and_blank_reason() {
    let r = repo();
    let refused = rewriting_after_own_words(
        &r,
        reworded(),
        json!([statement_rewrite("c1", "not s1", "s1, reworded")]),
        "approved",
    );
    let wrong = &mismatch(&refused)["wrong"];
    assert_eq!(wrong.as_array().unwrap().len(), 1);
    assert_eq!(wrong[0]["wrong"], json!("before"));
    assert_eq!(wrong[0]["expected"], json!("s1"));
    let refused = rewriting_after_own_words(
        &r,
        reworded(),
        json!([statement_rewrite("c1", "s1", "s1, reworded ")]),
        "approved",
    );
    assert_eq!(mismatch(&refused)["wrong"][0]["wrong"], json!("after"));
    for reason in ["", "  \n"] {
        let mut entry = statement_rewrite("c1", "s1", "s1, reworded");
        entry["reason"] = json!(reason);
        let refused = rewriting_after_own_words(&r, reworded(), json!([entry]), "approved");
        assert_eq!(mismatch(&refused)["wrong"][0]["wrong"], json!("reason"));
    }
}

#[test]
fn gate_delegated_rewrite_refuses_duplicate_entry() {
    let r = repo();
    let entry = statement_rewrite("c1", "s1", "s1, reworded");
    let refused =
        rewriting_after_own_words(&r, reworded(), json!([entry.clone(), entry]), "approved");
    assert_eq!(
        mismatch(&refused)["duplicate"],
        json!([{"criterion": "c1", "field": "statement"}])
    );
}

#[test]
fn gate_delegated_rewrite_refuses_unknown_rewrite_field_as_malformed() {
    let r = repo();
    let mut entry = statement_rewrite("c1", "s1", "s1, reworded");
    entry["extra"] = json!(true);
    let refused = rewriting_after_own_words(&r, reworded(), json!([entry]), "approved");
    // The record is malformed, so no decision counts.
    assert_eq!(code(&refused), "owner-decision-missing");
}

#[test]
fn gate_delegated_check_change_needs_no_rewrite_but_a_declared_one_must_match() {
    let r = repo();
    let mut changed = intent(false);
    changed["criteria"][0]["check"] = json!({"argv":["sh","-c","true"],"timeout_ms":5000});
    let before = intent(false)["criteria"][0]["check"].clone();
    let after = changed["criteria"][0]["check"].clone();
    // Undeclared: accepted, as before.
    allowed(&rewriting_after_own_words(
        &r,
        changed.clone(),
        json!([]),
        "approved",
    ));
    let entry = |before: &Value, after: &Value| json!({"criterion": "c1", "field": "check", "before": before, "after": after, "reason": "runner moved"});
    allowed(&rewriting_after_own_words(
        &r,
        changed.clone(),
        json!([entry(&before, &after)]),
        "approved",
    ));
    // Wrong before.
    let refused = rewriting_after_own_words(
        &r,
        changed.clone(),
        json!([entry(&after, &after)]),
        "approved",
    );
    assert_eq!(mismatch(&refused)["wrong"][0]["wrong"], json!("before"));
    // A check rewrite for a check that did not change.
    let refused = rewriting_after_own_words(
        &r,
        reworded(),
        json!([
            statement_rewrite("c1", "s1", "s1, reworded"),
            entry(&before, &after)
        ]),
        "approved",
    );
    assert_eq!(
        mismatch(&refused)["not_changed"],
        json!([{"criterion": "c1", "field": "check"}])
    );
    // A statement and a check rewrite for one criterion are two different entries, not a duplicate.
    let mut both = changed;
    both["criteria"][0]["statement"] = json!("s1, reworded");
    allowed(&rewriting_after_own_words(
        &r,
        both,
        json!([
            statement_rewrite("c1", "s1", "s1, reworded"),
            entry(&before, &after)
        ]),
        "approved",
    ));
}

#[test]
fn gate_delegated_rewrite_valid_approval_becomes_the_baseline() {
    let r = repo();
    // intent-2 rewords c1 under a valid delegation, so it is the baseline for intent-3.
    let mut ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, reworded()),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting(
                "intent-2",
                &["c1", "c2"],
                json!([statement_rewrite("c1", "s1", "s1, reworded")]),
            ),
        ),
    ];
    allowed(&ask(&r, true, "intent-approval", "approved", ctx.clone()));
    // intent-3 keeps the reworded text: no rewrite is needed, and a goal change is judged against intent-2.
    ctx.push(record("intent-3", "intent", 5, reworded()));
    ctx.push(record(
        "d3",
        "owner-decision",
        6,
        delegated("intent-3", &["c1", "c2"]),
    ));
    allowed(&ask(&r, true, "intent-approval", "approved", ctx.clone()));
    // Going back to the old text is a change from intent-2, so it needs its own rewrite.
    ctx.push(record("intent-4", "intent", 7, intent(false)));
    ctx.push(record(
        "d4",
        "owner-decision",
        8,
        delegated("intent-4", &["c1", "c2"]),
    ));
    let refused = ask(&r, true, "intent-approval", "approved", ctx);
    assert_eq!(mismatch(&refused)["undeclared"], json!(["c1"]));
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-3")
    );
}

#[test]
fn gate_delegated_rewrite_refused_approval_never_a_baseline() {
    let r = repo();
    // intent-2 declares a wrong rewrite, so it is refused and intent-1 stays the baseline.
    let mut ctx = vec![
        record("intent-1", "intent", 1, intent(false)),
        record("d1", "owner-decision", 2, decision("Yes.", "intent-1")),
        record("intent-2", "intent", 3, reworded()),
        record(
            "d2",
            "owner-decision",
            4,
            delegated_rewriting(
                "intent-2",
                &["c1", "c2"],
                json!([statement_rewrite("c1", "s1", "nope")]),
            ),
        ),
    ];
    mismatch(&ask(&r, true, "intent-approval", "approved", ctx.clone()));
    // intent-3 carries the same reworded text with no rewrites. Were intent-2 the baseline, nothing would differ.
    ctx.push(record("intent-3", "intent", 5, reworded()));
    ctx.push(record(
        "d3",
        "owner-decision",
        6,
        delegated("intent-3", &["c1", "c2"]),
    ));
    let refused = ask(&r, true, "intent-approval", "approved", ctx);
    assert_eq!(mismatch(&refused)["undeclared"], json!(["c1"]));
    assert_eq!(
        refused["feedback"]["details"]["approved_revision"],
        json!("intent-1")
    );
}

// ---- the kgate runner ----

/// A stand-in for kgate. `fake --profile P -- CMD...` prints kgate-style lines (job ids and times that differ on every
/// call) around CMD, as the real one does, and runs `body` in place of the pod. Returns the script and its log, which
/// gets one line per call: the directory it was called from and its arguments.
fn fake_kgate(body: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lean-change-fake-kgate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (script, log) = (dir.join("kgate"), dir.join("calls.log"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$PWD $*\" >> '{}'\necho \"kgate: pod kgate-$$ on node-$(date +%s%N)\"\n[ \"$1\" = --profile ] && [ \"$3\" = -- ] || exit 64\nshift 3\n{body}\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

const PASS_THROUGH: &str = "echo \"kgate[job-$$]: start $(date +%s%N)\"; \"$@\"; code=$?; echo \"kgate[job-$$]: end $(date +%s%N) exit $code\"; exit $code";

fn kgate_repo() -> PathBuf {
    let r = repo();
    std::fs::write(r.join("kgate.toml"), "image = \"x\"\n").unwrap();
    r
}

fn kgate_runner(r: &Path, script: &Path) -> crate::run::Runner {
    crate::run::Runner::Kgate {
        program: script.display().to_string(),
        profile: r.join("kgate.toml"),
    }
}

fn check_intent(argv: Value, timeout_ms: u64, old: Option<Value>) -> Value {
    let mut c = criterion("c1", old);
    c["check"] = json!({"argv": argv, "timeout_ms": timeout_ms});
    intent_with(vec![c])
}

#[test]
fn kgate_runner_needs_the_repositorys_profile() {
    let r = repo();
    assert!(crate::run::Runner::kgate(&r, "kgate.toml")
        .unwrap_err()
        .contains("kgate.toml"));
    std::fs::write(r.join("kgate.toml"), "image = \"x\"\n").unwrap();
    assert!(crate::run::Runner::kgate(&r, "kgate.toml").is_ok());
    // Another profile of the repository, but not one outside it.
    std::fs::write(r.join("small.toml"), "image = \"x\"\n").unwrap();
    match crate::run::Runner::kgate(&r, "small.toml").unwrap() {
        crate::run::Runner::Kgate { profile, .. } => assert_eq!(profile, r.join("small.toml")),
        crate::run::Runner::Host => panic!("not kgate"),
    }
    for outside in ["../kgate.toml", "/etc/hostname", "a/../../kgate.toml"] {
        assert!(
            crate::run::Runner::kgate(&r, outside)
                .unwrap_err()
                .contains("inside the repository"),
            "{outside}"
        );
    }
}

#[test]
fn kgate_runner_runs_the_check_through_kgate_and_keeps_only_the_checks_output() {
    let r = kgate_repo();
    let (script, log) = fake_kgate(PASS_THROUGH);
    let i = check_intent(json!(["sh", "-c", "echo out; test -f a.txt"]), 5000, None);
    let v = crate::run::verify_with(
        &show(&r, false, vec![record("intent-1", "intent", 1, i.clone())]),
        &kgate_runner(&r, &script),
    )
    .unwrap();
    assert_eq!(
        (v.results[0].exit_code, v.results[0].output_tail.as_str()),
        (Some(0), "out\n")
    );
    // The approved check, not the kgate command line, is what the result is bound to.
    let parsed: Intent = serde_json::from_value(i).unwrap();
    assert_eq!(v.results[0].check_sha256, parsed.criteria[0].check.sha256());
    // kgate was called from the repository with its profile and the check after `--`.
    let calls = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        calls.trim(),
        format!(
            "{r} --profile {r}/kgate.toml -- sh -c echo out; test -f a.txt",
            r = std::fs::canonicalize(&r).unwrap().display()
        )
    );
}

#[test]
fn kgate_runner_reports_a_failing_check_as_failing() {
    let r = kgate_repo();
    let (script, _) = fake_kgate(PASS_THROUGH);
    let i = check_intent(json!(["sh", "-c", "echo no; exit 1"]), 5000, None);
    let v = crate::run::verify_with(
        &show(&r, false, vec![record("intent-1", "intent", 1, i)]),
        &kgate_runner(&r, &script),
    )
    .unwrap();
    assert_eq!(v.results[0].exit_code, Some(1));
    assert_eq!(v.results[0].output_tail, "no\n");
}

#[test]
fn kgate_that_did_not_run_the_check_makes_no_verification() {
    let r = kgate_repo();
    let i = check_intent(json!(["true"]), 5000, None);
    let ctx = || vec![record("intent-1", "intent", 1, i.clone())];
    for code in [64, 69, 75, 130, 143] {
        let (script, _) = fake_kgate(&format!("echo 'kgate: cannot'; exit {code}"));
        let error = crate::run::verify_with(&show(&r, false, ctx()), &kgate_runner(&r, &script))
            .unwrap_err();
        assert!(
            error.contains("criterion c1 was not run on the current tree")
                && error.contains(&format!("exit {code}")),
            "{code}: {error}"
        );
    }
    // A kgate killed by a signal has no exit code and is not a result either.
    let (script, _) = fake_kgate("kill -9 $$");
    assert!(
        crate::run::verify_with(&show(&r, false, ctx()), &kgate_runner(&r, &script))
            .unwrap_err()
            .contains("was not run")
    );
    // A kgate that is not installed is the same.
    let missing = crate::run::Runner::Kgate {
        program: "/nonexistent/kgate".into(),
        profile: r.join("kgate.toml"),
    };
    assert!(crate::run::verify_with(&show(&r, false, ctx()), &missing)
        .unwrap_err()
        .contains("was not run"));
}

#[test]
fn kgate_that_did_not_run_on_the_old_tree_does_not_count_as_the_check_failing_there() {
    let r = kgate_repo();
    let i = check_intent(
        json!(["sh", "-c", "test -f a.txt"]),
        5000,
        Some(json!({"expect": "fails"})),
    );
    let input = old_vs_new_input(&r, false, &[]);
    let ctx = || vec![record("intent-1", "intent", 1, i.clone())];
    // Runs the check in the repository; refuses everywhere else, as kgate does when it cannot reach the cluster.
    let here = std::fs::canonicalize(&r).unwrap();
    let (script, _) = fake_kgate(&format!(
        "[ \"$PWD\" = '{}' ] && {{ \"$@\"; exit $?; }}; echo 'kgate: cluster unreachable'; exit 69",
        here.display()
    ));
    let error =
        crate::run::verify_with(&show_with(input.clone(), ctx()), &kgate_runner(&r, &script))
            .unwrap_err();
    assert!(
        error.contains("criterion c1 was not run on the old tree"),
        "{error}"
    );
    // The control: on the host a check that itself exits 69 is a real failure, twice, and satisfies `fails`.
    let mut host = i.clone();
    host["criteria"][0]["check"]["argv"] = json!(["sh", "-c", "exit 69"]);
    let v = crate::run::verify(&show_with(
        input,
        vec![record("intent-1", "intent", 1, host)],
    ))
    .unwrap();
    let runs = &v.results[0].old.as_ref().unwrap().runs;
    assert_eq!(
        runs.iter().map(|r| r.exit_code).collect::<Vec<_>>(),
        [Some(69), Some(69)]
    );
}

#[test]
fn kgate_old_run_output_matches_the_current_run_for_same() {
    let r = kgate_repo();
    let (script, _) = fake_kgate(PASS_THROUGH);
    let i = check_intent(
        json!(["sh", "-c", "echo hello"]),
        5000,
        Some(json!({"expect": "same"})),
    );
    let v = crate::run::verify_with(
        &show_with(
            old_vs_new_input(&r, false, &[]),
            vec![record("intent-1", "intent", 1, i)],
        ),
        &kgate_runner(&r, &script),
    )
    .unwrap();
    let old = v.results[0].old.as_ref().unwrap();
    assert_eq!(old.runs.len(), 1);
    assert_eq!(old.runs[0].output_tail, "hello\n");
    assert_eq!(v.results[0].output_tail, "hello\n");
}

#[test]
fn kgate_old_tree_carries_its_own_git_objects() {
    let r = kgate_repo();
    // The old tree is shipped to a pod, where the repository's objects are not: it must not borrow them.
    let i = check_intent(
        json!([
            "sh",
            "-c",
            "test ! -e .git/objects/info/alternates && git log -1 --format=%s"
        ]),
        5000,
        Some(json!({"expect": "same"})),
    );
    let ctx = || vec![record("intent-1", "intent", 1, i.clone())];
    let (script, _) = fake_kgate(PASS_THROUGH);
    let v = crate::run::verify_with(
        &show_with(old_vs_new_input(&r, false, &[]), ctx()),
        &kgate_runner(&r, &script),
    )
    .unwrap();
    assert_eq!(
        v.results[0].old.as_ref().unwrap().runs[0].exit_code,
        Some(0)
    );
    // The control: on the host the old tree borrows them, as it always did.
    let host = crate::run::verify(&show_with(old_vs_new_input(&r, false, &[]), ctx())).unwrap();
    assert_eq!(
        host.results[0].old.as_ref().unwrap().runs[0].exit_code,
        Some(1)
    );
}

#[test]
fn kgate_runner_stops_a_timed_out_client_with_sigterm_so_it_can_delete_its_pod() {
    let r = kgate_repo();
    let marker = std::env::temp_dir().join(format!("lean-change-term-{}", std::process::id()));
    let (script, _) = fake_kgate(&format!(
        "trap 'echo term > {}; exit 143' TERM; sleep 30 & wait",
        marker.display()
    ));
    let i = check_intent(json!(["true"]), 300, None);
    let started = std::time::Instant::now();
    let v = crate::run::verify_with(
        &show(&r, false, vec![record("intent-1", "intent", 1, i)]),
        &kgate_runner(&r, &script),
    )
    .unwrap();
    assert!(v.results[0].timed_out && v.results[0].exit_code.is_none());
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "term");
    let _ = std::fs::remove_file(marker);
}
