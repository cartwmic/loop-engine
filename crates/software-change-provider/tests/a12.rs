#[path = "support/mod.rs"]
mod support;

use serde_json::json;
use support::{base_request, checked, invoke, load_profile};

#[test]
fn malformed_shipped_schema_fails_closed_as_evaluation_error() {
    let mut config = load_profile("standard");
    config["artifact_schemas"]["intent.json"] = json!({
        "type": "object",
        "unknown_keyword": true
    });
    let output = invoke(base_request(
        config,
        checked("explore", "intent-ready", "intent-review"),
    ));

    support::assert_exit(&output, 1);
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("schema-invalid"));
    assert!(stderr.contains("unknown-keyword"));
}
