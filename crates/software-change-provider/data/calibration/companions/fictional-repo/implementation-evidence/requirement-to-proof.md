# NEW exact requirement-to-proof matrix

[docs/PRD.md](../docs/PRD.md) remains product authority. These are its four exact
acceptance requirements, not new obligations. Names below exist in NEW source.
The executed public outcome path is `scripts/production-journey.py::main`; its
four phases are described explicitly, not attributed to nonexistent helpers.

| Requirement | Truthful named proof | Boundary |
|---|---|---|
| A caller can inspect the frozen policies and artifact schemas before evaluating a transition. | `provider/tests/describe_protocol.rs::describe_matches_existing_component_byte_for_byte`; `provider/tests/opaque_describe.rs::optional_opaque_initial_input_is_ignored_without_allowing_other_fields`; `provider/tests/shipped_data.rs::all_profiles_pass_production_config_validation_and_have_exact_subjects`; `scripts/production-journey.py::main` | Actual installed0.25.0 start/full-show preserves every submitted canonical field plus allocated artifact_root, then checks frozen equality after each event; describe is exact unchanged topology, not selected policies. |
| A malformed subject artifact is denied with each violated structural rule named. | `provider/src/schema.rs::instance_collects_object_rules_and_nested_properties`; `provider/tests/evaluate.rs::schema_denial_projects_all_applicable_rules_before_evidence`; `provider/tests/a2.rs::absent_artifact_is_schema_deny`; `provider/tests/a2.rs::unparseable_artifact_is_schema_deny`; `provider/tests/boundary_v6.rs::revision_link_mismatch_names_both_actual_artifacts`; `provider/tests/boundary_v6.rs::artifact_escape_is_provider_error_not_allow_or_generic_deny`; `scripts/production-journey.py::main` | Actual CLI missing/unparseable and simultaneous required (containing object, named revision), minLength and additionalProperties denials precede review evidence and leave state unchanged. Process tests separately vary evidence while preserving structural denial. |
| A checked transition refuses stale, self-authored, duplicate-author, or configured-obligation-incomplete evidence. | `provider/src/evidence.rs::stale_revision_does_not_satisfy_and_names_both_revisions`; `provider/src/evidence.rs::stale_config_does_not_satisfy_and_names_both_versions`; `provider/src/evidence.rs::subject_author_identity_requires_exact_name_and_kind_pair`; `provider/src/evidence.rs::n_two_requires_distinct_non_subject_authors_and_rejects_standing_fail`; `provider/src/evidence.rs::latest_same_author_verdict_supersession`; `provider/src/evidence.rs::missing_config_after_sufficient_coverage_blocks_until_conforming_repair`; `provider/src/evidence.rs::missing_or_malformed_required_identity_cannot_masquerade_as_stale`; `provider/src/evidence.rs::conforming_wrong_string_version_keeps_coverage_and_never_counts`; `provider/src/evidence.rs::stale_string_keeps_original_author_verdict_filter_order`; `scripts/production-journey.py::main` | Actual public journey creates qualifying deficits before bad contributions, repairs only missing coverage, tests duplicates only at canonical N2 gates, and proves standing fail is not outvoted. Standard intent N1 is not dishonestly claimed to require two authors. |
| The reference workflow reaches its terminal state only after its configured validation obligations are satisfied. | `provider/tests/a9.rs::terminal_deficit_refused`; `provider/tests/a15.rs::terminal_complete_current_obligations_allow`; `provider/tests/boundary_v6.rs::every_reference_tuple_is_routed_without_inventing_an_allowlist`; `scripts/production-journey.py::main` | Root genuine receipt proves all three profiles at end/final/completed, no events/overrides, complete current coverage and exact retained denial/history lineage. Synthetic judgments are mechanics, never semantic quality. |

The supplied NEW source inventory lists these actual names and content hashes.
`scripts/assert-requirement-proof.py --receipt ROOT_RECEIPT --binding RUNTIME_BINDING --repair-proof ROOT_REPAIR_RECEIPT --evidence-proof ROOT_EVIDENCE_RECEIPT`
requires all four exact rows, real functions, matching inventory, a real passed
three-profile receipt, explicit unchanged-domain reuse, and the separately proved
linked-parent runtime repair and separately proved required revision/config identity guard. The latter preserves original stale-string author/verdict filter order, author floors and standings, with six public refusal/restoration controls. Original execution source identities are retained;
document-only successors do not relabel earlier receipts. It rejects a
journey that preloads complete coverage instead of an explicit deficit/repair.
The legacy illustrative argument/setup mapping is retained separately in
[journey-setup-binding.json](journey-setup-binding.json); old invocation claims
are not relabeled as executions. Documentation semantic adequacy is a separate
driver/reviewer judgment; authority/link mechanics alone do not establish it.
