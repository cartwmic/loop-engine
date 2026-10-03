//! bookends:LE-180 — deterministic base-derived preparation for providers with unstaged review axes.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

pub fn run(
    args: &[String],
    shipped: impl Fn(&str) -> Option<&'static [u8]>,
    validate: impl Fn(&Value) -> Result<(), String>,
    semantic: bool,
) -> i32 {
    if args == ["--help"] || args == ["-h"] {
        println!("setup (--rigor NAME | --profile FILE) --output FILE [--set review.GATE.AXIS.required_authors=N|self_review=BOOL|group=NAME] [--add-axis GATE.AXIS=GATE.AXIS] [--remove-axis GATE.AXIS] [--previous-explain FILE]");
        return 0;
    }
    match resolve(args, shipped, validate, semantic) {
        Ok(profile) => {
            println!("{profile}");
            0
        }
        Err(error) => {
            eprintln!("setup: {error}");
            2
        }
    }
}

fn rows<'a>(
    profile: &'a mut Value,
    gate: &str,
    semantic: bool,
) -> Result<&'a mut Vec<Value>, String> {
    let value = if semantic {
        if gate != "semantic-review" {
            return Err(format!("unknown gate {gate}"));
        }
        &mut profile["semantic_policies"]
    } else {
        &mut profile["review_policies"][gate]
    };
    value
        .as_array_mut()
        .ok_or_else(|| format!("unknown gate {gate}"))
}
fn target(name: &str) -> Result<(&str, &str), String> {
    let (gate, axis) = name.split_once('.').ok_or("axis must be GATE.AXIS")?;
    if axis.is_empty() || axis.contains('.') {
        return Err("axis must be GATE.AXIS".into());
    }
    Ok((gate, axis))
}
fn axis<'a>(profile: &'a mut Value, name: &str, semantic: bool) -> Result<&'a mut Value, String> {
    let (gate, id) = target(name)?;
    rows(profile, gate, semantic)?
        .iter_mut()
        .find(|row| row["id"] == id)
        .ok_or_else(|| format!("unknown axis {name}"))
}
fn resolve(
    args: &[String],
    shipped: impl Fn(&str) -> Option<&'static [u8]>,
    validate: impl Fn(&Value) -> Result<(), String>,
    semantic: bool,
) -> Result<Value, String> {
    let mut base = None;
    let mut output = None;
    let mut previous = None;
    let mut operations = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let (flag, inline) = args[i]
            .split_once('=')
            .map_or((args[i].as_str(), None), |(k, v)| (k, Some(v)));
        if ![
            "--rigor",
            "--profile",
            "--output",
            "--previous-explain",
            "--set",
            "--add-axis",
            "--remove-axis",
        ]
        .contains(&flag)
        {
            return Err(format!("unknown argument {flag}"));
        }
        let value = if let Some(value) = inline {
            value
        } else {
            i += 1;
            args.get(i).ok_or("missing argument value")?
        };
        match flag {
            "--rigor" | "--profile" => {
                if base.is_some() {
                    return Err("select exactly one base".into());
                }
                let bytes = if flag == "--profile" {
                    fs::read(value).map_err(|e| e.to_string())?
                } else {
                    shipped(value).ok_or("unknown shipped base")?.to_vec()
                };
                let parsed: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                base = Some((parsed, format!("sha256:{:x}", Sha256::digest(&bytes))));
            }
            "--output" => {
                if output.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate output".into());
                }
            }
            "--previous-explain" => {
                if previous.is_some() {
                    return Err("duplicate previous explain".into());
                }
                previous = Some(
                    serde_json::from_slice::<Value>(&fs::read(value).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?,
                );
            }
            _ => operations.push((flag.to_owned(), value.to_owned())),
        }
        i += 1;
    }
    let (base, digest) = base.ok_or("missing base")?;
    validate(&base)?;
    let output = output.ok_or("missing output")?;
    let mut profile = base.clone();
    let mut overrides = Vec::new();
    for (flag, spec) in operations {
        let (key, before, after) = if flag == "--set" {
            let (key, raw) = spec.split_once('=').ok_or("set requires KEY=VALUE")?;
            let rest = key
                .strip_prefix("review.")
                .ok_or("setting must start with review.")?;
            let (name, field) = rest.rsplit_once('.').ok_or("missing setting field")?;
            if !["required_authors", "self_review", "group"].contains(&field) {
                return Err(format!("unknown setting {key}"));
            }
            let mut base_copy = base.clone();
            let before = axis(&mut base_copy, name, semantic)
                .map(|row| row[field].clone())
                .unwrap_or(Value::Null);
            let value = match field {
                "group" => {
                    let v: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
                    if v.as_str().is_none_or(|s| s.trim().is_empty()) {
                        return Err("group must be nonempty string".into());
                    }
                    v
                }
                "required_authors" => {
                    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
                    if v.as_u64().is_none_or(|n| n == 0) {
                        return Err("required_authors must be integer >= 1".into());
                    }
                    v
                }
                _ => {
                    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
                    if !v.is_boolean() {
                        return Err("self_review must be boolean".into());
                    }
                    v
                }
            };
            axis(&mut profile, name, semantic)?[field] = value.clone();
            (key.to_owned(), before, value)
        } else {
            let (name, source) = if flag == "--add-axis" {
                let (n, s) = spec
                    .split_once('=')
                    .ok_or("add-axis requires target=source")?;
                (n, Some(s))
            } else {
                (spec.as_str(), None)
            };
            let mut base_copy = base.clone();
            let before = axis(&mut base_copy, name, semantic)
                .cloned()
                .unwrap_or(Value::Null);
            let (gate, id) = target(name)?;
            let after = if let Some(source) = source {
                if axis(&mut profile, name, semantic).is_ok() {
                    return Err("added axis already exists".into());
                }
                let mut row = axis(&mut profile, source, semantic)?.clone();
                row["id"] = json!(id);
                rows(&mut profile, gate, semantic)?.push(row.clone());
                row
            } else {
                axis(&mut profile, name, semantic)?;
                rows(&mut profile, gate, semantic)?.retain(|row| row["id"] != id);
                Value::Null
            };
            (format!("axis.{name}"), before, after)
        };
        overrides.push(json!({"key":key,"base_value":before,"value":after}));
    }
    validate(&profile)?;
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
    let id = if semantic {
        &base["profile_version"]
    } else {
        &base["config_version"]
    };
    let explain = json!({"base":{"id":id,"sha256":digest},"overrides":overrides,"effective":profile,"drift":drift});
    let mut explanation = output.as_os_str().to_owned();
    explanation.push(".explain.json");
    fs::write(
        &output,
        serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        PathBuf::from(explanation),
        serde_json::to_vec_pretty(&explain).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_and_research_targets_are_explicit() {
        let mut policy = json!({"semantic_policies":[{"id":"quality"}]});
        assert!(axis(&mut policy, "semantic-review.quality", true).is_ok());
        assert!(axis(&mut policy, "verify.quality", true).is_err());
        let mut research = json!({"review_policies":{"verify":[{"id":"grounded"}]}});
        assert!(axis(&mut research, "verify.grounded", false).is_ok());
        assert!(axis(&mut research, "verify.missing", false).is_err());
        assert!(target("verify.aggregate.grounded").is_err());
    }

    #[test]
    fn resolution_validates_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("profile.json");
        let args = vec![
            "--rigor".into(),
            "standard".into(),
            "--output".into(),
            output.to_string_lossy().into_owned(),
            "--set".into(),
            "review.verify.grounded.required_authors=0".into(),
        ];
        let shipped = |_: &str| {
            Some(
                br#"{"config_version":"test","review_policies":{"verify":[{"id":"grounded"}]}}"#
                    .as_slice(),
            )
        };
        assert!(resolve(&args, shipped, |_| Ok(()), false).is_err());
        assert!(!output.exists());
        assert!(resolve(&args[..4], shipped, |_| Err("invalid base".into()), false).is_err());
        assert!(!output.exists());
    }
}
