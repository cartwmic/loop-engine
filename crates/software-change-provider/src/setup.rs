//! Resolve policy without launching workers, advisors, or the engine.
use crate::{config, embedded_data, narratives, overlay, principles};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

const USAGE: &str = "usage: software-change setup (--rigor minimal|standard|high | --profile FILE) --output FILE (--enable-advice | --decline-advice) [--bookends] [--set KEY=VALUE (including principles.ref=PATH or principles.enabled=false, required for shipped profiles; narrative.<design|plan|validation>=true|false, checkpoint.<intent|design|plan|implementation|validation>=true|false, binding.<driver|author|implementer|ordinary-reviewer|challenge-reviewer|cold-reader|criterion-author>=provider/model@thinking and binding.driver-session=provider/model@thinking)] [--remove-axis gate.stage.axis] [--add-axis gate.stage.axis=gate.stage.axis] [--previous-explain FILE]";

pub fn run_from_args(args: &[String]) -> i32 {
    if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
        return 0;
    }
    match resolve(args) {
        Ok((profile, output, explain)) => {
            let mut explain_path = output.as_os_str().to_owned();
            explain_path.push(".explain.json");
            let result = fs::write(
                &output,
                serde_json::to_vec_pretty(&profile).expect("JSON profile"),
            )
            .and_then(|_| {
                fs::write(
                    PathBuf::from(explain_path),
                    serde_json::to_vec_pretty(&explain).expect("JSON explanation"),
                )
            });
            if let Err(error) = result {
                eprintln!("setup: {}: {error}", output.display());
                return 2;
            }
            println!("{profile}");
            0
        }
        Err(error) => {
            eprintln!("setup: {error}; {USAGE}");
            2
        }
    }
}

fn axis_mut<'a>(profile: &'a mut Value, name: &str) -> Result<&'a mut Value, String> {
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("invalid axis target {name}"));
    }
    profile["review_policies"][parts[0]]
        .as_array_mut()
        .ok_or_else(|| format!("unknown gate {}", parts[0]))?
        .iter_mut()
        .find(|entry| entry["review_stage"] == parts[1] && entry["id"] == parts[2])
        .ok_or_else(|| format!("unknown axis {name}"))
}

fn materialize_narrative_gate(profile: &mut Value, phase: &str, pass: &str) -> Result<(), String> {
    let mut entries = profile["extra"]["narrative_review"][pass]
        .as_array()
        .cloned()
        .ok_or_else(|| {
            format!("narrative.{phase} requires extra.narrative_review.{pass} template")
        })?;
    for entry in &mut entries {
        if let Some(group) = entry["group"].as_str() {
            entry["group"] = json!(group.replace("{phase}", phase));
        }
    }
    let gate = narrative_gate(phase, pass);
    profile["review_policies"]
        .as_object_mut()
        .ok_or("review_policies must be an object")?
        .insert(gate, json!(entries));
    Ok(())
}

fn narrative_gate(phase: &str, pass: &str) -> String {
    let suffix = if pass == "ordinary" {
        "review"
    } else {
        "adversarial-review"
    };
    format!("{phase}-narrative-{suffix}")
}

fn set_phase_option(profile: &mut Value, kind: &str, phase: &str, raw: &str) -> Result<(), String> {
    let (field, allowed) = if kind == "narrative" {
        ("narratives", narratives::NARRATIVE_PHASES)
    } else {
        ("checkpoints", narratives::CHECKPOINT_PHASES)
    };
    if !allowed.contains(&phase) {
        return Err(format!("unknown {kind} phase {phase}"));
    }
    let enabled = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_bool())
        .ok_or_else(|| format!("{kind}.{phase} must be boolean"))?;
    if profile["extra"].is_null() {
        profile["extra"] = json!({});
    }
    let extra = profile["extra"]
        .as_object_mut()
        .ok_or("extra must be an object to set phase options")?;
    let phases = extra.entry(field).or_insert_with(|| json!([]));
    let phases = phases
        .as_array_mut()
        .ok_or_else(|| format!("extra.{field} must be an array"))?;
    let value = json!(phase);
    if enabled {
        if !phases.contains(&value) {
            phases.push(value);
        }
    } else {
        phases.retain(|entry| entry != &value);
    }
    if kind == "narrative" {
        for pass in ["ordinary", "challenge"] {
            if enabled {
                materialize_narrative_gate(profile, phase, pass)?;
            } else {
                profile["review_policies"]
                    .as_object_mut()
                    .ok_or("review_policies must be an object")?
                    .remove(&narrative_gate(phase, pass));
            }
        }
    }
    Ok(())
}

/// Parse `provider/model@thinking` into a `{model, thinking}` binding value.
fn parse_binding_value(key: &str, raw: &str) -> Result<Value, String> {
    let (model, thinking) = raw
        .rsplit_once('@')
        .ok_or_else(|| format!("{key} must be provider/model@thinking"))?;
    if model.trim().is_empty() || thinking.trim().is_empty() {
        return Err(format!(
            "{key} must name a nonblank provider/model and a nonblank thinking level"
        ));
    }
    Ok(json!({"model": model, "thinking": thinking}))
}

/// Write `binding.<role>` or `binding.driver-session` into `model_bindings`.
/// The key is created only here, so a profile with no binding keeps none.
fn set_binding(profile: &mut Value, name: &str, raw: &str) -> Result<(), String> {
    let key = format!("binding.{name}");
    let slot = if name == "driver-session" {
        "driver_session"
    } else if loop_core::ModelRole::parse(name).is_some() {
        "roles"
    } else {
        return Err(format!("unknown binding role `{name}` in {key}"));
    };
    let value = parse_binding_value(&key, raw)?;
    let root = profile.as_object_mut().ok_or("profile must be an object")?;
    let bindings = root
        .entry(loop_core::MODEL_BINDINGS_INPUT_KEY)
        .or_insert_with(|| json!({"roles": {}}));
    let bindings = bindings
        .as_object_mut()
        .ok_or("model_bindings must be an object to set bindings")?;
    if slot == "driver_session" {
        bindings.insert(slot.into(), value);
    } else {
        bindings
            .entry("roles")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("model_bindings.roles must be an object to set bindings")?
            .insert(name.into(), value);
    }
    Ok(())
}

