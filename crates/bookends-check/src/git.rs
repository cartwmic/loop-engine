use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Read a repository tree without giving the caller a checkout to mutate.
///
/// The same small interface is used for the working tree and for immutable
/// commit snapshots.  This keeps historical checks from accidentally reading
/// the branch currently checked out by the caller.
pub(crate) trait TreeReader {
    fn tracked_files(&self) -> Result<Vec<String>, String>;
    fn pathspec_files(&self, pathspecs: &[String]) -> Result<Vec<String>, String>;
    fn read_text(&self, rel: &str) -> Result<Option<String>, String>;
}

pub(crate) struct Worktree<'a> {
    repo: &'a Path,
}

impl<'a> Worktree<'a> {
    pub(crate) fn new(repo: &'a Path) -> Self {
        Self { repo }
    }
}

impl TreeReader for Worktree<'_> {
    fn tracked_files(&self) -> Result<Vec<String>, String> {
        tracked_files(self.repo)
    }

    fn pathspec_files(&self, pathspecs: &[String]) -> Result<Vec<String>, String> {
        pathspec_files(self.repo, pathspecs)
    }

    fn read_text(&self, rel: &str) -> Result<Option<String>, String> {
        read_text(self.repo, rel)
    }
}

/// An immutable commit tree addressed by its full commit identity.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    repo: PathBuf,
    commit: String,
}

impl Snapshot {
    pub(crate) fn new(repo: &Path, commit: impl Into<String>) -> Self {
        Self {
            repo: repo.to_path_buf(),
            commit: commit.into(),
        }
    }

    pub(crate) fn parents(&self) -> Result<Vec<String>, String> {
        commit_parents(&self.repo, &self.commit)
            .map_err(|error| format!("cannot read parents of {}: {error}", self.commit))
    }
}

impl TreeReader for Snapshot {
    fn tracked_files(&self) -> Result<Vec<String>, String> {
        let args = vec![
            "ls-tree".to_owned(),
            "-r".to_owned(),
            "-z".to_owned(),
            "--name-only".to_owned(),
            self.commit.clone(),
        ];
        git_nul_lines_owned(&self.repo, &args)
    }

    fn pathspec_files(&self, pathspecs: &[String]) -> Result<Vec<String>, String> {
        // `ls-tree` does not accept the `:(glob)` magic used by `ls-files`.
        // Enumerate the immutable tree once and apply the small glob subset
        // used by the repository's config instead of falling back to the
        // checked-out worktree.
        let tracked = self.tracked_files()?;
        Ok(tracked
            .into_iter()
            .filter(|path| pathspecs.iter().any(|spec| path_matches_spec(path, spec)))
            .collect())
    }

    fn read_text(&self, rel: &str) -> Result<Option<String>, String> {
        show_blob(&self.repo, &self.commit, rel)
    }
}

pub(crate) fn is_git_repo(repo: &Path) -> bool {
    git_ok(repo, &["rev-parse", "--is-inside-work-tree"])
}

pub(crate) fn tracked_files(repo: &Path) -> Result<Vec<String>, String> {
    git_nul_lines(repo, &["ls-files", "-z"])
}

pub(crate) fn pathspec_files(repo: &Path, pathspecs: &[String]) -> Result<Vec<String>, String> {
    let mut args = vec!["ls-files".to_owned(), "-z".to_owned(), "--".to_owned()];
    args.extend(pathspecs.iter().map(|spec| git_pathspec(spec)));
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    git_nul_lines(repo, &args_ref)
}

