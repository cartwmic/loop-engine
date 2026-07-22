-- Logical corruption: sequence 1 exists but is not the run.create / run.created entry.
UPDATE journal_entries
SET encoded_payload_json = '{"journal_schema_version":1,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","ts":"2026-07-17T12:00:00.000Z","operation":"run.annotate","request_id":"req-annotate-001","entry_kind":"annotation","outcome":"completed","reason":null,"state_before":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"state_after":{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1},"note":"not a creation entry"}'
WHERE run_id = '019f0000-0000-7000-8000-000000000101'
  AND sequence = 1;
