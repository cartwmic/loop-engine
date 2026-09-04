use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::Command;

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];
const PARENT_GATES: &[&str] = &[
    "intent-review",
    "design-review",
    "plan-review",
    "implementation-review",
    "validation-review",
];
const ADVERSARIAL_GATES: &[&str] = &[
    "intent-adversarial-review",
    "design-adversarial-review",
    "plan-adversarial-review",
    "implementation-adversarial-review",
    "validation-adversarial-review",
];
const RETIRED_GATES: &[&str] = &["intent", "validation"];

fn shipped_text(relative: &str) -> String {
    let path = workspace_integration::package_root("software-change-provider").join(relative);
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
    let path = workspace_integration::package_root("software-change-provider")
        .join(format!("data/configs/{profile}.json"))
        .to_string_lossy()
        .into_owned();
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
    with_counterparts(BTreeMap::from([
        (
            "intent-review",
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
            "validation-review",
            BTreeSet::from(["intent-delivered", "docs-integrated"]),
        ),
    ]))
}

fn high_rigor_axes() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    with_counterparts(BTreeMap::from([
        (
            "intent-review",
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
            "validation-review",
            BTreeSet::from([
                "intent-delivered",
                "docs-integrated",
                "requirement-proof-mapping",
            ]),
        ),
    ]))
}

fn with_counterparts(
    parents: BTreeMap<&'static str, BTreeSet<&'static str>>,
) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut out = BTreeMap::new();
    for (gate, axes) in &parents {
        out.insert(*gate, axes.clone());
        out.insert(adversarial_gate(gate), axes.clone());
    }
    out
}

fn adversarial_gate(parent: &str) -> &'static str {
    match parent {
        "intent-review" => "intent-adversarial-review",
        "design-review" => "design-adversarial-review",
        "plan-review" => "plan-adversarial-review",
        "implementation-review" => "implementation-adversarial-review",
        "validation-review" => "validation-adversarial-review",
        other => panic!("unexpected parent gate {other}"),
    }
}

fn parent_gate(adversarial: &str) -> &'static str {
    match adversarial {
        "intent-adversarial-review" => "intent-review",
        "design-adversarial-review" => "design-review",
        "plan-adversarial-review" => "plan-review",
        "implementation-adversarial-review" => "implementation-review",
        "validation-adversarial-review" => "validation-review",
        other => panic!("unexpected adversarial gate {other}"),
    }
}

