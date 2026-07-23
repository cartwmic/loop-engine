//! SQLite migration, corruption, and tombstoned-registration harness setup (T145).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INITIAL_MIGRATION_SQL: &str =
    include_str!("../../../loop-engine-integrations/migrations/0001_initial.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptionKind {
    MalformedDatabaseHeader,
    NotADatabase,
    SchemaFutureVersion,
    IntegrityKeyMissing,
    IntegrityKeyInvalidLength,
    SqlitePhysicalCorruption,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TombstonedRegistrationSetup {
    pub registration_id: String,
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteFixtureError {
    Sqlite3Unavailable,
    CommandFailed {
        argv: Vec<String>,
        stdout: String,
        stderr: String,
        code: Option<i32>,
    },
    Io(String),
}

impl std::fmt::Display for SqliteFixtureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite3Unavailable => formatter.write_str("sqlite3 executable is unavailable"),
            Self::CommandFailed {
                argv,
                stdout,
                stderr,
                code,
            } => write!(
                formatter,
                "sqlite3 command failed (argv={argv:?}, code={code:?}, stdout={stdout}, stderr={stderr})"
            ),
            Self::Io(message) => write!(formatter, "sqlite fixture io error: {message}"),
        }
    }
}

impl std::error::Error for SqliteFixtureError {}

pub fn apply_initial_migration(db_path: &Path) -> Result<(), SqliteFixtureError> {
    if db_path.exists() {
        fs::remove_file(db_path).map_err(|error| SqliteFixtureError::Io(error.to_string()))?;
    }
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|error| SqliteFixtureError::Io(error.to_string()))?;
    }
    run_sqlite(db_path, INITIAL_MIGRATION_SQL)
}

pub fn corrupt_database(db_path: &Path, kind: CorruptionKind) -> Result<(), SqliteFixtureError> {
    match kind {
        CorruptionKind::MalformedDatabaseHeader => fs::write(db_path, b"not-sqlite-header!!!")
            .map_err(|error| SqliteFixtureError::Io(error.to_string())),
        CorruptionKind::NotADatabase => fs::write(db_path, b"definitely not a database")
            .map_err(|error| SqliteFixtureError::Io(error.to_string())),
        CorruptionKind::SchemaFutureVersion => {
            apply_initial_migration(db_path)?;
            run_sqlite(db_path, "PRAGMA user_version = 999;")
        }
        CorruptionKind::IntegrityKeyMissing => {
            apply_initial_migration(db_path)?;
            run_sqlite(
                db_path,
                "DELETE FROM integration_metadata WHERE key = 'integrity_key';",
            )
        }
        CorruptionKind::IntegrityKeyInvalidLength => {
            apply_initial_migration(db_path)?;
            run_sqlite(
                db_path,
                "UPDATE integration_metadata SET value = X'00' WHERE key = 'integrity_key';",
            )
        }
        CorruptionKind::SqlitePhysicalCorruption => {
            apply_initial_migration(db_path)?;
            let mut bytes =
                fs::read(db_path).map_err(|error| SqliteFixtureError::Io(error.to_string()))?;
            if bytes.len() > 128 {
                bytes[64..96].fill(0);
            }
            fs::write(db_path, bytes).map_err(|error| SqliteFixtureError::Io(error.to_string()))
        }
    }
}

pub fn insert_provider_registrations(
    db_path: &Path,
    prefix: &str,
    count: usize,
    argv_json: &str,
) -> Result<(), SqliteFixtureError> {
    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    let argv_json = sql_string(argv_json);
    for index in 0..count {
        let id = sql_string(&format!("{prefix}-{index:04}"));
        sql.push_str(&format!(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
            ) VALUES (
                {id}, {id}, 1, 1, '/bin/false', {argv_json}, '/tmp', 60,
                '2026-07-22T00:00:00.000Z', '2026-07-22T00:00:00.000Z'
            );\n"
        ));
    }
    sql.push_str("COMMIT;\n");
    run_sqlite(db_path, &sql)
}

pub fn count_journal_entries(db_path: &Path) -> Result<u64, SqliteFixtureError> {
    let output = run_sqlite_capture(db_path, "SELECT count(*) FROM journal_entries;")?;
    output
        .trim()
        .parse::<u64>()
        .map_err(|error| SqliteFixtureError::Io(error.to_string()))
}

