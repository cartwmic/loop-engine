use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use loop_engine_core::model::ids::RegistrationId;
use loop_engine_integrations::configuration::{
    CliDefaults, EnvironmentPaths, MachinePaths, OutputFormat, TargetProviderRequirement,
    discover_project_config, load_optional, provider_for_existing_run, provider_for_new_target,
    resolve_defaults,
};

#[test]
fn home_override_resolves_only_final_symlink_component() {
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let environment = EnvironmentPaths {
        home: Some(OsString::from("/home/test")),
        loop_engine_home: Some(link.into_os_string()),
        ..EnvironmentPaths::default()
    };
    let paths = MachinePaths::resolve(&environment).unwrap();
    assert_eq!(paths.machine_home_root, real);
    assert_eq!(paths.global_config, real.join("config.toml"));
    assert_eq!(paths.database, real.join("state.db"));
    assert_eq!(paths.traces, real.join("traces"));
}

#[test]
fn absolute_override_does_not_require_home_and_relative_override_fails() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("missing/../isolated");
    let paths = MachinePaths::resolve(&EnvironmentPaths {
        home: None,
        loop_engine_home: Some(target.into_os_string()),
        ..EnvironmentPaths::default()
    })
    .unwrap();
    assert_eq!(paths.machine_home_root, directory.path().join("isolated"));
    assert!(
        MachinePaths::resolve(&EnvironmentPaths {
            home: Some(OsString::from("/home/test")),
            loop_engine_home: Some(OsString::from("relative")),
            ..EnvironmentPaths::default()
        })
        .is_err()
    );
    assert!(
        MachinePaths::resolve(&EnvironmentPaths {
            home: Some(OsString::from("relative-home")),
            ..EnvironmentPaths::default()
        })
        .is_err()
    );
    assert!(
        MachinePaths::resolve(&EnvironmentPaths {
            home: None,
            loop_engine_home: Some(OsString::from(format!("/{}", "x".repeat(4_094)))),
            ..EnvironmentPaths::default()
        })
        .is_err()
    );
    assert!(
        MachinePaths::resolve(&EnvironmentPaths {
            home: None,
            loop_engine_home: Some(OsString::from_vec(vec![b'/', 0xff])),
            ..EnvironmentPaths::default()
        })
        .is_err()
    );
    #[cfg(target_os = "linux")]
    assert!(
        MachinePaths::resolve(&EnvironmentPaths {
            home: Some(OsString::from("/home/test")),
            xdg_state_home: Some(OsString::from("relative-state")),
            ..EnvironmentPaths::default()
        })
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn xdg_roots_expand_tilde_and_absolute_roots_do_not_require_home() {
    let expanded = MachinePaths::resolve(&EnvironmentPaths {
        home: Some(OsString::from("/home/test")),
        xdg_config_home: Some(OsString::from("~/cfg")),
        xdg_state_home: Some(OsString::from("~/state")),
        ..EnvironmentPaths::default()
    })
    .unwrap();
    assert_eq!(
        expanded.config_root,
        std::path::Path::new("/home/test/cfg/loop-engine")
    );
    assert_eq!(
        expanded.state_root,
        std::path::Path::new("/home/test/state/loop-engine")
    );

    let absolute = MachinePaths::resolve(&EnvironmentPaths {
        home: None,
        xdg_config_home: Some(OsString::from("/var/config")),
        xdg_state_home: Some(OsString::from("/var/state")),
        ..EnvironmentPaths::default()
    })
    .unwrap();
    assert_eq!(
        absolute.config_root,
        std::path::Path::new("/var/config/loop-engine")
    );
    assert_eq!(
        absolute.state_root,
        std::path::Path::new("/var/state/loop-engine")
    );
}

#[test]
fn nearest_project_file_wins_and_broken_symlink_is_skipped() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("a/b/c");
    std::fs::create_dir_all(&child).unwrap();
    let upper = directory.path().join(".loop-engine.toml");
    std::fs::write(&upper, "schema_version = 1").unwrap();
    let nearer = directory.path().join("a/.loop-engine.toml");
    std::os::unix::fs::symlink("missing", &nearer).unwrap();
    assert_eq!(
        discover_project_config(&child).unwrap(),
        Some(upper.clone())
    );
    std::fs::remove_file(&nearer).unwrap();
    let loop_target = directory.path().join("a/loop-target");
    std::os::unix::fs::symlink(&loop_target, &nearer).unwrap();
    std::os::unix::fs::symlink(&nearer, &loop_target).unwrap();
    assert_eq!(
        discover_project_config(&child).unwrap(),
        Some(upper.clone())
    );
    std::fs::remove_file(&nearer).unwrap();
    std::fs::remove_file(&loop_target).unwrap();
    std::fs::write(&nearer, "schema_version = 1").unwrap();
    assert_eq!(discover_project_config(&child).unwrap(), Some(nearer));
}

