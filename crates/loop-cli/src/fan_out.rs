//! Argv, stdin contracts, and process collector for `fan-out`.
//!
//! Workers come only from repeated `--worker` JSON objects.  The collector
//! does not open a database, encode a harness, or emit a run-state envelope.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ADHOC: AtomicU64 = AtomicU64::new(1);

/// Worker argv: JSON object with exactly string `command` and array-of-string `args`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerCli {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

/// Bound-worker stdin packet.  Exactly the four engine invoke keys; no extras.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvokePacket {
    pub(crate) run_id: String,
    pub(crate) slot_id: String,
    pub(crate) artifact_root: String,
    pub(crate) instruction_body: String,
}

/// Collected `fan-out` flags after the command name.  Zero `--worker` entries
/// are allowed at parse time; empty-worker execute fails closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FanOutArgs {
    pub(crate) workers: Vec<WorkerCli>,
    pub(crate) instructions_path: Option<PathBuf>,
}

/// Bound (invoke-packet stdin) versus ad-hoc (`--instructions FILE`) fan-out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FanOutMode {
    Bound {
        packet: InvokePacket,
        workers: Vec<WorkerCli>,
    },
    AdHoc {
        instructions_path: PathBuf,
        workers: Vec<WorkerCli>,
    },
}

/// Parse failure for worker JSON, invoke packets, argv, or mode detection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// Collector failure: invalid caller input versus incomplete collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CollectorError {
    Invalid(String),
    Failed(String),
}

impl From<ParseError> for CollectorError {
    fn from(error: ParseError) -> Self {
        CollectorError::Invalid(error.to_string())
    }
}

/// JSON summary printed on collector success.  Not a run-state envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FanOutSummary {
    pub(crate) output_dir: String,
    pub(crate) workers: Vec<FanOutWorkerResult>,
}

/// One reaped worker in `--worker` order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FanOutWorkerResult {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) exit_code: i32,
    pub(crate) stdout_path: String,
    pub(crate) stderr_path: String,
}

/// Parse one worker CLI JSON object (`{command, args}`).
pub(crate) fn parse_worker_cli_json(raw: &str) -> Result<WorkerCli, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "worker CLI JSON must be an object with exactly string `command` and array-of-string `args`: {error}"
        ))
    })
}

/// Parse the four-key engine invoke packet from stdin JSON.
pub(crate) fn parse_invoke_packet(raw: &str) -> Result<InvokePacket, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "invoke packet must be a JSON object with exactly `run_id`, `slot_id`, `artifact_root`, and `instruction_body`: {error}"
        ))
    })
}

/// Parse argv tokens after the `fan-out` command name.
///
/// Repeated `--worker JSON` (zero entries allowed).  Optional once:
/// `--instructions FILE`.  Unknown flags and leftover positionals are errors.
pub(crate) fn parse_fan_out_args<I, S>(args: I) -> Result<FanOutArgs, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|token| token.as_ref().to_owned())
        .collect::<Vec<String>>();
    let mut workers = Vec::new();
    let mut instructions_path = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if let Some(raw) = strip_option(token, "--worker") {
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--worker")?.to_owned(),
            };
            workers.push(parse_worker_cli_json(&raw)?);
            continue;
        }
        if let Some(raw) = strip_option(token, "--instructions") {
            if instructions_path.is_some() {
                return Err(ParseError::new(
                    "`--instructions` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--instructions")?.to_owned(),
            };
            if raw.is_empty() {
                return Err(ParseError::new("option `--instructions` requires a value"));
            }
            instructions_path = Some(PathBuf::from(raw));
            continue;
        }
        if token.starts_with('-') {
            return Err(ParseError::new(format!("unknown option `{token}`")));
        }
        return Err(ParseError::new(format!(
            "unexpected argument `{token}` for fan-out"
        )));
    }
    Ok(FanOutArgs {
        workers,
        instructions_path,
    })
}

