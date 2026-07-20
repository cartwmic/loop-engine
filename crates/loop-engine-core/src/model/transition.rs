use std::collections::BTreeSet;

use thiserror::Error;

use super::bounded::Metadata;
use super::ids::{EventId, GateId, StateId};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transition {
    source: StateId,
    event: EventId,
    target: StateId,
    required_gates: Vec<GateId>,
    metadata: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("transition contains duplicate required gate {0}")]
pub struct DuplicateGate(pub GateId);

impl Transition {
    pub fn new(
        source: StateId,
        event: EventId,
        target: StateId,
        required_gates: Vec<GateId>,
        metadata: Option<Metadata>,
    ) -> Result<Self, DuplicateGate> {
        let mut seen = BTreeSet::new();
        for gate in &required_gates {
            if !seen.insert(gate.clone()) {
                return Err(DuplicateGate(gate.clone()));
            }
        }
        Ok(Self {
            source,
            event,
            target,
            required_gates,
            metadata,
        })
    }

    pub fn source(&self) -> &StateId {
        &self.source
    }

    pub fn event(&self) -> &EventId {
        &self.event
    }

    pub fn target(&self) -> &StateId {
        &self.target
    }

    pub fn required_gates(&self) -> &[GateId] {
        &self.required_gates
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::{EventId, GateId, StateId, Transition};

    fn transition(source: &str, target: &str, gates: &[&str]) -> Transition {
        Transition::new(
            StateId::parse(source).unwrap(),
            EventId::parse("go").unwrap(),
            StateId::parse(target).unwrap(),
            gates
                .iter()
                .map(|gate| GateId::parse(*gate).unwrap())
                .collect(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn cycles_self_loops_gate_free_and_multi_gate_are_representable() {
        assert_eq!(
            transition("a", "a", &[]).source(),
            transition("a", "a", &[]).target()
        );
        assert_eq!(
            transition("a", "b", &["g1", "g2"]).required_gates().len(),
            2
        );
        assert_eq!(transition("b", "a", &[]).target().as_str(), "a");
    }
}