fn describe_profile(config: &Value) -> Value {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    let request = json!({"operation": "describe", "initial_input": config});
    let output = super::bounded_process::run_with_stdin(
        &mut command,
        "software-change shipped-profile describe",
        &serde_json::to_vec(&request).expect("serialize describe request"),
    )
    .expect("software-change process should exit")
    .output;
    assert!(
        output.status.success(),
        "describe of shipped profile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("describe JSON")
}

fn state_ids(workflow: &Value) -> BTreeSet<String> {
    array(&workflow["states"], "states")
        .iter()
        .map(|state| string(&object(state, "state")["id"], "state id").to_owned())
        .collect()
}

fn axis_ids(entries: &Value, path: &str) -> BTreeSet<String> {
    array(entries, path)
        .iter()
        .map(|entry| string(&object(entry, "policy entry")["id"], "policy id").to_owned())
        .collect()
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
            "minimal" => "minimal-8",
            "standard" => "standard-8",
            "high-rigor" => "high-rigor-8",
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
fn every_profile_exposes_exact_closed_operating_context_schema() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let intent = object(
            &object(&config["artifact_schemas"], "artifact_schemas")["intent.json"],
            "intent.json",
        );
        let required: BTreeSet<&str> = array(intent.get("required").unwrap(), "intent.required")
            .iter()
            .map(|value| string(value, "intent required entry"))
            .collect();
        assert!(required.contains("operating_context"));

        let properties = object(intent.get("properties").unwrap(), "intent.properties");
        let context = object(
            properties.get("operating_context").unwrap(),
            "intent.operating_context",
        );
        assert_eq!(context.get("type").and_then(Value::as_str), Some("object"));
        assert_eq!(context["additionalProperties"], Value::Bool(false));
        let context_required: BTreeSet<&str> = array(
            context.get("required").unwrap(),
            "operating_context.required",
        )
        .iter()
        .map(|value| string(value, "operating_context required entry"))
        .collect();
        assert_eq!(
            context_required,
            BTreeSet::from([
                "operators",
                "environment",
                "threat_boundary",
                "accepted_risks",
                "outside_obligations",
            ])
        );
        let context_properties = object(
            context.get("properties").unwrap(),
            "operating_context.properties",
        );

        for field in ["operators", "environment"] {
            let schema = object(context_properties.get(field).unwrap(), field);
            assert_eq!(schema["type"].as_str(), Some("array"));
            assert_eq!(schema["minItems"].as_u64(), Some(1));
            let items = object(schema.get("items").unwrap(), &format!("{field}.items"));
            assert_eq!(items["type"].as_str(), Some("string"));
            assert_eq!(items["minLength"].as_u64(), Some(1));
        }

        let boundary = object(
            context_properties.get("threat_boundary").unwrap(),
            "threat_boundary",
        );
        assert_eq!(boundary["type"].as_str(), Some("object"));
        assert_eq!(boundary["additionalProperties"], Value::Bool(false));
        let boundary_properties = object(
            boundary.get("properties").unwrap(),
            "threat_boundary.properties",
        );
        for field in ["in_scope", "excluded"] {
            let schema = object(boundary_properties.get(field).unwrap(), field);
            assert_eq!(schema["type"].as_str(), Some("array"));
            assert_eq!(schema["minItems"].as_u64(), Some(1));
            let items = object(schema.get("items").unwrap(), &format!("{field}.items"));
            assert_eq!(items["type"].as_str(), Some("string"));
            assert_eq!(items["minLength"].as_u64(), Some(1));
        }

        for (field, item_fields) in [
            ("accepted_risks", ["risk", "rationale"]),
            ("outside_obligations", ["source", "obligation"]),
        ] {
            let schema = object(context_properties.get(field).unwrap(), field);
            assert_eq!(schema["type"].as_str(), Some("array"));
            assert_eq!(schema["minItems"].as_u64(), Some(0));
            let item = object(schema.get("items").unwrap(), &format!("{field}.items"));
            assert_eq!(item["type"].as_str(), Some("object"));
            assert_eq!(item["additionalProperties"], Value::Bool(false));
            let required: BTreeSet<&str> = array(
                item.get("required").unwrap(),
                &format!("{field}.items.required"),
            )
            .iter()
            .map(|value| string(value, "item required entry"))
            .collect();
            assert_eq!(required, BTreeSet::from(item_fields));
            let item_properties = object(item.get("properties").unwrap(), "item.properties");
            for item_field in item_fields {
                let property = object(item_properties.get(item_field).unwrap(), item_field);
                assert_eq!(property["type"].as_str(), Some("string"));
                assert_eq!(property["minLength"].as_u64(), Some(1));
            }
        }
    }
}

