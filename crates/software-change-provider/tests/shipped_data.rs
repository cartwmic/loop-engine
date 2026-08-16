use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];

fn shipped_text(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("could not read {path:?}: {error}"))
}
const SUBJECTS: &[&str] = &[
    "intent.json",
    "design.json",
    "plan.json",
    "implementation-report.json",
    "validation-report.json",
];
const PROMPT_GUARDS: &[&str] = &[
    "silence is not a finding",
    "check omissions only against the named acceptance/requirements set — do not hunt for unlisted omissions",
    "do not invent norms",
    "length/count proxies are not findings",
    "Do not waive material finding",
];

fn load_profile(profile: &str) -> Value {
    let path = format!("{}/data/configs/{profile}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path)
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

fn expected_axes() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    BTreeMap::from([
        (
            "intent",
            BTreeSet::from([
                "solution-agnostic",
                "outside-verifiable",
                "scope-fenced",
                "constraints-are-limits",
                "problem-grounded",
            ]),
        ),
        (
            "design-review",
            BTreeSet::from([
                "intent-faithful",
                "acceptance-covered",
                "structural-not-procedural",
                "mechanism-forced",
            ]),
        ),
        ("plan-review", BTreeSet::new()),
        ("implementation-review", BTreeSet::new()),
        (
            "validation",
            BTreeSet::from(["intent-delivered", "docs-integrated"]),
        ),
    ])
}

fn high_rigor_axes() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    BTreeMap::from([
        (
            "intent",
            BTreeSet::from([
                "solution-agnostic",
                "outside-verifiable",
                "scope-fenced",
                "constraints-are-limits",
                "problem-grounded",
            ]),
        ),
        (
            "design-review",
            BTreeSet::from([
                "intent-faithful",
                "acceptance-covered",
                "structural-not-procedural",
                "decisions-justified",
                "risk-honest",
                "mechanism-forced",
            ]),
        ),
        (
            "plan-review",
            BTreeSet::from([
                "task-sized",
                "context-sufficient",
                "done-observable",
                "decision-free",
                "design-faithful",
                "dependencies-honest",
            ]),
        ),
        (
            "implementation-review",
            BTreeSet::from([
                "tasks-actually-done",
                "no-scope-creep",
                "design-faithful-final",
            ]),
        ),
        (
            "validation",
            BTreeSet::from([
                "intent-delivered",
                "docs-integrated",
                "requirement-proof-mapping",
            ]),
        ),
    ])
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

#[test]
fn all_profiles_pass_production_config_validation_and_have_exact_subjects() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let expected_version = match *profile {
            "minimal" => "minimal-3",
            "standard" => "standard-4",
            "high-rigor" => "high-rigor-4",
            _ => unreachable!("unknown profile {profile}"),
        };
        assert_eq!(config["config_version"], expected_version);
        assert!(
            config.get("artifact_root").is_none(),
            "{profile} shipped profile must omit artifact_root"
        );
        software_change_provider::validate_config_for_tests(&config)
            .unwrap_or_else(|error| panic!("{profile} rejected by production validator: {error}"));

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
                    "design.json".into(),
                    "intent_revision".into(),
                    "intent.json".into()
                ),
                (
                    "plan.json".into(),
                    "design_revision".into(),
                    "design.json".into()
                ),
                (
                    "implementation-report.json".into(),
                    "plan_revision".into(),
                    "plan.json".into(),
                ),
                (
                    "validation-report.json".into(),
                    "intent_revision".into(),
                    "intent.json".into(),
                ),
            ]
        );
    }
}

#[test]
fn shipped_work_slot_bindings_bind_only_implement_to_run_plan_graph() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let bindings = object(&config["work_slot_bindings"], "work_slot_bindings");
        let keys: BTreeSet<&str> = bindings.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            BTreeSet::from(["implement"]),
            "{profile} work_slot_bindings keys"
        );
        let implement = object(&bindings["implement"], "implement");
        assert_eq!(
            string(&implement["command"], "implement.command"),
            "software-change"
        );
        let args: Vec<&str> = array(&implement["args"], "implement.args")
            .iter()
            .map(|value| string(value, "implement.args item"))
            .collect();
        assert_eq!(args, vec!["run-plan-graph"]);
        for slot in ["design-review", "plan-review", "implementation-review"] {
            assert!(
                !bindings.contains_key(slot),
                "{profile} must omit review binding {slot}"
            );
        }
    }
}

