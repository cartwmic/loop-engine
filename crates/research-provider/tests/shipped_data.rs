use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const SUBJECTS: &[&str] = &[
    "brief.json",
    "sources.json",
    "verification.json",
    "report.json",
];
const PROMPT_GUARDS: &[&str] = &[
    "silence is not a finding",
    "check omissions only against the named acceptance/requirements set — do not hunt for unlisted omissions",
    "do not invent norms",
    "length/count proxies are not findings",
    "Do not waive material finding",
];

fn shipped_text(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("could not read {path:?}: {error}"))
}

fn load_profile() -> Value {
    let path = format!("{}/data/configs/standard.json", env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read shipped config {path}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("invalid JSON in {path}: {error}"))
}

fn object<'a>(value: &'a Value, path: &str) -> &'a serde_json::Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{path} must be an object"))
}

fn array<'a>(value: &'a Value, path: &str) -> &'a Vec<Value> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{path} must be an array"))
}

fn string<'a>(value: &'a Value, path: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{path} must be a string"))
}

fn axis_map(config: &Value) -> BTreeMap<String, BTreeMap<String, u64>> {
    let policies = object(&config["review_policies"], "review_policies");
    policies
        .iter()
        .map(|(gate, entries)| {
            let axes = array(entries, &format!("review_policies.{gate}"))
                .iter()
                .map(|entry| {
                    let entry = object(entry, "policy entry");
                    (
                        string(&entry["id"], "policy id").to_owned(),
                        entry
                            .get("required_authors")
                            .and_then(Value::as_u64)
                            .unwrap_or(1),
                    )
                })
                .collect();
            (gate.clone(), axes)
        })
        .collect()
}

fn assert_b2_schema(subject: &str, schema: &Value) {
    let schema = object(schema, subject);
    assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"));
    let required: BTreeSet<&str> = array(
        schema.get("required").expect("schema required"),
        &format!("{subject}.required"),
    )
    .iter()
    .map(|value| string(value, "required entry"))
    .collect();
    assert!(required.contains("revision"), "{subject} missing revision");
    assert!(required.contains("author"), "{subject} missing author");

    let properties = object(
        schema.get("properties").expect("schema properties"),
        &format!("{subject}.properties"),
    );
    assert_eq!(
        object(
            properties.get("revision").expect("revision schema"),
            "revision"
        )
        .get("type")
        .and_then(Value::as_str),
        Some("string")
    );
    let author = object(properties.get("author").expect("author schema"), "author");
    assert_eq!(author.get("type").and_then(Value::as_str), Some("object"));
    let author_required: BTreeSet<&str> = array(
        author.get("required").expect("author required"),
        "author.required",
    )
    .iter()
    .map(|value| string(value, "author required entry"))
    .collect();
    assert!(author_required.contains("name"));
    assert!(author_required.contains("kind"));
    let author_properties = object(
        author.get("properties").expect("author properties"),
        "author.properties",
    );
    assert_eq!(
        object(
            author_properties.get("name").expect("author.name schema"),
            "author.name"
        )
        .get("type")
        .and_then(Value::as_str),
        Some("string")
    );
    let kind = object(
        author_properties.get("kind").expect("author.kind schema"),
        "author.kind",
    );
    assert_eq!(kind.get("type").and_then(Value::as_str), Some("string"));
    let kinds: BTreeSet<&str> = array(kind.get("enum").expect("author.kind enum"), "kind.enum")
        .iter()
        .map(|value| string(value, "kind enum entry"))
        .collect();
    assert_eq!(kinds, BTreeSet::from(["human", "agent", "script"]));
}

fn markdown_files() -> Vec<(String, String)> {
    vec![
        ("README.md".to_owned(), shipped_text("README.md")),
        ("AGENTS.md".to_owned(), shipped_text("AGENTS.md")),
        (
            "skills/using-research-provider/SKILL.md".to_owned(),
            shipped_text("skills/using-research-provider/SKILL.md"),
        ),
    ]
}

