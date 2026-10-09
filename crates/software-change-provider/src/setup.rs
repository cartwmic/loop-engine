//! Resolve policy without launching workers, advisors, or the engine.
use crate::{config, embedded_data, overlay};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

const USAGE: &str = "usage: software-change setup (--rigor minimal|standard|high | --profile FILE) --output FILE (--enable-advice | --decline-advice) [--bookends] [--set KEY=VALUE] [--remove-axis gate.stage.axis] [--add-axis gate.stage.axis=gate.stage.axis] [--previous-explain FILE]";

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

fn set(profile: &mut Value, key: &str, raw: &str) -> Result<(), String> {
    let parts: Vec<_> = key.split('.').collect();
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

fn setting(profile: &Value, key: &str) -> Value {
    let parts: Vec<_> = key.split('.').collect();
    let value = match parts.as_slice() {
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
            .any(|name| (1..=14).any(|version| id == format!("{name}-{version}")) || id == *name)
    }) {
        return Err("historical shipped profile IDs cannot be used for new setup".into());
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
    let explain = json!({"base":{"id":base["config_version"],"sha256":digest},"overrides":overrides,"effective":{"config_version":profile["config_version"],"review_policies":profile["review_policies"],"criterion_policy":profile["criterion_policy"],"advice":profile["extra"]["advice"],"bookends":profile["extra"]["bookends"]},"drift":drift});
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
        ]
        .map(str::to_owned);
        let (profile, _, explain) = resolve(&args).unwrap();
        assert!(profile.get("work_slot_bindings").is_none());
        assert!(profile.get("advice_command").is_none());
        assert_eq!(profile["contract_version"], 4);
        assert_eq!(explain["base"]["id"], "standard-15");
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
            "historical shipped profile IDs cannot be used for new setup"
        );
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
}
