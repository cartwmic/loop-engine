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

fn answers() -> Vec<Value> {
    QUESTIONS
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
        ["use", "edge", "control", "terms", "criteria-gap"]
    );
    let le209 = section("LE-209");
    for (id, text) in INTENT_QUESTIONS {
        assert!(le209.contains(&format!("\"{text}\" ({id})")), "{id}");
    }
    assert!(le209.contains("five fixed intent-check questions"));
    let review_ids: Vec<&str> = QUESTIONS.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        review_ids,
        [
            "intent",
            "scope",
            "edge-cases",
            "checks",
            "docs",
            "principles"
        ]
    );
    let le207 = section("LE-207");
    assert!(le207.contains("(intent, scope, edge-cases, checks, docs, principles)"));
    let docs = QUESTIONS.iter().find(|(id, _)| *id == "docs").unwrap().1;
    assert!(docs.ends_with(
        "If the repository keeps a living PRD, name each requirement the change adds, changes or relies on, \
         and the check that proves it; a requirement that no check proves is a finding."
    ));
    assert!(le207.contains(&format!("\"{docs}\" (docs)")));
}

#[test]
fn intent_packet_carries_the_five_questions_and_the_draft_intent() {
    let r = repo();
    let draft = vec![record("intent-1", "intent", 1, intent(false))];
    // Needs no verification: the intent is still a draft.
    let packet = crate::packet::intent_packet(&show(&r, true, draft.clone())).unwrap();
    for (id, text) in INTENT_QUESTIONS {
        assert!(packet.contains(&format!("- {id}: {text}")), "{id}");
    }
    assert!(packet.contains("\"goal\": \"g\"") && packet.contains("(intent-1)"));
    assert!(packet.contains(&r.display().to_string()));
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
        ":!/etc",
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
