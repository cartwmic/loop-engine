-- Logical corruption: journal sequence gap (sequence 2 missing, 3 inserted).
INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
VALUES (
    '019f0000-0000-7000-8000-000000000101',
    3,
    'completed',
    '{"journal_schema_version":1,"sequence":3,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"completed"}'
);
