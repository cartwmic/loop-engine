-- Logical corruption: transition attempt gate verdict uses unsupported status token.
UPDATE run_journal_sequences
SET next_sequence = 3
WHERE run_id = '019f0000-0000-7000-8000-000000000101';

INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
VALUES (
    '019f0000-0000-7000-8000-000000000101',
    2,
    'rejected',
    '{"journal_schema_version":1,"sequence":2,"run_id":"019f0000-0000-7000-8000-000000000101","ts":"2026-07-17T13:00:00.000Z","operation":"run.request","request_id":"req-request-002","entry_kind":"transition.attempt","outcome":"rejected","reason":{"code":"gate.failed","message":"gate failed"},"state_before":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"state_after":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"transition":{"event":"go","source_state":"draft","target_state":"draft","applied":false},"evidence_associations":{"inline":[],"selected_ids":[],"provider_recorded_ids":[]},"evidence_recorded":{"inline":false,"selected_associations":false,"provider":false},"gate_verdict_facts":{"event":"go","gate_ids":["gate-1"],"verdicts":[{"gate_id":"gate-1","status":"unknown"}]}}'
);
