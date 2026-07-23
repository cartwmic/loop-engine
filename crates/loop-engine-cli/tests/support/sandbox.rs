//! Isolated E2E sandbox with private machine-local roots (T143).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tempfile::TempDir;

use super::cli::CliRunner;

/// Environment variables cleared so invocations never inherit caller machine config.
const ISOLATED_ENV_REMOVALS: &[&str] = &[
    "LOOP_ENGINE_HOME",
    "HOME",
    "USERPROFILE",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// Private black-box sandbox: unique `LOOP_ENGINE_HOME`, caller CWD, and provider CWD.
pub struct E2eSandbox {
    home: Option<TempDir>,
    home_path: PathBuf,
    caller_cwd: Option<TempDir>,
    caller_cwd_path: PathBuf,
    provider_cwd: Option<TempDir>,
    provider_cwd_path: PathBuf,
    transcripts_dir: PathBuf,
    next_transcript: AtomicU64,
}

impl E2eSandbox {
    pub fn new() -> Self {
        let home = TempDir::new().expect("e2e LOOP_ENGINE_HOME tempdir");
        let home_path = home.path().to_path_buf();
        fs::create_dir_all(home_path.join("traces")).expect("trace directory");
        let caller_cwd = TempDir::new().expect("caller cwd tempdir");
        let caller_cwd_path = caller_cwd.path().to_path_buf();
        let provider_cwd = TempDir::new().expect("provider cwd tempdir");
        let provider_cwd_path = provider_cwd.path().to_path_buf();
        let transcripts_dir = home_path.join("harness-transcripts");
        fs::create_dir_all(&transcripts_dir).expect("transcript directory");
        Self {
            home: Some(home),
            home_path,
            caller_cwd: Some(caller_cwd),
            caller_cwd_path,
            provider_cwd: Some(provider_cwd),
            provider_cwd_path,
            transcripts_dir,
            next_transcript: AtomicU64::new(0),
        }
    }

    pub fn loop_engine_home(&self) -> &Path {
        &self.home_path
    }

    pub fn caller_cwd(&self) -> &Path {
        &self.caller_cwd_path
    }

    pub fn provider_cwd(&self) -> &Path {
        &self.provider_cwd_path
    }

    pub fn config_path(&self) -> PathBuf {
        self.home_path.join("config.toml")
    }

    pub fn state_db_path(&self) -> PathBuf {
        self.home_path.join("state.db")
    }

    pub fn traces_dir(&self) -> PathBuf {
        self.home_path.join("traces")
    }

    pub fn transcripts_dir(&self) -> &Path {
        &self.transcripts_dir
    }

    pub fn allocate_transcript_path(&self, label: &str) -> PathBuf {
        let index = self.next_transcript.fetch_add(1, Ordering::Relaxed);
        self.transcripts_dir
            .join(format!("{index:04}-{label}.json"))
    }

    pub fn runner(&self) -> CliRunner<'_> {
        CliRunner::new(self)
    }

    pub fn isolated_env_removals() -> &'static [&'static str] {
        ISOLATED_ENV_REMOVALS
    }

    pub(crate) fn preserve_roots_on_panic(&mut self) {
        eprintln!(
            "\n=== E2E sandbox preserved on failure ===\n  LOOP_ENGINE_HOME={}\n  caller_cwd={}\n  provider_cwd={}\n  transcripts={}\n",
            self.home_path.display(),
            self.caller_cwd_path.display(),
            self.provider_cwd_path.display(),
            self.transcripts_dir.display(),
        );
        if let Some(home) = self.home.take() {
            let _ = home.keep();
        }
        if let Some(caller_cwd) = self.caller_cwd.take() {
            let _ = caller_cwd.keep();
        }
        if let Some(provider_cwd) = self.provider_cwd.take() {
            let _ = provider_cwd.keep();
        }
    }
}

impl Drop for E2eSandbox {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.preserve_roots_on_panic();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    #[test]
    fn preserves_all_roots_when_test_panics() {
        let mut preserved_home = None;
        let mut preserved_caller = None;
        let mut preserved_provider = None;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let sandbox = E2eSandbox::new();
            preserved_home = Some(sandbox.loop_engine_home().to_path_buf());
            preserved_caller = Some(sandbox.caller_cwd().to_path_buf());
            preserved_provider = Some(sandbox.provider_cwd().to_path_buf());
            fs::write(preserved_home.as_ref().unwrap().join("marker"), b"home")
                .expect("write home marker");
            fs::write(preserved_caller.as_ref().unwrap().join("marker"), b"caller")
                .expect("write caller marker");
            fs::write(
                preserved_provider.as_ref().unwrap().join("marker"),
                b"provider",
            )
            .expect("write provider marker");
            panic!("preserve sandbox roots");
        }));
        assert!(result.is_err());
        let home = preserved_home.expect("home path captured");
        let caller = preserved_caller.expect("caller path captured");
        let provider = preserved_provider.expect("provider path captured");
        assert!(home.join("marker").is_file());
        assert!(caller.join("marker").is_file());
        assert!(provider.join("marker").is_file());
    }
}