#[test]
fn typed_toml_accepts_contract_and_rejects_unknown_forbidden_and_malformed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        "schema_version = 1\n[defaults]\nformat = \"json\"\nprovider = \"demo\"\ntimeout_seconds = 3\n",
    )
    .unwrap();
    let valid = load_optional(&path).unwrap().unwrap();
    assert_eq!(valid.defaults.format, Some(OutputFormat::Json));
    assert_eq!(valid.defaults.provider.as_deref(), Some("demo"));

    let oversized = directory.path().join("oversized.toml");
    let file = std::fs::File::create(&oversized).unwrap();
    file.set_len(1_048_577).unwrap();
    assert!(load_optional(&oversized).is_err());

    for invalid in [
        "schema_version = 1\nunknown = true",
        "schema_version = 1\n[[providers]]\nexec = '/bin/x'",
        "schema_version = 1\n[defaults]\nworking_directory = '/tmp'",
        "schema_version = 2",
        "schema_version =",
        "schema_version = 1\n[defaults]\ntimeout_seconds = 0",
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(load_optional(&path).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn precedence_is_cli_then_project_then_global_then_builtin() {
    let directory = tempfile::tempdir().unwrap();
    let global_path = directory.path().join("global.toml");
    let project_path = directory.path().join("project.toml");
    std::fs::write(
        &global_path,
        "schema_version=1\n[defaults]\nformat='json'\nprovider='global'\ntimeout_seconds=10",
    )
    .unwrap();
    std::fs::write(
        &project_path,
        "schema_version=1\n[defaults]\nprovider='project'\ntimeout_seconds=20",
    )
    .unwrap();
    let global = load_optional(&global_path).unwrap().unwrap();
    let project = load_optional(&project_path).unwrap().unwrap();
    let resolved = resolve_defaults(
        &CliDefaults {
            format: Some(OutputFormat::Human),
            provider: None,
            timeout_seconds: Some(30),
        },
        Some(&project),
        Some(&global),
    );
    assert_eq!(resolved.format, OutputFormat::Human);
    assert_eq!(resolved.provider.as_deref(), Some("project"));
    assert_eq!(resolved.timeout_seconds, 30);

    let global_only = resolve_defaults(&CliDefaults::default(), None, Some(&global));
    assert_eq!(global_only.format, OutputFormat::Json);
    assert_eq!(global_only.provider.as_deref(), Some("global"));
    assert_eq!(global_only.timeout_seconds, 10);
    let built_in = resolve_defaults(&CliDefaults::default(), None, None);
    assert_eq!(built_in.format, OutputFormat::Human);
    assert_eq!(built_in.provider, None);
    assert_eq!(built_in.timeout_seconds, 60);

    assert_eq!(
        provider_for_new_target(None, &resolved, TargetProviderRequirement::Required,),
        None
    );
    assert_eq!(
        provider_for_new_target(None, &resolved, TargetProviderRequirement::Optional,),
        Some("project")
    );
    assert_eq!(
        provider_for_new_target(
            Some("explicit"),
            &resolved,
            TargetProviderRequirement::Required,
        ),
        Some("explicit")
    );

    let stored = RegistrationId::parse("stored-registration").unwrap();
    assert_eq!(provider_for_existing_run(&stored), &stored);
}

#[test]
fn ancestor_search_stops_at_root_and_absent_project_file_is_none() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("a/b");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(discover_project_config(&nested).unwrap(), None);
    assert_eq!(
        discover_project_config(std::path::Path::new("/")).unwrap(),
        None
    );
}
