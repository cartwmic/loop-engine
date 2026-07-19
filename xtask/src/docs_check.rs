use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

const DOCS_DIR: &str = "docs";

/// Foundation and contract markdown that must exist under the repository root.
const REQUIRED_DOC_FILES: &[&str] = &[
    "docs/intent.md",
    "docs/tenets.md",
    "docs/invariants.md",
    "docs/architecture.md",
    "docs/testing.md",
    "docs/technology.md",
    "docs/reference-workflow.md",
    "docs/ux-storyboards.md",
    "docs/development-policy.md",
    "docs/cli-contract.md",
    "docs/configuration.md",
    "docs/operation-catalog.md",
    "docs/provider-protocol-v1.md",
    "docs/persistence.md",
    "docs/journal-contract.md",
    "docs/operational-trace.md",
    "docs/graph-projection.md",
    "docs/export-contract.md",
    "docs/change/initial-implementation/README.md",
    "docs/change/initial-implementation/decisions.md",
    "docs/change/initial-implementation/tasks.md",
    "docs/change/initial-implementation/coverage.md",
];

struct FrozenTermRule {
    pattern: &'static str,
    guidance: &'static str,
}

const FROZEN_TERMINOLOGY_RULES: &[FrozenTermRule] = &[
    FrozenTermRule {
        pattern: "loop engine",
        guidance: "use `loop-engine`",
    },
    FrozenTermRule {
        pattern: "LoopEngine",
        guidance: "use `loop-engine`",
    },
    FrozenTermRule {
        pattern: "loop_engine",
        guidance: "use `loop-engine` or `LOOP_ENGINE_HOME`",
    },
    FrozenTermRule {
        pattern: "loop-engine_core",
        guidance: "use `loop-engine-core`",
    },
    FrozenTermRule {
        pattern: "loop_engine-core",
        guidance: "use `loop-engine-core`",
    },
    FrozenTermRule {
        pattern: "loop_engine_core",
        guidance: "use `loop-engine-core`",
    },
    FrozenTermRule {
        pattern: "loop-engine_integrations",
        guidance: "use `loop-engine-integrations`",
    },
    FrozenTermRule {
        pattern: "loop_engine-integrations",
        guidance: "use `loop-engine-integrations`",
    },
    FrozenTermRule {
        pattern: "loop_engine_integrations",
        guidance: "use `loop-engine-integrations`",
    },
    FrozenTermRule {
        pattern: "loop-engine_cli",
        guidance: "use `loop-engine-cli`",
    },
    FrozenTermRule {
        pattern: "loop_engine-cli",
        guidance: "use `loop-engine-cli`",
    },
    FrozenTermRule {
        pattern: "loop_engine_cli",
        guidance: "use `loop-engine-cli`",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    category: &'static str,
    path: String,
    line: Option<usize>,
    detail: String,
}

/// Verify deterministic documentation policy for the repository rooted at `root`.
///
/// When `root` is `None`, the repository root is resolved from the current directory.
pub fn run(root: Option<&Path>) -> Result<()> {
    let root = resolve_root(root)?;
    let mut violations = Vec::new();

    check_required_files(&root, &mut violations);
    check_markdown_files(&root, &mut violations)?;

    if violations.is_empty() {
        return Ok(());
    }

    violations.sort();
    bail!(format_violations(violations));
}

/// Repository root used by the canonical `docs-check` command.
pub fn default_repository_root() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn resolve_root(root: Option<&Path>) -> Result<Utf8PathBuf> {
    let path = match root {
        Some(path) => Utf8PathBuf::from_path_buf(path.to_path_buf())
            .map_err(|_| anyhow::anyhow!("repository root is not valid UTF-8"))?,
        None => find_repository_root().unwrap_or_else(default_repository_root),
    };

    if !path.is_dir() {
        bail!("repository root does not exist: {}", path);
    }

    canonicalize_utf8_path(&path)
}

fn find_repository_root() -> Option<Utf8PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        let docs = current.join(DOCS_DIR);
        let manifest = current.join("Cargo.toml");
        if manifest.is_file() && docs.is_dir() {
            return Utf8PathBuf::from_path_buf(current).ok();
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn check_required_files(root: &Utf8Path, violations: &mut Vec<Violation>) {
    for relative in REQUIRED_DOC_FILES {
        let path = root.join(relative);
        if !path.is_file() {
            violations.push(Violation {
                category: "required-file",
                path: relative.to_string(),
                line: None,
                detail: format!("missing required documentation file `{relative}`"),
            });
        }
    }
}

fn check_markdown_files(root: &Utf8Path, violations: &mut Vec<Violation>) -> Result<()> {
    let docs_root = root.join(DOCS_DIR);
    if !docs_root.is_dir() {
        violations.push(Violation {
            category: "required-file",
            path: DOCS_DIR.to_string(),
            line: None,
            detail: format!("missing required `{DOCS_DIR}/` directory"),
        });
        return Ok(());
    }

    let mut markdown_paths = Vec::new();
    for entry in WalkDir::new(&docs_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let utf8 = Utf8Path::from_path(path)
            .with_context(|| format!("markdown path is not valid UTF-8: {}", path.display()))?;
        markdown_paths.push(utf8.to_path_buf());
    }
    markdown_paths.sort();

    for path in markdown_paths {
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                violations.push(Violation {
                    category: "utf-8",
                    path: relative.clone(),
                    line: None,
                    detail: format!("failed to read markdown file: {error}"),
                });
                continue;
            }
        };

        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) => {
                let invalid_byte = bytes.get(error.valid_up_to()).copied().unwrap_or(0);
                violations.push(Violation {
                    category: "utf-8",
                    path: relative.clone(),
                    line: None,
                    detail: format!(
                        "markdown file is not valid UTF-8 (invalid byte 0x{invalid_byte:02x} at offset {})",
                        error.valid_up_to()
                    ),
                });
                continue;
            }
        };

        if !text.ends_with('\n') {
            violations.push(Violation {
                category: "final-newline",
                path: relative.clone(),
                line: None,
                detail: "markdown file must end with a single trailing newline".to_string(),
            });
        }

        for (line_number, line) in text.lines().enumerate() {
            if has_trailing_whitespace(line) {
                violations.push(Violation {
                    category: "trailing-whitespace",
                    path: relative.clone(),
                    line: Some(line_number + 1),
                    detail: "line has trailing whitespace".to_string(),
                });
            }
        }

        for (target, offset) in relative_link_targets(text) {
            if target.contains("://") {
                continue;
            }
            let resolved = path.parent().map(|parent| parent.join(&target));
            let exists = resolved
                .as_ref()
                .map(|candidate| candidate.is_file() || candidate.is_dir())
                .unwrap_or(false);
            if !exists {
                violations.push(Violation {
                    category: "relative-link",
                    path: relative.clone(),
                    line: line_number_for_offset(text, offset),
                    detail: format!("relative link target does not exist: `{target}`"),
                });
            }
        }

        let mut heading_counts = BTreeMap::<(usize, String), usize>::new();
        for line in text.lines() {
            if let Some((level, title)) = markdown_heading(line) {
                *heading_counts.entry((level, title)).or_insert(0) += 1;
            }
        }
        for ((level, title), count) in heading_counts {
            if count > 1 {
                violations.push(Violation {
                    category: "duplicate-heading",
                    path: relative.clone(),
                    line: None,
                    detail: format!(
                        "duplicate level-{level} heading `{title}` appears {count} times"
                    ),
                });
            }
        }

        let mut in_fence = false;
        for (line_number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence || is_markdown_heading_line(trimmed) {
                continue;
            }

            let prose = strip_markdown_links(&strip_inline_code(line));
            for rule in FROZEN_TERMINOLOGY_RULES {
                for matched in find_case_insensitive_matches(&prose, rule.pattern) {
                    violations.push(Violation {
                        category: "frozen-terminology",
                        path: relative.clone(),
                        line: Some(line_number + 1),
                        detail: format!("forbidden terminology `{matched}`: {}", rule.guidance),
                    });
                }
            }
        }
    }

    Ok(())
}