#[test]
fn standard_profile_passes_production_config_validation() {
    let config = load_profile();
    assert_eq!(config["config_version"], "research-1");
    assert!(
        config.get("artifact_root").is_none(),
        "shipped profile must omit artifact_root"
    );
    assert!(
        config.get("work_slot_bindings").is_none(),
        "shipped profile must omit work_slot_bindings"
    );
    research_provider::validate_config_for_tests(&config)
        .unwrap_or_else(|error| panic!("standard rejected by production validator: {error}"));

    let schemas = object(&config["artifact_schemas"], "artifact_schemas");
    let names: BTreeSet<&str> = schemas.keys().map(String::as_str).collect();
    assert_eq!(names, SUBJECTS.iter().copied().collect());
    for subject in SUBJECTS {
        assert_b2_schema(subject, &schemas[*subject]);
    }

    let links = array(&config["revision_links"], "revision_links");
    let actual: Vec<(String, String, String)> = links
        .iter()
        .map(|link| {
            let link = object(link, "revision link");
            (
                string(&link["from"], "link.from").to_owned(),
                string(&link["field"], "link.field").to_owned(),
                string(&link["to"], "link.to").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            (
                "sources.json".into(),
                "brief_revision".into(),
                "brief.json".into()
            ),
            (
                "verification.json".into(),
                "sources_revision".into(),
                "sources.json".into()
            ),
            (
                "report.json".into(),
                "verification_revision".into(),
                "verification.json".into()
            ),
        ]
    );

    assert_eq!(config["extra"]["profile"].as_str(), Some("standard"));
    assert_eq!(
        config["extra"]["template_root"].as_str(),
        Some("crates/research-provider/data/templates")
    );
}

#[test]
fn standard_profile_carries_exact_verify_and_synthesize_axes() {
    let standard = axis_map(&load_profile());
    assert_eq!(
        standard,
        BTreeMap::from([
            (
                "verify".into(),
                BTreeMap::from([("claim-grounded".into(), 1), ("adversarial".into(), 1),])
            ),
            (
                "synthesize".into(),
                BTreeMap::from([("cited-conclusion".into(), 1), ("scope-faithful".into(), 1),])
            ),
        ])
    );
}

#[test]
fn every_axis_prompt_references_subject_template_schema_and_all_antipedantry_guards() {
    let subject_by_gate = BTreeMap::from([
        ("verify", ("verification.md", "verification.json")),
        ("synthesize", ("report.md", "report.json")),
    ]);

    let config = load_profile();
    for (gate, entries) in object(&config["review_policies"], "review_policies") {
        let (template, subject) = subject_by_gate[gate.as_str()];
        for entry in array(entries, gate) {
            let prompt = string(&object(entry, "policy entry")["example_prompt"], "prompt");
            assert!(!prompt.trim().is_empty(), "{gate} empty prompt");
            assert!(
                prompt.contains(&format!("data/templates/{template}")),
                "{gate} prompt missing template reference"
            );
            assert!(
                prompt.contains(&format!(
                    "data/configs/standard.json#/artifact_schemas/{subject}"
                )),
                "{gate} prompt missing schema reference"
            );
            for guard in PROMPT_GUARDS {
                assert!(
                    prompt.contains(guard),
                    "{gate} prompt missing anti-pedantry guard: {guard}"
                );
            }
        }
    }
}

#[test]
fn review_worker_data_defines_read_only_attributed_judgments() {
    let schema: Value =
        serde_json::from_str(&shipped_text("data/review-worker-output-schema.json"))
            .expect("review worker output schema must be valid JSON");
    assert_eq!(
        schema,
        serde_json::json!({
            "required": ["axis", "author", "result", "findings"]
        })
    );

    let preamble = shipped_text("data/review-worker-preamble.txt");
    for clause in [
        "read-only",
        "Judge only the assigned axis.",
        "frozen review assignment",
        "artifact_root",
        "Everything after the --- separator is driver context only.",
        "Return only a review judgment",
        "axis, author, result, and findings",
        "Do not perform driver duties.",
        "gather sources",
        "synthesize conclusions",
        "deterministic checks",
        "loop-engine show",
        "append evidence",
        "request an event",
        "progress the run",
    ] {
        assert!(
            preamble.contains(clause),
            "review worker preamble missing clause: {clause}"
        );
    }
}

#[test]
fn crate_agents_records_common_worker_boundary_rules() {
    let agents = shipped_text("AGENTS.md");
    for clause in [
        "Providers author worker-facing role and output content; the engine only transports and mechanically enforces it.",
        "Review workers return judgments only; drivers own deterministic checks, show, append, event, and progression.",
        "Exit 0 does not establish a valid deliverable.",
    ] {
        assert!(
            agents.contains(clause),
            "AGENTS.md missing common worker-boundary rule: {clause}"
        );
    }
}

#[test]
fn reviewer_protocol_defines_verify_synthesize_convergence_contract() {
    let protocol = shipped_text("data/reviewer-protocol.md");
    for clause in [
        "before append or mutation",
        "mandatory failure burden",
        "scope and materiality",
        "consequence proof",
        "existing validation does not already resolve",
        "focused external reconsideration",
        "comprehensive first review",
        "confirmation review",
        "late material finding",
        "current supplied evidence",
        "validation gap",
        "newly exposed",
        "fix-introduced",
        "previously overlooked",
        "previous visibility or reviewer overlook does not waive",
        "three-round circuit breaker",
        "never waives a known defect",
        "no unresolved accepted in-scope material finding",
        "zero advisory comments is not required",
        "verification-local `verification.json` corrections stay in verify",
        "retry checked `verified`",
        "report-local `report.json` corrections stay in synthesize",
        "retry checked `completed`",
        "revise-brief",
        "revise-sources",
        "subject_revision",
        "config_version",
    ] {
        assert!(
            protocol
                .to_ascii_lowercase()
                .contains(&clause.to_ascii_lowercase()),
            "reviewer protocol missing convergence clause: {clause}"
        );
    }
}

#[test]
fn crate_markdown_local_links_have_no_parent_directory_segments() {
    for (name, text) in markdown_files() {
        for (index, line) in text.lines().enumerate() {
            assert!(
                !line.contains("](../") && !line.contains("](./../"),
                "{name}:{} contains a parent-directory markdown link: {line}",
                index + 1
            );
        }
    }
}
