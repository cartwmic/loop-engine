//! Pure fold of per-role model bindings and owner amendments.
//!
//! A run may freeze an optional `model_bindings` input key and later append
//! `model-binding-amendment` context records. This module folds both, in
//! durable order, into the original bindings, the bindings in force at any
//! prefix of history, the ordered amendment history and the stale marks. It
//! does no I/O and judges declared text only: provider/model and thinking
//! level are compared as exact strings, with no aliasing or normalization.
//!
//! Frozen input shape: `{"roles": {<role>: binding, ...}, "driver_session":
//! session}`, where `driver_session` is required exactly when `roles.driver`
//! exists. A binding and a session are both `{"model", "thinking"}`.
//!
//! Amendment record data shape: `{"actor": {"name", "kind"}, "authority",
//! "words", "changes": [{"role", "old": binding|null, "new": binding|null,
//! "mark_stale"?: bool}], "driver_session"?: session}`. `authority` is an
//! object with `kind: "owner"` (the shape of the author-count amendment) or the
//! bare string `"owner"`.

use crate::{ContextRecord, SemanticSequence, Timestamp};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// Frozen run-input key holding the original bindings.
pub const MODEL_BINDINGS_INPUT_KEY: &str = "model_bindings";
/// Context record kind of an owner binding amendment.
pub const MODEL_BINDING_AMENDMENT_RECORD_KIND: &str = "model-binding-amendment";
/// Context record kind of a declared session for an author draft or implementer report.
pub const SESSION_DECLARATION_RECORD_KIND: &str = "session-declaration";

/// The seven roles a model binding can govern, in display order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ModelRole {
    #[serde(rename = "driver")]
    Driver,
    #[serde(rename = "author")]
    Author,
    #[serde(rename = "implementer")]
    Implementer,
    #[serde(rename = "ordinary-reviewer")]
    OrdinaryReviewer,
    #[serde(rename = "challenge-reviewer")]
    ChallengeReviewer,
    #[serde(rename = "cold-reader")]
    ColdReader,
    #[serde(rename = "criterion-author")]
    CriterionAuthor,
}

impl ModelRole {
    /// Every role in display order.
    pub const ALL: [ModelRole; 7] = [
        ModelRole::Driver,
        ModelRole::Author,
        ModelRole::Implementer,
        ModelRole::OrdinaryReviewer,
        ModelRole::ChallengeReviewer,
        ModelRole::ColdReader,
        ModelRole::CriterionAuthor,
    ];

    /// The exact role id used in input, records and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelRole::Driver => "driver",
            ModelRole::Author => "author",
            ModelRole::Implementer => "implementer",
            ModelRole::OrdinaryReviewer => "ordinary-reviewer",
            ModelRole::ChallengeReviewer => "challenge-reviewer",
            ModelRole::ColdReader => "cold-reader",
            ModelRole::CriterionAuthor => "criterion-author",
        }
    }

    /// Parse an exact role id; `None` for any other text.
    pub fn parse(text: &str) -> Option<ModelRole> {
        Self::ALL.into_iter().find(|role| role.as_str() == text)
    }

    /// Whether an amendment may mark this role's earlier evidence stale.
    pub const fn may_be_marked_stale(self) -> bool {
        matches!(
            self,
            ModelRole::OrdinaryReviewer
                | ModelRole::ChallengeReviewer
                | ModelRole::ColdReader
                | ModelRole::CriterionAuthor
        )
    }
}

impl fmt::Display for ModelRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A required (or declared) provider/model and thinking level, as exact text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Binding {
    pub model: String,
    pub thinking: String,
}

/// A declared session has the same two fields as a binding.
pub type Session = Binding;

impl Binding {
    /// Parse `{"model", "thinking"}` with two nonblank strings and no other key.
    pub fn from_value(value: &Value) -> Result<Binding, String> {
        let object = value
            .as_object()
            .ok_or("must be an object with `model` and `thinking`")?;
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "model" | "thinking"))
        {
            return Err(format!("has unknown field `{key}`"));
        }
        let field = |name: &str| match object.get(name).and_then(Value::as_str) {
            Some(text) if !text.trim().is_empty() => Ok(text.to_owned()),
            _ => Err(format!("`{name}` must be a nonblank string")),
        };
        Ok(Binding {
            model: field("model")?,
            thinking: field("thinking")?,
        })
    }

    /// The `{"model", "thinking"}` JSON object.
    pub fn to_value(&self) -> Value {
        json!({"model": self.model, "thinking": self.thinking})
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at thinking {}", self.model, self.thinking)
    }
}

fn describe(binding: Option<&Binding>) -> String {
    binding.map_or_else(|| "none".to_owned(), Binding::to_string)
}

/// Bindings by role. A role absent from the map is unbound.
pub type RoleBindings = BTreeMap<ModelRole, Binding>;

/// The frozen `model_bindings` input after validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FrozenModelBindings {
    pub roles: RoleBindings,
    /// Present exactly when `roles` binds the driver.
    pub driver_session: Option<Session>,
}

