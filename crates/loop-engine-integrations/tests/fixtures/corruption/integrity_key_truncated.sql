-- Logical corruption: integrity key wrong length.
PRAGMA ignore_check_constraints = ON;
UPDATE integration_metadata
SET value = randomblob(16)
WHERE key = 'integrity_key';
