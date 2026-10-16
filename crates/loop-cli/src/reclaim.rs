//! Explicit legacy catalog reclaim: preview by default, `--apply` to clear and shrink.
use super::{resolve_paths, CliError, CliOptions, Execution, OutputFormat, EXIT_COMPLETED};
use loop_integrations::reclaim;
use serde_json::json;
use std::path::PathBuf;

const HELP: &str = "Usage: loop-engine [--database DB] [--json] reclaim [--apply]\n\nReports which final or terminated runs still hold bulky legacy work-slot invocation\npayloads (worker commands, prompts, completion snapshots) and how many bytes clearing\nthem would reclaim. Without --apply nothing is written.\n\n--apply clears those payload columns for final and terminated runs only, then runs\nVACUUM so the catalog file shrinks. Active runs are never touched and cleaned runs stay\nlisted and showable. Other commands may briefly wait or fail with a retryable busy\nerror while the file is rewritten. Reclaim never runs automatically.\n";

pub(super) fn dispatch(args: &[String]) -> Option<Execution> {
    let mut json = false;
    let mut database = None;
    let mut apply = false;
    let mut seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "reclaim" if !seen => seen = true,
            "--json" | "-j" | "--machine-readable" => json = true,
            "--apply" if seen => apply = true,
            "--help" | "-h" if seen => {
                return Some(Execution {
                    exit_code: EXIT_COMPLETED,
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                })
            }
            "--database" => {
                database = Some(PathBuf::from(args.get(index + 1)?));
                index += 1;
            }
            token if token.starts_with("--database=") => {
                database = Some(PathBuf::from(&token["--database=".len()..]));
            }
            token if token.starts_with("--format=") || token.starts_with("--output=") => {
                json = token.ends_with("json") || token.ends_with("machine");
            }
            "--config" | "--timeout-ms" | "--provider-timeout-ms" if !seen => index += 1,
            token if seen => {
                let output = if json {
                    OutputFormat::Json
                } else {
                    OutputFormat::Human
                };
                return Some(super::render_invalid_invocation_with_format(
                    CliError::new(
                        "invalid-invocation",
                        format!(
                            "unknown reclaim argument `{token}`; use --apply, --database, --json"
                        ),
                    ),
                    output,
                ));
            }
            _ => return None,
        }
        index += 1;
    }
    if !seen {
        return None;
    }
    let output = if json {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };
    Some(execute(database, apply, output))
}

fn execute(database: Option<PathBuf>, apply: bool, output: OutputFormat) -> Execution {
    let options = CliOptions {
        output,
        database,
        ..CliOptions::default()
    };
    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return super::render_operation_error("reclaim", output, error),
    };
    let result = if apply {
        reclaim::apply(&paths.database).map(|applied| {
            let human = format!(
                "reclaim applied to {}\ncleared {} invocation rows across {} finished runs\ncatalog file: {} -> {} bytes (wal {} -> {})\n",
                applied.database.display(),
                applied.invocation_rows_cleared,
                applied.runs.len(),
                applied.before.file_bytes,
                applied.after.file_bytes,
                applied.before.wal_bytes,
                applied.after.wal_bytes,
            );
            (json!({"mode": "apply", "applied": applied}), human)
        })
    } else {
        reclaim::preview(&paths.database).map(|preview| {
            let mut human = format!(
                "reclaim preview for {} (nothing written)\n{} finished runs, {} invocation rows, about {} reclaimable bytes; catalog file {} bytes (wal {})\n",
                preview.database.display(),
                preview.runs.len(),
                preview.invocation_rows,
                preview.reclaimable_bytes,
                preview.file_bytes,
                preview.wal_bytes,
            );
            for run in &preview.runs {
                human.push_str(&format!(
                    "  {} ({}): {} rows, {} bytes\n",
                    run.run_id, run.lifecycle, run.invocation_rows, run.reclaimable_bytes
                ));
            }
            human.push_str("run again with --apply to clear and shrink\n");
            (json!({"mode": "preview", "preview": preview}), human)
        })
    };
    match result {
        Ok((value, human)) => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: match output {
                OutputFormat::Json => format!(
                    "{}\n",
                    json!({"operation": "reclaim", "status": "completed", "result": value})
                ),
                OutputFormat::Human => human,
            },
            stderr: String::new(),
        },
        Err(error) => super::render_operation_error(
            "reclaim",
            output,
            CliError::new(error.code, error.message),
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::{execute, EXIT_COMPLETED, EXIT_INVALID_INVOCATION};
    use serde_json::Value;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "loop-cli-reclaim-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    #[test]
    fn reclaim_previews_by_default_and_applies_only_when_asked() {
        let directory = scratch("preview");
        let database = directory.join("loop.db");
        loop_integrations::SqlitePersistence::open(&database).expect("create catalog");
        let database = database.display().to_string();

        let preview = execute(["--database", &database, "--json", "reclaim"]);
        assert_eq!(preview.exit_code, EXIT_COMPLETED, "{}", preview.stdout);
        let value: Value = serde_json::from_str(&preview.stdout).expect("json");
        assert_eq!(value["operation"], "reclaim");
        assert_eq!(value["result"]["mode"], "preview");

        let applied = execute(["--json", "--database", &database, "reclaim", "--apply"]);
        assert_eq!(applied.exit_code, EXIT_COMPLETED, "{}", applied.stdout);
        let value: Value = serde_json::from_str(&applied.stdout).expect("json");
        assert_eq!(value["result"]["mode"], "apply");
        assert_eq!(value["result"]["applied"]["invocation_rows_cleared"], 0);

        let human = execute(["--database", &database, "reclaim"]);
        assert!(human.stdout.contains("nothing written"), "{}", human.stdout);

        let help = execute(["reclaim", "--help"]);
        assert!(help.stdout.contains("--apply"));

        let unknown = execute(["--database", &database, "reclaim", "--everything"]);
        assert_eq!(
            unknown.exit_code, EXIT_INVALID_INVOCATION,
            "{}",
            unknown.stdout
        );
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn reclaim_of_missing_catalog_is_a_clear_error() {
        let directory = scratch("missing");
        let database = directory.join("absent.db").display().to_string();
        let result = execute(["--database", &database, "--json", "reclaim"]);
        assert_ne!(result.exit_code, EXIT_COMPLETED);
        assert!(
            result.stdout.contains("catalog-not-found"),
            "{}",
            result.stdout
        );
        std::fs::remove_dir_all(directory).ok();
    }
}