/// Validate the value of the frozen `model_bindings` input key.
///
/// Checks the roles, both binding fields and that `driver_session` exists
/// exactly when `roles.driver` exists and equals it.
pub fn parse_frozen_model_bindings(value: &Value) -> Result<FrozenModelBindings, String> {
    let object = value
        .as_object()
        .ok_or("model_bindings must be an object with `roles`")?;
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "roles" | "driver_session"))
    {
        return Err(format!("model_bindings has unknown field `{key}`"));
    }
    let roles_object = object
        .get("roles")
        .and_then(Value::as_object)
        .ok_or("model_bindings.roles must be an object")?;
    let mut roles = RoleBindings::new();
    for (name, binding) in roles_object {
        let role = ModelRole::parse(name)
            .ok_or_else(|| format!("model_bindings.roles has unknown role `{name}`"))?;
        let binding = Binding::from_value(binding)
            .map_err(|error| format!("model_bindings.roles.{name} {error}"))?;
        roles.insert(role, binding);
    }
    let driver_session = object
        .get("driver_session")
        .map(|session| {
            Binding::from_value(session)
                .map_err(|error| format!("model_bindings.driver_session {error}"))
        })
        .transpose()?;
    match (roles.get(&ModelRole::Driver), &driver_session) {
        (Some(_), None) => {
            return Err(format!(
                "model_bindings binds the driver, so driver_session is required: {}",
                SessionMismatch::new(ModelRole::Driver, &roles[&ModelRole::Driver], None)
            ))
        }
        (Some(binding), Some(session)) => {
            if let Err(mismatch) =
                check_declared_session(ModelRole::Driver, Some(binding), Some(session))
            {
                return Err(format!(
                    "model_bindings driver_session is refused: {mismatch}"
                ));
            }
        }
        (None, Some(_)) => {
            return Err(
                "model_bindings.driver_session is only allowed when roles.driver is bound".into(),
            )
        }
        (None, None) => {}
    }
    Ok(FrozenModelBindings {
        roles,
        driver_session,
    })
}

/// Which part of a declared session differs from the binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum SessionDifference {
    /// No statement was supplied.
    Missing,
    /// Only the provider/model differs.
    Model,
    /// Only the thinking level differs.
    Thinking,
    /// Both differ.
    ModelAndThinking,
}

/// A declared session that is missing or unequal to the binding in force.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionMismatch {
    pub role: ModelRole,
    pub expected: Binding,
    /// `None` when the statement is missing.
    pub actual: Option<Session>,
    pub difference: SessionDifference,
}

impl SessionMismatch {
    fn new(role: ModelRole, expected: &Binding, actual: Option<&Session>) -> SessionMismatch {
        let difference = match actual {
            None => SessionDifference::Missing,
            Some(actual) => match (
                actual.model != expected.model,
                actual.thinking != expected.thinking,
            ) {
                (true, true) => SessionDifference::ModelAndThinking,
                (true, false) => SessionDifference::Model,
                _ => SessionDifference::Thinking,
            },
        };
        SessionMismatch {
            role,
            expected: expected.clone(),
            actual: actual.cloned(),
            difference,
        }
    }
}

impl fmt::Display for SessionMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "model binding for role `{}` is not met: expected model `{}` at thinking `{}`; ",
            self.role, self.expected.model, self.expected.thinking
        )?;
        match (&self.actual, self.difference) {
            (None, _) => formatter.write_str("actual: no session declared (statement missing)"),
            (Some(actual), difference) => write!(
                formatter,
                "actual model `{}` at thinking `{}`; differs in {}",
                actual.model,
                actual.thinking,
                match difference {
                    SessionDifference::Model => "model only",
                    SessionDifference::Thinking => "thinking level only",
                    _ => "model and thinking level",
                }
            ),
        }
    }
}

/// Compare a declared session with the binding in force for `role`.
///
/// `Ok` when the role is unbound (`in_force` is `None`) or the declared
/// session equals the binding exactly; otherwise the mismatch names the role,
/// expected, actual and which part differs.
pub fn check_declared_session(
    role: ModelRole,
    in_force: Option<&Binding>,
    declared: Option<&Session>,
) -> Result<(), SessionMismatch> {
    match in_force {
        Some(expected) if declared != Some(expected) => {
            Err(SessionMismatch::new(role, expected, declared))
        }
        _ => Ok(()),
    }
}

/// The cause of an amendment refusal. `code()` is the stable cause name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum AmendmentCause {
    /// Not an object, or `changes` missing, empty, malformed or naming a role twice.
    Malformed,
    ActorNotHuman,
    AuthorityNotOwner,
    WordsBlank,
    UnknownRole,
    /// A stated old binding is not the binding currently in force.
    OldBindingNotInForce,
    /// A stated new binding has a blank or missing provider/model or thinking level.
    NewBindingMalformed,
    /// The driver's declared session is missing but the driver is bound afterward.
    DriverDeclarationMissing,
    /// The driver's declared session does not equal the driver binding after the amendment.
    DriverDeclarationMismatch,
    /// Stale marking was requested for a role that produces no evidence.
    StaleMarkNotAllowed,
}

impl AmendmentCause {
    pub const fn code(self) -> &'static str {
        match self {
            AmendmentCause::Malformed => "malformed-amendment",
            AmendmentCause::ActorNotHuman => "actor-not-human",
            AmendmentCause::AuthorityNotOwner => "authority-not-owner",
            AmendmentCause::WordsBlank => "words-blank",
            AmendmentCause::UnknownRole => "unknown-role",
            AmendmentCause::OldBindingNotInForce => "old-binding-not-in-force",
            AmendmentCause::NewBindingMalformed => "new-binding-malformed",
            AmendmentCause::DriverDeclarationMissing => "driver-declaration-missing",
            AmendmentCause::DriverDeclarationMismatch => "driver-declaration-mismatch",
            AmendmentCause::StaleMarkNotAllowed => "stale-mark-not-allowed",
        }
    }
}

/// A refused amendment: the cause and a diagnostic that names it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AmendmentRefusal {
    pub cause: AmendmentCause,
    pub message: String,
}

impl fmt::Display for AmendmentRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "model binding amendment refused ({}): {}",
            self.cause.code(),
            self.message
        )
    }
}

fn refuse(cause: AmendmentCause, message: impl Into<String>) -> AmendmentRefusal {
    AmendmentRefusal {
        cause,
        message: message.into(),
    }
}

