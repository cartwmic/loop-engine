use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    match git_output(repo, &["rev-parse", "--verify", &format!("{commit}^1")]) {
        Ok(parent) => Ok(parent.lines().next().map(str::to_owned)),
        Err(error)
            if error.contains("Needed a single revision")
                || error.contains("unknown revision")
                || error.contains("bad revision") =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn read_text(repo: &Path, rel: &str) -> Result<Option<String>, String> {
    match fs::read_to_string(join_repo(repo, rel)) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {rel}: {err}")),
    }
}

pub(crate) fn show_blob(repo: &Path, commit: &str, path: &str) -> Result<Option<String>, String> {
    let spec = format!("{commit}:{path}");
    match git_output(repo, &["show", &spec]) {
        Ok(text) => Ok(Some(text)),
        Err(err)
            if err.contains("does not exist") || err.contains("exists on disk, but not in") =>
        {
            Ok(None)
        }
        Err(err) if err.contains("bad revision") || err.contains("invalid object") => Ok(None),
        Err(err) => Err(err),
    }
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

fn git_nul_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let text = git_output(repo, args)?;
    Ok(text
        .split('\0')
        .filter(|line| !line.is_empty())
        .map(|line| line.replace('\\', "/"))
        .collect())
}

pub(crate) fn join_repo(repo: &Path, rel: &str) -> PathBuf {
    repo.join(rel)
}