#[test]
fn shipped_review_prompts_and_calibration_fixtures_expose_the_new_boundaries() {
    let required_prompt_terms = [
        "operating_context",
        "threat_boundary",
        "accepted_risks",
        "outside_obligations",
        "hostile-user",
        "multi-tenant",
        "never waives",
    ];
    for profile in PROFILES {
        let config = load_profile(profile);
        for entries in object(&config["review_policies"], "review_policies").values() {
            for policy in array(entries, "review policy list") {
                let prompt = string(&object(policy, "policy")["example_prompt"], "prompt");
                for term in required_prompt_terms {
                    assert!(prompt.contains(term), "{profile} prompt omitted {term}");
                }
            }
        }
        for policy in config["review_policies"]["plan-review"]
            .as_array()
            .unwrap_or(&Vec::new())
        {
            let policy_object = object(policy, "plan policy");
            let prompt = string(&policy_object["example_prompt"], "prompt");
            match string(&policy_object["id"], "plan policy id") {
                "task-sized" => {
                    assert!(prompt.contains("user or operator"));
                    assert!(prompt.contains("vague task packet"));
                }
                "done-observable" => {
                    assert!(prompt.contains("black-box"));
                    assert!(prompt.contains("impractical"));
                }
                "decision-free" => assert!(prompt.contains("replaceable mechanism")),
                _ => {}
            }
        }
        for policy in config["review_policies"]["validation-review"]
            .as_array()
            .unwrap_or(&Vec::new())
        {
            let prompt = string(
                &object(policy, "validation policy")["example_prompt"],
                "prompt",
            );
            assert!(prompt.contains("activity-only"));
            assert!(prompt.contains("Bookends"));
            assert!(prompt.contains("requirement token"));
        }
    }

    let intent_good: Value =
        serde_json::from_str(&shipped_text("data/calibration/fixtures/intent-good.json")).unwrap();
    assert!(!intent_good["operating_context"]["accepted_risks"]
        .as_array()
        .unwrap()
        .is_empty());
    let intent_invalid_waiver: Value = serde_json::from_str(&shipped_text(
        "data/calibration/fixtures/intent-defective.json",
    ))
    .unwrap();
    assert!(
        intent_invalid_waiver["operating_context"]["accepted_risks"][0]["rationale"]
            .as_str()
            .unwrap()
            .contains("reviewers do not need")
    );
    let plan: Value =
        serde_json::from_str(&shipped_text("data/calibration/fixtures/plan-good.json")).unwrap();
    assert!(plan["objective"].as_str().unwrap().contains("black-box"));
    assert!(plan["objective"]
        .as_str()
        .unwrap()
        .contains("impracticality"));
    let overbuilt: Value = serde_json::from_str(&shipped_text(
        "data/calibration/fixtures/design-overbuilt.json",
    ))
    .unwrap();
    assert!(overbuilt
        .to_string()
        .contains("speculative plugin registry"));
    let validation: Value = serde_json::from_str(&shipped_text(
        "data/calibration/fixtures/validation-report-good.json",
    ))
    .unwrap();
    assert!(validation.to_string().contains("semantically prove"));
    assert!(validation.to_string().contains("bookends:LE-39"));
    let activity_only: Value = serde_json::from_str(&shipped_text(
        "data/calibration/fixtures/validation-report-defective.json",
    ))
    .unwrap();
    assert_eq!(activity_only["requirements"][0]["proof"], "bookends:LE-39");
}