fn has_trailing_whitespace(line: &str) -> bool {
    matches!(line.as_bytes().last(), Some(b' ' | b'\t'))
}

fn relative_link_targets(text: &str) -> Vec<(String, usize)> {
    let mut targets = Vec::new();
    let mut search_from = 0usize;

    while let Some(start) = text[search_from..].find('[') {
        let absolute_start = search_from + start;
        let after_label = match text[absolute_start..].find(']') {
            Some(offset) => absolute_start + offset + 1,
            None => break,
        };
        if text.as_bytes().get(after_label) != Some(&b'(') {
            search_from = after_label;
            continue;
        }
        let target_start = after_label + 1;
        let target_end = text[target_start..]
            .find([')', '#'])
            .map(|offset| target_start + offset)
            .unwrap_or(text.len());
        let target = text[target_start..target_end].to_string();
        targets.push((target, absolute_start));
        search_from = target_end;
    }

    targets
}

fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = trimmed[level..].trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((level, rest.to_ascii_lowercase()))
}

fn is_markdown_heading_line(trimmed: &str) -> bool {
    markdown_heading(trimmed).is_some()
}

fn strip_inline_code(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut in_code = false;

    for ch in line.chars() {
        if ch == '`' {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            output.push(ch);
        }
    }

    output
}

fn strip_markdown_links(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '[' {
            output.push(ch);
            continue;
        }

        let mut label = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == ']' {
                closed = true;
                break;
            }
            label.push(next);
        }
        if !closed {
            output.push('[');
            output.push_str(&label);
            break;
        }

        match chars.next() {
            Some('(') => {
                for next in chars.by_ref() {
                    if next == ')' {
                        break;
                    }
                }
                output.push_str(&label);
            }
            _ => {
                output.push('[');
                output.push_str(&label);
                output.push(']');
            }
        }
    }

    output
}

fn find_case_insensitive_matches(haystack: &str, needle: &str) -> Vec<String> {
    if needle.is_empty() {
        return Vec::new();
    }

    let lower_haystack = haystack.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut search_from = 0usize;

    while let Some(start) = lower_haystack[search_from..].find(&lower_needle) {
        let absolute_start = search_from + start;
        let absolute_end = absolute_start + needle.len();
        if is_word_boundary(&lower_haystack, absolute_start, absolute_end) {
            matches.push(haystack[absolute_start..absolute_end].to_string());
        }
        search_from = absolute_start + 1;
    }

    matches
}

fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !after.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn line_number_for_offset(text: &str, offset: usize) -> Option<usize> {
    if offset > text.len() {
        return None;
    }
    Some(text[..offset].lines().count())
}

fn format_violations(violations: Vec<Violation>) -> String {
    let mut message = String::from("documentation check failed:\n");
    for violation in violations {
        match violation.line {
            Some(line) => message.push_str(&format!(
                "- [{}] {}:{}: {}\n",
                violation.category, violation.path, line, violation.detail
            )),
            None => message.push_str(&format!(
                "- [{}] {}: {}\n",
                violation.category, violation.path, violation.detail
            )),
        }
    }
    message
}

fn canonicalize_utf8_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let canonical = path
        .as_std_path()
        .canonicalize()
        .with_context(|| format!("failed to canonicalize path {path}"))?;
    Utf8PathBuf::from_path_buf(canonical)
        .map_err(|_| anyhow::anyhow!("canonical path is not valid UTF-8: {path}"))
}