fn set(profile: &mut Value, key: &str, raw: &str) -> Result<(), String> {
    let parts: Vec<_> = key.split('.').collect();
    if let [kind @ ("narrative" | "checkpoint"), phase] = parts.as_slice() {
        return set_phase_option(profile, kind, phase, raw);
    }
    if let ["binding", name] = parts.as_slice() {
        return set_binding(profile, name, raw);
    }
    if let ["principles", field] = parts.as_slice() {
        return set_principles(profile, field, raw);
    }
    let (slot, field) = match parts.as_slice() {
        ["review", gate, stage, axis, field @ ("required_authors" | "self_review" | "group")] => (
            axis_mut(profile, &format!("{gate}.{stage}.{axis}"))?,
            *field,
        ),
        ["criterion", field @ ("required_authors" | "self_review")] => {
            (&mut profile["criterion_policy"], *field)
        }
        ["goal", "required_authors"] => (&mut profile["criterion_policy"], "goal_required_authors"),
        ["goal", "self_review"] => (&mut profile["criterion_policy"], "goal_self_review"),
        [section @ ("advice" | "bookends"), "enabled"] => {
            if profile["extra"].is_null() {
                profile["extra"] = json!({});
            }
            if profile["extra"][*section].is_null() {
                profile["extra"][*section] = json!({});
            }
            (&mut profile["extra"][*section], "enabled")
        }
        _ => return Err(format!("unknown setting {key}")),
    };
    let value = if field == "group" {
        let value: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
        if value.as_str().is_none_or(|s| s.trim().is_empty()) {
            return Err("group must be a nonempty string".into());
        }
        value
    } else {
        let value: Value =
            serde_json::from_str(raw).map_err(|_| format!("invalid value for {key}"))?;
        if field.ends_with("required_authors") {
            if value.as_u64().is_none_or(|v| v < 1) {
                return Err("required_authors must be an integer >= 1".into());
            }
        } else if !value.is_boolean() {
            return Err(format!("{key} must be boolean"));
        }
        value
    };
    slot[field] = value;
    Ok(())
}

fn set_principles(profile: &mut Value, field: &str, raw: &str) -> Result<(), String> {
    if !profile["extra"]["principles"].is_null() {
        return if field == "enabled"
            && raw == "true"
            && profile["extra"]["principles"]["enabled"] == true
        {
            Ok(())
        } else {
            Err("principles already set for this profile; set it once".into())
        };
    }
    if !principles::has_marker(profile) {
        return Err(
            "this profile has no principles marker; principles settings do not apply".into(),
        );
    }
    match (field, raw) {
        ("ref", path) if !path.is_empty() => {
            let bytes = fs::read(path).map_err(|e| format!("principles file {path}: {e}"))?;
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("principles file {path} is not valid UTF-8"))?;
            principles::apply(profile, &principles::snapshot(path, &text)?);
            Ok(())
        }
        ("enabled", "false") => {
            principles::decline(profile);
            Ok(())
        }
        ("enabled", "true") => Err("principles.enabled=true needs principles.ref=PATH".into()),
        _ => Err("principles settings are principles.ref=PATH or principles.enabled=false".into()),
    }
}

fn setting(profile: &Value, key: &str) -> Value {
    let parts: Vec<_> = key.split('.').collect();
    let value = match parts.as_slice() {
        ["principles", "ref"] => profile["extra"]["principles"]["source"].clone(),
        ["review", gate, stage, axis, field] => profile["review_policies"][*gate]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["review_stage"] == *stage && entry["id"] == *axis)
            .map_or(Value::Null, |entry| entry[*field].clone()),
        ["axis", gate, stage, axis] => profile["review_policies"][*gate]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["review_stage"] == *stage && entry["id"] == *axis)
            .cloned()
            .unwrap_or(Value::Null),
        ["criterion", field] => profile["criterion_policy"][*field].clone(),
        ["goal", "required_authors"] => {
            profile["criterion_policy"]["goal_required_authors"].clone()
        }
        ["goal", "self_review"] => profile["criterion_policy"]["goal_self_review"].clone(),
        ["narrative", phase] => json!(profile["extra"]["narratives"]
            .as_array()
            .is_some_and(|phases| phases.contains(&json!(phase)))),
        ["checkpoint", phase] => json!(profile["extra"]["checkpoints"]
            .as_array()
            .is_some_and(|phases| phases.contains(&json!(phase)))),
        ["binding", "driver-session"] => profile["model_bindings"]["driver_session"].clone(),
        ["binding", role] => profile["model_bindings"]["roles"][*role].clone(),
        [section, "enabled"] => profile["extra"][*section]["enabled"].clone(),
        _ => Value::Null,
    };
    if value.is_null() && (key.ends_with(".self_review") || key.ends_with(".enabled")) {
        json!(false)
    } else {
        value
    }
}

fn record_set(
    profile: &mut Value,
    base: &Value,
    overrides: &mut Vec<Value>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    set(profile, key, value)?;
    overrides.push(json!({"key":key,"base_value":setting(base,key),"value":setting(profile,key)}));
    Ok(())
}