/// One role change in an accepted amendment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RoleChange {
    pub role: ModelRole,
    pub old: Option<Binding>,
    pub new: Option<Binding>,
    pub mark_stale: bool,
}

/// The actor of an amendment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AmendmentActor {
    pub name: String,
    pub kind: String,
}

/// An amendment that passed validation against the bindings in force.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidAmendment {
    pub actor: AmendmentActor,
    pub authority: Value,
    pub words: String,
    pub changes: Vec<RoleChange>,
    pub driver_session: Option<Session>,
}

impl ValidAmendment {
    /// The bindings in force once this amendment takes effect.
    pub fn apply(&self, in_force: &RoleBindings) -> RoleBindings {
        let mut after = in_force.clone();
        for change in &self.changes {
            match &change.new {
                Some(binding) => after.insert(change.role, binding.clone()),
                None => after.remove(&change.role),
            };
        }
        after
    }

    /// Roles whose earlier evidence this amendment marks stale.
    pub fn stale_roles(&self) -> Vec<ModelRole> {
        self.changes
            .iter()
            .filter(|change| change.mark_stale)
            .map(|change| change.role)
            .collect()
    }
}

fn nonblank_text<'a>(data: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    data.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

/// Validate one candidate amendment against the bindings currently in force.
///
/// Refuses, with a distinct named cause, when: the actor is not human; the
/// authority is not owner; the words are blank; a role is unknown; a stated
/// old binding is not the binding in force (adding to a bound role, or
/// changing or removing an unbound one); a new binding is malformed; stale
/// marking names a role that produces no evidence; or the driver is bound
/// after the amendment and `driver_session` is missing or unequal to it. A
/// structurally broken candidate is refused as `Malformed`. Checks run in
/// that order and the first failure is reported.
pub fn validate_amendment(
    in_force: &RoleBindings,
    candidate: &Value,
) -> Result<ValidAmendment, AmendmentRefusal> {
    let data = candidate
        .as_object()
        .ok_or_else(|| refuse(AmendmentCause::Malformed, "amendment must be an object"))?;
    let actor = data.get("actor").and_then(Value::as_object);
    let actor_kind = actor
        .and_then(|actor| actor.get("kind"))
        .and_then(Value::as_str);
    if actor_kind != Some("human") {
        return Err(refuse(
            AmendmentCause::ActorNotHuman,
            format!(
                "only a human actor may amend bindings; actor.kind is {}",
                actor_kind.map_or("missing".to_owned(), |kind| format!("`{kind}`"))
            ),
        ));
    }
    let actor = actor.expect("human actor is an object");
    let actor_name = nonblank_text(actor, "name").ok_or_else(|| {
        refuse(
            AmendmentCause::Malformed,
            "actor.name must be a nonblank string",
        )
    })?;
    let authority_kind = match data.get("authority") {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(Value::Object(object)) => object.get("kind").and_then(Value::as_str),
        _ => None,
    };
    if authority_kind != Some("owner") {
        return Err(refuse(
            AmendmentCause::AuthorityNotOwner,
            "authority must be owner; standing delegation and other authority do not amend bindings",
        ));
    }
    let words = nonblank_text(data, "words").ok_or_else(|| {
        refuse(
            AmendmentCause::WordsBlank,
            "the owner's words must be a nonblank string",
        )
    })?;
    let entries = data
        .get("changes")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| {
            refuse(
                AmendmentCause::Malformed,
                "changes must be a nonempty array",
            )
        })?;
    let mut changes: Vec<RoleChange> = Vec::new();
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or_else(|| refuse(AmendmentCause::Malformed, "each change must be an object"))?;
        if let Some(key) = entry
            .keys()
            .find(|key| !matches!(key.as_str(), "role" | "old" | "new" | "mark_stale"))
        {
            return Err(refuse(
                AmendmentCause::Malformed,
                format!("change has unknown field `{key}`"),
            ));
        }
        let role_text = entry.get("role").and_then(Value::as_str).unwrap_or("");
        let role = ModelRole::parse(role_text).ok_or_else(|| {
            refuse(
                AmendmentCause::UnknownRole,
                format!(
                    "unknown role `{role_text}`; roles are {}",
                    ModelRole::ALL.map(ModelRole::as_str).join(", ")
                ),
            )
        })?;
        if changes.iter().any(|change| change.role == role) {
            return Err(refuse(
                AmendmentCause::Malformed,
                format!("role `{role}` is named by more than one change"),
            ));
        }
        let (Some(old), Some(new)) = (entry.get("old"), entry.get("new")) else {
            return Err(refuse(
                AmendmentCause::Malformed,
                format!("change for role `{role}` needs both `old` and `new` (null for none)"),
            ));
        };
        let current = in_force.get(&role);
        let old_matches = match (old, current) {
            (Value::Null, None) => true,
            (Value::Null, Some(_)) => false,
            (stated, current) => Binding::from_value(stated).ok().as_ref() == current,
        };
        if !old_matches {
            return Err(refuse(
                AmendmentCause::OldBindingNotInForce,
                format!(
                    "role `{role}`: stated old binding is {}, but the binding in force is {}",
                    describe_stated(old),
                    describe(current)
                ),
            ));
        }
        let new = match new {
            Value::Null => None,
            stated => Some(Binding::from_value(stated).map_err(|error| {
                refuse(
                    AmendmentCause::NewBindingMalformed,
                    format!("role `{role}`: new binding {error}"),
                )
            })?),
        };
        let mark_stale = match entry.get("mark_stale") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => {
                return Err(refuse(
                    AmendmentCause::Malformed,
                    format!("role `{role}`: mark_stale must be a boolean"),
                ))
            }
        };
        if mark_stale && !role.may_be_marked_stale() {
            return Err(refuse(
                AmendmentCause::StaleMarkNotAllowed,
                format!(
                    "role `{role}` produces no evidence; only ordinary-reviewer, challenge-reviewer, cold-reader and criterion-author may be marked stale"
                ),
            ));
        }
        changes.push(RoleChange {
            role,
            old: current.cloned(),
            new,
            mark_stale,
        });
    }
    let amendment = ValidAmendment {
        actor: AmendmentActor {
            name: actor_name.to_owned(),
            kind: "human".to_owned(),
        },
        authority: data["authority"].clone(),
        words: words.to_owned(),
        changes,
        driver_session: None,
    };
    let driver = amendment.apply(in_force).remove(&ModelRole::Driver);
    let driver_session = match data.get("driver_session") {
        None | Some(Value::Null) => None,
        Some(session) => Some(Binding::from_value(session).map_err(|error| {
            refuse(
                AmendmentCause::DriverDeclarationMismatch,
                format!("driver_session {error}"),
            )
        })?),
    };
    if let Some(binding) = &driver {
        if let Err(mismatch) =
            check_declared_session(ModelRole::Driver, Some(binding), driver_session.as_ref())
        {
            let cause = if driver_session.is_none() {
                AmendmentCause::DriverDeclarationMissing
            } else {
                AmendmentCause::DriverDeclarationMismatch
            };
            return Err(refuse(cause, format!("driver declaration: {mismatch}")));
        }
    }
    Ok(ValidAmendment {
        driver_session,
        ..amendment
    })
}

