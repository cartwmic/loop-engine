//! Frozen input and the closed record shapes the driver appends.
use loop_integrations::owner_checkpoint::Actor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const WORKFLOW_ID: &str = "lean-change";

/// The six fixed review questions: (id, text).
pub const QUESTIONS: [(&str, &str); 6] = [
    ("intent", "Does the change meet the approved intent: the goal, every constraint and every criterion? Name any that is not met."),
    ("scope", "Does the diff do only what the intent asks? Name any change the intent does not require."),
    ("edge-cases", "For each criterion, which edge and failure cases can still break it, and does the code or a check handle them? Name an unhandled case with a concrete input."),
    ("checks", "Does each criterion's check exercise the criterion from outside, the way a user would, and fail if the criterion were not met? Name any check that mirrors the implementation or cannot fail."),
    ("docs", "Are the docs, help text, examples and contracts that the change touches or invalidates updated? Name any that now say something false."),
    ("principles", "Against the owner's design principles (full text in the packet; sha256 in the profile): does the diff violate one? The Gate lines (sections 2, 3 and 7) apply to new components and new cross-component boundaries, not to small changes inside an existing component. Cite the section, the evidence in the diff and the consequence. A finding without an obligation, evidence and consequence is not a finding."),
];

fn nonblank(value: &str, what: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{what} must be nonempty"));
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Principles {
    pub source: String,
    pub sha256: String,
    pub text: String,
}

/// Frozen at run start by `setup`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitialInput {
    pub schema_version: u32,
    pub profile_version: String,
    pub repo: String,
    pub base: String,
    /// B: one enforced review. C: the same code with the review state absent.
    pub review: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principles: Option<Principles>,
    /// Added by the engine at start; this provider keeps no artifacts there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[allow(dead_code)]
    pub artifact_root: Option<String>,
}

impl InitialInput {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let input: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid lean-change initial input: {e}"))?;
        if input.schema_version != 1 {
            return Err("schema_version must be 1".into());
        }
        nonblank(&input.profile_version, "profile_version")?;
        nonblank(&input.base, "base")?;
        if !std::path::Path::new(&input.repo).is_absolute() {
            return Err("repo must be an absolute path".into());
        }
        match (&input.principles, input.review) {
            (Some(p), _) => {
                nonblank(&p.text, "principles.text")?;
                if sha256_hex(p.text.as_bytes()) != p.sha256 {
                    return Err("principles.sha256 does not match principles.text".into());
                }
            }
            (None, true) => return Err("a review profile requires principles".into()),
            (None, false) => {}
        }
        Ok(input)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub argv: Vec<String>,
    pub timeout_ms: u64,
}

impl Check {
    /// Identity of the approved check, recorded with every result.
    pub fn sha256(&self) -> String {
        sha256_hex(
            serde_json::to_string(self)
                .expect("check serializes")
                .as_bytes(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub statement: String,
    pub check: Check,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub author: Actor,
    pub goal: String,
    pub constraints: Vec<String>,
    pub hard: bool,
    pub criteria: Vec<Criterion>,
}

impl Intent {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let intent: Self = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        nonblank(&intent.author.name, "author.name")?;
        nonblank(&intent.goal, "goal")?;
        if intent.constraints.iter().any(|c| c.trim().is_empty()) {
            return Err("constraints must be nonempty strings".into());
        }
        if intent.criteria.is_empty() {
            return Err("at least one criterion is required".into());
        }
        let mut seen = BTreeSet::new();
        for criterion in &intent.criteria {
            nonblank(&criterion.id, "criterion id")?;
            nonblank(&criterion.statement, "criterion statement")?;
            if !seen.insert(criterion.id.as_str()) {
                return Err(format!("duplicate criterion id {}", criterion.id));
            }
            let check = &criterion.check;
            if check.argv.first().is_none_or(|c| c.trim().is_empty()) {
                return Err(format!("criterion {} needs a check command", criterion.id));
            }
            if !(1..=3_600_000).contains(&check.timeout_ms) {
                return Err(format!(
                    "criterion {} timeout_ms must be 1..=3600000",
                    criterion.id
                ));
            }
        }
        Ok(intent)
    }
}

/// Written by `lean-change verify`; the driver appends it as kind `verification`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    /// Record id of the intent whose criteria were run.
    pub intent: String,
    /// Repository proof identity after the checks ran.
    pub tree: String,
    pub results: Vec<CheckResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub criterion: String,
    pub check_sha256: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_tail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub intent: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub intent: String,
    pub tree: String,
    pub reviewer: Actor,
    pub fresh_context: bool,
    pub answers: Vec<Answer>,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub question: String,
    pub verdict: Verdict,
    pub note: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub question: String,
    pub text: String,
    pub evidence: String,
    pub consequence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Disposition {
    /// Record id of the review the finding belongs to.
    pub review: String,
    pub finding: String,
    pub disposition: DispositionKind,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DispositionKind {
    Fixed,
    Rejected,
}
