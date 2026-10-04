use crate::config::InitialInput;
use crate::document::Snapshot;
use loop_core::ContextRecord;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    #[serde(default, rename = "external_review")]
    _external_review: Option<Value>,
    #[serde(default)]
    assessment_id: Option<String>,
    gate: String,
    policy_id: String,
    result: String,
    findings: String,
    author: Author,
    target_id: String,
    target_sha256: String,
    profile_version: String,
}
#[derive(Debug, Deserialize, serde::Serialize, Clone, Hash, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Author {
    name: String,
    kind: String,
}

/// All declarations for these exact bytes contribute authors; no identity registry is inferred.
pub fn target_authorship(context: &[ContextRecord], snapshot: &Snapshot) -> (Value, Value) {
    let mut authors = HashSet::new();
    for record in context.iter().filter(|r| r.kind == "target-authorship") {
        if record.data["target_sha256"] != snapshot.sha256 {
            continue;
        }
        if let Ok(author) = serde_json::from_value::<Author>(record.data["author"].clone()) {
            if !author.name.trim().is_empty()
                && ["human", "agent", "script"].contains(&author.kind.as_str())
            {
                authors.insert(author);
            }
        }
    }
    let mut authors: Vec<_> = authors.into_iter().collect();
    authors.sort_by(|a, b| (&a.name, &a.kind).cmp(&(&b.name, &b.kind)));
    let missing = if authors.is_empty() {
        json!([{"input":"target-authorship","target_sha256":snapshot.sha256,
            "reason":"declare author {name,kind} for the current target bytes; missing, malformed or stale declarations cannot establish eligibility"}])
    } else {
        json!([])
    };
    (json!(authors), missing)
}

#[derive(Debug)]
struct CurrentEvidence {
    assessment_id: Option<String>,
    result: String,
    findings: String,
    author: Author,
    record_id: String,
    sequence: u64,
}