/// Choose bound versus ad-hoc mode from parsed flags and stdin bytes.
///
/// Bound: stdin parses as an invoke packet and `--instructions` is absent.
/// Ad hoc: `--instructions FILE` is present and stdin is not a packet.
/// Combining a valid packet with `--instructions` is a parse error.
/// Ad hoc without `--instructions` (empty or non-packet stdin) is a parse error.
///
/// `--instructions FILE` is required for ad-hoc mode, but the path is not
/// opened here.  Missing-file failures are deferred to execute.
pub(crate) fn detect_mode(
    parsed_args: FanOutArgs,
    stdin_bytes: &[u8],
) -> Result<FanOutMode, ParseError> {
    let packet = std::str::from_utf8(stdin_bytes)
        .ok()
        .and_then(|raw| parse_invoke_packet(raw).ok());
    match (packet, parsed_args.instructions_path) {
        (Some(_), Some(_)) => Err(ParseError::new(
            "cannot combine an invoke packet on stdin with `--instructions`",
        )),
        (Some(packet), None) => Ok(FanOutMode::Bound {
            packet,
            workers: parsed_args.workers,
        }),
        (None, Some(instructions_path)) => Ok(FanOutMode::AdHoc {
            instructions_path,
            workers: parsed_args.workers,
        }),
        (None, None) => Err(ParseError::new(
            "ad-hoc fan-out requires `--instructions FILE`; bound fan-out requires an invoke packet on stdin",
        )),
    }
}

/// Bound fan-out reads a piped invoke packet. A terminal stdin is not a packet
/// source: draining it hangs interactive ad-hoc `--instructions FILE`.
pub(crate) fn drain_stdin(is_terminal: bool) -> bool {
    !is_terminal
}

/// Run the fan-out collector: detect mode, spawn every worker in parallel,
/// reap each child, and return the JSON summary payload.
pub(crate) fn run_collector(
    parsed_args: FanOutArgs,
    stdin_bytes: &[u8],
    cwd: &Path,
) -> Result<FanOutSummary, CollectorError> {
    let mode = detect_mode(parsed_args, stdin_bytes)?;
    match mode {
        FanOutMode::Bound { packet, workers } => {
            ensure_workers(&workers)?;
            let artifact_root = absolute_from_cwd(cwd, Path::new(&packet.artifact_root));
            let payload = bound_stdin_payload(&packet, &artifact_root);
            let output_dir = artifact_root.join("fan-out").join(&packet.slot_id);
            collect_workers(&workers, payload.as_bytes(), &output_dir)
        }
        FanOutMode::AdHoc {
            instructions_path,
            workers,
        } => {
            ensure_workers(&workers)?;
            let instructions_path = absolute_from_cwd(cwd, &instructions_path);
            let payload = fs::read(&instructions_path).map_err(|error| {
                CollectorError::Invalid(format!(
                    "could not read instructions file `{}`: {error}",
                    instructions_path.display()
                ))
            })?;
            let unique = unique_adhoc_id();
            let output_dir = cwd.join("fan-out-adhoc").join(unique);
            collect_workers(&workers, &payload, &output_dir)
        }
    }
}

fn strip_option(token: &str, option: &str) -> Option<Option<String>> {
    if token == option {
        return Some(None);
    }
    let prefix = format!("{option}=");
    token.strip_prefix(&prefix).map(|raw| Some(raw.to_owned()))
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, ParseError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| ParseError::new(format!("option `{option}` requires a value")))?;
    if value.starts_with('-') && value != "-" {
        return Err(ParseError::new(format!(
            "option `{option}` requires a value"
        )));
    }
    *index += 1;
    Ok(value)
}

fn ensure_workers(workers: &[WorkerCli]) -> Result<(), CollectorError> {
    if workers.is_empty() {
        return Err(CollectorError::Invalid(
            "fan-out requires at least one `--worker`".to_owned(),
        ));
    }
    Ok(())
}

fn bound_stdin_payload(packet: &InvokePacket, artifact_root: &Path) -> String {
    let mut body = packet.instruction_body.clone();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push('\n');
    body.push_str("run_id: ");
    body.push_str(&packet.run_id);
    body.push('\n');
    body.push_str("slot_id: ");
    body.push_str(&packet.slot_id);
    body.push('\n');
    body.push_str("artifact_root: ");
    body.push_str(&artifact_root.to_string_lossy());
    body.push('\n');
    body
}

fn unique_adhoc_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_ADHOC.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{sequence}", std::process::id())
}