pub(crate) fn head_commit(repo: &Path) -> Result<Option<String>, String> {
    match git_output(repo, &["rev-parse", "--verify", "HEAD"]) {
        Ok(commit) => Ok(commit.lines().next().map(str::to_owned)),
        Err(error)
            if error.contains("Needed a single revision")
                || error.contains("does not have any commits yet")
                || error.contains("ambiguous argument 'HEAD'") =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn first_parent(repo: &Path, commit: &str) -> Result<Option<String>, String> {
    // Read the actual commit headers: revision traversal hides parents at a
    // shallow boundary and must not turn that boundary into a root commit.
    let object = git_output(repo, &["cat-file", "commit", commit])?;
    let parent = object
        .lines()
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix("parent "));
    if let Some(parent) = parent {
        git_output(repo, &["cat-file", "commit", parent])
            .map_err(|error| format!("required parent history unavailable ({parent}): {error}"))?;
    }
    Ok(parent.map(str::to_owned))
}

pub(crate) fn read_text(repo: &Path, rel: &str) -> Result<Option<String>, String> {
    match fs::read_to_string(join_repo(repo, rel)) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {rel}: {err}")),
    }
}

pub(crate) fn show_blob(repo: &Path, commit: &str, path: &str) -> Result<Option<String>, String> {
    // Absence is established by a successfully resolved tree, not stderr text.
    git_output(repo, &["cat-file", "commit", commit])?;
    let entry = git_output(
        repo,
        &["ls-tree", "-z", commit, "--", &format!(":(literal){path}")],
    )?;
    if entry.is_empty() {
        return Ok(None);
    }
    git_output(repo, &["show", &format!("{commit}:{path}")]).map(Some)
}

pub(crate) fn resolve_commit(repo: &Path, object: &str) -> Result<String, String> {
    git_output(
        repo,
        &["rev-parse", "--verify", &format!("{object}^{{commit}}")],
    )
    .map(|value| value.lines().next().unwrap_or_default().to_owned())
}

pub(crate) fn is_shallow(repo: &Path) -> Result<bool, String> {
    let value = git_output(repo, &["rev-parse", "--is-shallow-repository"])?;
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "git returned invalid shallow-repository value {other:?}"
        )),
    }
}

/// Fetch only from the configured source remote.  No refspec updates a local
/// branch and `--no-prune` keeps unrelated remote history intact.
pub(crate) fn fetch_unshallow(repo: &Path, remote: &str) -> Result<(), String> {
    let args = vec![
        "fetch".to_owned(),
        "--no-tags".to_owned(),
        "--no-prune".to_owned(),
        "--unshallow".to_owned(),
        remote.to_owned(),
    ];
    git_output_owned(repo, &args).map(|_| ())
}

pub(crate) fn fetch_object(repo: &Path, remote: &str, object: &str) -> Result<(), String> {
    let args = vec![
        "fetch".to_owned(),
        "--no-tags".to_owned(),
        "--no-prune".to_owned(),
        remote.to_owned(),
        object.to_owned(),
    ];
    git_output_owned(repo, &args).map(|_| ())
}

