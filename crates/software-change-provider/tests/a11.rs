use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[path = "support/mod.rs"]
mod support;

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];

#[test]
fn calibration_manifest_is_attested_and_covers_each_profile_axis_both_ways() {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("data/calibration/manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path).expect("read calibration manifest");
    let manifest: Value = serde_json::from_str(&manifest_text).expect("parse calibration manifest");
    let entries = manifest.as_array().expect("manifest array");
    assert_eq!(entries.len(), 66, "T08 manifest row count changed");

    let mut expected_keys = BTreeSet::new();
    for profile in PROFILES {
        let config = support::load_profile(profile);
        let version = config["config_version"]
            .as_str()
            .expect("config version")
            .to_owned();
        let policies = config["review_policies"]
            .as_object()
            .expect("policies object");
        for (gate, axes) in policies {
            for axis in axes.as_array().expect("policy axis array") {
                expected_keys.insert((
                    version.clone(),
                    gate.clone(),
                    axis["id"].as_str().expect("axis id").to_owned(),
                ));
            }
        }
    }

    let mut coverage: BTreeMap<(String, String, String), BTreeSet<String>> = BTreeMap::new();
    for entry in entries {
        let entry = entry.as_object().expect("manifest entry object");
        for field in [
            "fixture_id",
            "gate",
            "axis",
            "expected",
            "observed",
            "config_version",
            "model",
            "invocation",
            "attested_by",
        ] {
            assert!(
                entry.get(field).and_then(Value::as_str).is_some(),
                "missing string {field}"
            );
        }

        let fixture_id = entry["fixture_id"].as_str().unwrap();
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/calibration/fixtures")
                .join(format!("{fixture_id}.json"))
                .is_file(),
            "manifest fixture {fixture_id} missing"
        );
        let expected = entry["expected"].as_str().unwrap();
        let observed = entry["observed"].as_str().unwrap();
        assert!(matches!(expected, "pass" | "fail"));
        assert_eq!(observed, expected, "unattested or flipped calibration row");
        assert!(!entry["attested_by"].as_str().unwrap().is_empty());

        let key = (
            entry["config_version"].as_str().unwrap().to_owned(),
            entry["gate"].as_str().unwrap().to_owned(),
            entry["axis"].as_str().unwrap().to_owned(),
        );
        assert!(
            expected_keys.contains(&key),
            "manifest has unknown axis key {key:?}"
        );
        coverage.entry(key).or_default().insert(expected.to_owned());
    }

    assert_eq!(coverage.len(), expected_keys.len());
    for key in expected_keys {
        assert_eq!(
            coverage.get(&key),
            Some(&BTreeSet::from(["fail".to_owned(), "pass".to_owned()])),
            "calibration key lacks expected pass/fail coverage: {key:?}"
        );
    }
}
