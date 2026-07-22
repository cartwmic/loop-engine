-- Logical corruption: authoritative run exists but sequence allocator row was deleted.
DELETE FROM run_journal_sequences
WHERE run_id = '019f0000-0000-7000-8000-000000000101';
