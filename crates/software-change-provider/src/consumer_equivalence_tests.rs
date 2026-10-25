//! Consumer-equivalence check for effective review evidence (test only).
//!
//! The carry / effective-evidence defects (#18, #30, #31, #41, #48) escaped five
//! times because several consumers each decided "what counts as evidence" their own
//! way. PR #46 put one resolver (`evidence::resolve_gate`) behind them. This check keeps
//! them together: it generates review histories (carried, retired, superseded and
//! failing records, appended in varied order), builds every history through the real
//! append rules, then asks each consumer about the same history.
//!
//! Consumers of effective review evidence for one gate and subject revision:
//!
//! 1. the resolver, `evidence::resolve_gate` (the reference answer);
//! 2. the checked gate, `gates::evaluate` on the ordinary review gate's approval edge;
//! 3. commission of the corresponding challenge, through `commission::inspect_with_stage`:
//!    its parent-claim refusal (`missing_inputs`) and its labeled `parent_review` block.
//!
//! Two further consumers need no generated check: admission of a challenge write
//! (`challenge_admission`) reads record kinds, not evidence standing, and the
//! criterion-verdict command index is one function (`validation::verified_command_index`)
//! shared by the gate and by admission.

use crate::protocol::EvaluateRequest;
use crate::{evidence, gates};
use loop_core::{
    CandidateRecord, ContextRecord, EventId, SemanticSequence, StateId, Timestamp, Transition,
    TransitionKind, WorkSlot,
};
use serde_json::{json, Value};
use software_change_provider::{admission, commission};
use std::collections::BTreeSet;
use std::path::PathBuf;

const GATE: &str = "design-review";
const CHALLENGE: &str = "design-adversarial-review";
const SUBJECT: &str = "design.json";
const REVISION: &str = "5";
const AXES: [&str; 3] = ["a", "b", "c"];
const CONFIG: &str = "equivalence-16";

/// A small deterministic generator; the seed is printed with every failure.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn record(id: &str, kind: &str, data: Value) -> (String, String, Value) {
    (id.to_owned(), kind.to_owned(), data)
}

fn review(id: &str, axis: &str, revision: &str, author: &str, assessment: &str, pass: bool) -> Rec {
    record(
        id,
        "review-evidence",
        json!({
            "gate": GATE, "subject": SUBJECT, "subject_revision": revision,
            "review_stage": "aggregate", "policy_id": axis, "group": "design",
            "assessment_id": assessment, "config_version": CONFIG,
            "author": {"name": author, "kind": "agent"},
            "result": if pass { "pass" } else { "fail" },
            "findings": if pass { "" } else { "Finding" },
            "review_contract_version": 3,
            "grounds": {"reason": "inspected design",
                "evidence": [{"locator": "design.json#/elements"}]}
        }),
    )
}

type Rec = (String, String, Value);

fn carry(id: &str, origin: &str, revision: &str) -> Rec {
    record(
        id,
        "evidence-applicability",
        json!({
            "origin": {"kind": "context-record", "id": origin},
            "target": {"subject": SUBJECT, "revision": revision},
            "attesting_driver": {"name": "driver", "kind": "agent"},
            "reason": "design changed only its revision field"
        }),
    )
}

fn withdrawal(id: &str, target: &str) -> Rec {
    record(
        id,
        "record-withdrawal",
        json!({"record_ids": [target], "reason": "obsolete record",
            "attesting_driver": {"name": "driver", "kind": "agent"}}),
    )
}

fn ledger(findings: Vec<Value>) -> Rec {
    record(
        "ledger",
        "finding-ledger",
        json!({"schema_version": "1", "gate": GATE, "subject": SUBJECT,
            "subject_revision": REVISION, "author": {"name": "driver", "kind": "agent"},
            "findings": findings}),
    )
}

/// What one generated history looks like, for the coverage floor.
#[derive(Default, Debug)]
struct Shape {
    carried: bool,
    withdrawn: bool,
    superseded: bool,
    failing: bool,
}

