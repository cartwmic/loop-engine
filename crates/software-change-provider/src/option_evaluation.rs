//! Admission, byte freshness and owner routing for option-enabled phases.
//! These are provider facts over ordinary context, not engine approval semantics.
//!
//! Refusals: artifact-admission-invalid means reuse/missing revision;
//! artifact-stale means latest admission/approval identity or bytes mismatch;
//! narrative-binding-invalid means schema/local invariants or approval citation
//! mismatch; narrative-admission-invalid means a reused composite revision;
//! narrative-stale means changed markdown or an older admission restored;
//! owner-decision-required means the latest decision does not authorize this
//! exact checkpoint/subject/revision/route (diagnostics accompany that denial).
//! All codes have the `software-change-` prefix. Ordinary evidence, ledger,
//! repository checkpoint, criterion and Bookends codes remain unchanged.
use crate::artifacts::{self, ArtifactDocument, ArtifactReadOutcome};
use crate::config::ValidatedConfig;
use crate::gates::EvaluationOutcome;
use crate::narratives;
use crate::narratives::{latest_phase_record, phase_record_fact};
use crate::protocol::{allow_response, deny_response, EvaluateRequest};
use crate::workflow::{self, PhaseOptions, TransitionDuties};
use loop_core::ContextRecord;
use loop_integrations::owner_checkpoint::{self, Decision, Status};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

// Public refusal codes for T7 assertions and T9 operator documentation.
const ARTIFACT_ADMISSION: &str = "software-change-artifact-admission-invalid";
const ARTIFACT_FRESHNESS: &str = "software-change-artifact-stale";
const NARRATIVE_BINDING: &str = "software-change-narrative-binding-invalid";
const NARRATIVE_ADMISSION: &str = "software-change-narrative-admission-invalid";
const NARRATIVE_FRESHNESS: &str = "software-change-narrative-stale";
const OWNER_DECISION: &str = "software-change-owner-decision-required";

pub(crate) struct Prepared {
    pub duties: TransitionDuties,
    pub excluded_applicability: BTreeSet<String>,
    pub immediate: Option<Value>,
    artifact_admission: Option<Value>,
    approved_artifact: Option<Value>,
    narrative_admission: Option<Value>,
}

fn deny(code: &str, diagnostic: impl Into<String>) -> EvaluationOutcome {
    EvaluationOutcome::Response(deny_response(code, diagnostic.into(), None))
}

