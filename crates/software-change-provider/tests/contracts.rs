#[path = "../../../tests/bounded_process.rs"]
mod bounded_process;
mod support;

#[path = "contracts/a1.rs"]
mod a1;
#[path = "contracts/a11.rs"]
mod a11;
#[path = "contracts/a12.rs"]
mod a12;
#[path = "contracts/a13.rs"]
mod a13;
#[path = "contracts/a14.rs"]
mod a14;
#[path = "contracts/a15.rs"]
mod a15;
#[path = "contracts/a2.rs"]
mod a2;
#[path = "contracts/a3.rs"]
mod a3;
#[path = "contracts/a5.rs"]
mod a5;
#[path = "contracts/a7.rs"]
mod a7;
#[path = "contracts/a8.rs"]
mod a8;
#[path = "contracts/a9.rs"]
mod a9;
#[path = "contracts/bookends_overlay.rs"]
mod bookends_overlay;
#[path = "contracts/bookends_overlay_off.rs"]
mod bookends_overlay_off;
#[path = "contracts/finding_ledger.rs"]
mod finding_ledger;
#[path = "contracts/linkage.rs"]
mod linkage;
#[path = "contracts/process_timeout_probe.rs"]
mod process_timeout_probe;
#[path = "contracts/r25.rs"]
mod r25;

#[test]
fn former_sources_manifest_is_complete() {
    const SOURCES: &[(&str, &str)] = &[
        ("a1.rs", "contracts"),
        ("a2.rs", "contracts"),
        ("a3.rs", "contracts"),
        ("a5.rs", "contracts"),
        ("a7.rs", "contracts"),
        ("a8.rs", "contracts"),
        ("a9.rs", "contracts"),
        ("a11.rs", "contracts"),
        ("a12.rs", "contracts"),
        ("a13.rs", "contracts"),
        ("a14.rs", "contracts"),
        ("a15.rs", "contracts"),
        ("bookends_overlay.rs", "contracts"),
        ("bookends_overlay_off.rs", "contracts"),
        ("finding_ledger.rs", "contracts"),
        ("linkage.rs", "contracts"),
        ("r25.rs", "contracts"),
        ("bookends_shipped_json.rs", "provider"),
        ("describe_protocol.rs", "provider"),
        ("embedded_data.rs", "provider"),
        ("evaluate.rs", "provider"),
        ("shipped_data.rs", "provider"),
        ("cli.rs", "cli"),
        ("dagu_resolver.rs", "cli"),
        ("stdin_exec.rs", "cli"),
        ("run_plan_graph.rs", "plan_graph"),
        ("carry.rs", "engine"),
        ("change_report.rs", "engine"),
        ("invocation_progress.rs", "engine"),
        ("invoke.rs", "engine"),
        ("fan_out.rs", "workers"),
        ("stdin_exec.rs", "workers"),
        ("wait_invocation.rs", "workers"),
        ("dagu_resolver.rs", "dagu"),
    ];
    assert_eq!(SOURCES.len(), 34);
    assert_eq!(
        SOURCES
            .iter()
            .filter(|(_, suite)| *suite == "contracts")
            .count(),
        17
    );
    assert_eq!(
        SOURCES
            .iter()
            .filter(|(_, suite)| *suite == "provider")
            .count(),
        5
    );
    assert_eq!(
        SOURCES.iter().filter(|(_, suite)| *suite == "cli").count(),
        3
    );
    assert_eq!(
        SOURCES
            .iter()
            .filter(|(_, suite)| *suite == "plan_graph")
            .count(),
        1
    );
    assert_eq!(
        SOURCES
            .iter()
            .filter(|(_, suite)| *suite == "engine")
            .count(),
        4
    );
    assert_eq!(
        SOURCES
            .iter()
            .filter(|(_, suite)| *suite == "workers")
            .count(),
        3
    );
    assert_eq!(
        SOURCES.iter().filter(|(_, suite)| *suite == "dagu").count(),
        1
    );
}