pub fn evaluate(
    context: &[ContextRecord],
    config: &InitialInput,
    snapshot: &Snapshot,
) -> (bool, Value) {
    let (authors, missing_inputs) = target_authorship(context, snapshot);
    if !missing_inputs.as_array().unwrap().is_empty() {
        return (
            false,
            json!({"diagnostics":[{"kind":"missing-target-authorship"}],
            "missing_inputs":missing_inputs,"inert_records":[]}),
        );
    }
    let raw = json!({"semantic_policies":config.semantic_policies.iter().map(|p| json!({"id":p.id,"required_authors":p.required_authors})).collect::<Vec<_>>()});
    let resolution = crate::external::counts(&raw, &json!(context));
    let carried = loop_integrations::review_intake::carry(
        context,
        &snapshot.target_id,
        &snapshot.sha256,
        "profile_version",
        &config.profile_version,
        true,
    );
    let context = &carried;
    let ids: HashSet<&str> = config
        .semantic_policies
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    let mut malformed: HashMap<String, u64> = HashMap::new();
    let mut diagnostics = Vec::new();
    let mut inert = Vec::new();
    let mut valid: HashMap<(String, Author), CurrentEvidence> = HashMap::new();
    let mut latest_conforming: HashMap<String, u64> = HashMap::new();
    for record in context.iter().filter(|r| r.kind == "review-evidence") {
        let object = record.data.as_object();
        let attribution = object
            .and_then(|o| {
                (o.get("gate").and_then(Value::as_str) == Some("semantic-review"))
                    .then(|| o.get("policy_id").and_then(Value::as_str))
            })
            .flatten();
        let Some(policy_id) = attribution.filter(|id| ids.contains(id)) else {
            inert.push(json!({"record_id": record.id, "reason": "unattributable review-evidence"}));
            continue;
        };
        let e = match serde_json::from_value::<Evidence>(record.data.clone()) {
            Ok(evidence) => evidence,
            Err(error) => {
                malformed
                    .entry(policy_id.to_owned())
                    .and_modify(|sequence| *sequence = (*sequence).max(record.sequence.as_u64()))
                    .or_insert(record.sequence.as_u64());
                diagnostics.push(json!({
                    "policy_id": policy_id,
                    "kind": "malformed",
                    "record_id": record.id,
                    "reason": format!("deserialize: {error}"),
                }));
                continue;
            }
        };
        if let Some(reason) = shape_failure_reason(&e, policy_id) {
            malformed
                .entry(policy_id.to_owned())
                .and_modify(|sequence| *sequence = (*sequence).max(record.sequence.as_u64()))
                .or_insert(record.sequence.as_u64());
            diagnostics.push(json!({
                "policy_id": policy_id,
                "kind": "malformed",
                "record_id": record.id,
                "reason": format!("semantic shape: {reason}"),
            }));
            continue;
        }
        latest_conforming
            .entry(policy_id.to_owned())
            .and_modify(|sequence| *sequence = (*sequence).max(record.sequence.as_u64()))
            .or_insert(record.sequence.as_u64());
        if e.profile_version != config.profile_version
            || e.target_id != snapshot.target_id
            || e.target_sha256 != snapshot.sha256
        {
            diagnostics.push(json!({"policy_id": policy_id, "kind": "stale", "record_id": record.id, "reason": "profile, target, or digest does not match current evaluation"}));
            continue;
        }
        let policy = config
            .semantic_policies
            .iter()
            .find(|p| p.id == policy_id)
            .unwrap();
        let is_self = authors
            .as_array()
            .unwrap()
            .iter()
            .any(|author| author["name"] == e.author.name && author["kind"] == e.author.kind);
        if is_self {
            diagnostics.push(json!({"policy_id":policy_id,
                "kind":if policy.self_review { "self-review" } else { "self-excluded" },
                "record_id":record.id,"author":e.author}));
            if !policy.self_review {
                continue;
            }
        }
        let sequence = record.sequence.as_u64();
        let author = e.author.clone();
        let entry = valid.entry((policy_id.to_owned(), author.clone()));
        match entry {
            std::collections::hash_map::Entry::Occupied(mut current)
                if sequence > current.get().sequence =>
            {
                current.insert(CurrentEvidence {
                    assessment_id: e.assessment_id,
                    result: e.result,
                    findings: e.findings,
                    author,
                    record_id: record.id.to_string(),
                    sequence,
                });
            }
            std::collections::hash_map::Entry::Vacant(current) => {
                current.insert(CurrentEvidence {
                    assessment_id: e.assessment_id,
                    result: e.result,
                    findings: e.findings,
                    author,
                    record_id: record.id.to_string(),
                    sequence,
                });
            }
            _ => {}
        }
    }
    let mut failed = false;
    for policy in &config.semantic_policies {
        if resolution
            .blocked
            .contains(&format!("semantic-review.{}", policy.id))
        {
            failed = true;
            diagnostics.push(json!({"policy_id":policy.id,"kind":"invalid-author-count-amendment","amendments":resolution.amendments}));
            continue;
        }
        let required = resolution.effective[format!("semantic-review.{}", policy.id)]
            .as_u64()
            .unwrap_or(policy.required_authors);
        let malformed_active =
            malformed
                .get(policy.id.as_str())
                .is_some_and(|malformed_sequence| {
                    latest_conforming
                        .get(policy.id.as_str())
                        .is_none_or(|conforming_sequence| conforming_sequence <= malformed_sequence)
                });
        if malformed_active {
            failed = true;
            continue;
        }
        let mut verdicts: Vec<&CurrentEvidence> = valid
            .iter()
            .filter(|((id, _), _)| id == policy.id.as_str())
            .map(|(_, evidence)| evidence)
            .collect();
        verdicts.sort_by_key(|evidence| evidence.sequence);
        let has_fail = verdicts.iter().any(|evidence| evidence.result == "fail");
        let has_pass = verdicts
            .iter()
            .filter(|evidence| {
                if evidence.result != "pass" {
                    return false;
                }
                let peers: Vec<_> = config
                    .semantic_policies
                    .iter()
                    .filter(|p| {
                        p.group.as_ref().unwrap_or(&p.id)
                            == policy.group.as_ref().unwrap_or(&policy.id)
                    })
                    .collect();
                peers.len() == 1
                    || (evidence
                        .assessment_id
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty())
                        && peers.iter().all(|p| {
                            valid
                                .get(&(p.id.clone(), evidence.author.clone()))
                                .is_some_and(|other| {
                                    other.result == "pass"
                                        && other.assessment_id == evidence.assessment_id
                                })
                        }))
            })
            .count() as u64
            >= required;
        if has_fail || !has_pass {
            failed = true;
            if has_fail {
                for evidence in verdicts.iter().filter(|evidence| evidence.result == "fail") {
                    diagnostics.push(json!({
                        "policy_id": policy.id,
                        "kind": "standing-fail",
                        "findings": evidence.findings,
                        "author": evidence.author,
                        "record_id": evidence.record_id,
                    }));
                }
            } else {
                diagnostics.push(json!({"policy_id": policy.id, "kind": "missing"}));
            }
        }
    }
    (
        !failed,
        json!({"diagnostics": diagnostics, "inert_records": inert, "missing_inputs":[], "target_authors":authors}),
    )
}
fn shape_failure_reason(evidence: &Evidence, policy_id: &str) -> Option<String> {
    let mut failures = Vec::new();
    if evidence.gate != "semantic-review" {
        failures.push("gate must be semantic-review");
    }
    if evidence.policy_id != policy_id {
        failures.push("policy_id must match attributed policy_id");
    }
    if evidence.result != "pass" && evidence.result != "fail" {
        failures.push("result must be pass or fail");
    }
    if evidence.author.name.trim().is_empty() {
        failures.push("author.name must be non-empty");
    }
    if !["human", "agent", "script"].contains(&evidence.author.kind.as_str()) {
        failures.push("author.kind must be human, agent, or script");
    }
    if evidence.target_id.trim().is_empty() {
        failures.push("target_id must be non-empty");
    }
    if !valid_digest(&evidence.target_sha256) {
        failures.push("target_sha256 must be 64 lowercase hexadecimal characters");
    }
    if evidence.profile_version.trim().is_empty() {
        failures.push("profile_version must be non-empty");
    }
    if evidence.result == "fail" && evidence.findings.trim().is_empty() {
        failures.push("findings must be non-empty when result is fail");
    }
    (!failures.is_empty()).then(|| failures.join("; "))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::evaluate as evaluate_raw;
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;
    use std::path::PathBuf;

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn authorship(sequence: u64, digest: &str, author: &str) -> ContextRecord {
        ContextRecord::new(
            format!("authorship-{sequence}"),
            "target-authorship",
            json!({"target_sha256":digest,"author":{"name":author,"kind":"agent"}}),
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn evaluate(
        context: &[ContextRecord],
        config: &InitialInput,
        snapshot: &Snapshot,
    ) -> (bool, serde_json::Value) {
        let mut context = context.to_vec();
        context.push(authorship(0, &snapshot.sha256, "drafter"));
        evaluate_raw(&context, config, snapshot)
    }

    #[test]
    fn old_carry_leaves_current_failure_standing() {
        let cfg = config();
        let snap = snapshot();
        let mut old = record(1, "axis-a", "pass", "reader");
        old.data["target_sha256"] = json!("b".repeat(64));
        let mut context = vec![
            old,
            record(2, "axis-a", "fail", "reader"),
            record(3, "axis-b", "pass", "reader"),
        ];
        context.push(ContextRecord::new("carry", "evidence-applicability", json!({"origin":{"kind":"context-record","id":"record-1"},"target":{"subject":"doc","revision":DIGEST},"attesting_driver":{"name":"driver","kind":"human"},"reason":"unchanged"}), SemanticSequence::new(4), Timestamp::from_unix_millis(4)));
        let (ok, details) = evaluate(&context, &cfg, &snap);
        assert!(!ok);
        assert!(details.to_string().contains("standing-fail"));
        assert!(details.to_string().contains("material"));
        context.push(record(5, "axis-a", "pass", "reader"));
        assert!(evaluate(&context, &cfg, &snap).0);
    }

    #[test]
    fn current_authorship_is_required_and_self_review_is_explicit() {
        let mut cfg = config();
        let snap = snapshot();
        let mut context = vec![
            record(1, "axis-a", "pass", "one"),
            record(2, "axis-b", "pass", "one"),
        ];
        let (ok, details) = evaluate_raw(&context, &cfg, &snap);
        assert!(!ok);
        assert_eq!(details["missing_inputs"][0]["input"], "target-authorship");
        context.push(authorship(3, &"b".repeat(64), "old-drafter"));
        assert!(!evaluate_raw(&context, &cfg, &snap).0);
        context.push(authorship(4, &snap.sha256, "one"));
        let (ok, details) = evaluate_raw(&context, &cfg, &snap);
        assert!(!ok);
        assert_eq!(details["diagnostics"][0]["kind"], "self-excluded");
        for policy in &mut cfg.semantic_policies {
            policy.self_review = true;
        }
        let (ok, details) = evaluate_raw(&context, &cfg, &snap);
        assert!(ok);
        assert_eq!(details["diagnostics"][0]["kind"], "self-review");
        context.push(authorship(5, &snap.sha256, "coauthor"));
        assert_eq!(
            super::target_authorship(&context, &snap)
                .0
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    fn config() -> InitialInput {
        InitialInput::parse(&json!({
            "schema_version": 1,
            "profile_version": "test-1",
            "mode": "audit",
            "target": {"id": "doc", "path": "/tmp/doc"},
            "deterministic_policies": [{"id":"present","type":"non-empty"}],
            "semantic_policies": [
                {"id":"axis-a","description":"a","example_prompt":"a"},
                {"id":"axis-b","description":"b","example_prompt":"b"}
            ]
        }))
        .unwrap()
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            target_id: "doc".into(),
            path: PathBuf::from("/tmp/doc"),
            text: "ok".into(),
            sha256: DIGEST.into(),
        }
    }

    fn record(sequence: u64, policy: &str, result: &str, author: &str) -> ContextRecord {
        ContextRecord::new(
            format!("record-{sequence}"),
            "review-evidence",
            json!({"gate":"semantic-review","policy_id":policy,"result":result,"findings":if result == "fail" { "material" } else { "" },"author":{"name":author,"kind":"agent"},"target_id":"doc","target_sha256":DIGEST,"profile_version":"test-1"}),
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    #[test]
    fn requires_pass_for_every_axis_and_rejects_standing_fail() {
        let cfg = config();
        let snap = snapshot();
        let (ok, details) = evaluate(&[record(1, "axis-a", "pass", "one")], &cfg, &snap);
        assert!(!ok);
        assert_eq!(details["diagnostics"][0]["policy_id"], "axis-b");
        let context = [
            record(1, "axis-a", "pass", "one"),
            record(2, "axis-a", "fail", "two"),
            record(3, "axis-b", "pass", "one"),
        ];
        let (ok, details) = evaluate(&context, &cfg, &snap);
        assert!(!ok);
        assert!(details["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "standing-fail"));
    }

    #[test]
    fn standing_fail_details_are_actionable_and_deterministic() {
        let cfg = config();
        let snap = snapshot();
        let mut first = record(4, "axis-a", "fail", "zeta");
        first.id = "fail-zeta".into();
        first.data["findings"] = json!("fix zeta issue");
        let mut second = record(2, "axis-a", "fail", "alpha");
        second.id = "fail-alpha".into();
        second.data["findings"] = json!("fix alpha issue");
        let context = [first, record(5, "axis-b", "pass", "reviewer"), second];
        let (ok, details) = evaluate(&context, &cfg, &snap);
        assert!(!ok);
        let failures: Vec<&Value> = details["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "standing-fail")
            .collect();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["record_id"], "fail-alpha");
        assert_eq!(failures[0]["findings"], "fix alpha issue");
        assert_eq!(failures[0]["author"]["name"], "alpha");
        assert_eq!(failures[1]["record_id"], "fail-zeta");
    }

    #[test]
    fn latest_exact_author_verdict_stands_but_other_author_fail_remains() {
        let cfg = config();
        let snap = snapshot();
        let corrected = [
            record(1, "axis-a", "fail", "one"),
            record(2, "axis-a", "pass", "one"),
            record(3, "axis-b", "pass", "one"),
        ];
        assert!(evaluate(&corrected, &cfg, &snap).0);
        let blocked = [
            record(1, "axis-a", "fail", "other"),
            record(2, "axis-a", "pass", "one"),
            record(3, "axis-b", "pass", "one"),
        ];
        assert!(!evaluate(&blocked, &cfg, &snap).0);
    }

    #[test]
    fn later_shape_conforming_stale_record_clears_malformed_block() {
        let cfg = config();
        let snap = snapshot();
        let mut malformed = record(2, "axis-a", "pass", "bad");
        malformed.data.as_object_mut().unwrap().remove("author");
        let mut stale = record(3, "axis-a", "pass", "stale");
        stale.data["target_sha256"] =
            json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let context = [
            record(1, "axis-a", "pass", "current"),
            malformed,
            stale,
            record(4, "axis-b", "pass", "current"),
        ];
        assert!(evaluate(&context, &cfg, &snap).0);
    }

    #[test]
    fn malformed_deserialization_diagnostic_has_exact_actionable_reason() {
        let cfg = config();
        let snap = snapshot();
        let mut malformed = record(1, "axis-a", "pass", "bad");
        malformed.data.as_object_mut().unwrap().remove("author");
        let (ok, details) = evaluate(&[malformed], &cfg, &snap);
        assert!(!ok);
        assert_eq!(
            details["diagnostics"][0]["reason"],
            "deserialize: missing field `author`"
        );
    }

    #[test]
    fn malformed_semantic_shape_diagnostic_has_exact_actionable_reason() {
        let cfg = config();
        let snap = snapshot();
        let mut malformed = record(1, "axis-a", "fail", "bad");
        malformed.data["findings"] = json!("");
        let (ok, details) = evaluate(&[malformed], &cfg, &snap);
        assert!(!ok);
        assert_eq!(
            details["diagnostics"][0]["reason"],
            "semantic shape: findings must be non-empty when result is fail"
        );
    }

    #[test]
    fn malformed_and_stale_records_never_satisfy_missing_axis() {
        let cfg = config();
        let snap = snapshot();
        let mut malformed = record(1, "axis-a", "fail", "bad");
        malformed.data["findings"] = json!("");
        let mut stale = record(2, "axis-a", "pass", "stale");
        stale.data["profile_version"] = json!("old");
        let (ok, details) = evaluate(
            &[malformed, stale, record(3, "axis-b", "pass", "ok")],
            &cfg,
            &snap,
        );
        assert!(!ok);
        assert!(details["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "missing"));
    }

    #[test]
    fn wrong_profile_target_and_digest_are_stale_and_unattributable_is_inert() {
        let cfg = config();
        let snap = snapshot();
        let mut context = Vec::new();
        for (sequence, field, value) in [
            (1, "profile_version", "old"),
            (2, "target_id", "other"),
            (
                3,
                "target_sha256",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
        ] {
            let mut item = record(sequence, "axis-a", "pass", field);
            item.data[field] = json!(value);
            context.push(item);
        }
        context.push(ContextRecord::new(
            "inert",
            "review-evidence",
            json!({"gate":"other","policy_id":"axis-a"}),
            SemanticSequence::new(4),
            Timestamp::from_unix_millis(4),
        ));
        context.push(record(5, "axis-b", "pass", "ok"));
        let (ok, details) = evaluate(&context, &cfg, &snap);
        assert!(!ok);
        assert_eq!(
            details["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["kind"] == "stale")
                .count(),
            3
        );
        assert_eq!(details["inert_records"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn self_authored_and_all_supported_author_kinds_are_shape_valid() {
        let cfg = config();
        let snap = snapshot();
        for kind in ["human", "agent", "script"] {
            let mut item = record(1, "axis-a", "pass", "target-author");
            item.data["author"]["kind"] = json!(kind);
            let context = [item, record(2, "axis-b", "pass", "other")];
            assert!(evaluate(&context, &cfg, &snap).0);
        }
    }
}
