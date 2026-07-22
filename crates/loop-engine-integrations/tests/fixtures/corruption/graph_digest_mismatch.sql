-- Logical corruption: graph digest mismatch (registration binding intact).
UPDATE runs
SET graph_revision = 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
WHERE run_id = '019f0000-0000-7000-8000-000000000101';
