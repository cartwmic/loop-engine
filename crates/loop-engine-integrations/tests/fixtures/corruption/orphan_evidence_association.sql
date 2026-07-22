-- Logical corruption: orphan evidence association (evidence row removed).
PRAGMA foreign_keys = OFF;
DELETE FROM evidence
WHERE run_id = '019f0000-0000-7000-8000-000000000101'
  AND evidence_id = 'evidence-1';
PRAGMA foreign_keys = ON;
