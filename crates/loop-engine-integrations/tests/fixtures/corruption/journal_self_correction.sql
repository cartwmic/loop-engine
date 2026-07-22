-- Logical corruption: annotation corrects its own sequence (self-correction).
UPDATE run_journal_sequences
SET next_sequence = 3
WHERE run_id = '019f0000-0000-7000-8000-000000000101';

INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
VALUES (
    '019f0000-0000-7000-8000-000000000101',
    2,
    'completed',
    '{"journal_schema_version":1,"sequence":2,"run_id":"019f0000-0000-7000-8000-000000000101","ts":"2026-07-17T13:00:00.000Z","operation":"run.annotate","request_id":"req-annotate-002","entry_kind":"annotation","outcome":"completed","reason":null,"state_before":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"state_after":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"note":"self correction","corrects_sequence":2}'
);