fn describe_stated(stated: &Value) -> String {
    match stated {
        Value::Null => "none".to_owned(),
        other => match Binding::from_value(other) {
            Ok(binding) => binding.to_string(),
            Err(_) => other.to_string(),
        },
    }
}

/// One accepted amendment in history order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AmendmentEntry {
    pub record_id: String,
    pub sequence: SemanticSequence,
    pub created_at: Timestamp,
    pub actor: AmendmentActor,
    pub authority: Value,
    pub words: String,
    pub changes: Vec<RoleChange>,
    pub driver_session: Option<Session>,
    /// Roles whose earlier evidence this amendment marks stale.
    pub stale_roles: Vec<ModelRole>,
}

/// An amendment's stale mark: evidence of `role` stored before `sequence` is stale.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StaleMark {
    pub amendment_id: String,
    pub sequence: SemanticSequence,
    pub role: ModelRole,
}

/// The folded model-binding state of one run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelBindings {
    /// Frozen original bindings; empty when the input has none.
    pub original: RoleBindings,
    /// The frozen driver declaration, when the original binds the driver.
    pub original_driver_session: Option<Session>,
    /// Amendments in durable order.
    pub history: Vec<AmendmentEntry>,
}

/// Fold the frozen input and the ordered context records.
///
/// Records are taken in `sequence` order; only `model-binding-amendment`
/// records count. A frozen value or a stored amendment that does not validate
/// against the bindings in force at its position is an error naming it, since
/// admission refuses such records before they are stored. With no
/// `model_bindings` input key and no amendment the result is the empty fold.
pub fn fold_model_bindings(
    initial_input: &Value,
    context: &[ContextRecord],
) -> Result<ModelBindings, String> {
    let frozen = initial_input
        .get(MODEL_BINDINGS_INPUT_KEY)
        .map(parse_frozen_model_bindings)
        .transpose()?;
    let (original, original_driver_session) = frozen
        .map(|frozen| (frozen.roles, frozen.driver_session))
        .unwrap_or_default();
    let mut ordered: Vec<&ContextRecord> = context
        .iter()
        .filter(|record| record.kind == MODEL_BINDING_AMENDMENT_RECORD_KIND)
        .collect();
    ordered.sort_by_key(|record| record.sequence);
    let mut in_force = original.clone();
    let mut history = Vec::new();
    for record in ordered {
        let valid = validate_amendment(&in_force, &record.data).map_err(|refusal| {
            format!(
                "model binding amendment `{}` is invalid: {refusal}",
                record.id
            )
        })?;
        in_force = valid.apply(&in_force);
        history.push(AmendmentEntry {
            record_id: record.id.to_string(),
            sequence: record.sequence,
            created_at: record.created_at,
            stale_roles: valid.stale_roles(),
            actor: valid.actor,
            authority: valid.authority,
            words: valid.words,
            changes: valid.changes,
            driver_session: valid.driver_session,
        });
    }
    Ok(ModelBindings {
        original,
        original_driver_session,
        history,
    })
}

impl ModelBindings {
    /// True when the run never had bindings: none frozen and no amendment.
    pub fn never_bound(&self) -> bool {
        self.original.is_empty() && self.history.is_empty()
    }

    /// Bindings in force after every amendment in the context.
    pub fn in_force(&self) -> RoleBindings {
        self.in_force_before(None)
    }

    /// Bindings in force for a record stored at `sequence`: amendments at or
    /// after `sequence` have not taken effect. `None` means after all history.
    /// An amendment therefore never changes the answer for an earlier prefix.
    pub fn in_force_before(&self, sequence: Option<SemanticSequence>) -> RoleBindings {
        let mut in_force = self.original.clone();
        for entry in self
            .history
            .iter()
            .take_while(|entry| sequence.is_none_or(|limit| entry.sequence < limit))
        {
            for change in &entry.changes {
                match &change.new {
                    Some(binding) => in_force.insert(change.role, binding.clone()),
                    None => in_force.remove(&change.role),
                };
            }
        }
        in_force
    }

