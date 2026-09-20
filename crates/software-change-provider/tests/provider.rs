#[path = "../../../tests/bounded_process.rs"]
mod bounded_process;

fn reconciliation_fixture() -> serde_json::Value {
    serde_json::json!({
        "revision": "reconciliation-1",
        "author": {"name": "fixture-driver", "kind": "script"},
        "mode": "bookends-disabled",
        "branch": "sufficient-existing-wording",
        "document_observations": [{
            "path": "docs/README.md",
            "status": "unchanged",
            "observation": "The fixture document already matches the approved behavior."
        }],
        "behavior_observations": [{
            "status": "matches-intent",
            "observation": "The fixture behavior matches the approved intent."
        }],
        "action": "no-document-change",
        "action_reason": "Existing wording is sufficient; no document edit is required.",
        "authorization": "not-required",
        "application": "not-required",
        "commit": "not-required",
        "traceability": {"status": "not-applicable", "references": []},
        "proof_references": ["fixture:reconciliation"],
        "blockers": [],
        "decision": "complete"
    })
}

#[path = "provider/backlog_t05.rs"]
mod backlog_t05;
#[path = "provider/backlog_t05_validation.rs"]
mod backlog_t05_validation;
#[path = "provider/backlog_t06.rs"]
mod backlog_t06;
#[path = "provider/backlog_t07.rs"]
mod backlog_t07;
#[path = "provider/backlog_t10.rs"]
mod backlog_t10;
#[path = "provider/bookends_shipped_json.rs"]
mod bookends_shipped_json;
#[path = "provider/describe_protocol.rs"]
mod describe_protocol;
#[path = "provider/embedded_data.rs"]
mod embedded_data;
#[path = "provider/evaluate.rs"]
mod evaluate;
#[path = "provider/shipped_data.rs"]
mod shipped_data;