fn fill_missing_narrative_gates(
    profile: &mut Value,
    base: &Value,
    overrides: &mut Vec<Value>,
) -> Result<(), String> {
    // Profile reuse keeps customized gates; only absent gates need the template.
    for phase in narratives::NARRATIVE_PHASES {
        if setting(profile, &format!("narrative.{phase}")) == true {
            for pass in ["ordinary", "challenge"] {
                let gate = narrative_gate(phase, pass);
                if profile["review_policies"].get(&gate).is_none() {
                    materialize_narrative_gate(profile, phase, pass)?;
                    overrides.push(json!({
                        "key": format!("review_policies.{gate}"),
                        "base_value": base["review_policies"][&gate],
                        "value": profile["review_policies"][&gate]
                    }));
                }
            }
        }
    }
    Ok(())
}

fn resolve(args: &[String]) -> Result<(Value, PathBuf, Value), String> {
    let mut base = None;
    let mut output = None;
    let mut advice = None;
    let mut bookends = false;
    let mut operations = Vec::new();
    let mut previous = None;
    let mut i = 0;
    while i < args.len() {
        let (flag, inline) = args[i]
            .split_once('=')
            .map_or((args[i].as_str(), None), |(k, v)| (k, Some(v)));
        match flag {
            "--bookends" if inline.is_none() && !bookends => bookends = true,
            "--enable-advice" | "--decline-advice" if inline.is_none() && advice.is_none() => {
                advice = Some(flag == "--enable-advice")
            }
            "--rigor" | "--profile" | "--output" | "--set" | "--remove-axis" | "--add-axis"
            | "--previous-explain" => {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        i += 1;
                        args.get(i)
                            .filter(|v| !v.starts_with('-'))
                            .ok_or_else(|| format!("{flag} requires a value"))?
                    }
                };
                match flag {
                    "--output" => {
                        if output.replace(PathBuf::from(value)).is_some() {
                            return Err("duplicate --output".into());
                        }
                    }
                    "--previous-explain" => {
                        if previous.is_some() {
                            return Err("duplicate --previous-explain".into());
                        }
                        previous = Some(
                            serde_json::from_slice::<Value>(
                                &fs::read(value).map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())?,
                        );
                    }
                    "--set" | "--remove-axis" | "--add-axis" => {
                        operations.push((flag.to_owned(), value.to_owned()))
                    }
                    _ => {
                        if base.is_some() {
                            return Err("select exactly one base profile".into());
                        }
                        let bytes = if flag == "--profile" {
                            fs::read(value).map_err(|e| e.to_string())?
                        } else {
                            let name = match value {
                                "minimal" => "minimal",
                                "standard" => "standard",
                                "high" => "high-rigor",
                                _ => return Err("invalid rigor".into()),
                            };
                            let path =
                                format!("crates/software-change-provider/data/configs/{name}.json");
                            embedded_data::FILES
                                .iter()
                                .find(|f| f.path == path)
                                .ok_or("missing embedded profile")?
                                .bytes
                                .to_vec()
                        };
                        base = Some((
                            serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string())?,
                            format!("sha256:{:x}", Sha256::digest(&bytes)),
                        ));
                    }
                }
            }
            _ => return Err(format!("unknown or duplicate argument `{}`", args[i])),
        }
        i += 1;
    }
    let (base, digest) = base.ok_or("missing base profile")?;
    if base["config_version"].as_str().is_some_and(|id| {
        ["minimal", "standard", "high-rigor"]
            .iter()
            .any(|name| (1..=17).any(|version| id == format!("{name}-{version}")) || id == *name)
    }) {
        return Err("historical shipped profile IDs cannot be used for new setup; supported shipped IDs: minimal-18, standard-18, high-rigor-18".into());
    }
    if base["contract_version"] != 4 {
        return Err("profile must use contract_version 4".into());
    }
    let mut profile = base.clone();
    let root = profile.as_object_mut().ok_or("profile must be an object")?;
    root.remove("work_slot_bindings");
    root.remove("advice_command");
    root.remove("advice_departures");
    for (operation, value) in &operations {
        if operation == "--set" {
            if let Some(raw) = value.strip_prefix("advice.enabled=") {
                let choice = match raw {
                    "true" => true,
                    "false" => false,
                    _ => return Err("advice.enabled must be boolean".into()),
                };
                if advice.is_some_and(|previous| previous != choice) {
                    return Err("conflicting advice choices".into());
                }
                advice = Some(choice);
            }
        }
    }
    let mut overrides = Vec::new();
    record_set(
        &mut profile,
        &base,
        &mut overrides,
        "advice.enabled",
        if advice.ok_or("choose --enable-advice, --decline-advice or --set advice.enabled")? {
            "true"
        } else {
            "false"
        },
    )?;
    if bookends {
        record_set(
            &mut profile,
            &base,
            &mut overrides,
            "bookends.enabled",
            "true",
        )?;
    }
    for (operation, value) in operations {
        if operation == "--set" {
            let (key, raw) = value.split_once('=').ok_or("--set requires KEY=VALUE")?;
            record_set(&mut profile, &base, &mut overrides, key, raw)?;
        } else if operation == "--remove-axis" {
            let removed = axis_mut(&mut profile, &value)?.clone();
            let parts: Vec<_> = value.split('.').collect();
            profile["review_policies"][parts[0]]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry != &removed);
            overrides
                .push(json!({"key":format!("axis.{value}"),"base_value":setting(&base,&format!("axis.{value}")),"value":null}));
        } else {
            let (target, source) = value
                .split_once('=')
                .ok_or("--add-axis requires target=source")?;
            let mut entry = axis_mut(&mut profile, source)?.clone();
            let parts: Vec<_> = target.split('.').collect();
            if parts.len() != 3
                || !config::GATE_IDS.contains(&parts[0])
                || !["individual", "aggregate"].contains(&parts[1])
                || parts[2].is_empty()
            {
                return Err("invalid added axis target".into());
            }
            if axis_mut(&mut profile, target).is_ok() {
                return Err("added axis already exists".into());
            }
            entry["id"] = json!(parts[2]);
            entry["review_stage"] = json!(parts[1]);
            if profile["review_policies"][parts[0]].is_null() {
                profile["review_policies"][parts[0]] = json!([]);
            }
            profile["review_policies"][parts[0]]
                .as_array_mut()
                .ok_or("gate must contain an array")?
                .push(entry.clone());
            overrides.push(json!({"key":format!("axis.{target}"),"base_value":setting(&base,&format!("axis.{target}")),"value":entry}));
        }
    }
    if principles::has_marker(&profile) {
        return Err("choose --set principles.ref=PATH (owner principles file) or --set principles.enabled=false".into());
    }
    fill_missing_narrative_gates(&mut profile, &base, &mut overrides)?;
    config::validate_config(&profile).map_err(|e| e.to_string())?;
    config::validate_config(&overlay::apply(&profile)).map_err(|e| e.to_string())?;
    let mut drift = Vec::new();
    if let Some(previous) = previous {
        let old = previous["overrides"]
            .as_array()
            .ok_or("previous explain must contain overrides")?;
        for current in &overrides {
            if let Some(old) = old.iter().find(|old| old["key"] == current["key"]) {
                if old["base_value"] != current["base_value"] {
                    drift.push(json!({"key":current["key"],"previous_base_value":old["base_value"],"base_value":current["base_value"],"value":current["value"]}));
                }
            }
        }
    }
    let mut explain = json!({"base":{"id":base["config_version"],"sha256":digest},"overrides":overrides,"effective":{"config_version":profile["config_version"],"review_policies":profile["review_policies"],"criterion_policy":profile["criterion_policy"],"advice":profile["extra"]["advice"],"bookends":profile["extra"]["bookends"],"principles":profile["extra"]["principles"],"narratives":profile["extra"]["narratives"].as_array().cloned().unwrap_or_default(),"checkpoints":profile["extra"]["checkpoints"].as_array().cloned().unwrap_or_default()},"drift":drift});
    if let Some(bindings) = profile.get(loop_core::MODEL_BINDINGS_INPUT_KEY) {
        explain["effective"][loop_core::MODEL_BINDINGS_INPUT_KEY] = bindings.clone();
    }
    Ok((profile, output.ok_or("missing --output")?, explain))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_without_execution_bindings() {
        let args = [
            "--rigor",
            "standard",
            "--output",
            "profile.json",
            "--decline-advice",
            "--set",
            "principles.enabled=false",
        ]
        .map(str::to_owned);
        let (profile, _, explain) = resolve(&args).unwrap();
        assert!(profile.get("work_slot_bindings").is_none());
        assert!(profile.get("advice_command").is_none());
        assert_eq!(profile["contract_version"], 4);
        assert_eq!(explain["base"]["id"], "standard-18");
    }
    #[test]
    fn refuses_standard_14_for_new_setup() {
        let root = std::env::temp_dir().join(format!(
            "software-change-setup-historical-{}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("standard-14.json");
        let bytes = embedded_data::FILES
            .iter()
            .find(|file| file.path.ends_with("data/configs/standard.json"))
            .unwrap()
            .bytes;
        let mut profile: Value = serde_json::from_slice(bytes).unwrap();
        profile["config_version"] = json!("standard-14");
        fs::write(&source, serde_json::to_vec(&profile).unwrap()).unwrap();
        let args = vec![
            "--profile".into(),
            source.to_string_lossy().into_owned(),
            "--output".into(),
            root.join("profile.json").to_string_lossy().into_owned(),
            "--decline-advice".into(),
        ];
        let error = resolve(&args).unwrap_err();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            error,
            "historical shipped profile IDs cannot be used for new setup; supported shipped IDs: minimal-18, standard-18, high-rigor-18"
        );
    }
    #[test]
    fn successor_identity_preserves_entire_retained_15_profile_policy_and_defaults() {
        // Frozen canonical hashes of the shipped -18 profiles (with config_version rewritten to -15),
        // re-pinned in #58. They previously pinned the genuine -15 originals (HEAD
        // 29652002312d4ba7cb36b1e59d4b00db8a0ec11a); -18 adds principles-honored and tightens
        // mechanism-forced, so that historical-continuity assertion no longer holds.
        for (name, expected) in [
            (
                "minimal",
                "9dd3e9bf1ed54f48031d9c9a4bbfed34cac0b03ad507cb5aefeaf6e26f82416c",
            ),
            (
                "standard",
                "74dff2e485129346e3c4b2c4bf0c585af77357903493c754df7bc801f9b70cdf",
            ),
            (
                "high-rigor",
                "9c57020dfdffbd196d4013af7610e5cdf577fe23cac7c351ff6395c10b0276bc",
            ),
        ] {
            let path = format!("crates/software-change-provider/data/configs/{name}.json");
            let bytes = embedded_data::FILES
                .iter()
                .find(|file| file.path == path)
                .unwrap()
                .bytes;
            let mut value: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(value["config_version"], format!("{name}-18"));
            value["config_version"] = json!(format!("{name}-15"));
            assert_eq!(
                format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
                expected
            );
        }
    }

    #[test]
    fn shipped_recording_sources_have_current_forms_without_new_template_paths() {
        let read = |suffix: &str| {
            std::str::from_utf8(
                embedded_data::FILES
                    .iter()
                    .find(|file| file.path.ends_with(suffix))
                    .unwrap()
                    .bytes,
            )
            .unwrap()
        };
        let protocol = read("data/reviewer-protocol.md");
        for term in [
            "assessment_missing",
            "assessment_inconsistent",
            "mixed_authors",
            "record:CORRECTION_ID#/data/prior_intent/json_bytes",
            "intent-correction",
            "baseline-to-covered-commit",
            "literal two-character",
            "physical",
        ] {
            assert!(protocol.contains(term), "{term}");
        }
        for suffix in [
            "data/templates/implementation-report.md",
            "data/templates/validation-report.md",
        ] {
            assert!(read(suffix)
                .contains("{\"name\":\"actual accepting actor\",\"kind\":\"human|agent|script\"}"));
        }
        let ledger: Value =
            serde_json::from_str(read("data/templates/finding-ledger.json")).unwrap();
        assert!(ledger["data"]["findings"][0].get("statement").is_none());
        assert!(ledger["data"]["findings"][0]["summary"].is_string());
        assert!(read("data/review-worker-preamble.txt").contains("baseline-to-covered-commit"));
    }

    fn setup_args(sets: &[&str]) -> Vec<String> {
        let mut args = [
            "--rigor",
            "minimal",
            "--output",
            "p.json",
            "--decline-advice",
        ]
        .map(str::to_owned)
        .to_vec();
        for set in sets {
            args.push("--set".into());
            args.push((*set).into());
        }
        args.push("--set".into());
        args.push("principles.enabled=false".into());
        args
    }

    fn profile_bytes(profile: &Value) -> Vec<u8> {
        serde_json::to_vec_pretty(profile).unwrap()
    }

    #[test]
    fn binding_settings_write_model_bindings_and_record_each_override() {
        let (unbound, _, unbound_explain) = resolve(&setup_args(&[])).unwrap();
        assert!(unbound.get("model_bindings").is_none());
        assert!(unbound_explain["effective"].get("model_bindings").is_none());
        let (bound, _, explain) = resolve(&setup_args(&[
            "binding.ordinary-reviewer=anthropic/claude-sonnet-5@high",
            "binding.driver=openai/gpt-5@low",
            "binding.driver-session=openai/gpt-5@low",
        ]))
        .unwrap();
        assert_eq!(
            bound["model_bindings"],
            json!({
                "roles": {
                    "ordinary-reviewer": {"model": "anthropic/claude-sonnet-5", "thinking": "high"},
                    "driver": {"model": "openai/gpt-5", "thinking": "low"}
                },
                "driver_session": {"model": "openai/gpt-5", "thinking": "low"}
            })
        );
        assert_eq!(
            explain["effective"]["model_bindings"],
            bound["model_bindings"]
        );
        for (key, value) in [
            (
                "binding.ordinary-reviewer",
                json!({"model": "anthropic/claude-sonnet-5", "thinking": "high"}),
            ),
            (
                "binding.driver",
                json!({"model": "openai/gpt-5", "thinking": "low"}),
            ),
            (
                "binding.driver-session",
                json!({"model": "openai/gpt-5", "thinking": "low"}),
            ),
        ] {
            let change = explain["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["key"] == key)
                .unwrap_or_else(|| panic!("{key} not recorded"));
            assert_eq!(
                change,
                &json!({"key": key, "base_value": null, "value": value})
            );
        }
        let mut without = bound.clone();
        without.as_object_mut().unwrap().remove("model_bindings");
        assert_eq!(without, unbound);
        config::validate_config(&bound).unwrap();
        config::validate_config(&overlay::apply(&bound)).unwrap();
    }

    #[test]
    fn profiles_differing_only_in_a_binding_have_different_file_sha256() {
        let digest = |sets: &[&str]| {
            let (profile, _, _) = resolve(&setup_args(sets)).unwrap();
            format!("{:x}", Sha256::digest(profile_bytes(&profile)))
        };
        let unbound = digest(&[]);
        let reviewer = digest(&["binding.ordinary-reviewer=p/m@high"]);
        let other_thinking = digest(&["binding.ordinary-reviewer=p/m@low"]);
        let other_model = digest(&["binding.ordinary-reviewer=p/n@high"]);
        let unique: std::collections::BTreeSet<_> =
            [&unbound, &reviewer, &other_thinking, &other_model]
                .into_iter()
                .collect();
        assert_eq!(unique.len(), 4);
        assert_eq!(unbound, digest(&[]));
    }

    #[test]
    fn unbound_setup_output_has_no_binding_key_and_matches_shipped_profile_bytes() {
        let (profile, _, explain) = resolve(&setup_args(&[])).unwrap();
        let path = "crates/software-change-provider/data/configs/minimal.json";
        let mut expected: Value = serde_json::from_slice(
            embedded_data::FILES
                .iter()
                .find(|entry| entry.path == path)
                .unwrap()
                .bytes,
        )
        .unwrap();
        expected["extra"]["advice"]["enabled"] = json!(false);
        principles::decline(&mut expected);
        assert_eq!(profile_bytes(&profile), profile_bytes(&expected));
        assert!(!explain.to_string().contains("model_bindings"));
    }

    #[test]
    fn a_later_binding_setting_replaces_the_earlier_one_and_both_are_recorded() {
        let (profile, _, explain) = resolve(&setup_args(&[
            "binding.author=p/a@low",
            "binding.author=p/b@high",
        ]))
        .unwrap();
        assert_eq!(
            profile["model_bindings"]["roles"]["author"],
            json!({"model": "p/b", "thinking": "high"})
        );
        let recorded: Vec<_> = explain["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["key"] == "binding.author")
            .collect();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[1]["base_value"], Value::Null);
        assert_eq!(recorded[1]["value"]["model"], "p/b");
    }

    #[test]
    fn malformed_binding_settings_refuse_with_a_diagnostic() {
        for (set, expected) in [
            ("binding.author=p/m", "must be provider/model@thinking"),
            ("binding.author=@high", "nonblank provider/model"),
            ("binding.author=p/m@", "nonblank thinking level"),
            ("binding.author= @ ", "nonblank provider/model"),
            (
                "binding.reviewer=p/m@high",
                "unknown binding role `reviewer`",
            ),
            ("binding.driver.session=p/m@high", "unknown setting"),
            (
                "binding.driver-session=p/m",
                "must be provider/model@thinking",
            ),
            ("binding.driver=p/m@high", "driver_session is required"),
            (
                "binding.driver-session=p/m@high",
                "only allowed when roles.driver is bound",
            ),
        ] {
            let error = resolve(&setup_args(&[set])).expect_err(set);
            assert!(
                error.contains(expected),
                "{set}: `{error}` lacks `{expected}`"
            );
        }
        let error = resolve(&setup_args(&[
            "binding.driver=p/m@high",
            "binding.driver-session=p/m@low",
        ]))
        .unwrap_err();
        assert!(error.contains("driver_session is refused"), "{error}");
    }

    #[test]
    fn advice_setting_satisfies_explicit_choice_and_conflicts_refuse() {
        let args = [
            "--rigor",
            "minimal",
            "--output",
            "profile.json",
            "--set",
            "advice.enabled=true",
            "--set",
            "principles.enabled=false",
        ]
        .map(str::to_owned);
        assert_eq!(
            resolve(&args).unwrap().0["extra"]["advice"]["enabled"],
            true
        );
        let mut conflicting = args.to_vec();
        conflicting.push("--decline-advice".into());
        assert!(resolve(&conflicting).is_err());
    }
    fn shipped_profile(rigor: &str) -> Value {
        let args = [
            "--rigor",
            rigor,
            "--output",
            "profile.json",
            "--decline-advice",
            "--set",
            "principles.enabled=false",
        ]
        .map(str::to_owned);
        resolve(&args).unwrap().0
    }

    #[test]
    fn materializes_each_profile_template_preserving_every_field_except_group_phase() {
        for rigor in ["minimal", "standard", "high"] {
            let base = shipped_profile(rigor);
            let mut profile = base.clone();
            profile["extra"]["caller_extension"] = json!({"preserved": true});
            for phase in narratives::NARRATIVE_PHASES {
                set(&mut profile, &format!("narrative.{phase}"), "true").unwrap();
                for pass in ["ordinary", "challenge"] {
                    let mut expected = base["extra"]["narrative_review"][pass].clone();
                    for entry in expected.as_array_mut().unwrap() {
                        entry["group"] =
                            json!(entry["group"].as_str().unwrap().replace("{phase}", phase));
                    }
                    assert_eq!(
                        profile["review_policies"][narrative_gate(phase, pass)],
                        expected
                    );
                }
            }
            assert_eq!(
                profile["extra"]["narratives"],
                json!(narratives::NARRATIVE_PHASES)
            );
            for phase in narratives::CHECKPOINT_PHASES {
                set(&mut profile, &format!("checkpoint.{phase}"), "true").unwrap();
            }
            assert_eq!(
                profile["extra"]["checkpoints"],
                json!(narratives::CHECKPOINT_PHASES)
            );
            assert_eq!(
                profile["extra"]["caller_extension"],
                json!({"preserved": true})
            );
            config::validate_config(&profile).unwrap();
            config::validate_config(&overlay::apply(&profile)).unwrap();
        }
    }

    #[test]
    fn substitutes_only_literal_phase_in_groups() {
        let mut profile = shipped_profile("minimal");
        let template = &mut profile["extra"]["narrative_review"]["ordinary"][0];
        template["group"] = json!("{phase}.{phase}.{other}");
        template["description"] = json!("Keep {phase} in description");
        template["example_prompt"] = json!("Keep {phase} in prompt");
        let mut expected = template.clone();
        expected["group"] = json!("design.design.{other}");
        set(&mut profile, "narrative.design", "true").unwrap();
        assert_eq!(
            profile["review_policies"]["design-narrative-review"][0],
            expected
        );
    }

    #[test]
    fn explains_explicit_options_and_sequential_axis_edits() {
        let args = [
            "--rigor",
            "high",
            "--output",
            "profile.json",
            "--decline-advice",
            "--bookends",
            "--set",
            "principles.enabled=false",
            "--set",
            "narrative.design=true",
            "--set",
            "checkpoint.intent=true",
            "--set",
            "narrative.design=true",
            "--set",
            "review.design-narrative-review.aggregate.narrative-faithful.required_authors=3",
        ]
        .map(str::to_owned);
        let (profile, _, explain) = resolve(&args).unwrap();
        assert_eq!(profile["extra"]["narratives"], json!(["design"]));
        assert_eq!(profile["extra"]["checkpoints"], json!(["intent"]));
        assert_eq!(explain["effective"]["narratives"], json!(["design"]));
        assert_eq!(explain["effective"]["checkpoints"], json!(["intent"]));
        assert_eq!(
            explain["effective"]["review_policies"],
            profile["review_policies"]
        );
        for key in ["narrative.design", "checkpoint.intent"] {
            let change = explain["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["key"] == key)
                .unwrap();
            assert_eq!(
                change,
                &json!({"key":key, "base_value":false, "value":true})
            );
        }
        assert_eq!(
            explain["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["key"] == "narrative.design")
                .count(),
            2
        );
        assert_eq!(
            setting(
                &profile,
                "review.design-narrative-review.aggregate.narrative-faithful.required_authors"
            ),
            3
        );
    }

    #[test]
    fn disabling_removes_only_selected_phase_gates_and_records_false() {
        let base = shipped_profile("high");
        let mut profile = base.clone();
        let mut overrides = Vec::new();
        for key in [
            "narrative.design",
            "narrative.plan",
            "checkpoint.intent",
            "checkpoint.design",
        ] {
            record_set(&mut profile, &base, &mut overrides, key, "true").unwrap();
        }
        let plan = profile["review_policies"]["plan-narrative-review"].clone();
        let plan_challenge =
            profile["review_policies"]["plan-narrative-adversarial-review"].clone();
        record_set(
            &mut profile,
            &base,
            &mut overrides,
            "narrative.design",
            "false",
        )
        .unwrap();
        record_set(
            &mut profile,
            &base,
            &mut overrides,
            "checkpoint.intent",
            "false",
        )
        .unwrap();
        assert_eq!(profile["extra"]["narratives"], json!(["plan"]));
        assert_eq!(profile["extra"]["checkpoints"], json!(["design"]));
        assert!(profile["review_policies"]
            .get("design-narrative-review")
            .is_none());
        assert!(profile["review_policies"]
            .get("design-narrative-adversarial-review")
            .is_none());
        assert_eq!(profile["review_policies"]["plan-narrative-review"], plan);
        assert_eq!(
            profile["review_policies"]["plan-narrative-adversarial-review"],
            plan_challenge
        );
        for (gate, entries) in base["review_policies"].as_object().unwrap() {
            assert_eq!(&profile["review_policies"][gate], entries);
        }
        assert_eq!(
            overrides[4],
            json!({"key":"narrative.design", "base_value":false, "value":false})
        );
        config::validate_config(&profile).unwrap();
    }

    #[test]
    fn refuses_unknown_phases_and_non_boolean_values() {
        for key in [
            "narrative.intent",
            "narrative.implementation",
            "narrative.unknown",
            "checkpoint.reconciliation",
            "checkpoint.unknown",
            "narrative.design.extra",
        ] {
            for raw in ["true", "false"] {
                assert!(
                    set(&mut shipped_profile("minimal"), key, raw).is_err(),
                    "{key}={raw}"
                );
            }
        }
        for key in ["narrative.design", "checkpoint.intent"] {
            for raw in ["TRUE", "1", "null", "\"true\"", "yes", "[]", "{}", ""] {
                assert!(
                    set(&mut shipped_profile("minimal"), key, raw).is_err(),
                    "{key}={raw}"
                );
            }
        }
    }

    #[test]
    fn missing_template_refuses_enable_but_allows_disable_and_checkpoints() {
        for missing in ["narrative_review", "ordinary", "challenge"] {
            let mut profile = shipped_profile("minimal");
            if missing == "narrative_review" {
                profile["extra"].as_object_mut().unwrap().remove(missing);
            } else {
                profile["extra"]["narrative_review"]
                    .as_object_mut()
                    .unwrap()
                    .remove(missing);
            }
            assert!(set(&mut profile, "narrative.design", "true")
                .unwrap_err()
                .contains("requires extra.narrative_review"));
            set(&mut profile, "narrative.design", "false").unwrap();
            set(&mut profile, "checkpoint.intent", "true").unwrap();
        }
    }

    #[test]
    fn reused_enabled_profile_keeps_customizations_and_fills_only_missing_gates() {
        let mut profile = shipped_profile("standard");
        set(&mut profile, "narrative.design", "true").unwrap();
        set(
            &mut profile,
            "review.design-narrative-review.aggregate.narrative-faithful.required_authors",
            "3",
        )
        .unwrap();
        let customized = profile["review_policies"]["design-narrative-review"].clone();
        profile["review_policies"]
            .as_object_mut()
            .unwrap()
            .remove("design-narrative-adversarial-review");
        let base = profile.clone();
        let mut overrides = Vec::new();
        fill_missing_narrative_gates(&mut profile, &base, &mut overrides).unwrap();
        assert_eq!(
            profile["review_policies"]["design-narrative-review"],
            customized
        );
        assert_eq!(overrides.len(), 1);
        assert_eq!(
            overrides[0],
            json!({
                "key":"review_policies.design-narrative-adversarial-review",
                "base_value":null,
                "value":profile["review_policies"]["design-narrative-adversarial-review"]
            })
        );
        config::validate_config(&profile).unwrap();
        profile["extra"]
            .as_object_mut()
            .unwrap()
            .remove("narrative_review");
        fill_missing_narrative_gates(&mut profile, &base, &mut Vec::new()).unwrap();
        profile["review_policies"]
            .as_object_mut()
            .unwrap()
            .remove("design-narrative-adversarial-review");
        assert!(fill_missing_narrative_gates(&mut profile, &base, &mut Vec::new()).is_err());
    }

    #[test]
    fn flags_off_keeps_shipped_policy_and_unrelated_extra() {
        for rigor in ["minimal", "standard", "high"] {
            let args = [
                "--rigor",
                rigor,
                "--output",
                "profile.json",
                "--decline-advice",
                "--set",
                "principles.enabled=false",
            ]
            .map(str::to_owned);
            let (profile, _, explain) = resolve(&args).unwrap();
            let name = if rigor == "high" { "high-rigor" } else { rigor };
            let path = format!("crates/software-change-provider/data/configs/{name}.json");
            let mut expected: Value = serde_json::from_slice(
                embedded_data::FILES
                    .iter()
                    .find(|entry| entry.path == path)
                    .unwrap()
                    .bytes,
            )
            .unwrap();
            expected["extra"]["advice"]["enabled"] = json!(false);
            principles::decline(&mut expected);
            assert_eq!(profile, expected);
            assert_eq!(explain["effective"]["narratives"], json!([]));
            assert_eq!(explain["effective"]["checkpoints"], json!([]));
            assert_eq!(explain["overrides"].as_array().unwrap().len(), 2);
        }
    }

    #[test]
    fn refuses_retired_flags() {
        for flag in [
            "--roster",
            "--engine",
            "--provider",
            "--implementation",
            "--draft-worker",
            "--advice-config",
        ] {
            assert!(resolve(&[flag.into(), "x".into()]).is_err());
        }
    }

    const PRINCIPLES_FILE: &str = "# Fictional\n\n## 1. Alpha\nA\n\n## 2. Beta gamma\nB\n";

    fn principles_fixture(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "software-change-principles-{}-{name}.md",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        path
    }

    fn rigor_args(rigor: &str, sets: &[&str]) -> Vec<String> {
        let mut args = ["--rigor", rigor, "--output", "p.json", "--decline-advice"]
            .map(str::to_owned)
            .to_vec();
        for set in sets {
            args.push("--set".into());
            args.push((*set).into());
        }
        args
    }

    #[test]
    fn shipped_profiles_need_an_explicit_principles_choice() {
        for rigor in ["minimal", "standard", "high"] {
            let error = resolve(&rigor_args(rigor, &[])).unwrap_err();
            assert!(error.contains("principles.ref=PATH"), "{error}");
            assert!(error.contains("principles.enabled=false"), "{error}");
        }
    }

    #[test]
    fn principles_ref_freezes_the_file_into_the_profile_and_explain() {
        let path = principles_fixture("ref", PRINCIPLES_FILE);
        let set = format!("principles.ref={}", path.display());
        let (profile, _, explain) = resolve(&rigor_args("standard", &[&set])).unwrap();
        let record = &profile["extra"]["principles"];
        assert_eq!(record["source"], path.display().to_string());
        assert_eq!(
            record["sha256"],
            format!("{:x}", Sha256::digest(PRINCIPLES_FILE.as_bytes()))
        );
        assert_eq!(record["bytes"], PRINCIPLES_FILE.len());
        assert_eq!(record["headings"][1]["anchor"], "beta-gamma");
        assert!(!principles::has_marker(&profile));
        let gates = profile["review_policies"].as_object().unwrap();
        let carriers: Vec<_> = gates
            .iter()
            .flat_map(|(gate, entries)| {
                entries
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|entry| {
                        entry["example_prompt"]
                            .as_str()
                            .unwrap()
                            .contains(PRINCIPLES_FILE)
                    })
                    .map(move |entry| format!("{gate}/{}", entry["id"].as_str().unwrap()))
            })
            .collect();
        assert_eq!(carriers.len(), 7, "{carriers:?}");
        assert!(carriers.contains(&"design-adversarial-review/mechanism-forced".to_owned()));
        assert!(!carriers
            .iter()
            .any(|c| c.contains("adversarial") && c.contains("principles")));
        assert_eq!(explain["effective"]["principles"], *record);
        let change = explain["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["key"] == "principles.ref")
            .unwrap();
        assert_eq!(
            change,
            &json!({"key": "principles.ref", "base_value": null, "value": path.display().to_string()})
        );
        let again = resolve(&rigor_args("standard", &[&set])).unwrap().0;
        assert_eq!(profile_bytes(&again), profile_bytes(&profile));
        config::validate_config(&profile).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn principles_decline_drops_the_axis_everywhere() {
        let (profile, _, explain) =
            resolve(&rigor_args("high", &["principles.enabled=false"])).unwrap();
        assert_eq!(profile["extra"]["principles"], json!({"enabled": false}));
        for entries in profile["review_policies"].as_object().unwrap().values() {
            assert!(entries
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["id"] != "principles-honored"));
        }
        assert!(!principles::has_marker(&profile));
        let change = explain["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["key"] == "principles.enabled")
            .unwrap();
        assert_eq!(
            change,
            &json!({"key": "principles.enabled", "base_value": false, "value": false})
        );
    }

    #[test]
    fn principles_settings_refuse_bad_and_conflicting_choices() {
        let good = principles_fixture("good", PRINCIPLES_FILE);
        let headingless = principles_fixture("none", "no headings here\n");
        let dup = principles_fixture("dup", "## 1. Same\na\n## 2. Same\nb\n");
        let good_set = format!("principles.ref={}", good.display());
        let cases: Vec<(Vec<&str>, &str)> = vec![
            (
                vec!["principles.ref=/no/such/principles.md"],
                "principles file",
            ),
            (vec!["principles.enabled=true"], "needs principles.ref=PATH"),
            (
                vec!["principles.enabled=maybe"],
                "principles.ref=PATH or principles.enabled=false",
            ),
            (
                vec!["principles.ref="],
                "principles.ref=PATH or principles.enabled=false",
            ),
            (vec!["principles.enabled=false", &good_set], "already set"),
            (vec![&good_set, "principles.enabled=false"], "already set"),
            (vec![&good_set, &good_set], "already set"),
        ];
        for (sets, needle) in cases {
            let error = resolve(&rigor_args("minimal", &sets)).unwrap_err();
            assert!(error.contains(needle), "{sets:?}: {error}");
        }
        for (path, needle) in [
            (&headingless, "no \"## \" headings"),
            (&dup, "duplicate anchor"),
        ] {
            let set = format!("principles.ref={}", path.display());
            let error = resolve(&rigor_args("minimal", &[&set])).unwrap_err();
            assert!(error.contains(needle), "{error}");
        }
        for path in [good, headingless, dup] {
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn reused_profile_keeps_its_snapshot_and_needs_no_new_choice() {
        let path = principles_fixture("reuse", PRINCIPLES_FILE);
        let set = format!("principles.ref={}", path.display());
        let (first, _, _) = resolve(&rigor_args("minimal", &[&set])).unwrap();
        let profile_path = std::env::temp_dir().join(format!(
            "software-change-principles-{}-reuse.json",
            std::process::id()
        ));
        fs::write(&profile_path, serde_json::to_vec(&first).unwrap()).unwrap();
        let args = [
            "--profile",
            profile_path.to_str().unwrap(),
            "--output",
            "p.json",
            "--decline-advice",
        ]
        .map(str::to_owned);
        let (second, _, _) = resolve(&args).unwrap();
        assert_eq!(second["extra"]["principles"], first["extra"]["principles"]);
        let mut again = args.to_vec();
        again.extend(["--set".into(), set]);
        assert!(resolve(&again).unwrap_err().contains("already set"));
        let mut other = args.to_vec();
        other.extend(["--set".into(), "principles.enabled=false".into()]);
        assert!(resolve(&other).unwrap_err().contains("already set"));
        fs::remove_file(path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }
}
