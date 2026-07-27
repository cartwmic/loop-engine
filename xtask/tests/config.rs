use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;
use tempfile::TempDir;
use xtask::config::{
    Phase, Scope, SemanticRequirement, compute_binding, load_manifest, parse_manifest,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config")
        .join(name)
}

fn parse(
    text: &str,
    requirement: SemanticRequirement,
) -> anyhow::Result<xtask::config::ManifestDocument> {
    parse_manifest(text.as_bytes(), requirement)
}

fn minimal() -> String {
    fs::read_to_string(fixture("minimal.toml")).expect("minimal fixture")
}

fn assert_rejected(text: &str, needle: &str) {
    let error = parse(text, SemanticRequirement::Optional).expect_err("manifest must fail closed");
    assert!(
        format!("{error:#}").contains(needle),
        "expected `{needle}` in error, got: {error:#}"
    );
}

#[test]
fn minimal_manifest_parses_into_typed_immutable_configuration() {
    let document = load_manifest(&fixture("minimal.toml"), SemanticRequirement::Optional)
        .expect("minimal manifest");
    let manifest = document.manifest();

    assert_eq!(manifest.schema_version(), 2);
    assert_eq!(manifest.defaults().timeout_seconds(), 30);
    assert_eq!(manifest.defaults().max_output_bytes(), 4096);
    assert_eq!(
        manifest.runner().inputs(),
        &[Path::new("quality/manifest.toml")]
    );
    assert!(manifest.prerequisites().is_empty());
    assert!(manifest.semantic().is_none());

    let check = &manifest.checks()[0];
    assert_eq!(check.id(), "test");
    assert_eq!(check.phases(), &[Phase::PreCommit]);
    assert_eq!(check.scope(), Scope::Repository);
    assert_eq!(check.program(), "cargo");
    assert_eq!(check.args(), ["test", "--workspace", "--locked"]);
    assert_eq!(check.cwd(), "{candidate_root}");
    assert_eq!(check.timeout_seconds(), 30);
    assert_eq!(check.max_output_bytes(), 4096);
}

#[test]
fn full_manifest_parses_every_contract_field() {
    let document =
        load_manifest(&fixture("full.toml"), SemanticRequirement::Required).expect("full manifest");
    let manifest = document.manifest();

    assert_eq!(manifest.prerequisites().len(), 1);
    assert_eq!(manifest.checks().len(), 2);
    assert_eq!(
        manifest.checks()[0].phases(),
        &[Phase::PreCommit, Phase::Publication]
    );
    assert_eq!(manifest.checks()[0].scope(), Scope::ChangedFiles);
    assert_eq!(manifest.checks()[1].timeout_seconds(), 600);
    assert_eq!(manifest.checks()[1].max_output_bytes(), 1_048_576);
    assert_eq!(
        manifest.checks()[1]
            .environment()
            .set()
            .get("CHECK_BINDING")
            .map(String::as_str),
        Some("{candidate_tree}:{cache_root}")
    );

    let semantic = manifest.semantic().expect("semantic config");
    assert_eq!(semantic.axes().len(), 2);
    assert_eq!(semantic.coherence().id(), "coherence");
    assert_eq!(semantic.timeout_seconds(), 900);
    assert_eq!(semantic.max_output_bytes(), 8_388_608);
    assert_eq!(
        semantic.response_schema(),
        Path::new("quality/semantic-judge/v2/response.schema.json")
    );
}

#[test]
fn semantic_requirement_is_explicit() {
    parse(&minimal(), SemanticRequirement::Optional).expect("deterministic-only manifest");
    let error = parse(&minimal(), SemanticRequirement::Required)
        .expect_err("publication/advisory needs semantic config");
    assert!(
        error
            .to_string()
            .contains("semantic configuration is required")
    );
}

#[test]
fn exact_manifest_and_rubric_bytes_have_stable_path_sorted_digests() {
    let candidate = TempDir::new().expect("candidate");
    for (path, bytes) in [
        ("quality/rubrics/z-last.md", b"z\r\n".as_slice()),
        ("quality/rubrics/a-first.md", b"a\n".as_slice()),
        ("quality/rubrics/coherence.md", b"coherence\n".as_slice()),
    ] {
        let destination = candidate.path().join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }

    let document = load_manifest(&fixture("full.toml"), SemanticRequirement::Required).unwrap();
    let binding = compute_binding(&document, candidate.path()).expect("binding digests");

    assert_eq!(
        binding.manifest_digest(),
        "ed7d1a184d23e7a072beb2c7408f80cd1de6ae9a12d322a2f465db24deb78d3f"
    );
    let paths: Vec<_> = binding
        .rubric_digests()
        .keys()
        .map(PathBuf::as_path)
        .collect();
    assert_eq!(
        paths,
        [
            Path::new("quality/rubrics/a-first.md"),
            Path::new("quality/rubrics/coherence.md"),
            Path::new("quality/rubrics/z-last.md"),
        ]
    );
    assert_eq!(
        binding.rubric_digests()[Path::new("quality/rubrics/a-first.md")],
        "87428fc522803d31065e7bce3cf03fe475096631e5e07bbd7a0fde60c4cf25c7"
    );
    assert_eq!(
        binding.rubric_digests()[Path::new("quality/rubrics/z-last.md")],
        "ae7c791a527d756a15cdb11de8c13feefd266a81ea9671d3e65f1e7da00e2e5e"
    );
}

