use crate::tool_registry::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallInput {
    pub tool_id: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallOutput {
    pub summary: String,
    pub result: Value,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolExecutionError {
    #[error("tool is not registered: {0}")]
    NotRegistered(String),
    #[error("tool is not executable in this slice: {0}")]
    NotImplemented(String),
    #[error("tool arguments must be an object")]
    InvalidArguments,
    #[error("workspace does not exist or is not a directory: {0}")]
    InvalidWorkspace(String),
    #[error("tool path is outside workspace: {0}")]
    OutsideWorkspace(String),
    #[error("git status command timed out")]
    TimedOut,
    #[error("failed to start git status command")]
    StartFailed,
    #[error("failed while waiting for git status command")]
    WaitFailed,
    #[error("failed to collect git status output")]
    OutputCollectionFailed,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyToolExecutor;

impl ReadOnlyToolExecutor {
    pub fn execute(
        &self,
        registry: &ToolRegistry,
        workspace: &Path,
        input: ToolCallInput,
    ) -> Result<ToolCallOutput, ToolExecutionError> {
        let profile = registry
            .get(&input.tool_id)
            .ok_or_else(|| ToolExecutionError::NotRegistered(input.tool_id.clone()))?;
        if !input.arguments.is_object() {
            return Err(ToolExecutionError::InvalidArguments);
        }
        if !workspace.is_dir() {
            return Err(ToolExecutionError::InvalidWorkspace(
                workspace.display().to_string(),
            ));
        }
        if profile.recipe_ref.as_deref() == Some("git-status-readonly") {
            let mut command = Command::new("git");
            command
                .args(["status", "--short", "--branch"])
                .current_dir(workspace)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let output = run_bounded_command(command)?;
            let summary = output.stdout.text.clone();
            return Ok(ToolCallOutput {
                summary: summary.clone(),
                result: serde_json::json!({
                    "tool_id": input.tool_id,
                    "workspace": workspace,
                    "executed": true,
                    "exit_code": output.status.code(),
                    "stdout": summary,
                    "stdout_truncated": output.stdout.truncated,
                    "stderr": output.stderr.text,
                    "stderr_truncated": output.stderr.truncated,
                }),
            });
        }
        Err(ToolExecutionError::NotImplemented(input.tool_id))
    }
}

struct CapturedOutput {
    text: String,
    truncated: bool,
}

struct BoundedCommandOutput {
    status: std::process::ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

fn run_bounded_command(mut command: Command) -> Result<BoundedCommandOutput, ToolExecutionError> {
    let mut child = command
        .spawn()
        .map_err(|_| ToolExecutionError::StartFailed)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(ToolExecutionError::OutputCollectionFailed)?;
    let stderr = child
        .stderr
        .take()
        .ok_or(ToolExecutionError::OutputCollectionFailed)?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout, MAX_OUTPUT_BYTES));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, MAX_OUTPUT_BYTES));
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(ToolExecutionError::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(ToolExecutionError::WaitFailed);
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| ToolExecutionError::OutputCollectionFailed)?
        .map_err(|_| ToolExecutionError::OutputCollectionFailed)?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| ToolExecutionError::OutputCollectionFailed)?
        .map_err(|_| ToolExecutionError::OutputCollectionFailed)?;
    Ok(BoundedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded<R: Read>(mut reader: R, max_bytes: usize) -> io::Result<CapturedOutput> {
    let mut retained = Vec::with_capacity(max_bytes.min(4096));
    let mut truncated = false;
    let mut buffer = [0u8; 4096];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(retained.len());
        let retained_count = count.min(remaining);
        retained.extend_from_slice(&buffer[..retained_count]);
        if retained_count < count {
            truncated = true;
        }
    }
    let mut text = String::from_utf8_lossy(&retained).into_owned();
    while text.len() > max_bytes {
        text.pop();
    }
    Ok(CapturedOutput { text, truncated })
}

#[cfg(test)]
mod tests {
    use super::read_bounded;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_retains_only_limit_and_preserves_utf8() {
        let input = "你好世界".repeat(100);
        let captured = read_bounded(Cursor::new(input.as_bytes()), 9).unwrap();

        assert!(captured.truncated);
        assert!(captured.text.len() <= 9);
        assert!(captured.text.is_char_boundary(captured.text.len()));
    }
}
