-- Logical corruption: unsupported lifecycle enum value.
PRAGMA ignore_check_constraints = ON;
UPDATE runs
SET lifecycle = 'paused'
WHERE run_id = '019f0000-0000-7000-8000-000000000101';