/// Independent event chains (an event follows the events it depends on), plus the
/// finding-ledger variant. Chains are interleaved at random.
fn generate(rng: &mut Rng) -> (Vec<Vec<Rec>>, Option<Rec>, Shape) {
    let mut chains: Vec<Vec<Rec>> = Vec::new();
    let mut shape = Shape::default();
    let mut failing = Vec::new();
    // One history in three keeps a single reviewer and assessment, so cleared
    // groups (the interesting eligible cases) are common, not lucky.
    let tidy = rng.below(3) == 0;
    for axis in AXES {
        let author = move |rng: &mut Rng| if tidy { "A" } else { ["A", "B"][rng.below(2)] };
        let assessment = move |rng: &mut Rng| {
            if tidy {
                "s1"
            } else {
                ["s1", "s2"][rng.below(2)]
            }
        };
        let origin = |rng: &mut Rng| {
            review(
                &format!("orig-{axis}"),
                axis,
                "4",
                if tidy { "A" } else { ["A", "B"][rng.below(2)] },
                "orig",
                true,
            )
        };
        let pick = if tidy {
            [0, 2, 3, 4, 7][rng.below(5)]
        } else {
            rng.below(10)
        };
        match pick {
            // Carried by applicability to the current revision.
            0 | 1 => {
                chains.push(vec![
                    origin(rng),
                    carry(&format!("carry-{axis}"), &format!("orig-{axis}"), REVISION),
                ]);
                shape.carried = true;
            }
            // Fresh pass.
            2 | 3 => chains.push(vec![review(
                &format!("fresh-{axis}"),
                axis,
                REVISION,
                author(rng),
                assessment(rng),
                true,
            )]),
            // Fresh fail, to be recorded in the ledger (or not).
            4 => {
                chains.push(vec![review(
                    &format!("fail-{axis}"),
                    axis,
                    REVISION,
                    author(rng),
                    assessment(rng),
                    false,
                )]);
                failing.push(format!("fail-{axis}"));
                shape.failing = true;
            }
            // Nothing for this axis.
            5 => {}
            // Carried, then retired by withdrawing the carry or the origin.
            6 => {
                let target = if rng.below(2) == 0 {
                    format!("carry-{axis}")
                } else {
                    format!("orig-{axis}")
                };
                chains.push(vec![
                    origin(rng),
                    carry(&format!("carry-{axis}"), &format!("orig-{axis}"), REVISION),
                    withdrawal(&format!("withdraw-{axis}"), &target),
                ]);
                shape.carried = true;
                shape.withdrawn = true;
            }
            // Carried, and a fresh pass for the same axis appended before or after.
            7 => {
                chains.push(vec![
                    origin(rng),
                    carry(&format!("carry-{axis}"), &format!("orig-{axis}"), REVISION),
                ]);
                chains.push(vec![review(
                    &format!("fresh-{axis}"),
                    axis,
                    REVISION,
                    author(rng),
                    assessment(rng),
                    true,
                )]);
                shape.carried = true;
                shape.superseded = true;
            }
            // Old-revision judgment never carried, or carried to the wrong revision.
            8 => {
                let mut chain = vec![origin(rng)];
                if rng.below(2) == 0 {
                    chain.push(carry(
                        &format!("carry-{axis}"),
                        &format!("orig-{axis}"),
                        "9",
                    ));
                }
                chains.push(chain);
            }
            // A fresh pass that was then withdrawn.
            _ => {
                chains.push(vec![
                    review(
                        &format!("fresh-{axis}"),
                        axis,
                        REVISION,
                        author(rng),
                        assessment(rng),
                        true,
                    ),
                    withdrawal(&format!("withdraw-{axis}"), &format!("fresh-{axis}")),
                ]);
                shape.withdrawn = true;
            }
        }
    }
    // Ledger: absent, empty, or with every failing review recorded.
    let ledger = match (failing.is_empty(), rng.below(4)) {
        (_, 0) if !tidy => None,
        (true, _) | (false, 1) => Some(ledger(Vec::new())),
        (false, variant) => {
            let accepted = variant == 2;
            Some(ledger(
                failing
                    .iter()
                    .map(|id| {
                        let axis = id.trim_start_matches("fail-");
                        json!({"id": format!("F-{axis}"),
                            "source": {"kind": "context-record", "id": id},
                            "policy_id": axis, "statement": "Finding",
                            "disposition": if accepted { "accepted" } else { "rejected" },
                            "status": if accepted { "unresolved" } else { "recorded" },
                            "reason": "Recorded exact-source decision",
                            "owner_phase": if accepted { Some("design") } else { None },
                            "task_ids": [],
                            "review_axes": if accepted { vec![axis] } else { Vec::new() }})
                    })
                    .collect(),
            ))
        }
    };
    (chains, ledger, shape)
}

