-- Logical corruption: creation journal entry deleted while run and allocator remain.
PRAGMA foreign_keys = OFF;
DELETE FROM journal_entries
WHERE run_id = '019f0000-0000-7000-8000-000000000101'
  AND sequence = 1;
PRAGMA foreign_keys = ON;