#[cfg(unix)]
#[test]
fn external_rubric_symlink_escape_is_rejected() {
    let candidate = TempDir::new().expect("candidate");
    let outside = TempDir::new().expect("outside");
    let rubrics = candidate.path().join("quality/rubrics");
    fs::create_dir_all(&rubrics).unwrap();
    fs::write(rubrics.join("z-last.md"), b"z\n").unwrap();
    fs::write(rubrics.join("coherence.md"), b"coherence\n").unwrap();
    let outside_rubric = outside.path().join("a-first.md");
    fs::write(&outside_rubric, b"outside\n").unwrap();
    symlink(&outside_rubric, rubrics.join("a-first.md")).unwrap();

    let document = load_manifest(&fixture("full.toml"), SemanticRequirement::Required).unwrap();
    let error = compute_binding(&document, candidate.path()).expect_err("symlink must not escape");

    assert!(
        format!("{error:#}").contains("configured rubric escapes candidate root"),
        "unexpected error: {error:#}"
    );
}

#[cfg(unix)]
#[test]
fn internal_rubric_symlink_is_read_successfully() {
    let candidate = TempDir::new().expect("candidate");
    let rubrics = candidate.path().join("quality/rubrics");
    fs::create_dir_all(&rubrics).unwrap();
    fs::write(rubrics.join("inside.md"), b"a\n").unwrap();
    fs::write(rubrics.join("z-last.md"), b"z\n").unwrap();
    fs::write(rubrics.join("coherence.md"), b"coherence\n").unwrap();
    symlink("inside.md", rubrics.join("a-first.md")).unwrap();

    let document = load_manifest(&fixture("full.toml"), SemanticRequirement::Required).unwrap();
    let binding = compute_binding(&document, candidate.path()).expect("internal symlink binding");

    assert_eq!(binding.rubric_digests().len(), 3);
    assert_eq!(
        binding.rubric_digests()[Path::new("quality/rubrics/a-first.md")],
        "87428fc522803d31065e7bce3cf03fe475096631e5e07bbd7a0fde60c4cf25c7"
    );
}

#[test]
fn one_exact_byte_change_changes_manifest_digest() {
    let original = fs::read(fixture("minimal.toml")).unwrap();
    let mut changed = original.clone();
    changed.push(b'\n');
    let original = parse_manifest(&original, SemanticRequirement::Optional).unwrap();
    let changed = parse_manifest(&changed, SemanticRequirement::Optional).unwrap();
    let candidate = TempDir::new().unwrap();

    assert_ne!(
        compute_binding(&original, candidate.path())
            .unwrap()
            .manifest_digest(),
        compute_binding(&changed, candidate.path())
            .unwrap()
            .manifest_digest()
    );
}

#[test]
fn unknown_keys_and_enum_values_fail_closed() {
    assert_rejected(
        &minimal().replace("schema_version = 2", "schema_version = 2\nunknown = true"),
        "unknown",
    );
    assert_rejected(&minimal().replace("pre-commit", "commit-ish"), "commit-ish");
    assert_rejected(&minimal().replace("repository", "workspace"), "workspace");
    assert_rejected(
        &minimal().replace("schema_version = 2", "schema_version = 1"),
        "expected 2",
    );
}

#[test]
fn toml_duplicate_keys_and_tables_fail_closed() {
    let duplicate_key = minimal().replace(
        "schema_version = 2",
        "schema_version = 2\nschema_version = 2",
    );
    assert_rejected(&duplicate_key, "invalid schema-v2 quality manifest TOML");

    let duplicate_table = format!(
        "{}\n[runner]\ninputs = [\"quality/other.toml\"]\n",
        minimal()
    );
    assert_rejected(&duplicate_table, "invalid schema-v2 quality manifest TOML");
}