/// Random interleaving of the chains; the ledger lands after the records it cites.
fn interleave(rng: &mut Rng, mut chains: Vec<Vec<Rec>>, ledger: Option<Rec>) -> Vec<Rec> {
    chains.iter_mut().for_each(|chain| chain.reverse());
    let mut order = Vec::new();
    loop {
        let open: Vec<usize> = (0..chains.len())
            .filter(|i| !chains[*i].is_empty())
            .collect();
        if open.is_empty() {
            break;
        }
        let pick = open[rng.below(open.len())];
        order.push(chains[pick].pop().unwrap());
    }
    if let Some(ledger) = ledger {
        let cited: Vec<String> = ledger.2["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["source"]["id"].as_str().map(str::to_owned))
            .collect();
        let earliest = order
            .iter()
            .rposition(|(id, _, _)| cited.contains(id))
            .map_or(0, |index| index + 1);
        order.insert(earliest + rng.below(order.len() - earliest + 1), ledger);
    }
    order
}

struct Fixture {
    root: PathBuf,
    input: Value,
    workflow: loop_core::Workflow,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "software-change-consumer-equivalence-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("intent.json"), br#"{"revision":"1"}"#).unwrap();
        std::fs::write(
            root.join(SUBJECT),
            br#"{"revision":"5","author":{"name":"driver","kind":"agent"},"elements":[{"name":"x"},{"name":"y"}],"decisions":[]}"#,
        )
        .unwrap();
        let axes: Vec<Value> = AXES
            .iter()
            .map(|axis| json!({"id": axis, "description": axis, "group": "design", "review_stage": "aggregate"}))
            .collect();
        let input = json!({
            "contract_version": 3,
            "artifact_root": root,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": CONFIG,
            "artifact_schemas": {SUBJECT: {
                "type": "object",
                "properties": {
                    "revision": {"type": "string"},
                    "author": {"type": "object",
                        "properties": {"name": {"type": "string"},
                            "kind": {"type": "string", "enum": ["human", "agent", "script"]}},
                        "required": ["name", "kind"], "additionalProperties": false}
                },
                "required": ["revision", "author"]
            }},
            "review_policies": {GATE: axes.clone(), CHALLENGE: axes}
        });
        let workflow = crate::workflow::describe_workflow(Some(&input)).unwrap();
        Self {
            root,
            input,
            workflow,
        }
    }

    /// The history as the append path admits it: each record is checked against the
    /// records before it, and a refused record never enters the history.
    fn append(&self, records: &[Rec]) -> Vec<ContextRecord> {
        let mut history: Vec<ContextRecord> = Vec::new();
        for (id, kind, data) in records {
            let candidate = CandidateRecord {
                id: id.as_str().into(),
                kind: kind.clone(),
                data: data.clone(),
            };
            let admitted =
                admission::AdmissionContext::new(&self.input, &history, GATE, 0, &self.workflow)
                    .map_err(|e| e.to_string())
                    .and_then(|ctx| admission::check(&ctx, &candidate).map_err(|r| r.message));
            if admitted.is_ok() {
                let sequence = history.len() as u64 + 1;
                history.push(ContextRecord::new(
                    id.as_str(),
                    kind.as_str(),
                    data.clone(),
                    SemanticSequence::new(sequence),
                    Timestamp::from_unix_millis(sequence as i64),
                ));
            }
        }
        history
    }

    /// Resolver: eligibility, named cause categories, and whether any row is carried.
    fn resolver(&self, history: &[ContextRecord]) -> (bool, BTreeSet<String>, bool) {
        let owner = evidence::AuthorIdentity::new("driver", "agent");
        let resolved =
            evidence::resolve_gate(&self.input, history, GATE, SUBJECT, REVISION, &owner).unwrap();
        let categories = resolved.causes().into_iter().map(|(c, _)| c).collect();
        let carried = resolved
            .evidence
            .rows
            .iter()
            .any(|row| row.evidence.carried);
        (resolved.is_eligible(), categories, carried)
    }

    /// Checked gate: does the ordinary review gate's approval edge allow?
    fn gate(&self, history: &[ContextRecord]) -> (bool, Value) {
        let request = EvaluateRequest {
            operation: "evaluate".to_owned(),
            workflow: self.workflow.clone(),
            initial_input: self.input.clone(),
            context: history.to_vec(),
            transition: Transition::new(
                StateId::new(GATE),
                EventId::new("approved"),
                StateId::new(CHALLENGE),
                TransitionKind::Checked,
            ),
            prior_evaluations: Vec::new(),
            driver_act: None,
        };
        match gates::evaluate(&request) {
            gates::EvaluationOutcome::Response(value) => (value["result"] == "allow", value),
            gates::EvaluationOutcome::EvaluationError(error) => panic!("gate error: {error}"),
        }
    }

    /// Commission of the challenge: refusal lines about the parent review, and the
    /// labeled parent block when one is present.
    fn commission(&self, history: &[ContextRecord]) -> (Vec<String>, Option<Value>) {
        let slot = WorkSlot::new(CHALLENGE, CHALLENGE, "approved").with_stdin_context_kinds(vec![
            "review-evidence".into(),
            "evidence-applicability".into(),
            "finding-ledger".into(),
        ]);
        let input = json!({"status": "completed", "result": {
            "artifact_root": self.root, "work_slots": [slot], "context": history,
            "initial_input": self.input
        }});
        match commission::inspect_with_stage(&input, Some(CHALLENGE), None, None) {
            Ok(output) => (
                output["missing_inputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|line| line.as_str().map(str::to_owned))
                    .collect(),
                output
                    .get("parent_review")
                    .cloned()
                    .filter(|v| !v.is_null()),
            ),
            Err(error) => (vec![error], None),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn names(history: &[ContextRecord]) -> String {
    history
        .iter()
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>()
        .join(" < ")
}

#[test]
fn every_consumer_of_effective_review_evidence_agrees_with_the_resolver() {
    let fixture = Fixture::new();
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    for seed in 1..=150u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (chains, ledger, shape) = generate(&mut rng);
        let history = fixture.append(&interleave(&mut rng, chains, ledger));
        let at = format!("seed {seed}: {} {shape:?}", names(&history));

        let (eligible, causes, carried) = fixture.resolver(&history);
        let (allowed, gate_response) = fixture.gate(&history);
        let (lines, parent_review) = fixture.commission(&history);

        // The checked gate and the resolver give the same answer.
        assert_eq!(
            allowed, eligible,
            "{at}: gate {gate_response} vs causes {causes:?}"
        );

        // Commission never refuses what the gate accepts, and never accepts a parent
        // the gate refuses when carried evidence is in play.
        if eligible {
            assert!(
                lines.is_empty(),
                "{at}: commission refused an eligible parent: {lines:?}"
            );
        }
        let carry_in_play = history.iter().any(|r| r.kind == "evidence-applicability");
        if !eligible && (carried || carry_in_play) {
            assert!(
                !lines.is_empty(),
                "{at}: commission accepted an ineligible parent: {causes:?}"
            );
        }
        // Every cause commission names is a cause the resolver names.
        for line in &lines {
            if let Some(start) = line.find("cannot use parent review") {
                let category = line[start..]
                    .split('(')
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .unwrap_or("");
                assert!(
                    causes.contains(category),
                    "{at}: commission named {category:?}, resolver {causes:?}"
                );
            }
        }
        // The labeled parent block exists exactly when a row is carried, and agrees.
        assert_eq!(
            parent_review.is_some(),
            carried,
            "{at}: parent_review presence"
        );
        if let Some(block) = parent_review {
            assert_eq!(block["eligible"], eligible, "{at}: parent_review.eligible");
            let named: BTreeSet<String> = block["causes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["cause"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(named, causes, "{at}: parent_review causes");
        }

        *seen
            .entry(if eligible { "eligible" } else { "refused" })
            .or_default() += 1;
        for (name, hit) in [
            ("carried", shape.carried),
            ("withdrawn", shape.withdrawn),
            ("superseded", shape.superseded),
            ("failing", shape.failing),
            ("eligible-with-carry", eligible && carried),
            ("refused-with-carry", !eligible && carried),
        ] {
            if hit {
                *seen.entry(name).or_default() += 1;
            }
        }
    }
    eprintln!("consumer-equivalence histories: {seen:?}");
    // The generator must reach every kind of history, or the check proves nothing.
    for name in [
        "eligible",
        "refused",
        "carried",
        "withdrawn",
        "superseded",
        "failing",
        "eligible-with-carry",
        "refused-with-carry",
    ] {
        assert!(
            seen.get(name).copied().unwrap_or(0) >= 10,
            "generator coverage: {name} {seen:?}"
        );
    }
}