fn absolute_from_cwd(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn collect_workers(
    workers: &[WorkerCli],
    payload: &[u8],
    output_dir: &Path,
) -> Result<FanOutSummary, CollectorError> {
    fs::create_dir_all(output_dir).map_err(|error| {
        CollectorError::Failed(format!(
            "could not create fan-out output directory `{}`: {error}",
            output_dir.display()
        ))
    })?;

    struct Spawned {
        command: String,
        args: Vec<String>,
        child: std::process::Child,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
    }

    let mut spawned = Vec::new();
    let mut stdin_handles = Vec::new();
    let mut spawn_errors = Vec::new();

    for (index, worker) in workers.iter().enumerate() {
        let worker_dir = output_dir.join(index.to_string());
        if let Err(error) = fs::create_dir_all(&worker_dir) {
            spawn_errors.push(format!(
                "could not create worker output directory `{}`: {error}",
                worker_dir.display()
            ));
            continue;
        }
        let stdout_path = worker_dir.join("stdout");
        let stderr_path = worker_dir.join("stderr");
        let stdout_file = match File::create(&stdout_path) {
            Ok(file) => file,
            Err(error) => {
                spawn_errors.push(format!(
                    "could not create stdout file `{}`: {error}",
                    stdout_path.display()
                ));
                continue;
            }
        };
        let stderr_file = match File::create(&stderr_path) {
            Ok(file) => file,
            Err(error) => {
                spawn_errors.push(format!(
                    "could not create stderr file `{}`: {error}",
                    stderr_path.display()
                ));
                continue;
            }
        };

        match Command::new(&worker.command)
            .args(&worker.args)
            .stdin(Stdio::piped())
            .stdout(stdout_file)
            .stderr(stderr_file)
            .spawn()
        {
            Ok(mut child) => {
                let stdin = child.stdin.take();
                spawned.push(Spawned {
                    command: worker.command.clone(),
                    args: worker.args.clone(),
                    child,
                    stdout_path,
                    stderr_path,
                });
                if let Some(stdin) = stdin {
                    stdin_handles.push((worker.command.clone(), stdin));
                }
            }
            Err(error) => {
                spawn_errors.push(format!(
                    "could not spawn worker `{}`: {error}",
                    worker.command
                ));
            }
        }
    }

    // Spawn every child before writing stdin, then write in parallel so a
    // slow or deferred reader cannot serialize launch or deadlock a peer.
    let payload = Arc::<[u8]>::from(payload.to_vec());
    let mut writers = Vec::new();
    for (command, mut stdin) in stdin_handles {
        let payload = Arc::clone(&payload);
        writers.push(thread::spawn(move || {
            let result = stdin.write_all(&payload);
            drop(stdin);
            (command, result)
        }));
    }
    for handle in writers {
        match handle.join() {
            Ok((command, Err(error))) if error.kind() != io::ErrorKind::BrokenPipe => {
                spawn_errors.push(format!(
                    "could not write stdin to worker `{command}`: {error}"
                ));
            }
            Ok(_) => {}
            Err(_) => spawn_errors.push("fan-out stdin writer thread panicked".to_owned()),
        }
    }

    let mut results = Vec::new();
    let mut wait_errors = Vec::new();
    for mut job in spawned {
        match job.child.wait() {
            Ok(status) => {
                results.push(FanOutWorkerResult {
                    command: job.command,
                    args: job.args,
                    exit_code: status.code().unwrap_or(1),
                    stdout_path: path_to_string(&job.stdout_path),
                    stderr_path: path_to_string(&job.stderr_path),
                });
            }
            Err(error) => {
                wait_errors.push(format!(
                    "could not wait for worker `{}`: {error}",
                    job.command
                ));
            }
        }
    }

    if !spawn_errors.is_empty() || !wait_errors.is_empty() {
        let mut message = spawn_errors;
        message.extend(wait_errors);
        return Err(CollectorError::Failed(message.join("; ")));
    }

    Ok(FanOutSummary {
        output_dir: path_to_string(output_dir),
        workers: results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    fn valid_worker_json() -> &'static str {
        r#"{"command":"echo","args":["hello"]}"#
    }

    fn second_worker_json() -> &'static str {
        r#"{"command":"cat","args":["-"]}"#
    }

    fn valid_packet_json() -> &'static str {
        r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work"}"#
    }

    fn cat_worker(receipt: &Path) -> WorkerCli {
        WorkerCli {
            command: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "cat > \"$1\"".to_owned(),
                "_".to_owned(),
                receipt.to_string_lossy().into_owned(),
            ],
        }
    }

    fn sleep_worker(receipt: &Path, pid_path: &Path, done_path: &Path) -> WorkerCli {
        WorkerCli {
            command: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "cat > \"$1\"; echo $$ > \"$2\"; sleep 0.2; echo done > \"$3\"".to_owned(),
                "_".to_owned(),
                receipt.to_string_lossy().into_owned(),
                pid_path.to_string_lossy().into_owned(),
                done_path.to_string_lossy().into_owned(),
            ],
        }
    }

    fn delay_then_read_worker(receipt: &Path, pid_path: &Path) -> WorkerCli {
        WorkerCli {
            command: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "echo $$ > \"$1\"; sleep 0.4; cat > \"$2\"".to_owned(),
                "_".to_owned(),
                pid_path.to_string_lossy().into_owned(),
                receipt.to_string_lossy().into_owned(),
            ],
        }
    }

    fn exit_worker(receipt: &Path, code: i32) -> WorkerCli {
        WorkerCli {
            command: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                format!("cat > \"$1\"; exit {code}"),
                "_".to_owned(),
                receipt.to_string_lossy().into_owned(),
            ],
        }
    }

    fn pid_is_alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn valid_worker_json_parses_command_and_args() {
        let worker = parse_worker_cli_json(valid_worker_json()).expect("valid worker JSON");
        assert_eq!(worker.command, "echo");
        assert_eq!(worker.args, vec!["hello".to_owned()]);
    }

    #[test]
    fn worker_json_unknown_field_fails() {
        let raw = r#"{"command":"echo","args":["hello"],"extra":true}"#;
        assert!(parse_worker_cli_json(raw).is_err());
    }

    #[test]
    fn worker_json_non_object_fails() {
        assert!(parse_worker_cli_json(r#"["echo"]"#).is_err());
        assert!(parse_worker_cli_json("\"echo\"").is_err());
    }

    #[test]
    fn worker_json_non_string_command_fails() {
        assert!(parse_worker_cli_json(r#"{"command":1,"args":[]}"#).is_err());
    }

    #[test]
    fn worker_json_non_string_args_entry_fails() {
        assert!(parse_worker_cli_json(r#"{"command":"echo","args":[1]}"#).is_err());
    }

    #[test]
    fn valid_invoke_packet_parses_four_string_keys() {
        let packet = parse_invoke_packet(valid_packet_json()).expect("valid packet");
        assert_eq!(packet.run_id, "run-1");
        assert_eq!(packet.slot_id, "slot-1");
        assert_eq!(packet.artifact_root, "/tmp/artifacts");
        assert_eq!(packet.instruction_body, "Do the work");
    }

    #[test]
    fn invoke_packet_extra_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","extra":"no"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_missing_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn zero_workers_parses() {
        let parsed = parse_fan_out_args(&[] as &[&str]).expect("zero --worker");
        assert!(parsed.workers.is_empty());
        assert!(parsed.instructions_path.is_none());
    }

    #[test]
    fn repeated_worker_flags_collect_in_order() {
        let parsed = parse_fan_out_args([
            "--worker",
            valid_worker_json(),
            "--worker",
            second_worker_json(),
        ])
        .expect("repeated --worker");
        assert_eq!(parsed.workers.len(), 2);
        assert_eq!(parsed.workers[0].command, "echo");
        assert_eq!(parsed.workers[0].args, vec!["hello".to_owned()]);
        assert_eq!(parsed.workers[1].command, "cat");
        assert_eq!(parsed.workers[1].args, vec!["-".to_owned()]);
    }

    #[test]
    fn bound_mode_from_stdin_packet_without_instructions() {
        let parsed = parse_fan_out_args(["--worker", valid_worker_json()]).expect("args");
        let mode = detect_mode(parsed, valid_packet_json().as_bytes()).expect("bound mode");
        match mode {
            FanOutMode::Bound { packet, workers } => {
                assert_eq!(packet.run_id, "run-1");
                assert_eq!(workers.len(), 1);
                assert_eq!(workers[0].command, "echo");
            }
            FanOutMode::AdHoc { .. } => panic!("expected Bound mode"),
        }
    }

    #[test]
    fn ad_hoc_mode_from_instructions_when_stdin_is_not_a_packet() {
        let parsed = parse_fan_out_args(["--instructions", "/tmp/instructions.md"]).expect("args");
        let mode = detect_mode(parsed, b"not a packet").expect("ad-hoc mode");
        match mode {
            FanOutMode::AdHoc {
                instructions_path,
                workers,
            } => {
                assert_eq!(instructions_path, PathBuf::from("/tmp/instructions.md"));
                assert!(workers.is_empty());
            }
            FanOutMode::Bound { .. } => panic!("expected AdHoc mode"),
        }
    }

    #[test]
    fn valid_packet_combined_with_instructions_is_parse_error() {
        let parsed = parse_fan_out_args(["--instructions", "/tmp/instructions.md"]).expect("args");
        assert!(detect_mode(parsed, valid_packet_json().as_bytes()).is_err());
    }

    #[test]
    fn ad_hoc_missing_instructions_is_parse_error() {
        let empty = parse_fan_out_args(&[] as &[&str]).expect("zero flags");
        assert!(detect_mode(empty.clone(), b"").is_err());
        assert!(detect_mode(empty, b"not a packet").is_err());
    }

    #[test]
    fn terminal_stdin_is_not_drained() {
        assert!(
            !drain_stdin(true),
            "interactive stdin must not be drained to EOF"
        );
        assert!(drain_stdin(false), "piped stdin is the bound packet source");
    }

    #[test]
    fn bound_stdin_layout_is_instruction_body_blank_line_then_fields() {
        let packet = InvokePacket {
            run_id: "run-1".to_owned(),
            slot_id: "slot-1".to_owned(),
            artifact_root: "relative/root".to_owned(),
            instruction_body: "Do the work".to_owned(),
        };
        let payload = bound_stdin_payload(&packet, Path::new("/tmp/artifacts"));
        assert_eq!(
            payload,
            "Do the work\n\nrun_id: run-1\nslot_id: slot-1\nartifact_root: /tmp/artifacts\n"
        );
    }

    #[test]
    fn zero_workers_fail_closed_in_bound_and_ad_hoc() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.txt");
        fs::write(&instructions, b"shared").expect("write instructions");
        let packet = json!({
            "run_id": "run-1",
            "slot_id": "slot-1",
            "artifact_root": directory.path().join("artifacts").to_string_lossy(),
            "instruction_body": "Do the work",
        })
        .to_string();

        let bound = run_collector(
            FanOutArgs {
                workers: Vec::new(),
                instructions_path: None,
            },
            packet.as_bytes(),
            directory.path(),
        );
        assert!(matches!(bound, Err(CollectorError::Invalid(_))));

        let ad_hoc = run_collector(
            FanOutArgs {
                workers: Vec::new(),
                instructions_path: Some(instructions),
            },
            b"",
            directory.path(),
        );
        assert!(matches!(ad_hoc, Err(CollectorError::Invalid(_))));
    }

    #[test]
    fn two_dummies_record_the_same_shared_stdin_and_are_reaped() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.bin");
        let shared = b"shared-bytes-without-trailer";
        fs::write(&instructions, shared).expect("write instructions");
        let receipt_a = directory.path().join("a.stdin");
        let receipt_b = directory.path().join("b.stdin");
        let pid_a = directory.path().join("a.pid");
        let pid_b = directory.path().join("b.pid");
        let done_a = directory.path().join("a.done");
        let done_b = directory.path().join("b.done");

        let summary = run_collector(
            FanOutArgs {
                workers: vec![
                    sleep_worker(&receipt_a, &pid_a, &done_a),
                    sleep_worker(&receipt_b, &pid_b, &done_b),
                ],
                instructions_path: Some(instructions),
            },
            b"not a packet",
            directory.path(),
        )
        .expect("collector success");

        assert_eq!(fs::read(&receipt_a).expect("read a"), shared);
        assert_eq!(fs::read(&receipt_b).expect("read b"), shared);
        assert_eq!(
            fs::read(&receipt_a).expect("read a"),
            fs::read(&receipt_b).expect("read b")
        );
        assert!(
            done_a.is_file(),
            "worker A must finish before collector returns"
        );
        assert!(
            done_b.is_file(),
            "worker B must finish before collector returns"
        );
        let pid_a = fs::read_to_string(&pid_a).expect("pid a");
        let pid_b = fs::read_to_string(&pid_b).expect("pid b");
        assert!(
            !pid_is_alive(pid_a.trim()),
            "worker A must be reaped: {}",
            pid_a.trim()
        );
        assert!(
            !pid_is_alive(pid_b.trim()),
            "worker B must be reaped: {}",
            pid_b.trim()
        );
        assert!(summary.output_dir.contains("fan-out-adhoc"));
        assert_eq!(summary.workers.len(), 2);
        assert_eq!(summary.workers[0].exit_code, 0);
        assert_eq!(summary.workers[1].exit_code, 0);
        assert!(Path::new(&summary.workers[0].stdout_path).is_file());
        assert!(Path::new(&summary.workers[1].stderr_path).is_file());
    }

    #[test]
    fn dummy_nonzero_exit_still_succeeds_and_appears_in_summary() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.txt");
        fs::write(&instructions, b"judge this").expect("write instructions");
        let receipt = directory.path().join("nonzero.stdin");

        let summary = run_collector(
            FanOutArgs {
                workers: vec![exit_worker(&receipt, 7)],
                instructions_path: Some(instructions),
            },
            b"",
            directory.path(),
        )
        .expect("collector success despite worker nonzero");

        assert_eq!(fs::read(&receipt).expect("read receipt"), b"judge this");
        assert_eq!(summary.workers.len(), 1);
        assert_eq!(summary.workers[0].exit_code, 7);
        assert_eq!(summary.workers[0].command, "sh");
    }

    #[test]
    fn bound_mode_writes_outputs_under_artifact_root_and_resolves_relative_root() {
        let directory = tempdir().expect("tempdir");
        let receipt = directory.path().join("bound.stdin");
        let packet = json!({
            "run_id": "run-9",
            "slot_id": "design-review",
            "artifact_root": "artifacts",
            "instruction_body": "Review the design",
        })
        .to_string();

        let summary = run_collector(
            FanOutArgs {
                workers: vec![cat_worker(&receipt)],
                instructions_path: None,
            },
            packet.as_bytes(),
            directory.path(),
        )
        .expect("bound collector");

        let expected_root = directory.path().join("artifacts");
        let expected_dir = expected_root.join("fan-out").join("design-review");
        assert_eq!(summary.output_dir, expected_dir.to_string_lossy());
        assert_eq!(
            summary.workers[0].stdout_path,
            expected_dir.join("0").join("stdout").to_string_lossy()
        );
        assert_eq!(
            summary.workers[0].stderr_path,
            expected_dir.join("0").join("stderr").to_string_lossy()
        );
        let recorded = fs::read_to_string(&receipt).expect("bound stdin");
        assert_eq!(
            recorded,
            format!(
                "Review the design\n\nrun_id: run-9\nslot_id: design-review\nartifact_root: {}\n",
                expected_root.to_string_lossy()
            )
        );
    }

    #[test]
    fn spawn_failure_is_a_collector_failure_after_reaping_started_workers() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.txt");
        fs::write(&instructions, b"payload").expect("write instructions");
        let receipt = directory.path().join("started.stdin");
        let pid_path = directory.path().join("started.pid");
        let done_path = directory.path().join("started.done");

        let result = run_collector(
            FanOutArgs {
                workers: vec![
                    sleep_worker(&receipt, &pid_path, &done_path),
                    WorkerCli {
                        command: directory
                            .path()
                            .join("no-such-worker-binary")
                            .to_string_lossy()
                            .into_owned(),
                        args: vec![],
                    },
                ],
                instructions_path: Some(instructions),
            },
            b"",
            directory.path(),
        );
        assert!(matches!(result, Err(CollectorError::Failed(_))));
        assert!(done_path.is_file(), "started sibling must be reaped");
        let pid = fs::read_to_string(&pid_path).expect("pid");
        assert!(!pid_is_alive(pid.trim()), "started sibling must not remain");
    }

    #[test]
    fn deferred_readers_with_large_payload_start_in_parallel() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.bin");
        let payload = vec![b'x'; 2 * 1024 * 1024];
        fs::write(&instructions, &payload).expect("write instructions");
        let receipt_a = directory.path().join("a.stdin");
        let receipt_b = directory.path().join("b.stdin");
        let pid_a = directory.path().join("a.pid");
        let pid_b = directory.path().join("b.pid");
        let workers = vec![
            delay_then_read_worker(&receipt_a, &pid_a),
            delay_then_read_worker(&receipt_b, &pid_b),
        ];
        let work_dir = directory.path().to_path_buf();

        let handle = thread::spawn(move || {
            run_collector(
                FanOutArgs {
                    workers,
                    instructions_path: Some(instructions),
                },
                b"not a packet",
                &work_dir,
            )
        });

        let first_pid_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < first_pid_deadline && !(pid_a.is_file() || pid_b.is_file()) {
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(150));
        let both_started = pid_a.is_file() && pid_b.is_file();
        let summary = handle
            .join()
            .expect("collector thread")
            .expect("collector success");
        assert!(
            both_started,
            "both workers must start before either finishes reading a 2MiB payload"
        );
        assert_eq!(fs::read(&receipt_a).expect("read a"), payload);
        assert_eq!(fs::read(&receipt_b).expect("read b"), payload);
        assert_eq!(summary.workers.len(), 2);
        assert_eq!(summary.workers[0].exit_code, 0);
        assert_eq!(summary.workers[1].exit_code, 0);
    }
}