    /// Every stale mark in history order: one per marked role per amendment.
    pub fn stale_marks(&self) -> Vec<StaleMark> {
        self.history
            .iter()
            .flat_map(|entry| {
                entry.stale_roles.iter().map(|role| StaleMark {
                    amendment_id: entry.record_id.clone(),
                    sequence: entry.sequence,
                    role: *role,
                })
            })
            .collect()
    }
}

/// The explicit statement shown for a run that never had model bindings.
pub const NEVER_BOUND_STATEMENT: &str =
    "This run never had model bindings and has no model binding amendments.";

/// The `model_bindings` projection shown by `show` (status and full), the
/// explorer and the owner approval view, from the shared fold.
///
/// `state` is `never-bound` (with an explicit `statement`), `bound`, or
/// `unavailable` (with a `reason`) when the frozen value or a stored
/// amendment does not validate. A bound run lists every role with its
/// `original` and `in_force` binding (`null` for none) and the amendment
/// `history` in order: `when`, `actor`, `authority`, the owner's verbatim
/// `words`, each change's `old` and `new` binding and `mark_stale`, and the
/// declared `driver_session` when the amendment gave one.
pub fn model_bindings_view(initial_input: &Value, context: &[ContextRecord]) -> Value {
    let folded = match fold_model_bindings(initial_input, context) {
        Ok(folded) => folded,
        Err(reason) => return json!({"state": "unavailable", "reason": reason}),
    };
    if folded.never_bound() {
        return json!({"state": "never-bound", "statement": NEVER_BOUND_STATEMENT});
    }
    let in_force = folded.in_force();
    let roles: Vec<Value> = ModelRole::ALL
        .into_iter()
        .map(|role| {
            json!({
                "role": role,
                "original": folded.original.get(&role),
                "in_force": in_force.get(&role),
            })
        })
        .collect();
    let history: Vec<Value> = folded
        .history
        .iter()
        .map(|entry| {
            json!({
                "record_id": entry.record_id,
                "sequence": entry.sequence,
                "when": entry.created_at,
                "actor": entry.actor,
                "authority": entry.authority,
                "words": entry.words,
                "changes": entry.changes,
                "driver_session": entry.driver_session,
            })
        })
        .collect();
    json!({
        "state": "bound",
        "roles": roles,
        "original_driver_session": folded.original_driver_session,
        "history": history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(model: &str, thinking: &str) -> Value {
        json!({"model": model, "thinking": thinking})
    }

    fn amendment_record(id: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            MODEL_BINDING_AMENDMENT_RECORD_KIND,
            data,
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64 * 1000),
        )
    }

    fn amendment(changes: Value) -> Value {
        json!({
            "actor": {"name": "owner", "kind": "human"},
            "authority": {"kind": "owner", "reference": "chat"},
            "words": "Use a cheaper reviewer.",
            "changes": changes,
        })
    }

    fn reviewer_bound() -> RoleBindings {
        RoleBindings::from([(
            ModelRole::OrdinaryReviewer,
            Binding::from_value(&binding("p/m1", "high")).unwrap(),
        )])
    }

    fn cause_of(in_force: &RoleBindings, candidate: Value) -> AmendmentCause {
        validate_amendment(in_force, &candidate).unwrap_err().cause
    }

    #[test]
    fn role_ids_round_trip_and_unknown_is_rejected() {
        for role in ModelRole::ALL {
            assert_eq!(ModelRole::parse(role.as_str()), Some(role));
            assert_eq!(serde_json::to_value(role).unwrap(), json!(role.as_str()));
        }
        assert_eq!(ModelRole::parse("Driver"), None);
        assert_eq!(ModelRole::parse("reviewer"), None);
        let stale_allowed: Vec<_> = ModelRole::ALL
            .into_iter()
            .filter(|role| role.may_be_marked_stale())
            .collect();
        assert_eq!(
            stale_allowed,
            [
                ModelRole::OrdinaryReviewer,
                ModelRole::ChallengeReviewer,
                ModelRole::ColdReader,
                ModelRole::CriterionAuthor
            ]
        );
    }

    #[test]
    fn empty_fold_has_no_bindings_no_history_and_is_never_bound() {
        let folded = fold_model_bindings(&json!({}), &[]).unwrap();
        assert!(folded.original.is_empty());
        assert!(folded.history.is_empty());
        assert!(folded.in_force().is_empty());
        assert!(folded.stale_marks().is_empty());
        assert!(folded.never_bound());
        // Unrelated records do not matter.
        let other = ContextRecord::new(
            "r1",
            "review-evidence",
            json!({}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        );
        assert!(
            fold_model_bindings(&json!({"config_version": "14"}), &[other])
                .unwrap()
                .never_bound()
        );
    }

    #[test]
    fn frozen_bindings_fold_to_original_and_in_force() {
        let input = json!({"model_bindings": {
            "roles": {
                "driver": binding("p/d", "low"),
                "ordinary-reviewer": binding("p/m1", "high")
            },
            "driver_session": binding("p/d", "low")
        }});
        let folded = fold_model_bindings(&input, &[]).unwrap();
        assert!(!folded.never_bound());
        assert_eq!(folded.original.len(), 2);
        assert_eq!(folded.in_force(), folded.original);
        assert_eq!(
            folded.original_driver_session,
            Some(Binding::from_value(&binding("p/d", "low")).unwrap())
        );
    }

    #[test]
    fn frozen_bindings_with_empty_roles_are_still_never_bound() {
        let folded = fold_model_bindings(&json!({"model_bindings": {"roles": {}}}), &[]).unwrap();
        assert!(folded.never_bound());
    }

    #[test]
    fn frozen_validation_names_the_problem() {
        let cases = [
            (json!([]), "must be an object"),
            (json!({}), "roles must be an object"),
            (
                json!({"roles": {"boss": binding("p/m", "high")}}),
                "unknown role `boss`",
            ),
            (
                json!({"roles": {"author": binding("", "high")}}),
                "`model` must be a nonblank",
            ),
            (
                json!({"roles": {"author": binding("p/m", " ")}}),
                "`thinking` must be a nonblank",
            ),
            (
                json!({"roles": {"author": {"model": "p/m", "thinking": "high", "x": 1}}}),
                "unknown field `x`",
            ),
            (json!({"roles": {}, "extra": 1}), "unknown field `extra`"),
            (
                json!({"roles": {"driver": binding("p/d", "low")}}),
                "driver_session is required",
            ),
            (
                json!({"roles": {"driver": binding("p/d", "low")}, "driver_session": binding("p/d", "high")}),
                "thinking level only",
            ),
            (
                json!({"roles": {}, "driver_session": binding("p/d", "low")}),
                "only allowed when roles.driver is bound",
            ),
        ];
        for (value, expected) in cases {
            let error = parse_frozen_model_bindings(&value).unwrap_err();
            assert!(
                error.contains(expected),
                "{error} should contain {expected}"
            );
        }
    }

    #[test]
    fn add_change_remove_sequence_folds_in_order() {
        let input = json!({});
        let context = [
            amendment_record(
                "a3",
                30,
                amendment(
                    json!([{"role": "ordinary-reviewer", "old": binding("p/m2", "low"), "new": null}]),
                ),
            ),
            amendment_record(
                "a1",
                10,
                amendment(
                    json!([{"role": "ordinary-reviewer", "old": null, "new": binding("p/m1", "high")}]),
                ),
            ),
            amendment_record(
                "a2",
                20,
                amendment(
                    json!([{"role": "ordinary-reviewer", "old": binding("p/m1", "high"), "new": binding("p/m2", "low"), "mark_stale": true}]),
                ),
            ),
        ];
        let folded = fold_model_bindings(&input, &context).unwrap();
        let ids: Vec<_> = folded
            .history
            .iter()
            .map(|entry| entry.record_id.as_str())
            .collect();
        assert_eq!(ids, ["a1", "a2", "a3"]);
        assert!(!folded.never_bound());
        assert!(folded.in_force().is_empty());
        assert_eq!(
            folded.history[0].created_at,
            Timestamp::from_unix_millis(10_000)
        );
        assert_eq!(folded.history[0].words, "Use a cheaper reviewer.");
        assert_eq!(
            folded.history[1].changes[0].old,
            folded.history[0].changes[0].new
        );
        assert_eq!(folded.history[1].stale_roles, [ModelRole::OrdinaryReviewer]);
        assert_eq!(
            folded.stale_marks(),
            [StaleMark {
                amendment_id: "a2".into(),
                sequence: SemanticSequence::new(20),
                role: ModelRole::OrdinaryReviewer
            }]
        );
    }

    #[test]
    fn an_amendment_cannot_affect_a_prefix_before_it() {
        let context = [
            amendment_record(
                "a1",
                10,
                amendment(json!([{"role": "author", "old": null, "new": binding("p/m1", "high")}])),
            ),
            amendment_record(
                "a2",
                20,
                amendment(
                    json!([{"role": "author", "old": binding("p/m1", "high"), "new": binding("p/m2", "low")}]),
                ),
            ),
        ];
        let folded = fold_model_bindings(&json!({}), &context).unwrap();
        let at = |sequence| {
            folded
                .in_force_before(Some(SemanticSequence::new(sequence)))
                .get(&ModelRole::Author)
                .map(|binding| binding.model.clone())
        };
        assert_eq!(at(5), None);
        assert_eq!(
            at(10),
            None,
            "the amendment's own position is not yet in force for itself"
        );
        assert_eq!(at(11), Some("p/m1".into()));
        assert_eq!(at(20), Some("p/m1".into()));
        assert_eq!(at(21), Some("p/m2".into()));
        assert_eq!(
            folded.in_force().get(&ModelRole::Author).unwrap().model,
            "p/m2"
        );
    }

    #[test]
    fn stored_amendment_that_does_not_validate_is_an_error_naming_it() {
        let context = [amendment_record(
            "bad",
            1,
            amendment(json!([{"role": "author", "old": binding("p/m", "x"), "new": null}])),
        )];
        let error = fold_model_bindings(&json!({}), &context).unwrap_err();
        assert!(
            error.contains("`bad`") && error.contains("old-binding-not-in-force"),
            "{error}"
        );
    }

    #[test]
    fn validated_amendment_reports_effect_and_stale_roles() {
        let valid = validate_amendment(
            &reviewer_bound(),
            &amendment(json!([
                {"role": "ordinary-reviewer", "old": binding("p/m1", "high"), "new": binding("p/m2", "low"), "mark_stale": true},
                {"role": "author", "old": null, "new": binding("p/a", "high")}
            ])),
        )
        .unwrap();
        assert_eq!(valid.stale_roles(), [ModelRole::OrdinaryReviewer]);
        let after = valid.apply(&reviewer_bound());
        assert_eq!(after[&ModelRole::OrdinaryReviewer].model, "p/m2");
        assert_eq!(after[&ModelRole::Author].model, "p/a");
        assert_eq!(valid.driver_session, None);
    }

    #[test]
    fn each_refusal_cause_has_a_distinct_named_diagnostic() {
        let none = RoleBindings::new();
        let add = || json!([{"role": "author", "old": null, "new": binding("p/m", "high")}]);
        let mut refusals = Vec::new();

        let mut data = amendment(add());
        data["actor"]["kind"] = json!("agent");
        refusals.push((
            AmendmentCause::ActorNotHuman,
            validate_amendment(&none, &data),
        ));

        let mut data = amendment(add());
        data["authority"] = json!({"kind": "standing-delegation", "reference": "x"});
        refusals.push((
            AmendmentCause::AuthorityNotOwner,
            validate_amendment(&none, &data),
        ));

        let mut data = amendment(add());
        data["words"] = json!("  ");
        refusals.push((AmendmentCause::WordsBlank, validate_amendment(&none, &data)));

        refusals.push((
            AmendmentCause::UnknownRole,
            validate_amendment(
                &none,
                &amendment(
                    json!([{"role": "reviewer", "old": null, "new": binding("p/m", "high")}]),
                ),
            ),
        ));

        // Adding to a role that is already bound.
        refusals.push((
            AmendmentCause::OldBindingNotInForce,
            validate_amendment(
                &reviewer_bound(),
                &amendment(json!([{"role": "ordinary-reviewer", "old": null, "new": binding("p/m2", "low")}])),
            ),
        ));
        // Changing a role that is unbound.
        refusals.push((
            AmendmentCause::OldBindingNotInForce,
            validate_amendment(
                &none,
                &amendment(json!([{"role": "author", "old": binding("p/m", "high"), "new": binding("p/n", "high")}])),
            ),
        ));
        // Removing with a stale old binding.
        refusals.push((
            AmendmentCause::OldBindingNotInForce,
            validate_amendment(
                &reviewer_bound(),
                &amendment(json!([{"role": "ordinary-reviewer", "old": binding("p/other", "high"), "new": null}])),
            ),
        ));

        for new in [
            binding("", "high"),
            binding("p/m", "  "),
            json!({"model": "p/m"}),
            json!("p/m"),
        ] {
            refusals.push((
                AmendmentCause::NewBindingMalformed,
                validate_amendment(
                    &none,
                    &amendment(json!([{"role": "author", "old": null, "new": new}])),
                ),
            ));
        }

        refusals.push((
            AmendmentCause::StaleMarkNotAllowed,
            validate_amendment(
                &none,
                &amendment(json!([{"role": "author", "old": null, "new": binding("p/m", "high"), "mark_stale": true}])),
            ),
        ));

        let driver_add = || json!([{"role": "driver", "old": null, "new": binding("p/d", "low")}]);
        refusals.push((
            AmendmentCause::DriverDeclarationMissing,
            validate_amendment(&none, &amendment(driver_add())),
        ));
        let mut data = amendment(driver_add());
        data["driver_session"] = binding("p/d", "high");
        refusals.push((
            AmendmentCause::DriverDeclarationMismatch,
            validate_amendment(&none, &data),
        ));

        refusals.push((
            AmendmentCause::Malformed,
            validate_amendment(&none, &json!([])),
        ));
        refusals.push((
            AmendmentCause::Malformed,
            validate_amendment(&none, &amendment(json!([]))),
        ));

        for (cause, result) in &refusals {
            let refusal = result.as_ref().expect_err("must be refused");
            assert_eq!(&refusal.cause, cause, "{refusal}");
            assert!(refusal.to_string().contains(cause.code()), "{refusal}");
        }
        let mut codes: Vec<_> = refusals.iter().map(|(cause, _)| cause.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 10, "every cause is exercised: {codes:?}");
    }

    #[test]
    fn driver_declaration_is_checked_against_the_driver_binding_after_the_amendment() {
        let bound_driver = RoleBindings::from([(
            ModelRole::Driver,
            Binding::from_value(&binding("p/d", "low")).unwrap(),
        )]);
        // Changing a non-driver role while the driver stays bound still needs a declaration.
        let other =
            amendment(json!([{"role": "author", "old": null, "new": binding("p/a", "high")}]));
        assert_eq!(
            cause_of(&bound_driver, other.clone()),
            AmendmentCause::DriverDeclarationMissing
        );
        let mut with_session = other;
        with_session["driver_session"] = binding("p/d", "low");
        assert!(validate_amendment(&bound_driver, &with_session).is_ok());
        // Changing the driver is checked against the new binding, not the old.
        let mut change = amendment(
            json!([{"role": "driver", "old": binding("p/d", "low"), "new": binding("p/e", "low")}]),
        );
        change["driver_session"] = binding("p/d", "low");
        assert_eq!(
            cause_of(&bound_driver, change.clone()),
            AmendmentCause::DriverDeclarationMismatch
        );
        change["driver_session"] = binding("p/e", "low");
        assert!(validate_amendment(&bound_driver, &change).is_ok());
        // Removing the driver needs no declaration.
        let remove =
            amendment(json!([{"role": "driver", "old": binding("p/d", "low"), "new": null}]));
        assert!(validate_amendment(&bound_driver, &remove).is_ok());
    }

    #[test]
    fn authority_may_be_the_bare_owner_string() {
        let mut data =
            amendment(json!([{"role": "author", "old": null, "new": binding("p/m", "high")}]));
        data["authority"] = json!("owner");
        assert!(validate_amendment(&RoleBindings::new(), &data).is_ok());
    }

    #[test]
    fn session_mismatch_names_role_expected_actual_and_differing_part() {
        let expected = Binding::from_value(&binding("p/m1", "high")).unwrap();
        let declare = |model, thinking| Binding::from_value(&binding(model, thinking)).unwrap();
        // Unbound role and exact match pass.
        assert!(check_declared_session(ModelRole::ColdReader, None, None).is_ok());
        assert!(
            check_declared_session(ModelRole::ColdReader, Some(&expected), Some(&expected)).is_ok()
        );

        let missing =
            check_declared_session(ModelRole::ColdReader, Some(&expected), None).unwrap_err();
        assert_eq!(missing.difference, SessionDifference::Missing);
        let text = missing.to_string();
        assert!(
            text.contains("cold-reader")
                && text.contains("p/m1")
                && text.contains("high")
                && text.contains("missing"),
            "{text}"
        );

        let model_only = declare("p/m2", "high");
        let mismatch = check_declared_session(
            ModelRole::CriterionAuthor,
            Some(&expected),
            Some(&model_only),
        )
        .unwrap_err();
        assert_eq!(mismatch.difference, SessionDifference::Model);
        let text = mismatch.to_string();
        assert!(
            text.contains("criterion-author")
                && text.contains("p/m1")
                && text.contains("p/m2")
                && text.contains("model only"),
            "{text}"
        );

        let thinking_only = declare("p/m1", "low");
        let mismatch =
            check_declared_session(ModelRole::Author, Some(&expected), Some(&thinking_only))
                .unwrap_err();
        assert_eq!(mismatch.difference, SessionDifference::Thinking);
        assert!(mismatch.to_string().contains("thinking level only"));

        let both = declare("p/m2", "low");
        let mismatch =
            check_declared_session(ModelRole::Author, Some(&expected), Some(&both)).unwrap_err();
        assert_eq!(mismatch.difference, SessionDifference::ModelAndThinking);

        // Exact text: case and whitespace differences are mismatches.
        let cased = declare("P/M1", "high");
        assert!(check_declared_session(ModelRole::Author, Some(&expected), Some(&cased)).is_err());
    }

    fn input_with_reviewer() -> Value {
        json!({"model_bindings": {"roles": {"ordinary-reviewer": binding("p/m1", "high")}}})
    }

    #[test]
    fn view_of_a_run_without_bindings_is_the_explicit_never_bound_statement() {
        let view = model_bindings_view(&json!({"objective": "x"}), &[]);
        assert_eq!(
            view,
            json!({"state": "never-bound", "statement": NEVER_BOUND_STATEMENT})
        );
        // A frozen but empty roles object is also never bound.
        assert_eq!(
            model_bindings_view(&json!({"model_bindings": {"roles": {}}}), &[]),
            view
        );
    }

    #[test]
    fn view_of_a_bound_run_lists_every_role_original_and_in_force() {
        let view = model_bindings_view(&input_with_reviewer(), &[]);
        assert_eq!(view["state"], "bound");
        assert_eq!(view["history"], json!([]));
        let roles = view["roles"].as_array().unwrap();
        let ids: Vec<_> = roles
            .iter()
            .map(|row| row["role"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ModelRole::ALL.map(ModelRole::as_str));
        for row in roles {
            let (expected_original, expected_in_force) = if row["role"] == "ordinary-reviewer" {
                (binding("p/m1", "high"), binding("p/m1", "high"))
            } else {
                (Value::Null, Value::Null)
            };
            assert_eq!(row["original"], expected_original);
            assert_eq!(row["in_force"], expected_in_force);
        }
    }

    #[test]
    fn view_history_matches_the_stored_amendment_records_in_order() {
        let first = amendment(json!([{
            "role": "ordinary-reviewer",
            "old": binding("p/m1", "high"),
            "new": binding("p/m2", "low"),
            "mark_stale": true
        }]));
        let second = amendment(json!([
            {"role": "ordinary-reviewer", "old": binding("p/m2", "low"), "new": null},
            {"role": "cold-reader", "old": null, "new": binding("p/m3", "xhigh")}
        ]));
        let records = [
            amendment_record("a-2", 9, second.clone()),
            amendment_record("a-1", 4, first.clone()),
        ];
        let view = model_bindings_view(&input_with_reviewer(), &records);
        let history = view["history"].as_array().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["record_id"], "a-1");
        assert_eq!(
            history[0]["when"],
            serde_json::to_value(Timestamp::from_unix_millis(4000)).unwrap()
        );
        assert_eq!(history[0]["actor"], first["actor"]);
        assert_eq!(history[0]["authority"], first["authority"]);
        assert_eq!(history[0]["words"], first["words"]);
        assert_eq!(history[0]["changes"][0]["old"], binding("p/m1", "high"));
        assert_eq!(history[0]["changes"][0]["new"], binding("p/m2", "low"));
        assert_eq!(history[0]["changes"][0]["mark_stale"], true);
        assert_eq!(history[1]["record_id"], "a-2");
        assert_eq!(history[1]["changes"][0]["new"], Value::Null);
        assert_eq!(history[1]["changes"][0]["mark_stale"], false);
        assert_eq!(history[1]["changes"][1]["old"], Value::Null);
        let after: Vec<_> = view["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["role"].as_str().unwrap(),
                    row["original"].clone(),
                    row["in_force"].clone(),
                )
            })
            .filter(|(_, original, in_force)| !(original.is_null() && in_force.is_null()))
            .collect();
        assert_eq!(
            after,
            [
                ("ordinary-reviewer", binding("p/m1", "high"), Value::Null),
                ("cold-reader", Value::Null, binding("p/m3", "xhigh")),
            ]
        );
    }

    #[test]
    fn view_of_a_run_whose_only_change_is_an_amendment_is_bound() {
        let record = amendment_record(
            "a-1",
            3,
            amendment(json!([{"role": "author", "old": null, "new": binding("p/m1", "high")}])),
        );
        let view = model_bindings_view(&json!({}), &[record]);
        assert_eq!(view["state"], "bound");
        assert_eq!(view["history"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn view_reports_unavailable_when_a_stored_amendment_does_not_validate() {
        let bad =
            amendment(json!([{"role": "author", "old": binding("p/m1", "high"), "new": null}]));
        let view = model_bindings_view(&json!({}), &[amendment_record("a-bad", 3, bad)]);
        assert_eq!(view["state"], "unavailable");
        assert!(view["reason"].as_str().unwrap().contains("a-bad"));
    }
}
