use std::io::Write;
use std::process::{Command, Output, Stdio};

fn invoke(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_policy-document"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("policy-document binary should spawn");
    child
        .stdin
        .take()
        .expect("policy-document stdin should be available")
        .write_all(input)
        .expect("request should reach policy-document");
    child
        .wait_with_output()
        .expect("policy-document process should exit")
}

#[test]
fn unknown_describe_envelope_field_exits_with_protocol_error() {
    let output = invoke(br#"{"operation":"describe","unexpected":true}"#);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field `unexpected`"));
}

#[test]
fn describe_with_and_without_initial_input_return_the_same_workflow_bytes() {
    let without = invoke(br#"{"operation":"describe"}"#);
    let with = invoke(
        br#"{"operation":"describe","initial_input":{"review_policies":{"design-review":["axis"]},"objective":"ignored"}}"#,
    );

    assert!(without.status.success(), "stderr: {:?}", without.stderr);
    assert!(with.status.success(), "stderr: {:?}", with.stderr);
    assert_eq!(without.stdout, with.stdout);
    assert!(!without.stdout.is_empty());
}
