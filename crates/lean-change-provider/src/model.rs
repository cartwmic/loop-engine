//! Frozen input and the closed record shapes the driver appends.
use loop_integrations::owner_checkpoint::Actor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

pub const WORKFLOW_ID: &str = "lean-change";

/// The six fixed review questions: (id, text).
pub const QUESTIONS: [(&str, &str); 6] = [
    ("intent", "Does the change meet the approved intent: the goal, every constraint and every criterion? Name any that is not met."),
    ("scope", "Does the diff do only what the intent asks? Name any change the intent does not require."),
    ("edge-cases", "For each criterion, which edge and failure cases can still break it, and does the code or a check handle them? Name an unhandled case with a concrete input."),
    ("checks", "Does each criterion's check exercise the criterion from outside, the way a user would, and fail if the criterion were not met? Name any check that mirrors the implementation or cannot fail."),
    ("docs", "Are the docs, help text, examples and contracts that the change touches or invalidates updated? Name any that now say something false. If the repository keeps a living PRD, name each requirement the change adds, changes or relies on, and the check that proves it; a requirement that no check proves is a finding."),
    ("principles", "Against the owner's design principles (full text in the packet; sha256 in the profile): does the diff violate one? The Gate lines (sections 2, 3 and 7) apply to new components and new cross-component boundaries, not to small changes inside an existing component. Cite the section, the evidence in the diff and the consequence. A finding without an obligation, evidence and consequence is not a finding."),
];

/// The five fixed intent-check questions: (id, text). A failed one carries the concrete question for the owner.
pub const INTENT_QUESTIONS: [(&str, &str); 5] = [
    ("use", "Once this exists, what will the owner or its users need to see or do with it, and which of those can they not do yet under this intent?"),
    ("edge", "What is the likeliest case the listed checks miss, and what should happen then?"),
    ("control", "Where is this intent unclear or silent, such that a delivery would have to guess?"),
    ("terms", "Which term in this intent could mean two things to different readers, and which meaning do you intend?"),
    ("criteria-gap", "Could a plausible delivery pass every criterion's check and still leave the goal, or a problem the goal names, unmet? Give the delivery, show why each check still passes, and state the consequence. Do not ask for more than the goal promises."),
];

fn nonblank(value: &str, what: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{what} must be nonempty"));
    }
    Ok(())
}

/// A proof path is a git pathspec relative to the repository root: nonblank, not absolute, no `..` part.
/// The checks apply to the path after git's pathspec magic, so `:(glob)../x` is refused as `../x` is. `:/` and
/// `:(top)` name the whole repository and stay allowed.
fn proof_path(path: &str) -> Result<(), String> {
    nonblank(path, "proof path")?;
    if path == ":/" {
        return Ok(());
    }
    let (rest, top) = pathspec_after_magic(path);
    if rest.is_empty() {
        return if top {
            Ok(())
        } else {
            Err(format!(
                "proof path {path} must name a path after its magic"
            ))
        };
    }
    if Path::new(rest).is_absolute() {
        return Err(format!(
            "proof path {path} must be relative to the repository"
        ));
    }
    if rest.split('/').any(|part| part == "..") {
        return Err(format!("proof path {path} must not have a .. part"));
    }
    Ok(())
}

/// The path after git's pathspec magic, which is `:(words)` or a short `:` with `!` and `^` characters, and
/// whether the words include `top`. A short `/` is left in the path, so `:!/etc` is absolute after its magic.
fn pathspec_after_magic(path: &str) -> (&str, bool) {
    if let Some((words, rest)) = path
        .strip_prefix(":(")
        .and_then(|long| long.split_once(')'))
    {
        return (rest, words.split(',').any(|word| word == "top"));
    }
    let rest = path
        .strip_prefix(':')
        .map_or(path, |short| short.trim_start_matches(['!', '^']));
    (rest, false)
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
    /// Written by every setup of this version; absent in a profile made by lean-change 0.31.0 or earlier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_vs_new: Option<OldVsNew>,
}

/// Where the proof lives: the old tree is the base commit with these paths taken from the current tree.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OldVsNew {
    pub proof_paths: Vec<String>,
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
        for path in input.old_vs_new.iter().flat_map(|o| &o.proof_paths) {
            proof_path(path)?;
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

/// What a criterion's check must do on the old tree.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    /// A bug fix or a feature: a real failure on the old tree, twice.
    Fails,
    /// A refactor: exit 0 with the output the current tree gives.
    Same,
    /// Brand-new behavior: any result but a pass.
    Absent,
    /// Work the check cannot apply to; the intent gives a reason.
    #[serde(rename = "none")]
    NotApplicable,
}

impl Expect {
    /// The spelling the intent uses.
    pub fn name(self) -> &'static str {
        match self {
            Expect::Fails => "fails",
            Expect::Same => "same",
            Expect::Absent => "absent",
            Expect::NotApplicable => "none",
        }
    }

    /// Runs on the old tree: fails twice, so one lucky pass cannot make a flaky check count; same and absent once; none never.
    pub fn old_runs(self) -> usize {
        match self {
            Expect::Fails => 2,
            Expect::Same | Expect::Absent => 1,
            Expect::NotApplicable => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OldExpectation {
    pub expect: Expect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub statement: String,
    pub check: Check,
    /// Required in an old-vs-new profile, refused in any other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<OldExpectation>,
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

    /// `old` is required in an old-vs-new profile and refused in any other; `none` needs a reason.
    pub fn check_old(&self, old_vs_new: bool) -> Result<(), String> {
        for criterion in &self.criteria {
            match (&criterion.old, old_vs_new) {
                (None, true) => {
                    return Err(format!(
                        "criterion {} needs old: expect fails, same, absent, or none with a reason",
                        criterion.id
                    ))
                }
                (Some(_), false) => {
                    return Err(format!(
                        "criterion {} has old, but its profile was set up by lean-change 0.31.0 or earlier",
                        criterion.id
                    ))
                }
                (Some(old), true)
                    if old.expect == Expect::NotApplicable
                        && old.reason.as_deref().is_none_or(|r| r.trim().is_empty()) =>
                {
                    return Err(format!(
                        "criterion {} expects none, which needs a nonblank reason",
                        criterion.id
                    ))
                }
                _ => {}
            }
        }
        Ok(())
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
    /// Old-vs-new profiles only: the check's runs on the old tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<OldResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OldResult {
    pub expect: Expect,
    pub runs: Vec<OldRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OldRun {
    /// None when the command could not start on the old tree, or was killed by a signal.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_tail: String,
}

impl OldRun {
    /// A real failure exits other than 0, 126 or 127 (not executable, or not found), without timeout.
    pub fn is_real_failure(&self) -> bool {
        !self.timed_out
            && self
                .exit_code
                .is_some_and(|code| !matches!(code, 0 | 126 | 127))
    }
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

/// Profile b: one fresh-context check of the draft intent, before the owner sees it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentCheck {
    /// Record id of the intent that was checked.
    pub intent: String,
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
    /// Record id of the review or intent-check the finding belongs to.
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
