mod packet;
mod run;
#[cfg(test)]
mod tests;

use lean_change_provider::protocol::{DescribeRequest, EvaluateRequest};
use lean_change_provider::{evaluate, model, workflow};
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Read, Write};

fn main() {
    std::process::exit(dispatch());
}

fn dispatch() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("lean-change {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("setup") => report(
            packet::setup(&args[1..])
                .and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string())),
        ),
        Some("verify") => report(
            stdin_json()
                .and_then(|show| run::verify(&show))
                .and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string())),
        ),
        Some("intent-packet") => report(stdin_json().and_then(|show| packet::intent_packet(&show))),
        Some("review-packet") => report(stdin_json().and_then(|show| packet::review_packet(&show))),
        Some(other) => {
            eprintln!("unsupported command `{other}`");
            2
        }
        None => protocol(),
    }
}

fn report(result: Result<String, String>) -> i32 {
    match result {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

fn stdin_json() -> Result<Value, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("could not read stdin: {e}"))?;
    serde_json::from_str(&input).map_err(|e| format!("stdin is not JSON: {e}"))
}

fn protocol() -> i32 {
    let value = match stdin_json() {
        Ok(value) => value,
        Err(message) => return protocol_error(message),
    };
    match value.get("operation").and_then(Value::as_str) {
        Some("describe") => match serde_json::from_value::<DescribeRequest>(value) {
            Ok(request) if request.operation == "describe" => {
                match request
                    .initial_input
                    .as_ref()
                    .map(model::InitialInput::parse)
                    .transpose()
                {
                    // Bare describe is topology discovery; the frozen input picks B or C and its old-vs-new check.
                    Ok(Some(input)) => write_json(&workflow::workflow(
                        input.review,
                        input.old_vs_new.is_some(),
                    )),
                    Ok(None) => write_json(&workflow::workflow(true, false)),
                    Err(message) => protocol_error(message),
                }
            }
            Ok(_) => protocol_error("describe request has wrong operation".into()),
            Err(e) => protocol_error(format!("invalid describe request: {e}")),
        },
        Some("evaluate") => match serde_json::from_value::<EvaluateRequest>(value) {
            Ok(request) if request.operation == "evaluate" => {
                match evaluate::evaluate(&request, &run::tree_identity) {
                    Ok(response) => write_json(&response),
                    Err(message) => {
                        eprintln!("{message}");
                        1
                    }
                }
            }
            Ok(_) => protocol_error("evaluate request has wrong operation".into()),
            Err(e) => protocol_error(format!("invalid evaluate request: {e}")),
        },
        Some(other) => protocol_error(format!("unsupported provider operation `{other}`")),
        None => protocol_error("request must contain string operation".into()),
    }
}

fn write_json<T: Serialize>(value: &T) -> i32 {
    let mut out = io::stdout().lock();
    match serde_json::to_writer(&mut out, value)
        .map_err(|e| e.to_string())
        .and_then(|()| out.flush().map_err(|e| e.to_string()))
    {
        Ok(()) => 0,
        Err(e) => protocol_error(format!("could not write response: {e}")),
    }
}

fn protocol_error(message: String) -> i32 {
    eprintln!("{message}");
    2
}