#[test]
fn shipped_work_slot_bindings_are_unbound() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let bindings = match config.get("work_slot_bindings") {
            None => continue,
            Some(Value::Object(map)) if map.is_empty() => continue,
            Some(Value::Object(map)) => map,
            Some(other) => {
                panic!("{profile} work_slot_bindings must be omitted or an object, got {other}")
            }
        };
        for slot in [
            "implement",
            "design-review",
            "plan-review",
            "implementation-review",
            "validation-draft",
        ] {
            assert!(
                !bindings.contains_key(slot),
                "{profile} must leave {slot} unbound"
            );
        }
        assert!(
            bindings.is_empty(),
            "{profile} work_slot_bindings must be omitted or empty, got keys {:?}",
            bindings.keys().collect::<Vec<_>>()
        );
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
            ("intent-review".into(), BTreeMap::new()),
            ("plan-review".into(), BTreeMap::new()),
            (
                "validation-review".into(),
                BTreeMap::from([("intent-delivered".into(), 1)])
            ),
        ])
    );
    for gate in ADVERSARIAL_GATES {
        assert!(
            !minimal.contains_key(*gate),
            "minimal must omit adversarial list {gate}"
        );
    }
    for gate in RETIRED_GATES {
        assert!(
            !minimal.contains_key(*gate),
            "minimal still has retired gate {gate}"
        );
    }

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
    for gate in RETIRED_GATES {
        assert!(
            !standard.contains_key(*gate),
            "standard still has retired gate {gate}"
        );
    }

    let high = axis_map(&load_profile("high-rigor"));
    let expected_high = high_rigor_axes()
        .into_iter()
        .map(|(gate, axes)| {
            (
                gate.to_owned(),
                axes.into_iter()
                    .map(|axis| {
                        let n = if gate == "design-review" || gate == "validation-review" {
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
    for gate in RETIRED_GATES {
        assert!(
            !high.contains_key(*gate),
            "high-rigor still has retired gate {gate}"
        );
    }
    for axis in [
        "intent-faithful",
        "intent-delivered",
        "requirement-proof-mapping",
    ] {
        let parent_gate = if axis == "intent-faithful" {
            "design-review"
        } else {
            "validation-review"
        };
        assert_eq!(
            high[parent_gate][axis], 2,
            "parent required_authors on high-rigor {parent_gate}/{axis} must remain 2"
        );
        assert_eq!(
            high[adversarial_gate(parent_gate)][axis],
            1,
            "adversarial required_authors on high-rigor {parent_gate} counterpart {axis} must be 1"
        );
    }
}

#[test]
fn every_axis_prompt_references_subject_template_schema_and_all_antipedantry_guards() {
    let subject_by_gate = BTreeMap::from([
        ("intent-review", ("intent.md", "intent.json")),
        ("intent-adversarial-review", ("intent.md", "intent.json")),
        ("design-review", ("design.md", "design.json")),
        ("design-adversarial-review", ("design.md", "design.json")),
        ("plan-review", ("task-packet.md", "plan.json")),
        ("plan-adversarial-review", ("task-packet.md", "plan.json")),
        (
            "implementation-review",
            ("implementation-report.md", "implementation-report.json"),
        ),
        (
            "implementation-adversarial-review",
            ("implementation-report.md", "implementation-report.json"),
        ),
        (
            "validation-review",
            ("validation-report.md", "validation-report.json"),
        ),
        (
            "validation-adversarial-review",
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
                if gate.contains("adversarial") {
                    let id = string(&object(entry, "policy entry")["id"], "policy id");
                    assert!(
                        prompt.starts_with(&format!("Falsify {id} only.")),
                        "{profile}/{gate}/{id} adversarial prompt must be falsifier stance"
                    );
                    assert!(
                        prompt.contains("pass claim against that named obligation"),
                        "{profile}/{gate}/{id} adversarial prompt must attack the named parent obligation"
                    );
                    assert_eq!(
                        object(entry, "policy entry")
                            .get("required_authors")
                            .and_then(Value::as_u64),
                        Some(1),
                        "{profile}/{gate}/{id} must set required_authors 1"
                    );
                }
            }
        }
    }
}

#[test]
fn shipped_profiles_describe_live_graph_and_keep_one_to_one_counterparts() {
    for profile in PROFILES {
        let config = load_profile(profile);
        let policies = object(&config["review_policies"], "review_policies");
        for retired in RETIRED_GATES {
            assert!(
                !policies.contains_key(*retired),
                "{profile} still ships retired gate {retired}"
            );
        }

        let workflow = describe_profile(&config);
        let states = state_ids(&workflow);
        for state in array(&workflow["states"], "describe.states") {
            if state["id"] != "end" {
                let instructions = string(
                    &object(state, "describe state")["instructions"],
                    "state instructions",
                );
                assert!(
                    instructions.contains("operating_context"),
                    "{profile} describe instructions omitted frozen operating_context"
                );
            }
        }

        for parent in PARENT_GATES {
            let parent_ids = policies
                .get(*parent)
                .map(|entries| axis_ids(entries, parent))
                .unwrap_or_default();
            let parent_live = !parent_ids.is_empty();
            assert_eq!(
                states.contains(*parent),
                parent_live,
                "{profile} describe live state {parent}: expected {parent_live}, states={states:?}"
            );

            let adversarial = adversarial_gate(parent);
            let adversarial_ids = policies
                .get(adversarial)
                .map(|entries| axis_ids(entries, adversarial))
                .unwrap_or_default();

            if *profile == "minimal" {
                assert!(
                    !policies.contains_key(adversarial),
                    "minimal must omit adversarial list {adversarial}"
                );
                assert!(
                    !states.contains(adversarial),
                    "minimal describe must omit adversarial state {adversarial}"
                );
                continue;
            }

            assert!(
                policies.contains_key(adversarial),
                "{profile} missing adversarial key {adversarial} for parent {parent}"
            );
            assert_eq!(
                adversarial_ids, parent_ids,
                "{profile} {adversarial} axis id set must equal {parent} (1:1, same policy_id)"
            );
            assert_eq!(
                states.contains(adversarial),
                parent_live,
                "{profile} describe must include {adversarial} iff {parent} is nonempty"
            );

            if let Some(entries) = policies.get(adversarial) {
                for entry in array(entries, adversarial) {
                    let entry = object(entry, "policy entry");
                    assert_eq!(
                        entry.get("required_authors").and_then(Value::as_u64),
                        Some(1),
                        "{profile}/{adversarial}/{} required_authors must be 1",
                        string(&entry["id"], "policy id")
                    );
                }
            }
        }

        for adversarial in ADVERSARIAL_GATES {
            if policies.contains_key(*adversarial) {
                let parent = parent_gate(adversarial);
                let parent_ids = policies
                    .get(parent)
                    .map(|entries| axis_ids(entries, parent))
                    .unwrap_or_default();
                let adversarial_ids = axis_ids(&policies[*adversarial], adversarial);
                assert_eq!(
                    adversarial_ids, parent_ids,
                    "{profile} shipped subset of counterparts on {adversarial}"
                );
            } else {
                assert_eq!(
                    *profile, "minimal",
                    "{profile} omitted adversarial key {adversarial}"
                );
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
        "quiet, progress, and thrash count per review state on the post-triage accepted-finding set",
        "confirmation consumes the durable ledger set and does not search again except for fix-introduced holes",
        "extra mechanism, unlisted requirements, and hypothetical-future fails are not appended",
        "bound workers do not use previously overlooked",
        "humans still may with full failure burden",
        "known accepted material defects are never waived",
        "operating_context",
        "threat_boundary",
        "accepted_risks",
        "outside_obligations",
        "hostile-user",
        "multi-tenant",
        "never waives",
        "adversarial output is candidate data",
        "no unresolved accepted in-scope material finding",
        "zero advisory comments is not required",
        "validation-local `validation-report.json` corrections stay in validation",
        "retry the next checked hop",
        "`revise-implementation`",
        "evidence-applicability",
        "selected-assignment-output",
        "context-record",
        "engine-resolved",
        "current target",
    ] {
        assert!(
            protocol.to_ascii_lowercase().contains(clause),
            "reviewer protocol missing convergence clause: {clause}"
        );
    }
    for forbidden in [
        "three-round",
        "circuit breaker",
        "verdict cap",
        "loop_engine_carry",
        "originating_output",
        "external-artifact",
        "repository_state",
        "unchanged-carry",
        "override-carry",
    ] {
        assert!(
            !protocol.to_ascii_lowercase().contains(forbidden),
            "reviewer protocol retained retired linkage language: {forbidden}"
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
        ("provider PRD", provider_prd.as_str()),
        ("provider README", provider_readme.as_str()),
        ("engine PRD", engine_prd.as_str()),
    ] {
        for clause in [
            "revise-intent",
            "revise-design",
            "revise-plan",
            "focused external reconsideration",
            "validation gap",
            "previously overlooked",
            "validation-report-local",
            "retry the next checked hop",
            "revise-implementation",
            "quiet, progress, and thrash",
            "intent-draft",
            "validation-draft",
        ] {
            assert!(
                text.to_ascii_lowercase().contains(clause),
                "{name} missing convergence clause: {clause}"
            );
        }
    }
    for (name, text) in [
        ("provider README", provider_readme.as_str()),
        ("engine PRD", engine_prd.as_str()),
    ] {
        for forbidden in ["three-round", "circuit breaker", "verdict cap"] {
            assert!(
                !text.to_ascii_lowercase().contains(forbidden),
                "{name} retained breaker-as-verdict language: {forbidden}"
            );
        }
    }
    let agents = shipped_text("AGENTS.md");
    assert!(
        !agents.contains(
            "explore → design → design-review → plan → plan-review → implement → implementation-review → validation → end"
        ),
        "AGENTS.md retained the retired nine-state topology"
    );
    let agents_lower = agents.to_ascii_lowercase();
    for clause in [
        "sixteen-state",
        "intent-review",
        "validation-review",
        "revise-implementation",
        "intent-draft",
        "validation-draft",
        "quiet, progress, and thrash",
    ] {
        assert!(
            agents_lower.contains(clause),
            "AGENTS.md missing live-graph clause: {clause}"
        );
    }
}

#[test]
fn finding_ledger_template_matches_well_formed_shape() {
    let envelope: Value = serde_json::from_str(&shipped_text("data/templates/finding-ledger.json"))
        .expect("finding-ledger template must be JSON");
    assert_eq!(envelope["kind"].as_str(), Some("finding-ledger"));
    let data = object(&envelope["data"], "finding-ledger.data");
    assert_eq!(string(&data["schema_version"], "schema_version"), "1");
    for key in ["gate", "subject", "subject_revision"] {
        let value = string(&data[key], key);
        assert!(!value.is_empty(), "finding-ledger.{key} must be nonempty");
    }
    let author = object(&data["author"], "author");
    assert!(!string(&author["name"], "author.name").is_empty());
    assert!(matches!(
        string(&author["kind"], "author.kind"),
        "human" | "agent"
    ));
    assert!(data.get("repository_state").is_none());
    let findings = array(&data["findings"], "findings");
    for (index, item) in findings.iter().enumerate() {
        let object = object(item, &format!("findings[{index}]"));
        for key in [
            "id",
            "source",
            "policy_id",
            "statement",
            "disposition",
            "reason",
            "owner_phase",
            "task_ids",
            "review_axes",
            "status",
        ] {
            assert!(object.get(key).is_some(), "findings[{index}] missing {key}");
        }
        assert_eq!(object["source"]["kind"], "context-record");
        assert!(object["source"]["id"].as_str().is_some());
    }
}

#[test]
fn advisory_finding_proposal_is_a_closed_inert_context_template() {
    let envelope: Value = serde_json::from_str(&shipped_text(
        "data/templates/advisory-finding-proposal.json",
    ))
    .expect("advisory proposal must be JSON");
    assert_eq!(envelope["kind"].as_str(), Some("advisory-finding-proposal"));
    let data = object(&envelope["data"], "advisory-finding-proposal.data");
    assert_eq!(data["schema_version"].as_str(), Some("1"));
    let proposals = array(&data["proposals"], "proposals");
    assert!(!proposals.is_empty());
    for (index, proposal) in proposals.iter().enumerate() {
        let proposal = object(proposal, &format!("proposals[{index}]"));
        for key in [
            "candidate_source_ids",
            "proposed_disposition",
            "proposed_reason",
            "proposed_owner_phase",
            "proposed_task_ids",
            "proposed_review_axes",
            "rationale",
        ] {
            assert!(proposal.get(key).is_some(), "proposal missing {key}");
        }
    }
}

#[test]
fn review_worker_preamble_carries_yagni_bar_and_confirmation_stdin_rule() {
    let preamble = shipped_text("data/review-worker-preamble.txt").to_ascii_lowercase();
    for clause in [
        "extra mechanism",
        "unlisted requirements",
        "hypothetical-future",
        "confirmation consumes the durable finding-ledger history",
        "does not search again except for fix-introduced holes",
        "bound workers do not use previously overlooked",
        "operating_context",
        "threat_boundary",
        "outside_obligations",
        "review_axes",
        "never waive a stated outcome",
    ] {
        assert!(
            preamble.contains(clause),
            "review-worker preamble missing {clause}"
        );
    }
}

fn skill_constructor_jq() -> String {
    let skill = shipped_text("skills/using-software-change-provider/SKILL.md");
    let anchor = "--slurpfile roster \"$ROSTER\" '";
    let start = skill.find(anchor).expect("constructor must slurp ROSTER") + anchor.len();
    let rest = skill[start..].strip_prefix('\n').unwrap_or(&skill[start..]);
    let end = rest
        .find("' \"$PROFILE\"")
        .expect("constructor jq must end before PROFILE");
    rest[..end].to_string()
}

fn run_review_constructor(
    profile: &std::path::Path,
    slot: &str,
    roster: &std::path::Path,
) -> Result<Value, String> {
    let crate_dir = workspace_integration::package_root("software-change-provider");
    let output = Command::new("jq")
        .args([
            "--arg",
            "slot",
            slot,
            "--arg",
            "engine",
            "/tmp/loop-engine-constructor-proof",
            "--arg",
            "pi",
            "/tmp/pi-constructor-proof",
            "--arg",
            "cursor",
            "/tmp/cursor-provider-extension",
            "--arg",
            "bridge",
            "/tmp/claude-bridge-extension",
            "--rawfile",
            "base_preamble",
            crate_dir
                .join("data/review-worker-preamble.txt")
                .to_str()
                .expect("preamble path utf-8"),
            "--slurpfile",
            "output_schema",
            crate_dir
                .join("data/review-worker-output-schema.json")
                .to_str()
                .expect("schema path utf-8"),
            "--slurpfile",
            "roster",
            roster.to_str().expect("roster path utf-8"),
        ])
        .arg(skill_constructor_jq())
        .arg(profile)
        .output()
        .expect("jq should spawn");
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

#[test]
fn constructor_rejects_draft_slots_and_accepts_intent_and_adversarial_review() {
    let crate_dir = workspace_integration::package_root("software-change-provider");
    let profile = crate_dir.join("data/configs/high-rigor.json");
    let temp = std::env::temp_dir().join(format!(
        "software-change-constructor-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp).expect("constructor temp dir");
    let roster = temp.join("roster.json");
    fs::write(
        &roster,
        r#"[{"author":"reviewer-a","model":"model-a"},{"author":"reviewer-b","model":"model-b"}]"#,
    )
    .expect("write roster");

    let intent = run_review_constructor(&profile, "intent-review", &roster)
        .expect("intent-review must be a live constructor slot");
    let intent_bindings = object(
        &intent["work_slot_bindings"],
        "intent-review work_slot_bindings",
    );
    assert!(
        intent_bindings.contains_key("intent-review"),
        "intent-review constructor omitted the review binding"
    );
    for draft in [
        "intent-draft",
        "design-draft",
        "plan-draft",
        "implement",
        "validation-draft",
    ] {
        assert!(
            !intent_bindings.contains_key(draft),
            "constructor emitted draft binding {draft}"
        );
    }
    assert!(
        !intent_bindings.contains_key("intent-adversarial-review"),
        "same-slot mixed parent and adversarial fan-out is not the enabled path"
    );
    let binding_args = array(
        &intent_bindings["intent-review"]["args"],
        "intent-review constructor args",
    );
    assert_eq!(binding_args[0], "fan-out");
    let expected_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"type": "string", "minLength": 1},
            "author": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                }
            },
            "result": {"type": "string", "enum": ["pass", "fail"]},
            "findings": {"type": "string"}
        },
        "oneOf": [
            {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
            {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}}
        ]
    });
    let high_rigor = load_profile("high-rigor");
    let intent_policies = array(
        &object(&high_rigor["review_policies"], "review_policies")["intent-review"],
        "intent-review policies",
    );
    for (worker_index, worker_pair) in binding_args[1..].as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(worker_pair[0], "--worker");
        let worker: Value = serde_json::from_str(string(&worker_pair[1], "constructor worker"))
            .expect("constructor worker must be JSON");
        assert!(
            worker.get("output_schema").is_none(),
            "constructor must not emit legacy output_schema: {worker}"
        );
        let mut expected_worker_schema = expected_schema.clone();
        expected_worker_schema["properties"]["axis"]["const"] = json!(string(
            &object(&intent_policies[worker_index], "intent policy")["id"],
            "policy id"
        ));
        expected_worker_schema["properties"]["author"]["const"] =
            json!({"name": "reviewer-a", "kind": "agent"});
        assert_eq!(
            worker["full_output_schema"], expected_worker_schema,
            "constructor must emit assignment-specific axis and author constants"
        );
    }

    let adversarial = run_review_constructor(&profile, "design-adversarial-review", &roster)
        .expect("design-adversarial-review must be a live constructor slot");
    let adversarial_bindings = object(
        &adversarial["work_slot_bindings"],
        "adversarial work_slot_bindings",
    );
    assert!(
        adversarial_bindings.contains_key("design-adversarial-review"),
        "adversarial constructor omitted the review binding"
    );
    assert!(
        !adversarial_bindings.contains_key("design-review"),
        "adversarial constructor mixed parent fan-out into the adversarial slot"
    );

    for draft in [
        "intent-draft",
        "design-draft",
        "plan-draft",
        "implement",
        "validation-draft",
    ] {
        let error = run_review_constructor(&profile, draft, &roster)
            .expect_err(&format!("{draft} must be rejected"));
        assert!(
            error.contains("constructor does not emit draft bindings"),
            "{draft} failed for the wrong reason: {error}"
        );
    }
    let _ = fs::remove_dir_all(&temp);
}
