//! Owner design principles snapshot: read once at setup, frozen into the profile.
//!
//! Shipped profiles carry a marker where the snapshot goes and never a path or any
//! principle text. Setup replaces the marker with the file's bytes (or a decline line)
//! and records the file's identity under `extra.principles`. The provider checks
//! structure only; whether an artifact honors a heading is the reviewer's judgment.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Placeholder in shipped example prompts that setup replaces.
pub const MARKER: &str = "<<owner-principles-snapshot>>";
/// Replaces the marker when the owner declines a principles file.
pub const DECLINED: &str = "No owner design principles snapshot is configured for this run; judge only against the intent and the operating context.";
/// Axis that only exists when a snapshot does.
pub const AXIS: &str = "principles-honored";

#[derive(Debug)]
pub struct Snapshot {
    pub record: Value,
    pub rendered: String,
}

fn hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Stable heading anchor: drop a leading "N. ", lowercase, one hyphen per run of
/// non-alphanumerics, trim hyphens.
pub fn anchor(title: &str) -> String {
    let trimmed = title.trim();
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    let rest = match trimmed[digits..].strip_prefix(". ") {
        Some(rest) if digits > 0 => rest,
        _ => trimmed,
    };
    let mut out = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Build the snapshot record and the text that replaces the marker.
pub fn snapshot(source: &str, text: &str) -> Result<Snapshot, String> {
    if text.trim().is_empty() {
        return Err(format!("principles file {source} is empty"));
    }
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("## ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    if starts.is_empty() {
        return Err(format!("principles file {source} has no \"## \" headings"));
    }
    let mut headings = Vec::new();
    let mut anchors: Vec<String> = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        let section = &text[*start..end];
        let title = section.lines().next().unwrap_or("")[3..].trim().to_owned();
        let anchor = anchor(&title);
        if anchor.is_empty() || anchors.contains(&anchor) {
            return Err(format!(
                "principles file {source}: heading \"{title}\" gives an empty or duplicate anchor"
            ));
        }
        headings.push(json!({"anchor": anchor, "title": title, "sha256": hex(section)}));
        anchors.push(anchor);
    }
    let sha256 = hex(text);
    let record = json!({
        "enabled": true,
        "source": source,
        "sha256": sha256,
        "bytes": text.len(),
        "headings": headings,
    });
    let rendered = format!(
        "Heading anchors in this snapshot: {}.\n\nPrinciples snapshot (verbatim, sha256 {sha256}):\n{text}",
        anchors.join(", ")
    );
    Ok(Snapshot { record, rendered })
}

fn prompts_mut(profile: &mut Value) -> impl Iterator<Item = &mut Value> {
    profile["review_policies"]
        .as_object_mut()
        .into_iter()
        .flat_map(|gates| gates.values_mut())
        .filter_map(Value::as_array_mut)
        .flatten()
}

