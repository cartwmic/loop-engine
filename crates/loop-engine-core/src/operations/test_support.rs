use crate::model::attempt::{AttemptFacts, JournalExtension};
use crate::model::evidence::{EvidenceRecord, EvidenceSource};
use crate::model::graph::{State, WorkflowGraph};
use crate::model::graph_validation::ValidatedGraph;
use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use crate::model::ids::{
    EvidenceId, EvidenceKind, GraphRevision, RegistrationId, RequestId, RunId, StateId,
};
use crate::model::journal::JournalDraft;
use crate::model::outcome::OutcomeClass;
use crate::model::reason::{Reason, ReasonCode};
use crate::model::run::Run;
use crate::model::run_input::{InputDeclarations, RunInputs};
use crate::model::time::ObservedAt;

pub fn evidence() -> EvidenceRecord {
    EvidenceRecord::new(
        EvidenceId::parse("evidence-1").unwrap(),
        EvidenceKind::parse("artifact").unwrap(),
        "opaque:locator",
        None,
        None,
        None,
        EvidenceSource::Caller,
        ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
    )
    .unwrap()
}

pub fn draft(
    operation: &str,
    outcome: OutcomeClass,
    extension: JournalExtension,
    attempt: Option<AttemptFacts>,
) -> JournalDraft {
    let reason = if outcome == OutcomeClass::Completed {
        None
    } else {
        Some(
            Reason::new(
                match outcome {
                    OutcomeClass::Rejected => ReasonCode::RunLifecycleTerminal,
                    OutcomeClass::Error => ReasonCode::StateStaleVersion,
                    OutcomeClass::Completed => unreachable!(),
                },
                "test disposition",
            )
            .unwrap(),
        )
    };
    JournalDraft::new(
        RunId::parse("run-1").unwrap(),
        ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        operation,
        RequestId::parse("request-1").unwrap(),
        outcome,
        reason,
        attempt,
        extension,
    )
    .unwrap()
}

pub fn run() -> Run {
    let state = State::new(
        StateId::parse("ready").unwrap(),
        false,
        StaticGuidance::NoneRequired,
        None,
    );
    let graph = ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
        StateId::parse("ready").unwrap(),
        vec![state],
        vec![],
        InputDeclarations::default(),
        LiveGuidanceCapability::Unsupported,
        None,
    ))
    .unwrap();
    Run::create(
        RunId::parse("run-1").unwrap(),
        RegistrationId::parse("registration-1").unwrap(),
        graph,
        GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        RunInputs::default(),
        None,
    )
    .unwrap()
}