pub(crate) fn commit_parents(repo: &Path, commit: &str) -> Result<Vec<String>, String> {
    let object = git_output(repo, &["cat-file", "commit", commit])?;
    Ok(object
        .lines()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.strip_prefix("parent ").map(str::to_owned))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReachableError {
    Missing { object: String, detail: String },
    Limit { limit: usize },
}

/// Walk commit headers directly so a shallow boundary or a missing parent is
/// an explicit failure instead of an apparent root.  The limit is a safety
/// boundary, not a completeness claim.
pub(crate) fn reachable_commits(
    repo: &Path,
    tips: &[String],
    limit: usize,
) -> Result<BTreeSet<String>, ReachableError> {
    let mut pending: Vec<String> = tips.to_vec();
    let mut seen = BTreeSet::new();
    while let Some(commit) = pending.pop() {
        if !seen.insert(commit.clone()) {
            continue;
        }
        if seen.len() > limit {
            return Err(ReachableError::Limit { limit });
        }
        let parents = commit_parents(repo, &commit).map_err(|detail| ReachableError::Missing {
            object: commit.clone(),
            detail,
        })?;
        pending.extend(parents);
    }
    Ok(seen)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalState {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub local_refs: String,
    pub index: String,
    pub worktree: String,
    pub status: String,
}

/// Capture the caller's meaningful Git state around history acquisition.  A
/// fetch may update its own bookkeeping (for example FETCH_HEAD and the
/// shallow file), but it must not alter any local branch, HEAD, index, or
/// working-tree content.
pub(crate) fn local_state(repo: &Path) -> Result<LocalState, String> {
    let branch = match git_output(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(value) => Some(value.trim().to_owned()),
        Err(_) => None,
    };
    Ok(LocalState {
        head: head_commit(repo)?,
        branch,
        local_refs: git_output(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
            ],
        )?,
        index: git_output(repo, &["ls-files", "-s"])?,
        worktree: git_output(repo, &["diff", "--no-ext-diff", "--raw"])?,
        status: git_output(repo, &["status", "--porcelain=v1", "--untracked-files=all"])?,
    })
}

pub(crate) fn git_pathspec(spec: &str) -> String {
    if spec.starts_with(':') {
        spec.to_owned()
    } else if spec.contains(['*', '?', '[', ']']) {
        format!(":(glob){spec}")
    } else {
        spec.to_owned()
    }
}

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|err| format!("git {}: {err}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn git_output_owned(repo: &Path, args: &[String]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|err| format!("git {}: {err}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn git_nul_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let text = git_output(repo, args)?;
    Ok(text
        .split('\0')
        .filter(|line| !line.is_empty())
        .map(|line| line.replace('\\', "/"))
        .collect())
}

fn git_nul_lines_owned(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    let text = git_output_owned(repo, args)?;
    Ok(text
        .split('\0')
        .filter(|line| !line.is_empty())
        .map(|line| line.replace('\\', "/"))
        .collect())
}

fn path_matches_spec(path: &str, spec: &str) -> bool {
    let normalized = spec.replace('\\', "/");
    let pattern = normalized
        .strip_prefix(":(glob)")
        .or_else(|| normalized.strip_prefix(":(top,glob)"))
        .or_else(|| normalized.strip_prefix(":(literal)"))
        .or_else(|| normalized.strip_prefix(":(top,literal)"))
        .or_else(|| normalized.strip_prefix(":(top)"))
        .unwrap_or(&normalized)
        .trim_start_matches("./");
    if pattern.contains(['*', '?', '[', ']']) {
        glob_path_match(pattern, path)
    } else {
        path == pattern || path.starts_with(&format!("{pattern}/"))
    }
}

fn glob_path_match(pattern: &str, path: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();
    glob_path_parts(&pattern_parts, &path_parts)
}

fn glob_path_parts(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if pattern[0] == "**" {
        return pattern.len() == 1
            || (0..=path.len()).any(|index| glob_path_parts(&pattern[1..], &path[index..]));
    }
    !path.is_empty()
        && component_glob_match(pattern[0], path[0])
        && glob_path_parts(&pattern[1..], &path[1..])
}

fn component_glob_match(pattern: &str, value: &str) -> bool {
    // Configured Bookends pathspecs use ordinary '*'/'?' globs.  Treat a
    // bracket expression conservatively as a literal rather than widening
    // historical eligibility unexpectedly.
    if pattern.contains(['[', ']']) {
        return pattern == value;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut states = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    states[0][0] = true;
    for (i, character) in pattern.iter().enumerate() {
        for j in 0..=value.len() {
            if !states[i][j] {
                continue;
            }
            if *character == '*' {
                states[i + 1][j] = true;
                if j < value.len() {
                    states[i][j + 1] = true;
                }
            } else if j < value.len() && (*character == '?' || *character == value[j]) {
                states[i + 1][j + 1] = true;
            }
        }
    }
    states[pattern.len()][value.len()]
}

pub(crate) fn join_repo(repo: &Path, rel: &str) -> PathBuf {
    repo.join(rel)
}