/// True while any shipped prompt still carries the marker.
pub fn has_marker(profile: &Value) -> bool {
    profile["review_policies"]
        .as_object()
        .into_iter()
        .flat_map(|gates| gates.values())
        .filter_map(Value::as_array)
        .flatten()
        .any(|entry| {
            entry["example_prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains(MARKER))
        })
}

fn replace_marker(profile: &mut Value, replacement: &str) {
    for entry in prompts_mut(profile) {
        if let Some(prompt) = entry["example_prompt"].as_str() {
            if prompt.contains(MARKER) {
                entry["example_prompt"] = json!(prompt.replace(MARKER, replacement));
            }
        }
    }
}

fn record(profile: &mut Value, value: Value) {
    if !profile["extra"].is_object() {
        profile["extra"] = json!({});
    }
    profile["extra"]["principles"] = value;
}

/// Freeze the snapshot into every prompt that carries the marker.
pub fn apply(profile: &mut Value, snapshot: &Snapshot) {
    replace_marker(profile, &snapshot.rendered);
    record(profile, snapshot.record.clone());
}

/// Owner declines a principles file: the axis goes, the other prompts say so.
pub fn decline(profile: &mut Value) {
    if let Some(gates) = profile["review_policies"].as_object_mut() {
        for entries in gates.values_mut().filter_map(Value::as_array_mut) {
            entries.retain(|entry| entry["id"] != AXIS);
        }
    }
    replace_marker(profile, DECLINED);
    record(profile, json!({"enabled": false}));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Title\nintro\n\n## 1. Alpha beta\nA text\n\n## 2. Gamma: delta (x)\nG text\n\n## Enforcement (strongest first)\nE text\n";

    #[test]
    fn anchors_follow_the_rule() {
        assert_eq!(anchor("1. Composable components"), "composable-components");
        assert_eq!(
            anchor("12. Verified from the outside"),
            "verified-from-the-outside"
        );
        assert_eq!(
            anchor("Enforcement (strongest first)"),
            "enforcement-strongest-first"
        );
        assert_eq!(
            anchor("9. Fix the cause: no arbitrary caps or unrequested defense in depth"),
            "fix-the-cause-no-arbitrary-caps-or-unrequested-defense-in-depth"
        );
        assert_eq!(anchor("2024 plan"), "2024-plan");
        assert_eq!(anchor("---"), "");
    }

    #[test]
    fn snapshot_records_identity_and_embeds_the_file_verbatim() {
        let snap = snapshot("/p/principles.md", FILE).unwrap();
        assert_eq!(snap.record["enabled"], true);
        assert_eq!(snap.record["source"], "/p/principles.md");
        assert_eq!(snap.record["bytes"], FILE.len());
        assert_eq!(snap.record["sha256"], hex(FILE));
        let anchors: Vec<_> = snap.record["headings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["anchor"].as_str().unwrap())
            .collect();
        assert_eq!(
            anchors,
            ["alpha-beta", "gamma-delta-x", "enforcement-strongest-first"]
        );
        let sections = [
            "## 1. Alpha beta\nA text\n\n",
            "## 2. Gamma: delta (x)\nG text\n\n",
            "## Enforcement (strongest first)\nE text\n",
        ];
        for (heading, section) in snap.record["headings"]
            .as_array()
            .unwrap()
            .iter()
            .zip(sections)
        {
            assert_eq!(heading["sha256"], hex(section));
        }
        assert!(snap
            .rendered
            .contains(&format!("sha256 {}):\n{FILE}", hex(FILE))));
        assert!(snap.rendered.starts_with("Heading anchors in this snapshot: alpha-beta, gamma-delta-x, enforcement-strongest-first."));
        assert_eq!(
            snapshot("/p/principles.md", FILE).unwrap().rendered,
            snap.rendered
        );
    }

    #[test]
    fn refuses_empty_headingless_and_duplicate_anchor_files() {
        assert!(snapshot("p", "  \n").unwrap_err().contains("empty"));
        assert!(snapshot("p", "no headings\n")
            .unwrap_err()
            .contains("no \"## \" headings"));
        let duplicate = snapshot("p", "## 1. Same\na\n## 2. Same\nb\n").unwrap_err();
        assert!(duplicate.contains("duplicate anchor"), "{duplicate}");
        assert!(snapshot("p", "## ---\na\n")
            .unwrap_err()
            .contains("empty or duplicate"));
    }

    fn profile() -> Value {
        json!({"review_policies": {
            "design-review": [
                {"id": AXIS, "example_prompt": format!("head {MARKER}")},
                {"id": "mechanism-forced", "example_prompt": format!("mf {MARKER}")},
                {"id": "risk-honest", "example_prompt": "plain"}],
            "design-adversarial-review": [
                {"id": "mechanism-forced", "example_prompt": format!("ch {MARKER}")}]}})
    }

    #[test]
    fn apply_replaces_every_marker_and_records_the_snapshot() {
        let mut profile = profile();
        assert!(has_marker(&profile));
        let snap = snapshot("p", FILE).unwrap();
        apply(&mut profile, &snap);
        assert!(!has_marker(&profile));
        assert_eq!(profile["extra"]["principles"], snap.record);
        let design = profile["review_policies"]["design-review"]
            .as_array()
            .unwrap();
        assert_eq!(
            design[0]["example_prompt"],
            format!("head {}", snap.rendered)
        );
        assert_eq!(design[2]["example_prompt"], "plain");
        assert_eq!(
            profile["review_policies"]["design-adversarial-review"][0]["example_prompt"],
            format!("ch {}", snap.rendered)
        );
    }

    #[test]
    fn decline_removes_the_axis_and_states_the_absence() {
        let mut profile = profile();
        decline(&mut profile);
        assert!(!has_marker(&profile));
        assert_eq!(profile["extra"]["principles"], json!({"enabled": false}));
        let design = profile["review_policies"]["design-review"]
            .as_array()
            .unwrap();
        assert_eq!(design.len(), 2);
        assert!(design.iter().all(|entry| entry["id"] != AXIS));
        assert_eq!(design[0]["example_prompt"], format!("mf {DECLINED}"));
    }
}
