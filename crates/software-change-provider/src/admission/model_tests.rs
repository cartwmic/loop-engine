use super::*;
use loop_core::{SemanticSequence, Timestamp};
use serde_json::json;

fn input(bindings: Option<Value>) -> Value {
    let mut input: Value =
        serde_json::from_str(include_str!("../../data/configs/minimal.json")).unwrap();
    if let Some(bindings) = bindings {
        input["model_bindings"] = bindings;
    }
    input
}
fn ctx<'a>(
    input: &'a Value,
    workflow: &'a Workflow,
    records: &'a [ContextRecord],
) -> AdmissionContext<'a> {
    AdmissionContext::new(input, records, "intent-review", 1, workflow).unwrap()
}
fn workflow(input: &Value) -> Workflow {
    crate::workflow::describe_workflow(Some(input)).unwrap()
}
fn candidate(kind: &str, data: Value) -> CandidateRecord {
    CandidateRecord {
        id: "candidate".into(),
        kind: kind.into(),
        data,
    }
}
fn stored(id: &str, kind: &str, data: Value, sequence: u64) -> ContextRecord {
    ContextRecord::new(
        id,
        kind,
        data,
        SemanticSequence::new(sequence),
        Timestamp::from_unix_millis(0),
    )
}
fn binding(model: &str, thinking: &str) -> Value {
    json!({"model": model, "thinking": thinking})
}
fn bound_input() -> Value {
    input(Some(json!({"roles": {
        "ordinary-reviewer": binding("p/ordinary", "high"),
        "challenge-reviewer": binding("p/challenge", "high"),
        "cold-reader": binding("p/cold", "low"),
        "criterion-author": binding("p/criterion", "medium"),
        "author": binding("p/author", "high")
    }})))
}
fn review(gate: &str, policy_id: &str, session: Option<Value>) -> Value {
    let mut data = json!({"gate": gate, "policy_id": policy_id});
    if let Some(session) = session {
        data["session"] = session;
    }
    data
}
fn declaration(role: &str, session: Value) -> Value {
    json!({"subject": "design.json", "subject_revision": "r1", "role": role, "session": session})
}
fn amendment(changes: Value) -> Value {
    json!({
        "actor": {"name": "owner", "kind": "human"},
        "authority": {"kind": "owner", "reference": "retained answer"},
        "words": "Use the other model from here",
        "changes": changes
    })
}
fn refusal_of(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Refusal {
    let before = serde_json::to_value(ctx.context).unwrap();
    let refusal = check(ctx, record).expect_err("record must be refused");
    assert_eq!(serde_json::to_value(ctx.context).unwrap(), before);
    refusal
}

#[test]
fn new_kinds_are_validated_and_session_checks_are_write_only() {
    for kind in ["model-binding-amendment", "session-declaration"] {
        assert!(VALIDATED_KINDS.contains(&kind));
    }
    assert!(unmigrated_kinds().is_empty());
    for kind in [
        "review-evidence",
        "criterion-verdict",
        "goal-verdict",
        "session-declaration",
    ] {
        let rule = RULES
            .iter()
            .find(|rule| rule.kind == kind && rule.id == MODEL_SESSION_RULE)
            .unwrap_or_else(|| panic!("{kind} has no session rule"));
        assert_eq!(rule.applicability, Applicability::WriteOnly);
    }
    let amendment_rule = RULES
        .iter()
        .find(|rule| rule.kind == "model-binding-amendment")
        .unwrap();
    assert_eq!(amendment_rule.applicability, Applicability::Always);
}

#[test]
fn unbound_run_checks_no_session_for_any_role() {
    let input = input(None);
    let workflow = workflow(&input);
    let ctx = ctx(&input, &workflow, &[]);
    for (kind, data) in [
        (
            "review-evidence",
            review("intent-review", "ids-grounded", None),
        ),
        (
            "review-evidence",
            review(
                "design-adversarial-review",
                "x",
                Some(binding("any", "any")),
            ),
        ),
        ("criterion-verdict", json!({})),
        ("goal-verdict", json!({"session": "not even an object"})),
        (
            "session-declaration",
            declaration("author", binding("any", "any")),
        ),
    ] {
        assert!(
            model_rules::session(&ctx, &candidate(kind, data.clone())).is_ok(),
            "{kind} {data}"
        );
    }
}

#[test]
fn each_bound_role_refuses_missing_and_unequal_sessions_naming_role_and_values() {
    let input = bound_input();
    let workflow = workflow(&input);
    let ctx = ctx(&input, &workflow, &[]);
    let table = [
        (
            "ordinary-reviewer",
            "review-evidence",
            "design-review",
            "ids-grounded",
            "p/ordinary",
            "high",
        ),
        (
            "challenge-reviewer",
            "review-evidence",
            "plan-adversarial-review",
            "ids-grounded",
            "p/challenge",
            "high",
        ),
        (
            "challenge-reviewer",
            "review-evidence",
            "intent-adversarial-review",
            "intent-sufficient",
            "p/challenge",
            "high",
        ),
        (
            "cold-reader",
            "review-evidence",
            "intent-review",
            "intent-sufficient",
            "p/cold",
            "low",
        ),
        (
            "criterion-author",
            "criterion-verdict",
            "",
            "",
            "p/criterion",
            "medium",
        ),
        (
            "criterion-author",
            "goal-verdict",
            "",
            "",
            "p/criterion",
            "medium",
        ),
    ];
    for (role, kind, gate, policy, model, thinking) in table {
        let data = |session: Option<Value>| {
            let mut data = if gate.is_empty() {
                json!({})
            } else {
                review(gate, policy, None)
            };
            if let Some(session) = session {
                data["session"] = session;
            }
            data
        };
        let exact = candidate(kind, data(Some(binding(model, thinking))));
        assert!(check_session(&ctx, &exact).is_ok(), "{role} match");
        for (name, session, difference) in [
            ("missing", None, "statement missing"),
            (
                "model",
                Some(binding("p/other", thinking)),
                "differs in model only",
            ),
            (
                "thinking",
                Some(binding(model, "other")),
                "differs in thinking level only",
            ),
            (
                "both",
                Some(binding("p/other", "other")),
                "differs in model and thinking level",
            ),
            (
                "malformed",
                Some(json!("not an object")),
                "must be an object",
            ),
            ("blank", Some(binding(" ", thinking)), "nonblank"),
        ] {
            let message = check_session(&ctx, &candidate(kind, data(session)))
                .expect_err(&format!("{role} {name}"));
            assert!(
                message.contains(&format!("`{role}`")) && message.contains(difference),
                "{role} {name}: {message}"
            );
            if name != "malformed" && name != "blank" {
                assert!(
                    message.contains(model) && message.contains(thinking),
                    "{message}"
                );
            }
        }
    }
}

fn check_session(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    model_rules::session(ctx, record)
}

#[test]
fn an_unbound_role_is_not_checked_while_other_roles_are() {
    let input = input(Some(
        json!({"roles": {"cold-reader": binding("p/cold", "low")}}),
    ));
    let workflow = workflow(&input);
    let ctx = ctx(&input, &workflow, &[]);
    let ordinary = candidate(
        "review-evidence",
        review("intent-review", "ids-grounded", None),
    );
    assert!(check_session(&ctx, &ordinary).is_ok());
    let cold = candidate(
        "review-evidence",
        review("intent-review", "intent-sufficient", None),
    );
    assert!(check_session(&ctx, &cold)
        .unwrap_err()
        .contains("`cold-reader`"));
}

#[test]
fn dispatch_reports_the_shared_session_rule_id() {
    let input = bound_input();
    let workflow = workflow(&input);
    let ctx = ctx(&input, &workflow, &[]);
    let wrong = candidate(
        "session-declaration",
        declaration("author", binding("p/else", "high")),
    );
    let refusal = refusal_of(&ctx, &wrong);
    assert_eq!(refusal.rule, MODEL_SESSION_RULE);
    assert!(refusal.message.contains("record `candidate`"));
    assert!(refusal
        .message
        .contains("expected model `p/author` at thinking `high`"));
    let right = candidate(
        "session-declaration",
        declaration("author", binding("p/author", "high")),
    );
    assert!(check(&ctx, &right).is_ok());
    let implementer = candidate(
        "session-declaration",
        declaration("implementer", binding("p/else", "x")),
    );
    assert!(check(&ctx, &implementer).is_ok(), "implementer is unbound");
}

#[test]
fn declaration_shape_refuses_each_malformed_cause() {
    let input = input(None);
    let workflow = workflow(&input);
    let ctx = ctx(&input, &workflow, &[]);
    let good = declaration("implementer", binding("p/m", "high"));
    assert!(check(&ctx, &candidate("session-declaration", good.clone())).is_ok());
    for (name, field, value, expected) in [
        (
            "reviewer role",
            "role",
            json!("ordinary-reviewer"),
            "`author` or `implementer`",
        ),
        (
            "driver role",
            "role",
            json!("driver"),
            "`author` or `implementer`",
        ),
        (
            "unknown role",
            "role",
            json!("boss"),
            "`author` or `implementer`",
        ),
        ("blank subject", "subject", json!(" "), "`subject`"),
        (
            "missing revision",
            "subject_revision",
            Value::Null,
            "`subject_revision`",
        ),
        (
            "blank session model",
            "session",
            binding("", "high"),
            "`session`",
        ),
        (
            "session not object",
            "session",
            json!("p/m@high"),
            "`session`",
        ),
    ] {
        let mut bad = good.clone();
        bad[field] = value;
        let refusal = refusal_of(&ctx, &candidate("session-declaration", bad));
        assert_eq!(refusal.rule, "session-declaration-shape", "{name}");
        assert!(
            refusal.message.contains(expected),
            "{name}: {}",
            refusal.message
        );
    }
}

#[test]
fn each_amendment_refusal_cause_is_named_and_leaves_the_context_unchanged() {
    let input = input(Some(
        json!({"roles": {"cold-reader": binding("p/cold", "low")}}),
    ));
    let workflow = workflow(&input);
    let records = vec![stored("note", "user-steering", json!({}), 1)];
    let ctx = ctx(&input, &workflow, &records);
    let add = |role: &str| json!({"role": role, "old": null, "new": binding("p/new", "high")});
    let mut agent = amendment(json!([add("author")]));
    agent["actor"]["kind"] = json!("agent");
    let mut delegated = amendment(json!([add("author")]));
    delegated["authority"] = json!({"kind": "standing-delegation"});
    let mut blank = amendment(json!([add("author")]));
    blank["words"] = json!("  ");
    let cases = [
        ("malformed-amendment", json!([])),
        ("malformed-amendment", amendment(json!([]))),
        ("actor-not-human", agent),
        ("authority-not-owner", delegated),
        ("words-blank", blank),
        ("unknown-role", amendment(json!([add("reviewer")]))),
        (
            "old-binding-not-in-force",
            amendment(json!([{"role": "cold-reader", "old": null, "new": binding("p/x", "low")}])),
        ),
        (
            "old-binding-not-in-force",
            amendment(
                json!([{"role": "cold-reader", "old": binding("p/stale", "low"), "new": null}]),
            ),
        ),
        (
            "new-binding-malformed",
            amendment(json!([{"role": "author", "old": null, "new": binding("p/x", " ")}])),
        ),
        (
            "driver-declaration-missing",
            amendment(json!([add("driver")])),
        ),
        ("driver-declaration-mismatch", {
            let mut data = amendment(json!([add("driver")]));
            data["driver_session"] = binding("p/other", "high");
            data
        }),
        (
            "stale-mark-not-allowed",
            amendment(
                json!([{"role": "author", "old": null, "new": binding("p/x", "low"), "mark_stale": true}]),
            ),
        ),
    ];
    for (code, data) in cases {
        let refusal = refusal_of(&ctx, &candidate("model-binding-amendment", data.clone()));
        assert_eq!(refusal.rule, "model-binding-amendment-valid", "{data}");
        assert!(
            refusal.message.contains(&format!("({code})")),
            "{code}: {}",
            refusal.message
        );
    }
}

#[test]
fn valid_amendments_add_change_and_remove_including_on_an_unbound_run() {
    let unbound = input(None);
    let workflow = workflow(&unbound);
    let ctx = ctx(&unbound, &workflow, &[]);
    let add = amendment(json!([{"role": "author", "old": null, "new": binding("p/a", "low")}]));
    assert!(check(&ctx, &candidate("model-binding-amendment", add.clone())).is_ok());
    let records = vec![stored("a1", "model-binding-amendment", add, 1)];
    let ctx = self::ctx(&unbound, &workflow, &records);
    let change = amendment(json!([{
        "role": "author", "old": binding("p/a", "low"), "new": binding("p/b", "high")
    }]));
    assert!(check(&ctx, &candidate("model-binding-amendment", change.clone())).is_ok());
    let stale_old = amendment(json!([{
        "role": "author", "old": binding("p/other", "low"), "new": null
    }]));
    assert!(
        check(&ctx, &candidate("model-binding-amendment", stale_old))
            .unwrap_err()
            .message
            .contains("(old-binding-not-in-force)")
    );
    let remove = amendment(json!([{"role": "author", "old": binding("p/a", "low"), "new": null}]));
    assert!(check(&ctx, &candidate("model-binding-amendment", remove)).is_ok());
    let driver = {
        let mut data =
            amendment(json!([{"role": "driver", "old": null, "new": binding("p/d", "low")}]));
        data["driver_session"] = binding("p/d", "low");
        data
    };
    assert!(check(&ctx, &candidate("model-binding-amendment", driver)).is_ok());
}

#[test]
fn an_amendment_governs_later_declarations_only_and_never_rejudges_history() {
    let input = input(None);
    let workflow = workflow(&input);
    let before = declaration("author", binding("p/early", "low"));
    let add = amendment(json!([{"role": "author", "old": null, "new": binding("p/late", "high")}]));
    let records = vec![
        stored("early", "session-declaration", before.clone(), 1),
        stored("bind", "model-binding-amendment", add.clone(), 2),
    ];
    let full = ctx(&input, &workflow, &records);
    let classified = classify(&full, &BTreeSet::new(), &[]);
    assert_eq!(classified.len(), 2);
    assert!(classified.iter().all(|(_, result)| result.is_ok()));
    // The early declaration is refused as a new write after the amendment.
    let refusal = refusal_of(&full, &candidate("session-declaration", before));
    assert_eq!(refusal.rule, MODEL_SESSION_RULE);
    let late = candidate(
        "session-declaration",
        declaration("author", binding("p/late", "high")),
    );
    assert!(check(&full, &late).is_ok());
    // The write before the amendment stays judged by the empty prefix.
    let prefix = ctx(&input, &workflow, &records[..1]);
    assert!(check(&prefix, &late).is_ok());
}

#[test]
fn classify_never_rejudges_a_stored_record_against_a_binding_added_or_changed_later() {
    let input = input(Some(
        json!({"roles": {"author": binding("p/author", "high")}}),
    ));
    let workflow = workflow(&input);
    let mismatched = declaration("author", binding("p/elsewhere", "high"));
    let change = amendment(json!([{
        "role": "author", "old": binding("p/author", "high"), "new": binding("p/elsewhere", "high")
    }]));
    let records = vec![
        stored("mismatch", "session-declaration", mismatched, 1),
        stored("change", "model-binding-amendment", change, 2),
    ];
    let full = ctx(&input, &workflow, &records);
    // Stored before the change: refused if written now, valid when classified.
    let write = check(
        &ctx(&input, &workflow, &records[..0]),
        &candidate("session-declaration", records[0].data.clone()),
    );
    assert_eq!(write.unwrap_err().rule, MODEL_SESSION_RULE);
    assert!(classify(&full, &BTreeSet::new(), &[])
        .iter()
        .all(|(_, result)| result.is_ok()));
}
