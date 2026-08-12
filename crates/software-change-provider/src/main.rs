//! `software-change` subprocess provider.
//!
//! One process handles one request.  Protocol errors use exit 2, evaluation
//! errors use exit 1, and a successfully written result uses exit 0.

mod artifacts;
mod config;
mod evidence;
mod gates;
mod protocol;
mod schema;
mod workflow;

use protocol::{DescribeRequest, EvaluateRequest};
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Read, Write};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        return protocol_error(format!("could not read request: {error}"));
    }

    let request = match serde_json::from_str::<Value>(&input) {
        Ok(request) => request,
        Err(error) => return protocol_error(format!("malformed JSON request: {error}")),
    };

    let operation = match request
        .as_object()
        .and_then(|object| object.get("operation"))
        .and_then(Value::as_str)
    {
        Some(operation) => operation.to_owned(),
        None => {
            return protocol_error("request must be a JSON object with a string `operation`".into())
        }
    };

    match operation.as_str() {
        "describe" => describe(request),
        "evaluate" => evaluate(request),
        other => protocol_error(format!("unsupported provider operation `{other}`")),
    }
}

fn describe(request: Value) -> i32 {
    let request = match serde_json::from_value::<DescribeRequest>(request) {
        Ok(request) if request.operation == "describe" => request,
        Ok(_) => return protocol_error("describe request has the wrong operation".into()),
        Err(error) => return protocol_error(format!("invalid describe request: {error}")),
    };
    let _ = request;

    write_json(&workflow::software_change_workflow())
}

fn evaluate(request: Value) -> i32 {
    let request = match serde_json::from_value::<EvaluateRequest>(request) {
        Ok(request) if request.operation == "evaluate" => request,
        Ok(_) => return protocol_error("evaluate request has the wrong operation".into()),
        Err(error) => return protocol_error(format!("invalid evaluate request: {error}")),
    };

    match gates::evaluate(&request) {
        gates::EvaluationOutcome::Response(response) => write_json(&response),
        gates::EvaluationOutcome::EvaluationError(message) => evaluation_error(&message),
    }
}

fn write_json<T: Serialize>(value: &T) -> i32 {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match serde_json::to_writer(&mut stdout, value) {
        Ok(()) => match stdout.flush() {
            Ok(()) => 0,
            Err(error) => protocol_error(format!("could not flush response: {error}")),
        },
        Err(error) => protocol_error(format!("could not write response: {error}")),
    }
}

fn protocol_error(message: String) -> i32 {
    eprintln!("{message}");
    2
}

fn evaluation_error(message: &str) -> i32 {
    eprintln!("{message}");
    1
}
