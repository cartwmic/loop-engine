#![allow(clippy::duplicate_mod)]

// Keep each former Cargo integration root in its own namespace.  The source
// files remain at their owning package paths so the migration can be audited
// against the T01 byte inventory; `autotests = false` in those manifests keeps
// Cargo from emitting their former targets.

#[path = "../../../crates/bookends-check/tests/cli.rs"]
mod bookends_check_cli;
#[path = "../../../crates/bookends-check/tests/graph.rs"]
mod bookends_check_graph;
#[path = "../../../crates/loop-cli/tests/dagu.rs"]
mod loop_cli_dagu;
#[path = "../../../crates/loop-cli/tests/engine.rs"]
mod loop_cli_engine;
#[path = "../../../crates/loop-cli/tests/workers.rs"]
mod loop_cli_workers;
#[path = "../../../crates/loop-integrations/tests/concurrency.rs"]
mod loop_integrations_concurrency;
#[path = "../../../crates/loop-integrations/tests/provider_gateway.rs"]
mod loop_integrations_provider_gateway;
#[path = "../../../crates/loop-integrations/tests/sqlite_persistence.rs"]
mod loop_integrations_sqlite_persistence;
#[path = "../../../crates/policy-document-provider/tests/describe_protocol.rs"]
mod policy_document_describe_protocol;
#[path = "../../../tests/fixtures/tests/reference_providers.rs"]
mod reference_fixture_providers;
#[path = "../../../tests/fixtures/tests/reference_workflows.rs"]
mod reference_fixture_workflows;
#[path = "../../../crates/research-provider/tests/cli.rs"]
mod research_cli;
#[path = "../../../crates/research-provider/tests/describe_protocol.rs"]
mod research_describe_protocol;
#[path = "../../../crates/research-provider/tests/embedded_data.rs"]
mod research_embedded_data;
#[path = "../../../crates/research-provider/tests/evaluate.rs"]
mod research_evaluate;
#[path = "../../../crates/research-provider/tests/shipped_data.rs"]
mod research_shipped_data;
#[path = "../../../crates/software-change-provider/tests/cli.rs"]
mod software_change_cli;
#[path = "../../../crates/software-change-provider/tests/contracts.rs"]
mod software_change_contracts;
#[path = "../../../crates/software-change-provider/tests/plan_graph.rs"]
mod software_change_plan_graph;
#[path = "../../../crates/software-change-provider/tests/provider.rs"]
mod software_change_provider;
