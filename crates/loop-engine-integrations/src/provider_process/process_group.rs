use std::process::{Command, Stdio};

use super::error::ProcessError;

pub fn verify_group(process_id: u32, expected_group_id: u32) -> Result<(), ProcessError> {
    let output = Command::new("/bin/ps")
        .args(["-o", "pgid=", "-p", &process_id.to_string()])
        .output()
        .map_err(ProcessError::Spawn)?;
    if !output.status.success() {
        return Err(ProcessError::Spawn(std::io::Error::other(format!(
            "provider PGID inspection exited {}",
            output.status
        ))));
    }
    let actual = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|error| {
            ProcessError::Spawn(std::io::Error::other(format!(
                "provider PGID inspection returned invalid output: {error}"
            )))
        })?;
    if actual != expected_group_id {
        return Err(ProcessError::Spawn(std::io::Error::other(format!(
            "provider PGID verification failed: expected {expected_group_id}, got {actual}"
        ))));
    }
    Ok(())
}

pub fn leader_exited(process_id: u32, expected_group_id: u32) -> Result<bool, ProcessError> {
    let output = Command::new("/bin/ps")
        .args(["-o", "pgid=,stat=", "-p", &process_id.to_string()])
        .output()
        .map_err(ProcessError::Spawn)?;
    if !output.status.success() {
        return Err(ProcessError::Spawn(std::io::Error::other(format!(
            "provider status inspection exited {}",
            output.status
        ))));
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let mut fields = line.split_whitespace();
    let actual_group = fields
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| {
            ProcessError::Spawn(std::io::Error::other(
                "provider status inspection returned no process",
            ))
        })?;
    if actual_group != expected_group_id {
        return Err(ProcessError::Spawn(std::io::Error::other(format!(
            "provider PGID changed: expected {expected_group_id}, got {actual_group}"
        ))));
    }
    Ok(fields.next().is_some_and(|state| state.starts_with('Z')))
}

pub fn signal_group(process_group_id: u32, signal: &str) -> Result<(), ProcessError> {
    let target = format!("-{process_group_id}");
    let status = Command::new("/bin/kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(target)
        .stderr(Stdio::null())
        .status()
        .map_err(|error| ProcessError::Termination(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(ProcessError::Termination(format!(
            "kill -{signal} exited {status}"
        )))
    }
}

pub fn group_exists(process_group_id: u32) -> Result<bool, ProcessError> {
    let output = Command::new("/bin/ps")
        .args(["-e", "-o", "pgid=,stat="])
        .output()
        .map_err(|error| ProcessError::Termination(error.to_string()))?;
    if !output.status.success() {
        return Err(ProcessError::Termination(format!(
            "process-group inspection exited {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        let group = fields.next().and_then(|value| value.parse::<u32>().ok());
        let state = fields.next().unwrap_or("");
        group == Some(process_group_id) && !state.starts_with('Z')
    }))
}
