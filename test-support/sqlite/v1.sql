-- Frozen external v1 database fixture. Do not replace from current migrations.
-- Schema body originated from migrations/0001_initial.sql at WP4 closure.

-- Loop Engine persistence schema v0001 (T105).
-- Transaction wrapper and version recording are owned by rusqlite_migration.

CREATE TABLE integration_metadata (
    key TEXT NOT NULL PRIMARY KEY CHECK (key = 'integrity_key'),
    value BLOB NOT NULL CHECK (length(value) = 32)
);

INSERT INTO integration_metadata (key, value)
VALUES ('integrity_key', randomblob(32));

CREATE TABLE provider_registrations (
    registration_id TEXT NOT NULL PRIMARY KEY CHECK (length(registration_id) > 0),
    handle TEXT CHECK (handle IS NULL OR length(handle) > 0),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    config_revision INTEGER NOT NULL CHECK (config_revision > 0),
    executable TEXT NOT NULL CHECK (length(executable) > 0),
    argv_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(argv_json)),
    working_directory TEXT NOT NULL CHECK (length(working_directory) > 0),
    timeout_seconds INTEGER NOT NULL CHECK (timeout_seconds > 0),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
    CHECK (
        (enabled = 1 AND handle IS NOT NULL)
        OR (enabled = 0 AND handle IS NULL)
    )
);

CREATE UNIQUE INDEX idx_provider_registrations_handle_enabled
    ON provider_registrations (handle)
    WHERE enabled = 1;

CREATE INDEX idx_provider_registrations_created_id
    ON provider_registrations (created_at, registration_id);

CREATE INDEX idx_provider_registrations_enabled_created_id
    ON provider_registrations (created_at, registration_id)
    WHERE enabled = 1;

CREATE TABLE runs (
    run_id TEXT NOT NULL PRIMARY KEY CHECK (length(run_id) > 0),
    registration_id TEXT NOT NULL
        REFERENCES provider_registrations (registration_id)
        ON DELETE RESTRICT,
    config_revision_at_create INTEGER NOT NULL CHECK (config_revision_at_create > 0),
    current_state TEXT NOT NULL CHECK (length(current_state) > 0),
    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('active', 'final', 'terminated')),
    workflow_state_version INTEGER NOT NULL CHECK (workflow_state_version > 0),
    lifecycle_version INTEGER NOT NULL CHECK (lifecycle_version > 0),
    label_version INTEGER NOT NULL CHECK (label_version > 0),
    label TEXT,
    graph_revision TEXT NOT NULL CHECK (length(graph_revision) > 0),
    canonical_graph_version INTEGER NOT NULL CHECK (canonical_graph_version = 1),
    graph_canonical_projection_json TEXT NOT NULL CHECK (json_valid(graph_canonical_projection_json)),
    inputs_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(inputs_json)),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    CHECK (label IS NULL OR length(label) > 0)
);

CREATE INDEX idx_runs_registration_lifecycle_id
    ON runs (registration_id, lifecycle, run_id);

CREATE INDEX idx_runs_registration_active_id
    ON runs (registration_id, run_id)
    WHERE lifecycle = 'active';

CREATE INDEX idx_runs_created_id
    ON runs (created_at, run_id);

CREATE TABLE run_journal_sequences (
    run_id TEXT NOT NULL PRIMARY KEY
        REFERENCES runs (run_id)
        ON DELETE RESTRICT,
    next_sequence INTEGER NOT NULL CHECK (next_sequence >= 1)
);

CREATE TABLE evidence (
    run_id TEXT NOT NULL
        REFERENCES runs (run_id)
        ON DELETE RESTRICT,
    evidence_id TEXT NOT NULL CHECK (length(evidence_id) > 0),
    kind TEXT NOT NULL CHECK (length(kind) > 0),
    locator TEXT NOT NULL CHECK (length(locator) > 0),
    digest TEXT CHECK (digest IS NULL OR length(digest) > 0),
    media_type TEXT CHECK (media_type IS NULL OR length(media_type) > 0),
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    source TEXT NOT NULL CHECK (source IN ('caller', 'provider')),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    PRIMARY KEY (run_id, evidence_id)
);

CREATE INDEX idx_evidence_run_created_id
    ON evidence (run_id, created_at, evidence_id);

CREATE TABLE journal_entries (
    run_id TEXT NOT NULL
        REFERENCES runs (run_id)
        ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'rejected', 'error')),
    encoded_payload_json TEXT NOT NULL CHECK (json_valid(encoded_payload_json)),
    PRIMARY KEY (run_id, sequence)
);

CREATE TABLE evidence_associations (
    run_id TEXT NOT NULL,
    journal_sequence INTEGER NOT NULL CHECK (journal_sequence > 0),
    evidence_id TEXT NOT NULL CHECK (length(evidence_id) > 0),
    event_id TEXT CHECK (event_id IS NULL OR length(event_id) > 0),
    gate_id TEXT CHECK (gate_id IS NULL OR length(gate_id) > 0),
    PRIMARY KEY (run_id, journal_sequence, evidence_id),
    FOREIGN KEY (run_id, journal_sequence)
        REFERENCES journal_entries (run_id, sequence)
        ON DELETE RESTRICT,
    FOREIGN KEY (run_id, evidence_id)
        REFERENCES evidence (run_id, evidence_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_evidence_associations_run_journal
    ON evidence_associations (run_id, journal_sequence);

CREATE INDEX idx_evidence_associations_run_evidence
    ON evidence_associations (run_id, evidence_id);

PRAGMA user_version = 1;