#[test]
fn profiles_carry_exact_shipped_profile_mapping_and_author_counts() {
    let minimal = axis_map(&load_profile("minimal"));
    assert_eq!(
        minimal,
        BTreeMap::from([
            ("design-review".into(), BTreeMap::new()),
            ("implementation-review".into(), BTreeMap::new()),
            ("intent".into(), BTreeMap::new()),
            ("plan-review".into(), BTreeMap::new()),
            (
                "validation".into(),
                BTreeMap::from([("intent-delivered".into(), 1)])
            ),
        ])
    );

    let standard_config = load_profile("standard");
    let standard = axis_map(&standard_config);
    let expected = expected_axes()
        .into_iter()
        .map(|(gate, axes)| {
            (
                gate.to_owned(),
                axes.into_iter().map(|axis| (axis.to_owned(), 1)).collect(),
            )
        })
        .collect();
    assert_eq!(standard, expected);

    let high = axis_map(&load_profile("high-rigor"));
    let expected_high = high_rigor_axes()
        .into_iter()
        .map(|(gate, axes)| {
            (
                gate.to_owned(),
                axes.into_iter()
                    .map(|axis| {
                        let n = if gate == "design-review" || gate == "validation" {
                            2
                        } else {
                            1
                        };
                        (axis.to_owned(), n)
                    })
                    .collect(),
            )
        })
        .collect();
    assert_eq!(high, expected_high);
}

#[test]
fn every_axis_prompt_references_subject_template_schema_and_all_antipedantry_guards() {
    let subject_by_gate = BTreeMap::from([
        ("intent", ("intent.md", "intent.json")),
        ("design-review", ("design.md", "design.json")),
        ("plan-review", ("task-packet.md", "plan.json")),
        (
            "implementation-review",
            ("implementation-report.md", "implementation-report.json"),
        ),
        (
            "validation",
            ("validation-report.md", "validation-report.json"),
        ),
    ]);

    for profile in PROFILES {
        let config = load_profile(profile);
        for (gate, entries) in object(&config["review_policies"], "review_policies") {
            let (template, subject) = subject_by_gate[gate.as_str()];
            for entry in array(entries, gate) {
                let prompt = string(&object(entry, "policy entry")["example_prompt"], "prompt");
                assert!(!prompt.trim().is_empty(), "{profile}/{gate} empty prompt");
                assert!(
                    prompt.contains(&format!("data/templates/{template}")),
                    "{profile}/{gate} prompt missing template reference"
                );
                assert!(
                    prompt.contains(&format!(
                        "data/configs/{profile}.json#/artifact_schemas/{subject}"
                    )),
                    "{profile}/{gate} prompt missing schema reference"
                );
                for guard in PROMPT_GUARDS {
                    assert!(
                        prompt.contains(guard),
                        "{profile}/{gate} prompt missing anti-pedantry guard: {guard}"
                    );
                }
            }
        }
    }
}

#[test]
fn report_schemas_require_coverage_manifest() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let schemas = object(&config["artifact_schemas"], "artifact_schemas");
        for subject in ["implementation-report.json", "validation-report.json"] {
            let schema = object(&schemas[subject], subject);
            let required: BTreeSet<&str> = array(schema.get("required").unwrap(), "required")
                .iter()
                .map(|value| string(value, "required entry"))
                .collect();
            assert!(
                required.contains("coverage"),
                "{profile}/{subject} missing coverage"
            );
            let coverage = object(
                &object(schema.get("properties").unwrap(), "properties")["coverage"],
                "coverage",
            );
            let coverage_required: BTreeSet<&str> =
                array(coverage.get("required").unwrap(), "coverage.required")
                    .iter()
                    .map(|value| string(value, "coverage required entry"))
                    .collect();
            assert_eq!(coverage_required, BTreeSet::from(["commit", "documents"]));
        }
    }
}

#[test]
fn reviewer_protocol_defines_convergence_contract() {
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
        "validation-local `validation-report.json` corrections stay in validation",
        "retry checked `passed`",
        "in `validation`, use nearest `revise` only for an implementation-owned defect",
    ] {
        assert!(
            protocol.to_ascii_lowercase().contains(clause),
            "reviewer protocol missing convergence clause: {clause}"
        );
    }
    assert!(!protocol.contains(
        "why it was not visible in earlier supplied evidence or was introduced by an accepted fix"
    ));
}

#[test]
fn authoritative_docs_integrate_convergence_contract_and_routes() {
    let provider_prd = shipped_text("docs/prd.md");
    let provider_readme = shipped_text("README.md");
    let engine_prd = shipped_text("../../docs/PRD.md");
    for (name, text) in [
        ("provider PRD", provider_prd),
        ("provider README", provider_readme),
        ("engine PRD", engine_prd),
    ] {
        for clause in [
            "revise-intent",
            "revise-design",
            "revise-plan",
            "focused external reconsideration",
            "validation gap",
            "previously overlooked",
            "three-round",
            "validation-report-local",
            "retry checked `passed`",
            "implementation-owned",
        ] {
            assert!(
                text.to_ascii_lowercase().contains(clause),
                "{name} missing convergence clause: {clause}"
            );
        }
    }
}