#[test]
fn duplicate_and_empty_ids_fail_closed() {
    let duplicate_check = format!(
        "{}\n[[checks]]\nid = \"test\"\nphases = [\"publication\"]\nscope = \"repository\"\nprogram = \"cargo\"\nargs = []\ncwd = \"{{candidate_root}}\"\n",
        minimal()
    );
    assert_rejected(&duplicate_check, "duplicate check id");
    assert_rejected(
        &minimal().replace("id = \"test\"", "id = \"\""),
        "non-empty",
    );

    let duplicate_phase = minimal().replace(
        "phases = [\"pre-commit\"]",
        "phases = [\"pre-commit\", \"pre-commit\"]",
    );
    assert_rejected(&duplicate_phase, "duplicate phase");
}

#[test]
fn invalid_bounds_environment_and_placeholders_fail_closed() {
    assert_rejected(
        &minimal().replace("timeout_seconds = 30", "timeout_seconds = 0"),
        "positive",
    );
    assert_rejected(
        &minimal().replace("max_output_bytes = 4096", "max_output_bytes = 0"),
        "positive",
    );
    assert_rejected(
        &minimal().replace("program = \"cargo\"", "program = \"cargo-{unknown}\""),
        "unknown placeholder",
    );
    assert_rejected(
        &minimal().replace("program = \"cargo\"", "program = \"cargo-{candidate_root\""),
        "unclosed placeholder",
    );

    let multiline_probe = minimal().replace(
        "[runner]",
        "[[prerequisites]]\nid = \"probe\"\nprogram = \"tool\"\nstdout_equals = \"first\\nsecond\"\ninstall_hint = \"install tool\"\n\n[runner]",
    );
    assert_rejected(&multiline_probe, "stdout_equals must be one line");

    let bad_environment = minimal().replace(
        "[runner]",
        "[defaults.environment.set]\n\"BAD=KEY\" = \"value\"\n\n[runner]",
    );
    assert_rejected(&bad_environment, "environment variable name");
}

#[test]
fn repository_paths_and_candidate_cwds_cannot_escape() {
    assert_rejected(
        &minimal().replace("quality/manifest.toml", "../quality/manifest.toml"),
        "runner input",
    );
    assert_rejected(
        &minimal().replace("quality/manifest.toml", "quality//manifest.toml"),
        "runner input",
    );
    assert_rejected(
        &minimal().replace(
            "cwd = \"{candidate_root}\"",
            "cwd = \"{candidate_root}/../real\"",
        ),
        "cwd",
    );
    assert_rejected(
        &minimal().replace("cwd = \"{candidate_root}\"", "cwd = \"/tmp\""),
        "cwd",
    );
}

#[test]
fn schema_and_parser_align_on_candidate_cwd_shape() {
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../quality/validation/v2/manifest.schema.json");
    let schema: serde_json::Value =
        serde_json::from_slice(&fs::read(schema_path).unwrap()).unwrap();
    let expected_pattern = r"^\{candidate_root\}(?:/(?!\.{1,2}(?:/|$))[^/{}]+)*$";
    assert_eq!(
        schema["$defs"]["check"]["properties"]["cwd"],
        serde_json::json!({
            "type": "string",
            "pattern": expected_pattern,
        })
    );
    assert_eq!(
        schema["$defs"]["semantic"]["properties"]["cwd"],
        serde_json::json!({
            "type": "string",
            "pattern": expected_pattern,
        })
    );

    for cwd in [
        "{candidate_root}",
        "{candidate_root}/nested/path",
        "{candidate_root}/.hidden/...",
    ] {
        parse(
            &minimal().replace("cwd = \"{candidate_root}\"", &format!("cwd = \"{cwd}\"")),
            SemanticRequirement::Optional,
        )
        .unwrap_or_else(|error| panic!("parser rejected schema-valid cwd `{cwd}`: {error:#}"));
    }

    for cwd in [
        "{candidate_root}/../escape",
        "{candidate_root}/./nested",
        "{candidate_root}//nested",
        "{candidate_root}/nested/",
        "{candidate_root}/nested/{cache_root}",
        "{candidate_root}/nested/{literal",
        "{candidate_root}/nested/literal}",
        "{candidate_root}{cache_root}",
    ] {
        let text = minimal().replace("cwd = \"{candidate_root}\"", &format!("cwd = \"{cwd}\""));
        let error = match parse(&text, SemanticRequirement::Optional) {
            Ok(_) => panic!("parser accepted schema-invalid cwd `{cwd}`"),
            Err(error) => error,
        };
        assert!(
            format!("{error:#}").contains("cwd"),
            "cwd `{cwd}` failed without cwd context: {error:#}"
        );
    }
}

#[test]
fn tracked_schema_closes_unknown_fields_and_uses_toml_integer_limit() {
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../quality/validation/v2/manifest.schema.json");
    let schema: serde_json::Value =
        serde_json::from_slice(&fs::read(schema_path).unwrap()).unwrap();
    assert_eq!(schema["properties"]["schema_version"]["const"], 2);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["positiveInteger"]["maximum"],
        9_223_372_036_854_775_807_i64
    );
}
