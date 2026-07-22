-- Logical corruption: run bound to missing registration.
PRAGMA foreign_keys = OFF;
UPDATE runs
SET registration_id = '019f0000-0000-7000-8000-000000000099'
WHERE run_id = '019f0000-0000-7000-8000-000000000101';
PRAGMA foreign_keys = ON;
