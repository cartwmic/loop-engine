//! Resolve a profile without launching workers, advisors, or the engine.
use crate::{config, embedded_data, overlay};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

const USAGE: &str = "usage: software-change setup (--rigor minimal|standard|high | --profile FILE) --output FILE (--enable-advice | --decline-advice) [--bookends]";

pub fn run_from_args(args: &[String]) -> i32 {
    if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
        return 0;
    }
    match resolve(args) {
        Ok((profile, output)) => {
            let bytes = serde_json::to_vec_pretty(&profile).expect("JSON profile");
            if let Err(error) = fs::write(&output, &bytes) {
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

fn resolve(args: &[String]) -> Result<(Value, PathBuf), String> {
    let mut base = None;
    let mut output = None;
    let mut advice = None;
    let mut bookends = false;
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
            "--rigor" | "--profile" | "--output" => {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        i += 1;
                        args.get(i)
                            .filter(|v| !v.starts_with('-'))
                            .ok_or_else(|| format!("{flag} requires a value"))?
                    }
                };
                if flag == "--output" {
                    if output.replace(PathBuf::from(value)).is_some() {
                        return Err("duplicate --output".into());
                    }
                } else {
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
                    base =
                        Some(serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string())?);
                }
            }
            _ => return Err(format!("unknown or duplicate argument `{}`", args[i])),
        }
        i += 1;
    }
    let mut profile = base.ok_or("missing base profile")?;
    let enabled = advice.ok_or("choose --enable-advice or --decline-advice")?;
    let root = profile.as_object_mut().ok_or("profile must be an object")?;
    root.remove("work_slot_bindings");
    root.remove("advice_command");
    root.remove("advice_departures");
    let extra = root
        .entry("extra")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("extra must be an object")?;
    extra
        .entry("advice")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("advice must be an object")?
        .insert("enabled".into(), json!(enabled));
    if bookends {
        let extra = root
            .entry("extra")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("extra must be an object")?;
        let policy = extra
            .entry("bookends")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("bookends must be an object")?;
        policy.insert("enabled".into(), json!(true));
    }
    if profile["contract_version"] != 3 {
        return Err("profile must use contract_version 3".into());
    }
    config::validate_config(&profile).map_err(|e| e.to_string())?;
    config::validate_config(&overlay::apply(&profile)).map_err(|e| e.to_string())?;
    Ok((profile, output.ok_or("missing --output")?))
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
        let (profile, _) = resolve(&args).unwrap();
        assert!(profile.get("work_slot_bindings").is_none());
        assert!(profile.get("advice_command").is_none());
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