fn read(config: &ValidatedConfig, subject: &str) -> Result<ArtifactDocument, EvaluationOutcome> {
    match artifacts::read_artifact(config.artifact_root(), subject) {
        ArtifactReadOutcome::Present(document) => Ok(document),
        ArtifactReadOutcome::Deny(error) => {
            Err(deny("software-change-schema-invalid", error.to_string()))
        }
        ArtifactReadOutcome::EvaluationError(error) => {
            Err(EvaluationOutcome::EvaluationError(error.to_string()))
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn artifact_fact(document: &ArtifactDocument) -> Result<Value, EvaluationOutcome> {
    let revision = document.value()["revision"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| deny(ARTIFACT_ADMISSION, "artifact must have a nonempty revision"))?;
    Ok(
        json!({"subject": document.subject(), "revision": revision, "sha256": digest(document.bytes())}),
    )
}

use crate::record_facts::{valid_narrative_fact, validate_artifact_record};

fn require_artifact(
    context: &[ContextRecord],
    phase: &str,
    current: &Value,
    kind: &str,
) -> Result<(), EvaluationOutcome> {
    let (record, fact) =
        latest_phase_record(context, kind, phase, current["subject"].as_str().unwrap())
            .ok_or_else(|| {
                deny(
                    ARTIFACT_FRESHNESS,
                    format!("missing latest {kind} for {phase}"),
                )
            })?;
    if !validate_artifact_record(
        record,
        fact,
        kind,
        phase,
        current["subject"].as_str().unwrap(),
    ) || fact["revision"] != current["revision"]
        || fact["sha256"] != current["sha256"]
    {
        return Err(deny(
            ARTIFACT_FRESHNESS,
            format!("current artifact must match the latest {kind} revision and sha256"),
        ));
    }
    Ok(())
}

/// Deny a checked author-draft or implementer-report boundary when that role
/// is bound and the latest `session-declaration` for the current subject
/// revision is missing or differs from the binding in force now. An unbound
/// role, or an artifact the ordinary checks will report as unreadable, adds
/// nothing here.
pub(crate) fn session_declaration_denial(
    request: &EvaluateRequest,
    duties: TransitionDuties,
) -> Option<EvaluationOutcome> {
    let (role, subject) = workflow::declaration_boundary(
        request.transition.source.as_str(),
        request.transition.event.as_str(),
        duties,
    )?;
    let bindings = match loop_core::fold_model_bindings(&request.initial_input, &request.context) {
        Ok(bindings) => bindings,
        Err(error) => return Some(deny(ARTIFACT_ADMISSION, error)),
    };
    let binding = bindings.in_force().remove(&role)?;
    let document =
        match artifacts::read_artifact(request.initial_input.get("artifact_root"), subject) {
            ArtifactReadOutcome::Present(document) => document,
            _ => return None,
        };
    let revision = document.value()["revision"]
        .as_str()
        .filter(|revision| !revision.trim().is_empty())?;
    let declared = request
        .context
        .iter()
        .filter(|record| {
            record.kind == loop_core::SESSION_DECLARATION_RECORD_KIND
                && record.data["role"] == role.as_str()
                && record.data["subject"] == subject
                && record.data["subject_revision"] == revision
        })
        .max_by_key(|record| record.sequence)
        .and_then(|record| loop_core::Session::from_value(&record.data["session"]).ok());
    loop_core::check_declared_session(role, Some(&binding), declared.as_ref())
        .err()
        .map(|mismatch| {
            deny(
                ARTIFACT_ADMISSION,
                format!(
                    "{subject} revision `{revision}` needs a matching session-declaration: {mismatch}"
                ),
            )
        })
}

/// Only applicability is cut off. The immutable original reviewer record,
/// never the declaration append time, determines eligibility. Direct failures
/// and exact-source ledger dispositions remain in the full context.
pub(crate) fn excluded_applicability(
    context: &[ContextRecord],
    options: &PhaseOptions,
    narrative_enabled: bool,
) -> BTreeSet<String> {
    let cutoff = context
        .iter()
        .filter_map(|r| {
            if r.kind != workflow::OWNER_DECISION_KIND {
                return None;
            }
            let decision = owner_checkpoint::parse(&r.data, &["artifact", "narrative"]).ok()?;
            (decision.checkpoint == options.checkpoint
                && decision.subject == workflow::checkpoint_subject(options, narrative_enabled)
                && decision.decision == Decision::Reject
                && decision.target.as_deref() == Some("artifact"))
            .then_some(r.sequence)
        })
        .max();
    let Some(cutoff) = cutoff else {
        return BTreeSet::new();
    };
    context
        .iter()
        .filter(|r| {
            r.kind == "evidence-applicability"
                && r.data["target"]["subject"] == options.phase.subject
                && context.iter().any(|source| {
                    source.kind == "review-evidence"
                        && source.id.as_str() == r.data["origin"]["id"].as_str().unwrap_or("")
                        && source.data["subject"] == options.phase.subject
                        && [
                            options.phase.parent_review,
                            options.phase.adversarial_review,
                        ]
                        .contains(&source.data["gate"].as_str().unwrap_or(""))
                        && source.sequence < cutoff
                })
        })
        .map(|r| r.id.as_str().to_owned())
        .collect()
}

pub(crate) fn prepare(
    request: &EvaluateRequest,
    config: &ValidatedConfig,
    duties: TransitionDuties,
    options: &PhaseOptions,
) -> Result<Prepared, EvaluationOutcome> {
    let phase = options.phase;
    let mut selected = BTreeSet::new();
    for kind in [
        workflow::ARTIFACT_ADMITTED_KIND,
        workflow::APPROVED_ARTIFACT_KIND,
        workflow::NARRATIVE_ADMITTED_KIND,
    ] {
        if let Some((record, _)) =
            latest_phase_record(&request.context, kind, phase.name, phase.subject)
        {
            selected.insert(record.id.as_str().to_owned());
        }
    }
    if let Some(record) = request
        .context
        .iter()
        .filter(|record| {
            record.kind == workflow::OWNER_DECISION_KIND
                && record.data["checkpoint"] == options.checkpoint
                && owner_checkpoint::parse(&record.data, &["artifact", "narrative"]).is_ok()
        })
        .max_by_key(|record| record.sequence)
    {
        selected.insert(record.id.as_str().to_owned());
    }
    let ctx = crate::admission::AdmissionContext::new(
        &request.initial_input,
        &request.context,
        request.transition.source.as_str(),
        0,
        &request.workflow,
    )
    .map_err(|error| deny(ARTIFACT_FRESHNESS, error))?;
    for (_, verdict) in
        crate::admission::classify_selected(&ctx, &BTreeSet::new(), Some(&selected), &[])
    {
        if let Err(refusal) = verdict {
            return Err(deny(
                ARTIFACT_FRESHNESS,
                format!("{}: {}", refusal.rule, refusal.message),
            ));
        }
    }
    let source = request.transition.source.as_str();
    let event = request.transition.event.as_str();
    let target = request.transition.target.as_str();
    let narrative_enabled =
        workflow::option_enabled(Some(&request.initial_input), "narratives", phase.name);
    let exit = workflow::phase_exit_duties(source, event, target);
    let mut prepared = Prepared {
        duties,
        excluded_applicability: BTreeSet::new(),
        immediate: None,
        artifact_admission: None,
        approved_artifact: None,
        narrative_admission: None,
    };
    if duties == TransitionDuties::CheckFree {
        return Ok(prepared);
    }

    if source == options.checkpoint {
        let subject = workflow::checkpoint_subject(options, narrative_enabled);
        let kind = if narrative_enabled {
            workflow::NARRATIVE_ADMITTED_KIND
        } else {
            workflow::APPROVED_ARTIFACT_KIND
        };
        let (record, fact) = latest_phase_record(&request.context, kind, phase.name, subject)
            .ok_or_else(|| {
                deny(
                    OWNER_DECISION,
                    "checkpoint subject has no latest approved/admitted identity",
                )
            })?;
        if (narrative_enabled && !valid_narrative_fact(fact, phase.name))
            || (!narrative_enabled
                && !validate_artifact_record(record, fact, kind, phase.name, subject))
        {
            return Err(deny(
                OWNER_DECISION,
                "checkpoint subject approval/admission fact is malformed",
            ));
        }
        let revision = fact["revision"]
            .as_str()
            .ok_or_else(|| deny(OWNER_DECISION, "checkpoint subject revision is malformed"))?;
        let targets: &[&str] = if narrative_enabled {
            &["artifact", "narrative"]
        } else {
            &["artifact"]
        };
        let evaluation = owner_checkpoint::evaluate(
            &request.context,
            options.checkpoint,
            subject,
            revision,
            targets,
        );
        let matches = match event {
            "approved" | "passed" => evaluation.status == Status::Approved,
            "revise" => {
                matches!(&evaluation.status, Status::Rejected { target, .. } if target == "artifact")
            }
            "revise-narrative" => {
                matches!(&evaluation.status, Status::Rejected { target, .. } if target == "narrative")
            }
            _ => false,
        };
        if !matches {
            return Err(EvaluationOutcome::Response(deny_response(
                OWNER_DECISION,
                "checkpoint exit requires the latest matching owner decision",
                Some(
                    json!({"checkpoint":options.checkpoint,"subject":subject,"revision":revision,"status":format!("{:?}",evaluation.status),
                    "diagnostics":evaluation.diagnostics.iter().map(|d| json!({"record_id":d.record_id,"message":d.message})).collect::<Vec<_>>()}),
                ),
            )));
        }
        if matches!(event, "revise" | "revise-narrative") {
            prepared.immediate = Some(allow_response());
            return Ok(prepared); // Recovery must not read prematurely edited files.
        }
        prepared.duties = TransitionDuties::Checked {
            subject: phase.subject,
            gate: None,
        };
    }

    // Contract-v4 implement -> reconciliation deliberately requires no report.
    // Admit on the first artifact-bearing post-reconciliation ready boundary.
    let artifact_ready = (source == phase.draft_state
        && (event == phase.ready_event || event == "passed")
        && duties != TransitionDuties::CheckedNoArtifact)
        || (source == workflow::RECONCILIATION_STATE
            && event == workflow::RECONCILIATION_READY_EVENT);
    let artifact_review = [phase.parent_review, phase.adversarial_review].contains(&source);
    let narrative_state = options
        .narrative
        .filter(|n| [n.draft, n.review, n.challenge].contains(&source));
    if duties == TransitionDuties::CheckedNoArtifact && source != options.checkpoint {
        return Ok(prepared);
    }
    let document = read(config, phase.subject)?;
    let fact = artifact_fact(&document)?;
    if artifact_ready {
        if request.context.iter().any(|r| {
            phase_record_fact(r, workflow::ARTIFACT_ADMITTED_KIND).is_some_and(|admitted| {
                admitted["phase"] == phase.name && admitted["revision"] == fact["revision"]
            })
        }) {
            return Err(deny(
                ARTIFACT_ADMISSION,
                "artifact revision was already admitted; use a new revision",
            ));
        }
        let mut admitted = fact.clone();
        admitted["phase"] = json!(phase.name);
        prepared.artifact_admission = Some(admitted);
    } else if artifact_review || narrative_state.is_some() || source == options.checkpoint {
        require_artifact(
            &request.context,
            phase.name,
            &fact,
            workflow::ARTIFACT_ADMITTED_KIND,
        )?;
    }
    if artifact_review {
        prepared.excluded_applicability =
            excluded_applicability(&request.context, options, narrative_enabled);
    }
    if exit.approved_artifact.is_some() {
        prepared.approved_artifact = Some(fact.clone());
    }
    if narrative_state.is_some() || source == options.checkpoint {
        require_artifact(
            &request.context,
            phase.name,
            &fact,
            workflow::APPROVED_ARTIFACT_KIND,
        )?;
    }
    if narrative_enabled && (narrative_state.is_some() || source == options.checkpoint) {
        let n = options.narrative.expect("enabled narrative phase");
        let binding_document = read(config, n.subject)?;
        let binding =
            narratives::validate_binding(binding_document.value()).map_err(|violations| {
                deny(
                    NARRATIVE_BINDING,
                    violations
                        .iter()
                        .map(|v| format!("{}: {}", v.path, v.message))
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
        let (approved_record, approved) = latest_phase_record(
            &request.context,
            workflow::APPROVED_ARTIFACT_KIND,
            phase.name,
            phase.subject,
        )
        .ok_or_else(|| deny(NARRATIVE_BINDING, "missing approved-artifact record"))?;
        if !crate::record_facts::valid_binding_source(
            &binding,
            approved_record,
            approved,
            phase.subject,
            n.markdown,
        ) {
            return Err(deny(
                NARRATIVE_BINDING,
                "binding must cite the latest approved artifact identity and exact phase files",
            ));
        }
        let markdown = artifacts::read_markdown_bytes(config.artifact_root(), n.markdown).map_err(
            |error| match error {
                ArtifactReadOutcome::Deny(error) => deny(NARRATIVE_BINDING, error.to_string()),
                ArtifactReadOutcome::EvaluationError(error) => {
                    EvaluationOutcome::EvaluationError(error.to_string())
                }
                ArtifactReadOutcome::Present(_) => unreachable!(),
            },
        )?;
        let markdown_hash = digest(&markdown);
        if binding.markdown.sha256 != markdown_hash {
            return Err(deny(
                NARRATIVE_FRESHNESS,
                "markdown bytes do not match binding sha256",
            ));
        }
        if source == n.draft {
            if request.context.iter().any(|r| {
                r.kind == workflow::NARRATIVE_ADMITTED_KIND
                    && r.data["phase"] == phase.name
                    && r.data["revision"] == binding.revision
            }) {
                return Err(deny(
                    NARRATIVE_ADMISSION,
                    "narrative revision was already admitted; use a new composite revision",
                ));
            }
            prepared.narrative_admission = Some(
                json!({"phase":phase.name,"revision":binding.revision,"sha256":markdown_hash}),
            );
        } else {
            let (_, admitted) = latest_phase_record(
                &request.context,
                workflow::NARRATIVE_ADMITTED_KIND,
                phase.name,
                n.subject,
            )
            .ok_or_else(|| deny(NARRATIVE_FRESHNESS, "missing latest narrative admission"))?;
            if !valid_narrative_fact(admitted, phase.name)
                || admitted["revision"] != binding.revision
                || admitted["sha256"] != markdown_hash
            {
                return Err(deny(
                    NARRATIVE_FRESHNESS,
                    "binding revision and markdown must match the latest narrative admission",
                ));
            }
        }
    }
    Ok(prepared)
}

impl Prepared {
    pub(crate) fn finish(self, mut response: Value) -> Value {
        if let Some(admission) = self.narrative_admission {
            response["context_append"] =
                json!({"kind":workflow::NARRATIVE_ADMITTED_KIND,"data":admission});
        } else if response["context_append"]["kind"] == workflow::RECONCILIATION_DECISION_KIND {
            // Preserve every existing reconciliation field and its real ID.
            if let Some(admission) = self.artifact_admission {
                response["context_append"]["data"]["artifact_admitted"] = admission;
            }
            if let Some(approval) = self.approved_artifact {
                response["context_append"]["data"]["approved_artifact"] = approval;
            }
        } else if let Some(mut admission) = self.artifact_admission {
            if let Some(approval) = self.approved_artifact {
                admission["approved_artifact"] = approval;
            }
            response["context_append"] =
                json!({"kind":workflow::ARTIFACT_ADMITTED_KIND,"data":admission});
        } else if let Some(approval) = self.approved_artifact {
            response["context_append"] =
                json!({"kind":workflow::APPROVED_ARTIFACT_KIND,"data":approval});
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates;
    use loop_core::{EvaluationResult, SemanticSequence, Timestamp};
    use std::path::PathBuf;

    struct Fixture {
        root: PathBuf,
        input: Value,
        context: Vec<ContextRecord>,
    }
    impl Fixture {
        fn new(narrative: bool, checkpoint: bool, reviews: bool) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "software-change-options-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let mut input = json!({"contract_version":4,"criterion_policy":{"required_authors":1,"goal_required_authors":1},"config_version":"option-test", "artifact_root":root,"review_policies":{},
                "extra":{"narratives":if narrative {vec!["design"]} else {vec![]},"checkpoints":if checkpoint {vec!["design"]} else {vec![]}}});
            if reviews {
                input["artifact_schemas"] = json!({"design.json":{"type":"object","required":["revision","author"],"properties":{
                    "revision":{"type":"string","minLength":1},"author":{"type":"object","required":["name","kind"],"properties":{"name":{"type":"string","minLength":1},"kind":{"type":"string","enum":["human","agent","script"]}}}}}});
                for gate in [
                    "design-review",
                    "design-adversarial-review",
                    "design-narrative-review",
                    "design-narrative-adversarial-review",
                ] {
                    if narrative || !gate.contains("narrative") {
                        input["review_policies"][gate] = json!([{"id":"axis","description":"fixture axis","review_stage":"aggregate"}]);
                    }
                }
            }
            let f = Self {
                root,
                input,
                context: vec![],
            };
            f.artifact("1");
            f
        }
        fn artifact(&self, revision: &str) {
            self.write(
                "design.json",
                &json!({"revision":revision,"author":{"name":"writer","kind":"agent"}}),
            );
        }
        fn write(&self, file: &str, value: &Value) {
            std::fs::write(self.root.join(file), serde_json::to_vec(value).unwrap()).unwrap();
        }
        fn append(&mut self, kind: &str, data: Value) -> String {
            let sequence = self.context.len() as u64 + 1;
            let id = format!("record-{sequence}");
            self.context.push(ContextRecord::new(
                id.clone(),
                kind,
                data,
                SemanticSequence::new(sequence),
                Timestamp::from_unix_millis(0),
            ));
            id
        }
        fn evaluate(&self, source: &str, event: &str) -> Value {
            let workflow = workflow::describe_workflow(Some(&self.input)).unwrap();
            let edge = workflow
                .transitions
                .iter()
                .find(|e| e.source.as_str() == source && e.event.as_str() == event)
                .unwrap()
                .clone();
            let request = EvaluateRequest {
                operation: "evaluate".into(),
                workflow,
                initial_input: self.input.clone(),
                context: self.context.clone(),
                transition: edge,
                prior_evaluations: vec![],
                driver_act: None,
            };
            match gates::evaluate(&request) {
                EvaluationOutcome::Response(v) => {
                    serde_json::from_value::<EvaluationResult>(v.clone()).unwrap();
                    v
                }
                EvaluationOutcome::EvaluationError(error) => panic!("{error}"),
            }
        }
        fn allow(&mut self, source: &str, event: &str) -> Value {
            let v = self.evaluate(source, event);
            assert_eq!(v["result"], "allow", "{v}");
            if let Some(effect) = v.get("context_append") {
                self.append(effect["kind"].as_str().unwrap(), effect["data"].clone());
            }
            v
        }
        fn deny(&self, source: &str, event: &str, code: &str) {
            let v = self.evaluate(source, event);
            assert_eq!(v["result"], "deny", "{v}");
            assert_eq!(v["feedback"]["code"], code, "{v}");
        }
        fn judgment(
            &mut self,
            gate: &str,
            subject: &str,
            revision: &str,
            result: &str,
            reviewer: &str,
        ) -> String {
            self.append("review-evidence",json!({"gate":gate,"policy_id":"axis","result":result,"findings":if result=="fail" {"Standing defect"} else {""},
                "author":{"name":reviewer,"kind":"agent"},"subject":subject,"subject_revision":revision,"config_version":"option-test","review_stage":"aggregate"}))
        }
        fn ledger(&mut self, gate: &str, subject: &str, revision: &str, findings: Value) {
            self.append("finding-ledger",json!({"schema_version":"1","gate":gate,"subject":subject,"subject_revision":revision,"author":{"name":"driver","kind":"agent"},"findings":findings}));
        }
        fn clear(&mut self, gate: &str, subject: &str, revision: &str) {
            self.judgment(gate, subject, revision, "pass", "reviewer");
            self.ledger(gate, subject, revision, json!([]));
        }
        fn approved_artifact(&mut self, reviews: bool) {
            self.allow("design", "design-ready");
            if reviews {
                self.clear("design-review", "design.json", "1");
                self.allow("design-review", "approved");
                self.clear("design-adversarial-review", "design.json", "1");
                self.allow("design-adversarial-review", "approved");
            }
        }
        fn binding(&self, revision: &str, prose: &str) {
            let options = &workflow::PHASE_OPTIONS[1];
            let (record, fact) = latest_phase_record(
                &self.context,
                workflow::APPROVED_ARTIFACT_KIND,
                "design",
                "design.json",
            )
            .unwrap();
            let n = options.narrative.unwrap();
            std::fs::write(self.root.join(n.markdown), prose).unwrap();
            self.write(n.subject,&json!({"revision":revision,"author":{"name":"writer","kind":"agent"},"artifact":fact,
                "approved_artifact_record_id":record.id.as_str(),"markdown":{"file":n.markdown,"sha256":digest(prose.as_bytes())}}));
        }
        fn decision(
            &mut self,
            revision: &str,
            narrative: bool,
            decision: &str,
            target: Option<&str>,
        ) {
            self.append("owner-decision",json!({"checkpoint":"design-checkpoint","subject":if narrative {"design-narrative.json"} else {"design.json"},
                "subject_revision":revision,"decision":decision,"target":target,"reason":target.map(|_| "Owner reason"),"text":"Owner's verbatim words\n","actor":{"name":"owner","kind":"human"}}));
        }
    }

    #[test]
    fn artifact_admission_and_approval_pin_exact_bytes_and_latest_revision() {
        let mut f = Fixture::new(false, true, true);
        let admission = f.allow("design", "design-ready");
        assert_eq!(admission["context_append"]["kind"], "artifact-admitted");
        f.clear("design-review", "design.json", "1");
        f.allow("design-review", "approved");
        let original = std::fs::read(f.root.join("design.json")).unwrap();
        std::fs::write(
            f.root.join("design.json"),
            [original.clone(), b" ".to_vec()].concat(),
        )
        .unwrap();
        f.deny("design-adversarial-review", "approved", ARTIFACT_FRESHNESS);
        std::fs::write(f.root.join("design.json"), &original).unwrap();
        f.clear("design-adversarial-review", "design.json", "1");
        let approval = f.allow("design-adversarial-review", "approved");
        assert_eq!(approval["context_append"]["kind"], "approved-artifact");
        f.decision("1", false, "approve", None);
        f.allow("design-checkpoint", "approved");
        std::fs::write(
            f.root.join("design.json"),
            [original.clone(), b" ".to_vec()].concat(),
        )
        .unwrap();
        f.deny("design-checkpoint", "approved", ARTIFACT_FRESHNESS);
        f.artifact("2");
        f.allow("design", "design-ready");
        std::fs::write(f.root.join("design.json"), &original).unwrap();
        f.deny("design-review", "approved", ARTIFACT_FRESHNESS);
    }

    #[test]
    fn artifact_revision_single_use_with_and_without_rejection() {
        for rejected in [false, true] {
            let mut f = Fixture::new(false, true, false);
            f.approved_artifact(false);
            if rejected {
                f.decision("1", false, "reject", Some("artifact"));
                f.allow("design-checkpoint", "revise");
            }
            f.deny("design", "design-ready", ARTIFACT_ADMISSION);
            f.artifact("2");
            f.allow("design", "design-ready");
            f.artifact("1");
            f.deny("design", "design-ready", ARTIFACT_ADMISSION);
            f.decision("2", false, "approve", None);
            f.deny("design-checkpoint", "approved", ARTIFACT_FRESHNESS);
        }
    }

    #[test]
    fn reviewless_ready_combines_facts_under_one_real_record_id() {
        let mut f = Fixture::new(true, true, false);
        let response = f.allow("design", "design-ready");
        assert_eq!(response["context_append"]["kind"], "artifact-admitted");
        assert_eq!(
            response["context_append"]["data"]["approved_artifact"]["revision"],
            "1"
        );
        f.binding("1/1", "Readable prose");
        f.allow("design-narrative", "narrative-ready");
        assert_eq!(
            latest_phase_record(
                &f.context,
                workflow::APPROVED_ARTIFACT_KIND,
                "design",
                "design.json"
            )
            .unwrap()
            .0
            .id
            .as_str(),
            "record-1"
        );
        f.decision("1/1", true, "approve", None);
        f.allow("design-checkpoint", "approved");
        f.context[0].data["approved_artifact"]["sha256"] = json!("a".repeat(64));
        f.deny("design-checkpoint", "approved", ARTIFACT_FRESHNESS);
    }

    #[test]
    fn narrative_binding_rejects_wrong_phase_prefix_citation_artifact_and_markdown() {
        let mut f = Fixture::new(true, true, false);
        f.approved_artifact(false);
        f.binding("1/1", "Faithful prose");
        let original: Value =
            serde_json::from_slice(&std::fs::read(f.root.join("design-narrative.json")).unwrap())
                .unwrap();
        for (pointer, bad) in [
            ("/revision", json!("2/1")),
            ("/revision", json!("1/01")),
            ("/artifact/subject", json!("plan.json")),
            ("/markdown/file", json!("plan-narrative.md")),
            ("/approved_artifact_record_id", json!("missing")),
            ("/artifact/sha256", json!("a".repeat(64))),
        ] {
            let mut binding = original.clone();
            *binding.pointer_mut(pointer).unwrap() = bad;
            f.write("design-narrative.json", &binding);
            f.deny("design-narrative", "narrative-ready", NARRATIVE_BINDING);
        }
        f.write("design-narrative.json", &original);
        std::fs::write(f.root.join("design-narrative.md"), "Changed prose").unwrap();
        f.deny("design-narrative", "narrative-ready", NARRATIVE_FRESHNESS);
        f.artifact("2");
        f.allow("design", "design-ready");
        f.deny("design-narrative", "narrative-ready", NARRATIVE_BINDING);
    }

    #[test]
    fn narrative_single_use_and_latest_admission_refuse_restored_revision() {
        let mut f = Fixture::new(true, true, false);
        f.approved_artifact(false);
        f.binding("1/1", "Old prose");
        f.allow("design-narrative", "narrative-ready");
        f.deny("design-narrative", "narrative-ready", NARRATIVE_ADMISSION);
        f.binding("1/2", "New prose");
        f.allow("design-narrative", "narrative-ready");
        f.binding("1/1", "Old prose");
        f.deny("design-narrative", "narrative-ready", NARRATIVE_ADMISSION);
        f.decision("1/2", true, "approve", None);
        f.deny("design-checkpoint", "approved", NARRATIVE_FRESHNESS);
    }

    #[test]
    fn admitted_prose_is_pinned_even_if_binding_hash_is_updated() {
        let mut f = Fixture::new(true, true, true);
        f.approved_artifact(true);
        f.binding("1/1", "Prose");
        f.allow("design-narrative", "narrative-ready");
        std::fs::write(f.root.join("design-narrative.md"), "Changed").unwrap();
        f.deny("design-narrative-review", "approved", NARRATIVE_FRESHNESS);
        f.binding("1/1", "Changed");
        f.deny("design-narrative-review", "approved", NARRATIVE_FRESHNESS);
        f.binding("1/2", "New");
        f.allow("design-narrative", "narrative-ready");
        f.binding("1/1", "Prose");
        f.deny("design-narrative-review", "approved", NARRATIVE_FRESHNESS);
    }

    #[test]
    fn owner_exits_use_latest_decision_and_record_identity_not_early_edits() {
        let mut f = Fixture::new(true, true, false);
        f.approved_artifact(false);
        f.binding("1/1", "Prose");
        f.allow("design-narrative", "narrative-ready");
        f.deny("design-checkpoint", "approved", OWNER_DECISION);
        f.deny("design-checkpoint", "revise", OWNER_DECISION);
        f.deny("design-checkpoint", "revise-narrative", OWNER_DECISION);
        f.decision("0/1", true, "approve", None);
        f.deny("design-checkpoint", "approved", OWNER_DECISION);
        f.decision("1/1", true, "approve", None);
        f.allow("design-checkpoint", "approved");
        f.decision("1/1", true, "reject", Some("narrative"));
        f.deny("design-checkpoint", "approved", OWNER_DECISION);
        f.binding("1/2", "Already rewritten");
        f.allow("design-checkpoint", "revise-narrative");
        f.deny("design-checkpoint", "revise", OWNER_DECISION);
        f.allow("design-narrative", "narrative-ready");
        f.deny("design-checkpoint", "approved", OWNER_DECISION);
        f.decision("1/2", true, "reject", Some("artifact"));
        f.artifact("2");
        std::fs::write(f.root.join("design-narrative.json"), "not JSON").unwrap();
        f.allow("design-checkpoint", "revise");
        f.deny("design-checkpoint", "revise-narrative", OWNER_DECISION);
    }

    #[test]
    fn artifact_rejection_recovery_needs_no_restored_file_and_malformed_decisions_diagnose() {
        let mut f = Fixture::new(false, true, false);
        f.approved_artifact(false);
        f.decision("1", false, "reject", Some("artifact"));
        f.artifact("2");
        f.allow("design-checkpoint", "revise");
        f.decision("wrong", false, "reject", Some("artifact"));
        f.deny("design-checkpoint", "revise", OWNER_DECISION);
        f.decision("1", false, "approve", None);
        let mut malformed = f.context.last().unwrap().data.clone();
        malformed["text"] = json!("");
        f.append("owner-decision", malformed);
        f.artifact("1");
        f.allow("design-checkpoint", "approved");
        f.context.retain(|r| r.data["decision"] != "approve");
        let denial = f.evaluate("design-checkpoint", "approved");
        assert_eq!(denial["feedback"]["code"], OWNER_DECISION);
    }

    #[test]
    fn narrative_reviews_reuse_evidence_ledger_challenge_clearance_and_failure_blocking() {
        let mut f = Fixture::new(true, true, true);
        f.approved_artifact(true);
        f.binding("1/1", "Prose");
        f.allow("design-narrative", "narrative-ready");
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-finding-ledger-invalid",
        );
        f.ledger(
            "design-narrative-review",
            "design-narrative.json",
            "1/1",
            json!([]),
        );
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-review-incomplete",
        );
        f.clear(
            "design-narrative-adversarial-review",
            "design-narrative.json",
            "1/1",
        );
        // Early preparation is allowed. Evaluate challenge only after ordinary
        // checked entry: ordinary evidence is no longer re-judged there, while
        // the early-assessment cutoff below still requires a fresh challenge.
        f.clear("design-narrative-review", "design-narrative.json", "1/1");
        f.allow("design-narrative-review", "approved");
        f.deny(
            "design-narrative-adversarial-review",
            "approved",
            "software-change-review-incomplete",
        );
        f.clear(
            "design-narrative-adversarial-review",
            "design-narrative.json",
            "1/1",
        );
        f.allow("design-narrative-adversarial-review", "approved");
        f.judgment(
            "design-narrative-adversarial-review",
            "design-narrative.json",
            "1/1",
            "fail",
            "other-reviewer",
        );
        f.deny(
            "design-narrative-adversarial-review",
            "approved",
            "software-change-review-incomplete",
        );
        f.binding("1/2", "New");
        f.allow("design-narrative", "narrative-ready");
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-finding-ledger-invalid",
        );
    }

    #[test]
    fn challenge_clearance_keeps_missing_stale_and_unresolved_ordinary_ledger_safeguards() {
        let mut f = Fixture::new(false, false, true);
        f.allow("design", "design-ready");
        f.clear("design-review", "design.json", "1");
        f.allow("design-review", "approved");
        f.clear("design-adversarial-review", "design.json", "1");
        f.allow("design-adversarial-review", "approved");
        // In-memory evaluation fixtures can remove a ledger; public immutable
        // histories cannot lose the ledger required by normal ordinary entry.
        f.context
            .retain(|r| !(r.kind == "finding-ledger" && r.data["gate"] == "design-review"));
        let missing = f.evaluate("design-adversarial-review", "approved");
        assert_eq!(missing["result"], "deny", "{missing}");
        assert_eq!(
            missing["feedback"]["details"]["ordinary_ledger"]["status"], "missing",
            "{missing}"
        );
        f.ledger("design-review", "design.json", "0", json!([]));
        let stale = f.evaluate("design-adversarial-review", "approved");
        assert_eq!(stale["result"], "deny", "{stale}");
        assert_eq!(
            stale["feedback"]["details"]["ordinary_ledger"]["status"], "stale_subject",
            "{stale}"
        );
        let source = f.judgment(
            "design-review",
            "design.json",
            "1",
            "fail",
            "finding-reviewer",
        );
        f.ledger("design-review", "design.json", "1", json!([{
            "id":"F-defect","source":{"kind":"context-record","id":source},"policy_id":"axis",
            "statement":"Standing defect","disposition":"accepted","reason":"Accepted scripted defect",
            "owner_phase":"design","task_ids":[],"review_axes":["axis"],"status":"unresolved"
        }]));
        let unresolved = f.evaluate("design-adversarial-review", "approved");
        assert_eq!(unresolved["result"], "deny", "{unresolved}");
        assert_eq!(
            unresolved["feedback"]["details"]["ordinary_ledger"]["status"], "accepted_unresolved",
            "{unresolved}"
        );
    }

    fn applicability(f: &mut Fixture, origin: &str, subject: &str, revision: &str) {
        f.append("evidence-applicability",json!({"origin":{"kind":"context-record","id":origin},"target":{"subject":subject,"revision":revision},
            "attesting_driver":{"name":"driver","kind":"agent"},"reason":"Unchanged judging scope"}));
    }

    #[test]
    fn pre_rejection_original_judgment_cannot_be_carried_by_later_applicability() {
        for rejected in [false, true] {
            let mut f = Fixture::new(false, true, true);
            f.approved_artifact(true);
            let origin = f
                .context
                .iter()
                .find(|r| r.kind == "review-evidence" && r.data["gate"] == "design-review")
                .unwrap()
                .id
                .as_str()
                .to_owned();
            if rejected {
                f.decision("1", false, "reject", Some("artifact"));
            }
            f.artifact("2");
            f.allow("design", "design-ready");
            applicability(&mut f, &origin, "design.json", "2");
            f.ledger("design-review", "design.json", "2", json!([]));
            if rejected {
                f.deny(
                    "design-review",
                    "approved",
                    "software-change-review-incomplete",
                );
            } else {
                f.allow("design-review", "approved");
            }
        }
    }

    #[test]
    fn cutoff_preserves_direct_standing_failure_and_post_rejection_applicability() {
        let mut f = Fixture::new(false, true, true);
        f.approved_artifact(true);
        f.decision("1", false, "reject", Some("artifact"));
        let after = f.judgment(
            "design-review",
            "design.json",
            "1",
            "pass",
            "post-rejection-reviewer",
        );
        f.artifact("2");
        f.allow("design", "design-ready");
        applicability(&mut f, &after, "design.json", "2");
        f.ledger("design-review", "design.json", "2", json!([]));
        f.allow("design-review", "approved");
        // A current direct failure recorded before a rejection is not filtered.
        f.judgment(
            "design-review",
            "design.json",
            "2",
            "fail",
            "standing-reviewer",
        );
        f.decision("1", false, "reject", Some("artifact"));
        f.deny(
            "design-review",
            "approved",
            "software-change-review-incomplete",
        );
        let excluded = excluded_applicability(&f.context, &workflow::PHASE_OPTIONS[1], false);
        assert!(f
            .context
            .iter()
            .filter(|r| r.kind == "review-evidence")
            .all(|r| !excluded.contains(r.id.as_str())));
    }

    #[test]
    fn narrative_stage_author_sets_and_self_review_are_not_weakened() {
        let mut f = Fixture::new(true, true, true);
        f.approved_artifact(true);
        for gate in [
            "design-narrative-review",
            "design-narrative-adversarial-review",
        ] {
            f.input["review_policies"][gate] = json!([
                {"id":"axis","description":"axis","review_stage":"individual"},
                {"id":"axis","description":"axis","review_stage":"aggregate"}
            ]);
        }
        f.binding("1/1", "Prose");
        f.allow("design-narrative", "narrative-ready");
        f.clear("design-narrative-review", "design-narrative.json", "1/1");
        let mut individual = f
            .context
            .iter()
            .find(|r| r.kind == "review-evidence" && r.data["gate"] == "design-narrative-review")
            .unwrap()
            .data
            .clone();
        individual["review_stage"] = json!("individual");
        individual["author"]["name"] = json!("different");
        f.append("review-evidence", individual.clone());
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-review-incomplete",
        );
        individual["author"]["name"] = json!("reviewer");
        f.append("review-evidence", individual.clone());
        // Retained different individual authors still cannot disappear silently.
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-review-incomplete",
        );
        f.context
            .retain(|r| r.data["author"]["name"] != "different");
        f.allow("design-narrative-review", "approved");
        for r in &mut f.context {
            if r.kind == "review-evidence" && r.data["gate"] == "design-narrative-review" {
                r.data["author"]["name"] = json!("writer");
            }
        }
        f.deny(
            "design-narrative-review",
            "approved",
            "software-change-review-incomplete",
        );
    }

    #[test]
    fn pre_rejection_dispositioned_fail_carry_cannot_fill_author_floor() {
        let mut f = Fixture::new(false, true, true);
        f.approved_artifact(true);
        let failed = f.judgment(
            "design-review",
            "design.json",
            "1",
            "fail",
            "failed-reviewer",
        );
        f.decision("1", false, "reject", Some("artifact"));
        f.artifact("2");
        f.allow("design", "design-ready");
        applicability(&mut f, &failed, "design.json", "2");
        f.ledger("design-review","design.json","2",json!([{"id":"F-disposed","source":{"kind":"context-record","id":failed},
            "policy_id":"axis","statement":"Standing defect","disposition":"rejected","reason":"Exact-source triage","owner_phase":null,"task_ids":[],"review_axes":[],"status":"recorded"}]));
        f.deny(
            "design-review",
            "approved",
            "software-change-review-incomplete",
        );
    }

    fn reconciliation() -> Value {
        json!({"revision":"r1","author":{"name":"driver","kind":"agent"},"mode":"bookends-disabled","branch":"sufficient-existing-wording",
            "document_observations":[{"path":"docs/PRD.md","status":"sufficient","observation":"Existing wording covers delivered meaning."}],
            "behavior_observations":[{"status":"matches-intent","observation":"The fixture matches."}],"action":"no-document-change","action_reason":"No enduring wording change.",
            "authorization":"not-required","application":"not-required","commit":"not-required","traceability":{"status":"not-applicable","references":[]},
            "proof_references":["fixture-proof"],"blockers":[],"decision":"complete"})
    }

    fn checkpoint_documents(f: &Fixture) {
        for file in [
            "intent.json",
            "design.json",
            "plan.json",
            "implementation-report.json",
            "validation-report.json",
        ] {
            f.write(
                file,
                &json!({"revision":"1","author":{"name":"writer","kind":"agent"}}),
            );
        }
    }

    #[test]
    fn reconciliation_collision_preserves_primary_fields_and_checkpoint_entry_records_proof() {
        let mut f = Fixture::new(false, false, false);
        f.input["extra"]["checkpoints"] = json!(["implementation"]);
        // The inherited implementation-ready handoff is still report-free.
        f.allow("implement", "implementation-ready");
        checkpoint_documents(&f);
        f.write("reconciliation.json", &reconciliation());
        let cwd = std::env::current_dir().unwrap();
        crate::checkpoint::create(
            crate::checkpoint::CheckpointPhase::Implementation,
            &f.root,
            &cwd,
        )
        .unwrap();
        let response = f.allow("reconciliation", "reconciliation-ready");
        let data = &response["context_append"]["data"];
        assert_eq!(
            response["context_append"]["kind"],
            workflow::RECONCILIATION_DECISION_KIND
        );
        assert_eq!(data["revision"], "r1");
        assert_eq!(
            data["documents"],
            json!({"intent":"1","design":"1","plan":"1"})
        );
        assert!(data["sha256"].as_str().unwrap().starts_with("sha256:"));
        assert_eq!(data["artifact_admitted"]["phase"], "implementation");
        assert_eq!(
            data["approved_artifact"]["subject"],
            "implementation-report.json"
        );
        assert!(crate::checkpoint::verify_accepted_implementation_from_cwd(&f.root).is_err());
        let (record, _) = latest_phase_record(
            &f.context,
            workflow::APPROVED_ARTIFACT_KIND,
            "implementation",
            "implementation-report.json",
        )
        .unwrap();
        assert_eq!(record.kind, workflow::RECONCILIATION_DECISION_KIND);
        f.append("owner-decision",json!({"checkpoint":"implementation-checkpoint","subject":"implementation-report.json","subject_revision":"1","decision":"approve",
            "text":"Approved after reconciliation.","actor":{"name":"owner","kind":"human"}}));
        f.allow("implementation-checkpoint", "approved");
        crate::checkpoint::verify_accepted_implementation_from_cwd(&f.root).unwrap();
        f.deny("reconciliation", "reconciliation-ready", ARTIFACT_ADMISSION);
    }

    #[test]
    fn reconciliation_ready_with_reviews_admits_without_approving_and_reviewless_entry_keeps_proof()
    {
        for reviews in [false, true] {
            let mut f = Fixture::new(false, false, false);
            f.input["extra"]["checkpoints"] = json!(["implementation"]);
            if reviews {
                let sample = Fixture::new(false, false, true);
                f.input["artifact_schemas"] = json!({"implementation-report.json":sample.input["artifact_schemas"]["design.json"].clone()});
                f.input["review_policies"]["implementation-review"] =
                    json!([{"id":"axis","description":"axis","review_stage":"aggregate"}]);
            }
            checkpoint_documents(&f);
            f.write("reconciliation.json", &reconciliation());
            crate::checkpoint::create(
                crate::checkpoint::CheckpointPhase::Implementation,
                &f.root,
                &std::env::current_dir().unwrap(),
            )
            .unwrap();
            let response = f.allow("reconciliation", "reconciliation-ready");
            assert!(response["context_append"]["data"]
                .get("artifact_admitted")
                .is_some());
            assert_eq!(
                response["context_append"]["data"]
                    .get("approved_artifact")
                    .is_none(),
                reviews
            );
            if !reviews {
                // Separate actual-entry case: no implementation option routes directly into validation.
                let mut off = Fixture::new(false, false, false);
                checkpoint_documents(&off);
                off.write("reconciliation.json", &reconciliation());
                crate::checkpoint::create(
                    crate::checkpoint::CheckpointPhase::Implementation,
                    &off.root,
                    &std::env::current_dir().unwrap(),
                )
                .unwrap();
                off.allow("reconciliation", "reconciliation-ready");
                crate::checkpoint::verify_accepted_implementation_from_cwd(&off.root).unwrap();
            }
        }
    }

    #[test]
    fn relocated_final_completion_checks_validation_report_after_narrative_prerequisites() {
        for (checkpoint, reviews) in [(false, false), (true, false), (false, true)] {
            let mut f = Fixture::new(false, false, false);
            f.input["extra"]["narratives"] = json!(["validation"]);
            if reviews {
                for gate in [
                    "validation-narrative-review",
                    "validation-narrative-adversarial-review",
                ] {
                    f.input["review_policies"][gate] =
                        json!([{"id":"axis","description":"axis","review_stage":"aggregate"}]);
                }
            }
            f.input["extra"]["checkpoints"] = if checkpoint {
                json!(["validation"])
            } else {
                json!([])
            };
            checkpoint_documents(&f);
            let cwd = std::env::current_dir().unwrap();
            crate::checkpoint::create(
                crate::checkpoint::CheckpointPhase::Implementation,
                &f.root,
                &cwd,
            )
            .unwrap();
            crate::checkpoint::record_accepted_implementation_from_cwd(&f.root).unwrap();
            crate::checkpoint::create(
                crate::checkpoint::CheckpointPhase::Validation,
                &f.root,
                &cwd,
            )
            .unwrap();
            let config = crate::config::parse_initial_input(&f.input).unwrap();
            let document =
                read(&config, "validation-report.json").unwrap_or_else(|e| panic!("{e}"));
            let mut admitted = artifact_fact(&document).unwrap_or_else(|e| panic!("{e}"));
            admitted["phase"] = json!("validation");
            f.append(workflow::ARTIFACT_ADMITTED_KIND, admitted);
            let approved_id = f.append(
                workflow::APPROVED_ARTIFACT_KIND,
                artifact_fact(&document).unwrap_or_else(|e| panic!("{e}")),
            );
            let prose = "Validation narrative";
            std::fs::write(f.root.join("validation-narrative.md"), prose).unwrap();
            f.write("validation-narrative.json",&json!({"revision":"1/1","author":{"name":"writer","kind":"agent"},"artifact":artifact_fact(&document).unwrap_or_else(|e| panic!("{e}")),
                "approved_artifact_record_id":approved_id,"markdown":{"file":"validation-narrative.md","sha256":digest(prose.as_bytes())}}));
            if reviews {
                f.allow("validation-narrative", "narrative-ready");
                f.deny(
                    "validation-narrative-adversarial-review",
                    "passed",
                    "software-change-finding-ledger-invalid",
                );
                f.clear(
                    "validation-narrative-review",
                    "validation-narrative.json",
                    "1/1",
                );
                f.allow("validation-narrative-review", "approved");
                f.clear(
                    "validation-narrative-adversarial-review",
                    "validation-narrative.json",
                    "1/1",
                );
                f.deny(
                    "validation-narrative-adversarial-review",
                    "passed",
                    "software-change-criterion-incomplete",
                );
            } else if checkpoint {
                f.allow("validation-narrative", "narrative-ready");
                f.append("owner-decision",json!({"checkpoint":"validation-checkpoint","subject":"validation-narrative.json","subject_revision":"1/1","decision":"approve",
                    "text":"Approved.","actor":{"name":"owner","kind":"human"}}));
                f.deny(
                    "validation-checkpoint",
                    "passed",
                    "software-change-criterion-incomplete",
                );
            } else {
                // T4's reviewless final narrative draft uses passed, not narrative-ready.
                f.deny(
                    "validation-narrative",
                    "passed",
                    "software-change-criterion-incomplete",
                );
            }
        }
    }
    #[test]
    fn rejection_cutoff_scopes_checkpoint_subject_and_keeps_prior_revision_history() {
        for narrative in [false, true] {
            let mut f = Fixture::new(narrative, true, true);
            let origin = f.judgment("design-review", "design.json", "old", "pass", "reviewer");
            applicability(&mut f, &origin, "design.json", "new");
            f.decision("old", !narrative, "reject", Some("artifact"));
            assert!(
                excluded_applicability(&f.context, &workflow::PHASE_OPTIONS[1], narrative)
                    .is_empty()
            );
            let mut other = f.context.last().unwrap().data.clone();
            other["checkpoint"] = json!("plan-checkpoint");
            f.append("owner-decision", other);
            assert!(
                excluded_applicability(&f.context, &workflow::PHASE_OPTIONS[1], narrative)
                    .is_empty()
            );
            f.decision("old", narrative, "reject", Some("artifact"));
            let excluded =
                excluded_applicability(&f.context, &workflow::PHASE_OPTIONS[1], narrative);
            assert_eq!(excluded.len(), 1);
            assert!(f
                .context
                .iter()
                .filter(|r| r.kind == "review-evidence")
                .all(|r| !excluded.contains(r.id.as_str())));
            // Wrong-subject decisions remain in the separate T1 evaluation.
            f.decision("current", narrative, "approve", None);
            f.decision("wrong", !narrative, "reject", Some("artifact"));
            let evaluation = owner_checkpoint::evaluate(
                &f.context,
                "design-checkpoint",
                workflow::checkpoint_subject(&workflow::PHASE_OPTIONS[1], narrative),
                "current",
                &["artifact", "narrative"],
            );
            assert_eq!(evaluation.status, Status::Pending);
        }
    }
    #[test]
    fn option_enabled_narrative_repairs_stay_check_free_after_early_edits() {
        let mut f = Fixture::new(true, true, true);
        f.approved_artifact(true);
        f.binding("1/1", "Prose");
        f.allow("design-narrative", "narrative-ready");
        std::fs::write(f.root.join("design.json"), "Not JSON").unwrap();
        std::fs::write(f.root.join("design-narrative.json"), "Not JSON").unwrap();
        f.allow("design-narrative-review", "revise");
        f.allow("design-narrative-adversarial-review", "revise-design");
        f.allow("design-narrative", "revise-design");
    }
    #[test]
    fn nested_artifact_facts_are_closed_consistent_and_keep_enclosing_sequence() {
        let admission = json!({"phase":"implementation","subject":"implementation-report.json","revision":"2","sha256":"a".repeat(64)});
        let approval =
            json!({"subject":"implementation-report.json","revision":"2","sha256":"a".repeat(64)});
        let mut record = ContextRecord::new(
            "real-enclosing-id",
            workflow::RECONCILIATION_DECISION_KIND,
            json!({"revision":"reconciliation-r3","sha256":format!("sha256:{}","b".repeat(64)),"documents":{"intent":"1","design":"1","plan":"1"},"artifact_admitted":admission,"approved_artifact":approval}),
            SemanticSequence::new(99),
            Timestamp::from_unix_millis(0),
        );
        for kind in [
            workflow::ARTIFACT_ADMITTED_KIND,
            workflow::APPROVED_ARTIFACT_KIND,
        ] {
            let (enclosing, fact) = latest_phase_record(
                &[record.clone()],
                kind,
                "implementation",
                "implementation-report.json",
            )
            .map(|(r, f)| (r.clone(), f.clone()))
            .unwrap();
            assert_eq!(enclosing.id.as_str(), "real-enclosing-id");
            assert_eq!(enclosing.sequence, SemanticSequence::new(99));
            assert!(validate_artifact_record(
                &enclosing,
                &fact,
                kind,
                "implementation",
                "implementation-report.json"
            ));
        }
        for (pointer, bad) in [
            ("/artifact_admitted/phase", json!("design")),
            ("/approved_artifact/revision", json!("1")),
            ("/approved_artifact/sha256", json!("b".repeat(64))),
            ("/approved_artifact/unexpected", json!(true)),
        ] {
            let mut broken = record.clone();
            if let Some(value) = broken.data.pointer_mut(pointer) {
                *value = bad;
            } else {
                broken.data["approved_artifact"]["unexpected"] = bad;
            }
            assert!(!validate_artifact_record(
                &broken,
                phase_record_fact(&broken, workflow::APPROVED_ARTIFACT_KIND).unwrap(),
                workflow::APPROVED_ARTIFACT_KIND,
                "implementation",
                "implementation-report.json"
            ));
        }
        record.data["artifact_admitted"]["approved_artifact"] =
            record.data["approved_artifact"].clone();
        assert!(!validate_artifact_record(
            &record,
            phase_record_fact(&record, workflow::ARTIFACT_ADMITTED_KIND).unwrap(),
            workflow::ARTIFACT_ADMITTED_KIND,
            "implementation",
            "implementation-report.json"
        ));
    }
    fn bind(f: &mut Fixture, role: &str, model: &str, thinking: &str) {
        f.input["model_bindings"] =
            json!({"roles": {role: {"model": model, "thinking": thinking}}});
    }

    fn declare(f: &mut Fixture, role: &str, subject: &str, revision: &str, model: &str) {
        f.append(
            loop_core::SESSION_DECLARATION_RECORD_KIND,
            json!({"subject":subject,"subject_revision":revision,"role":role,
                "session":{"model":model,"thinking":"high"}}),
        );
    }

    fn deny_message(f: &Fixture, source: &str, event: &str) -> String {
        let v = f.evaluate(source, event);
        assert_eq!(v["result"], "deny", "{v}");
        assert_eq!(v["feedback"]["code"], ARTIFACT_ADMISSION, "{v}");
        v["feedback"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{v}"))
            .to_owned()
    }

    #[test]
    fn bound_author_draft_needs_a_matching_declaration_for_the_current_revision() {
        let mut f = Fixture::new(false, false, false);
        bind(&mut f, "author", "p/author", "high");
        let before = f.context.clone();
        let missing = deny_message(&f, "design", "design-ready");
        assert!(
            missing.contains("`author`") && missing.contains("statement missing"),
            "{missing}"
        );
        assert!(missing.contains("design.json revision `1`"), "{missing}");
        declare(&mut f, "author", "design.json", "1", "p/other");
        let unequal = deny_message(&f, "design", "design-ready");
        assert!(
            unequal.contains("expected model `p/author`")
                && unequal.contains("differs in model only"),
            "{unequal}"
        );
        declare(&mut f, "author", "plan.json", "1", "p/author");
        declare(&mut f, "author", "design.json", "0", "p/author");
        deny_message(&f, "design", "design-ready"); // another subject or an older revision
        declare(&mut f, "implementer", "design.json", "1", "p/author");
        deny_message(&f, "design", "design-ready"); // another role
        assert_eq!(f.context[..before.len()], before[..]);
        declare(&mut f, "author", "design.json", "1", "p/author");
        f.allow("design", "design-ready");
        // A new artifact revision needs its own declaration.
        f.artifact("2");
        assert!(deny_message(&f, "design", "design-ready").contains("revision `2`"));
    }

    #[test]
    fn the_latest_declaration_for_the_revision_decides_after_an_amendment() {
        let mut f = Fixture::new(false, false, false);
        bind(&mut f, "author", "p/author", "high");
        declare(&mut f, "author", "design.json", "1", "p/author");
        f.append(
            loop_core::MODEL_BINDING_AMENDMENT_RECORD_KIND,
            json!({"actor":{"name":"owner","kind":"human"},"authority":{"kind":"owner"},"words":"switch",
                "changes":[{"role":"author","old":{"model":"p/author","thinking":"high"},
                    "new":{"model":"p/next","thinking":"high"}}]}),
        );
        assert!(deny_message(&f, "design", "design-ready").contains("expected model `p/next`"));
        declare(&mut f, "author", "design.json", "1", "p/next");
        f.allow("design", "design-ready");
    }

    #[test]
    fn unbound_author_and_other_roles_leave_drafts_unrequired() {
        let mut f = Fixture::new(false, false, false);
        f.allow("design", "design-ready");
        let mut g = Fixture::new(false, false, false);
        bind(&mut g, "implementer", "p/impl", "high");
        g.allow("design", "design-ready");
    }

    #[test]
    fn intent_and_plan_drafts_need_the_author_declaration_too() {
        for (state, event, subject) in [
            ("explore", "intent-ready", "intent.json"),
            ("plan", "plan-ready", "plan.json"),
        ] {
            let mut f = Fixture::new(false, false, false);
            f.write(
                subject,
                &json!({"revision":"7","author":{"name":"writer","kind":"agent"}}),
            );
            bind(&mut f, "author", "p/author", "high");
            assert!(deny_message(&f, state, event).contains(&format!("{subject} revision `7`")));
            declare(&mut f, "author", subject, "7", "p/author");
            assert_ne!(
                f.evaluate(state, event)["feedback"]["code"],
                ARTIFACT_ADMISSION
            );
        }
    }

    #[test]
    fn validation_draft_is_declared_at_ready_event_and_at_passed() {
        for (event, narrative) in [("validation-ready", true), ("passed", false)] {
            let mut f = Fixture::new(false, false, false);
            if narrative {
                f.input["extra"]["narratives"] = json!(["validation"]);
            }
            checkpoint_documents(&f);
            let cwd = std::env::current_dir().unwrap();
            for phase in [
                crate::checkpoint::CheckpointPhase::Implementation,
                crate::checkpoint::CheckpointPhase::Validation,
            ] {
                crate::checkpoint::create(phase, &f.root, &cwd).unwrap();
                if phase == crate::checkpoint::CheckpointPhase::Implementation {
                    crate::checkpoint::record_accepted_implementation_from_cwd(&f.root).unwrap();
                }
            }
            bind(&mut f, "author", "p/author", "high");
            let missing = deny_message(&f, "validation", event);
            assert!(
                missing.contains("validation-report.json revision `1`"),
                "{event}: {missing}"
            );
            declare(&mut f, "author", "validation-report.json", "1", "p/other");
            deny_message(&f, "validation", event);
            declare(&mut f, "author", "validation-report.json", "1", "p/author");
            // Past the declaration the ordinary validation checks decide, so
            // the answer equals the same evaluation with the role unbound.
            let accepted = f.evaluate("validation", event);
            f.input.as_object_mut().unwrap().remove("model_bindings");
            assert_eq!(accepted, f.evaluate("validation", event), "{event}");
            assert_ne!(accepted["feedback"]["code"], ARTIFACT_ADMISSION);
        }
    }

    #[test]
    fn bound_implementer_report_is_declared_at_reconciliation_ready_not_at_the_handoff() {
        let mut f = Fixture::new(false, false, false);
        bind(&mut f, "implementer", "p/impl", "high");
        // The report-free handoff needs no declaration and no report.
        f.allow("implement", "implementation-ready");
        checkpoint_documents(&f);
        f.write("reconciliation.json", &reconciliation());
        crate::checkpoint::create(
            crate::checkpoint::CheckpointPhase::Implementation,
            &f.root,
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        let before = f.context.clone();
        let missing = deny_message(&f, "reconciliation", "reconciliation-ready");
        assert!(
            missing.contains("`implementer`")
                && missing.contains("implementation-report.json revision `1`"),
            "{missing}"
        );
        declare(
            &mut f,
            "implementer",
            "implementation-report.json",
            "1",
            "p/other",
        );
        deny_message(&f, "reconciliation", "reconciliation-ready");
        declare(
            &mut f,
            "author",
            "implementation-report.json",
            "1",
            "p/impl",
        );
        deny_message(&f, "reconciliation", "reconciliation-ready"); // author declaration is not the implementer's
        assert_eq!(f.context[..before.len()], before[..]);
        declare(
            &mut f,
            "implementer",
            "implementation-report.json",
            "1",
            "p/impl",
        );
        f.allow("reconciliation", "reconciliation-ready");
    }

    #[test]
    fn declaration_boundary_names_exactly_the_artifact_bearing_edges() {
        use loop_core::ModelRole::{Author, Implementer};
        let checked = |subject| TransitionDuties::Checked {
            subject,
            gate: None,
        };
        for (source, event, subject, expected) in [
            (
                "explore",
                "intent-ready",
                "intent.json",
                Some((Author, "intent.json")),
            ),
            (
                "design",
                "design-ready",
                "design.json",
                Some((Author, "design.json")),
            ),
            (
                "plan",
                "plan-ready",
                "plan.json",
                Some((Author, "plan.json")),
            ),
            (
                "validation",
                "validation-ready",
                "validation-report.json",
                Some((Author, "validation-report.json")),
            ),
            (
                "validation",
                "passed",
                "validation-report.json",
                Some((Author, "validation-report.json")),
            ),
            (
                "reconciliation",
                "reconciliation-ready",
                "reconciliation.json",
                Some((Implementer, "implementation-report.json")),
            ),
            ("design-review", "approved", "design.json", None),
            ("explore", "passed", "intent.json", None),
        ] {
            assert_eq!(
                workflow::declaration_boundary(source, event, checked(subject)),
                expected,
                "{source} {event}"
            );
        }
        assert_eq!(
            workflow::declaration_boundary(
                "implement",
                "implementation-ready",
                TransitionDuties::CheckedNoArtifact
            ),
            None
        );
        assert_eq!(
            workflow::declaration_boundary("design", "design-ready", TransitionDuties::CheckFree),
            None
        );
    }
}
