use std::process::{Command, Output};

fn invoke(arguments: &[&str], stdin: &[u8]) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    command.args(arguments);
    let completed =
        super::bounded_process::run_with_stdin(&mut command, "software-change CLI protocol", stdin)
            .expect("software-change process should exit");
    if let Some(error) = completed.stdin_error {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write failure: {error}"
        );
    }
    completed.output
}

#[test]
fn help_flags_return_conventional_stdout_without_reading_protocol() {
    for flag in ["--help", "-h"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("software-change"));
        assert!(help.contains("describe"));
        assert!(help.contains("evaluate"));
        assert!(help.contains("data-dump"));
        assert!(help.contains("run-plan-graph"));
        assert!(help.contains('4'));
    }
}

#[test]
fn version_flags_report_workspace_package_version_on_stdout() {
    for flag in ["--version", "-V"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("software-change {}\n", env!("CARGO_PKG_VERSION")),
            "flag={flag}"
        );
    }
}

#[test]
fn no_arguments_keep_stdin_protocol_and_unsupported_arguments_keep_error_taxonomy() {
    let describe = invoke(&[], br#"{"operation":"describe"}"#);
    assert_eq!(describe.status.code(), Some(0));
    assert!(!describe.stdout.is_empty());

    let unsupported = invoke(&["unknown"], b"");
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(unsupported.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported command"));
}
