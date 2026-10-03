use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn invoke(binary: &str, args: &[String], packet: &Value) -> Value {
    let mut child = Command::new(workspace_integration::binary(binary))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(packet).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{binary} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn record(id: &str, kind: &str, data: Value, sequence: u64) -> Value {
    json!({"id":id,"kind":kind,"data":data,"sequence":sequence,"created_at":1})
}

fn exercise(binary: &str) {
    let policy = binary == "policy-document";
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join(if policy {
        "target.md"
    } else {
        "verification.json"
    });
    let author = json!({"name":"owner","kind":"human"});
    let schema = json!({"type":"object","properties":{"revision":{"type":"string","minLength":1},"author":{"type":"object","properties":{"name":{"type":"string"},"kind":{"type":"string","enum":["human","agent","script"]}},"required":["name","kind"],"additionalProperties":false}},"required":["revision","author"],"additionalProperties":false});
    std::fs::write(
        &target,
        if policy {
            "# Title\n".into()
        } else {
            json!({"revision":"r1","author":author}).to_string()
        },
    )
    .unwrap();
    let mut initial = if policy {
        json!({"schema_version":1,"profile_version":"test-successor","mode":"audit","target":{"id":"doc","path":target},"target_author":author,"deterministic_policies":[{"id":"title","type":"any-heading","level":1}],"semantic_policies":[{"id":"axis","description":"quality","example_prompt":"review quality"}]})
    } else {
        json!({"config_version":"test-successor","artifact_root":root.path(),"review_policies":{"verify":[{"id":"axis","description":"quality"}]},"artifact_schemas":{"verification.json":schema}})
    };
    let workflow = invoke(binary, &[], &json!({"operation":"describe"}));
    let gate = if policy { "semantic-review" } else { "verify" };
    let transition = if policy {
        json!({"source":gate,"event":"passed","target":"end","kind":"checked"})
    } else {
        json!({"source":gate,"event":"verified","target":"synthesize","kind":"checked"})
    };
    let mut request = json!({"operation":"evaluate","workflow":workflow,"initial_input":initial,"transition":transition,"context":[],"prior_evaluations":[]});
    let denied = invoke(binary, &[], &request);
    assert_eq!(denied["result"], "deny");
    let subject = if policy { "doc" } else { "verification.json" };
    let revision = if policy {
        denied["feedback"]["details"]["target_sha256"]
            .as_str()
            .unwrap()
            .to_owned()
    } else {
        "r1".into()
    };
    let evidence = |name: &str, result: &str| {
        let a = json!({"name":name,"kind":if name == "owner" {"human"} else {"agent"}});
        if policy {
            json!({"gate":gate,"policy_id":"axis","result":result,"findings":if result == "fail" {"material defect"} else {""},"author":a,"target_id":subject,"target_sha256":revision,"profile_version":"test-successor"})
        } else {
            json!({"gate":gate,"policy_id":"axis","result":result,"findings":if result == "fail" {"material defect"} else {""},"author":a,"subject":subject,"subject_revision":revision,"config_version":"test-successor"})
        }
    };
    let policy_path = if policy {
        "/semantic_policies/0"
    } else {
        "/review_policies/verify/0"
    };
    request["context"] = json!([record(
        "self",
        "review-evidence",
        evidence("owner", "pass"),
        1
    )]);
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    initial.pointer_mut(policy_path).unwrap()["self_review"] = json!(true);
    request["initial_input"] = initial.clone();
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    initial.pointer_mut(policy_path).unwrap()["self_review"] = json!(false);
    if !policy {
        request["initial_input"] = initial.clone();
        request["context"] = json!([record("one", "review-evidence", evidence("one", "pass"), 1)]);
        for artifact in ["brief.json", "sources.json", "report.json"] {
            std::fs::write(
                root.path().join(artifact),
                json!({"revision":"a1","author":{"name":"one","kind":"agent"}}).to_string(),
            )
            .unwrap();
            assert_eq!(invoke(binary, &[], &request)["result"], "deny");
            request["initial_input"].pointer_mut(policy_path).unwrap()["self_review"] = json!(true);
            assert_eq!(invoke(binary, &[], &request)["result"], "allow");
            request["initial_input"] = initial.clone();
            std::fs::remove_file(root.path().join(artifact)).unwrap();
        }
    }
    initial.pointer_mut(policy_path).unwrap()["required_authors"] = json!(2);
    request["initial_input"] = initial.clone();
    request["context"] = json!([
        record("one", "review-evidence", evidence("one", "pass"), 1),
        record("duplicate", "review-evidence", evidence("one", "pass"), 2)
    ]);
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    request["context"][1] = record("two", "review-evidence", evidence("two", "pass"), 2);
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    let amendment = |before: u64, after: u64| json!({"targets":[{"gate":gate,"axis":"axis"}],"before":before,"after":after,"actor":author,"authority":{"kind":"owner","reference":"owner approval"},"reason":"adjust review floor"});
    request["context"] = json!([
        record("one", "review-evidence", evidence("one", "pass"), 1),
        record("decrease", "author-count-amendment", amendment(2, 1), 2)
    ]);
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    request["context"].as_array_mut().unwrap().push(record(
        "failure",
        "review-evidence",
        evidence("two", "fail"),
        3,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    request["context"].as_array_mut().unwrap().pop();
    request["context"].as_array_mut().unwrap().push(record(
        "increase",
        "author-count-amendment",
        amendment(1, 2),
        3,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    request["context"].as_array_mut().unwrap().push(record(
        "second",
        "review-evidence",
        evidence("two", "pass"),
        4,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    request["context"].as_array_mut().unwrap().push(record(
        "bad",
        "author-count-amendment",
        amendment(99, 1),
        5,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    let mut corrected = amendment(2, 2);
    corrected["corrects"] = json!("bad");
    request["context"].as_array_mut().unwrap().push(record(
        "correction",
        "author-count-amendment",
        corrected,
        6,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    let mut unknown = amendment(2, 2);
    unknown["targets"][0]["axis"] = json!("unknown");
    request["context"].as_array_mut().unwrap().push(record(
        "unknown",
        "author-count-amendment",
        unknown,
        7,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    let mut fix_unknown = amendment(2, 2);
    fix_unknown["corrects"] = json!("unknown");
    request["context"].as_array_mut().unwrap().push(record(
        "fix-unknown",
        "author-count-amendment",
        fix_unknown,
        8,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    request["context"].as_array_mut().unwrap().push(record(
        "standing",
        "review-evidence",
        evidence("third", "fail"),
        9,
    ));
    request["context"].as_array_mut().unwrap().push(record(
        "decrease-with-fail",
        "author-count-amendment",
        amendment(2, 1),
        10,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    request["context"].as_array_mut().unwrap().push(record(
        "increase-with-fail",
        "author-count-amendment",
        amendment(1, 2),
        11,
    ));
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    for _ in 0..3 {
        request["context"].as_array_mut().unwrap().pop();
    }
    let show = |initial: &Value, context: &Value| json!({"operation":"show","status":"completed","result":{"workflow_id":binary,"initial_input":initial,"context":context,"current_state":"prepare"}});
    let inspection = invoke(
        binary,
        &["author-counts".into()],
        &show(&initial, &request["context"]),
    );
    assert_eq!(inspection["effective"][format!("{gate}.axis")], 2);
    let mut proposal = amendment(2, 1);
    proposal.as_object_mut().unwrap().remove("before");
    assert_eq!(
        invoke(
            binary,
            &[
                "author-counts".into(),
                "--propose".into(),
                proposal.to_string()
            ],
            &show(&initial, &request["context"])
        )["data"]["before"],
        2
    );
    // Explicit applicability changes only freshness, never deterministic checks.
    let new_revision = if policy {
        std::fs::write(&target, "# Changed\n").unwrap();
        ({
            let mut fresh = request.clone();
            fresh["context"] = json!([]);
            invoke(binary, &[], &fresh)
        })["feedback"]["details"]["target_sha256"]
            .as_str()
            .unwrap()
            .to_owned()
    } else {
        std::fs::write(
            &target,
            json!({"revision":"r2","author":author}).to_string(),
        )
        .unwrap();
        "r2".into()
    };
    assert_eq!(invoke(binary, &[], &request)["result"], "deny");
    for (id, seq) in [("one", 12), ("second", 13)] {
        request["context"].as_array_mut().unwrap().push(record(&format!("carry-{id}"),"evidence-applicability",json!({"origin":{"kind":"context-record","id":id},"target":{"subject":subject,"revision":new_revision},"attesting_driver":author,"reason":"unchanged judgment scope"}),seq));
    }
    assert_eq!(invoke(binary, &[], &request)["result"], "allow");
    if policy {
        std::fs::write(&target, "no heading").unwrap();
    } else {
        std::fs::write(
            &target,
            json!({"revision":"r2","author":author,"unexpected":true}).to_string(),
        )
        .unwrap();
    }
    let deterministic_denial = invoke(binary, &[], &request);
    assert_eq!(deterministic_denial["result"], "deny");
    assert_eq!(
        deterministic_denial["feedback"]["code"],
        if policy {
            "policy-document-nonconforming"
        } else {
            "research-schema-invalid"
        }
    );
    if policy {
        std::fs::write(&target, "# Changed\n").unwrap();
    } else {
        std::fs::write(
            &target,
            json!({"revision":"r2","author":author}).to_string(),
        )
        .unwrap();
    }
    let intake = json!({"show":show(&initial,&request["context"]),"assignment":{"gate":gate,"review_stage":"review","subject":if policy {new_revision.as_str()} else {subject},"subject_revision":new_revision,"policy_ids":["axis"]},"author":{"name":"external","kind":"agent"},"request":{"text":"Review quality."},"result":{"text":"Looks good to me, for these reasons."},"projection":{"items":[{"policy_id":"axis","result":"pass","findings":""},{"policy_id":"axis","result":"bad","findings":""}]}});
    let mut intake = intake;
    let mut contradictory = json!({"policy_id":"axis","result":"pass","findings":""});
    contradictory[if policy {
        "target_sha256"
    } else {
        "config_version"
    }] = json!("contradictory");
    intake["projection"]["items"]
        .as_array_mut()
        .unwrap()
        .push(contradictory);
    let projected = invoke(binary, &["review-candidates".into()], &intake);
    assert_eq!(projected["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(projected["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(projected["original"]["original"], intake["result"]["text"]);
    assert_eq!(projected["request"]["original"], intake["request"]["text"]);
    let mut current_other = evidence("another", "pass");
    current_other[if policy {
        "target_sha256"
    } else {
        "subject_revision"
    }] = json!(new_revision);
    let mut completed_intake = request.clone();
    completed_intake["context"] = json!([
        record(
            "external",
            "review-evidence",
            projected["candidates"][0]["data"].clone(),
            1
        ),
        record("other", "review-evidence", current_other, 2)
    ]);
    assert_eq!(invoke(binary, &[], &completed_intake)["result"], "allow");
    let mut combined = intake.clone();
    let mut grouped_initial = initial.clone();
    let second = json!({"id":"axis-two","description":"second","example_prompt":"second","group":"combined","self_review":true});
    grouped_initial.pointer_mut(policy_path).unwrap()["group"] = json!("combined");
    grouped_initial.pointer_mut(policy_path).unwrap()["self_review"] = json!(true);
    let entries = if policy {
        &mut grouped_initial["semantic_policies"]
    } else {
        &mut grouped_initial["review_policies"][gate]
    };
    entries.as_array_mut().unwrap().push(second);
    combined["show"] = show(&grouped_initial, &json!([]));
    combined["assignment"]["policy_ids"] = json!(["axis", "axis-two"]);
    combined["assignment"]["assessment_id"] = json!("combined-assessment");
    combined["self_review"] = json!(true);
    combined["projection"]["items"] = json!([{"policy_id":"axis","result":"pass","findings":""},{"policy_id":"axis-two","result":"pass","findings":""}]);
    let grouped = invoke(binary, &["review-candidates".into()], &combined);
    assert_eq!(grouped["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        grouped["candidates"][0]["assessment_id"],
        grouped["candidates"][1]["assessment_id"]
    );
    assert_eq!(grouped["candidates"][0]["self_review"], true);
    let commission_args = if policy {
        vec!["commission".into(), initial.to_string()]
    } else {
        vec!["commission".into(), "verify".into()]
    };
    let early = invoke(binary, &commission_args, &show(&initial, &json!([])));
    assert!(early["missing_inputs"].is_array());
    // Early commissioning succeeds even before review artifacts exist.
    std::fs::remove_file(&target).unwrap();
    let args = if policy {
        vec!["commission".into(), initial.to_string()]
    } else {
        vec!["commission".into(), "verify".into()]
    };
    let commissioned = invoke(binary, &args, &show(&initial, &json!([])));
    assert!(!commissioned["missing_inputs"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn policy_document_external_review_contract() {
    exercise("policy-document");
}
#[test]
fn research_external_review_contract() {
    exercise("research");
}
