-- Baseline seed for T117 logical corruption fixtures.
-- Applied against a migrated empty store before targeted corruption UPDATEs.

INSERT INTO provider_registrations (
    registration_id, handle, enabled, config_revision, executable, argv_json,
    working_directory, timeout_seconds, created_at, updated_at
) VALUES (
    '019f0000-0000-7000-8000-000000000001',
    'provider-a',
    1,
    1,
    '/bin/provider',
    '[]',
    '/work',
    60,
    '2026-07-17T12:00:00.000Z',
    '2026-07-17T12:00:00.000Z'
);

INSERT INTO runs (
    run_id, registration_id, config_revision_at_create, current_state, lifecycle,
    workflow_state_version, lifecycle_version, label_version, label, graph_revision,
    canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
) VALUES (
    '019f0000-0000-7000-8000-000000000101',
    '019f0000-0000-7000-8000-000000000001',
    1,
    'draft',
    'active',
    1,
    1,
    1,
    'corruption-fixture',
    'sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77',
    1,
    '{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}}],"transitions":[]}',
    '{}',
    '2026-07-17T12:00:00.000Z'
);

INSERT INTO run_journal_sequences (run_id, next_sequence)
VALUES ('019f0000-0000-7000-8000-000000000101', 2);

INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
VALUES (
    '019f0000-0000-7000-8000-000000000101',
    1,
    'completed',
    '{"journal_schema_version":1,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","ts":"2026-07-17T12:00:00.000Z","operation":"run.create","request_id":"req-create-001","entry_kind":"run.created","outcome":"completed","reason":null,"state_before":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"state_after":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"provider_observations":[{"registration_id":"019f0000-0000-7000-8000-000000000001","config_revision":1,"role":"describe","invocation_id":"pv-describe-001","executable":"/bin/provider","outcome":"completed"}],"graph_revision":"sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77"}'
);

INSERT INTO evidence (
    run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
) VALUES (
    '019f0000-0000-7000-8000-000000000101',
    'evidence-1',
    'artifact',
    'opaque://fixture',
    NULL,
    NULL,
    NULL,
    'caller',
    '2026-07-17T12:00:01.000Z'
);

INSERT INTO evidence_associations (run_id, journal_sequence, evidence_id, event_id, gate_id)
VALUES (
    '019f0000-0000-7000-8000-000000000101',
    1,
    'evidence-1',
    NULL,
    NULL
);
