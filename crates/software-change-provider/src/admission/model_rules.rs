//! Model-binding admission: declared sessions and owner binding amendments.
//!
//! The binding in force is always folded from the frozen input and the stored
//! context before the candidate, so an amendment applies from its own position
//! and never re-judges earlier records. The check compares declared text only.
use super::AdmissionContext;
use crate::model_roles::role_of_record;
use loop_core::{
    check_declared_session, fold_model_bindings, validate_amendment, Binding, CandidateRecord,
    ModelBindings, ModelRole, SESSION_DECLARATION_RECORD_KIND,
};
use serde_json::Value;

fn fold(ctx: &AdmissionContext<'_>) -> Result<ModelBindings, String> {
    fold_model_bindings(ctx.initial_input, ctx.context)
}

fn text<'a>(data: &'a Value, field: &str) -> Result<&'a str, String> {
    data[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty `{field}`"))
}

/// Refuse a missing or unequal declared `session` when the record's role is
/// bound at this point in the context. An unbound role is not checked.
pub(super) fn session(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let role = if record.kind == SESSION_DECLARATION_RECORD_KIND {
        record.data["role"].as_str().and_then(ModelRole::parse)
    } else {
        role_of_record(&record.kind, &record.data)
    };
    let Some(role) = role else {
        return Ok(());
    };
    let Some(binding) = fold(ctx)?.in_force().remove(&role) else {
        return Ok(());
    };
    let declared = match record.data.get("session") {
        None | Some(Value::Null) => None,
        Some(value) => Some(Binding::from_value(value).map_err(|error| {
            format!("model binding for role `{role}` is not met: declared session {error}")
        })?),
    };
    check_declared_session(role, Some(&binding), declared.as_ref())
        .map_err(|mismatch| mismatch.to_string())
}

/// Validate one candidate amendment against the bindings in force before it.
pub(super) fn amendment(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    validate_amendment(&fold(ctx)?.in_force(), &record.data)
        .map(|_| ())
        .map_err(|refusal| refusal.to_string())
}

/// A session declaration names the author or implementer role, a subject and
/// revision, and a well-formed session.
pub(super) fn declaration(
    _ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let data = &record.data;
    match data["role"].as_str().and_then(ModelRole::parse) {
        Some(ModelRole::Author | ModelRole::Implementer) => {}
        _ => return Err("session-declaration `role` must be `author` or `implementer`".into()),
    }
    text(data, "subject")?;
    text(data, "subject_revision")?;
    Binding::from_value(&data["session"])
        .map(|_| ())
        .map_err(|error| format!("session-declaration `session` {error}"))
}