pub fn tombstone_provider_registration(
    db_path: &Path,
    registration_id: &str,
) -> Result<(), SqliteFixtureError> {
    let registration_id = sql_string(registration_id);
    run_sqlite(
        db_path,
        &format!(
            "UPDATE provider_registrations
             SET handle = NULL, enabled = 0, config_revision = config_revision + 1
             WHERE registration_id = {registration_id};"
        ),
    )
}

pub fn insert_tombstoned_registration(
    db_path: &Path,
    setup: &TombstonedRegistrationSetup,
) -> Result<(), SqliteFixtureError> {
    apply_initial_migration(db_path)?;
    let sql = format!(
        "INSERT INTO provider_registrations (
            registration_id, handle, enabled, config_revision, executable, argv_json,
            working_directory, timeout_seconds, created_at, updated_at
        ) VALUES (
            '{registration_id}', NULL, 0, {config_revision}, '/bin/true', '[]',
            '/tmp', 60, '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z'
        );",
        registration_id = setup.registration_id,
        config_revision = setup.config_revision,
    );
    run_sqlite(db_path, &sql)
}

pub fn validate_tombstoned_registration(
    db_path: &Path,
    registration_id: &str,
) -> Result<TombstonedRegistrationSetup, SqliteFixtureError> {
    let sql = format!(
        "SELECT registration_id, config_revision, enabled, handle
         FROM provider_registrations
         WHERE registration_id = '{registration_id}';"
    );
    let output = run_sqlite_capture(db_path, &sql)?;
    let line = output
        .lines()
        .find(|line| !line.is_empty())
        .ok_or_else(|| SqliteFixtureError::Io("tombstoned registration row missing".into()))?;
    let mut fields = line.split('|');
    let observed_id = fields.next().unwrap_or_default().to_owned();
    let config_revision = fields
        .next()
        .unwrap_or_default()
        .parse::<u64>()
        .map_err(|error| SqliteFixtureError::Io(error.to_string()))?;
    let enabled = fields.next().unwrap_or_default();
    let handle = fields.next();
    if observed_id != registration_id {
        return Err(SqliteFixtureError::Io(format!(
            "expected registration_id {registration_id}, observed {observed_id}"
        )));
    }
    if enabled != "0" {
        return Err(SqliteFixtureError::Io(format!(
            "expected tombstoned enabled=0, observed {enabled}"
        )));
    }
    if handle.is_some_and(|value| !value.is_empty()) {
        return Err(SqliteFixtureError::Io(
            "tombstoned registration must have NULL handle".into(),
        ));
    }
    Ok(TombstonedRegistrationSetup {
        registration_id: observed_id,
        config_revision,
    })
}

pub fn require_sqlite3() -> Result<(), SqliteFixtureError> {
    if sqlite3_available() {
        Ok(())
    } else {
        Err(SqliteFixtureError::Sqlite3Unavailable)
    }
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sqlite3_available() -> bool {
    Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_sqlite(db_path: &Path, sql: &str) -> Result<(), SqliteFixtureError> {
    let output = sqlite_command(db_path, sql)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            vec!["sqlite3".to_owned(), db_path.display().to_string()],
            output,
        ))
    }
}

fn run_sqlite_capture(db_path: &Path, sql: &str) -> Result<String, SqliteFixtureError> {
    let output = sqlite_command(db_path, sql)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(command_error(
            vec!["sqlite3".to_owned(), db_path.display().to_string()],
            output,
        ))
    }
}

fn sqlite_command(db_path: &Path, sql: &str) -> Result<std::process::Output, SqliteFixtureError> {
    let mut child = Command::new("sqlite3")
        .arg(db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| SqliteFixtureError::Sqlite3Unavailable)?;
    child
        .stdin
        .take()
        .expect("sqlite stdin is piped")
        .write_all(sql.as_bytes())
        .map_err(|error| SqliteFixtureError::Io(error.to_string()))?;
    child
        .wait_with_output()
        .map_err(|error| SqliteFixtureError::Io(error.to_string()))
}

fn command_error(argv: Vec<String>, output: std::process::Output) -> SqliteFixtureError {
    SqliteFixtureError::CommandFailed {
        argv,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Harness-only SQLite path under `harness-fixtures/`; never the production `state.db`.
pub fn harness_fixture_db_path(base: &Path, label: &str) -> PathBuf {
    base.join("harness-fixtures")
        .join(format!("{label}.sqlite"))
}
