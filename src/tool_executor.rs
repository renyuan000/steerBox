use crate::domain::{GoalContract, RiskLevel};
use crate::model_contracts::ModelActionEnvelope;
use crate::tool_registry::{SideEffectClass, ToolRegistry};
use crate::workspace_edit::{edit_markdown, EditResult, WorkspaceEditError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredToolAction {
    GitStatus,
    GitDiff,
    ReadFile {
        path: String,
    },
    RgFixed {
        query: String,
        path: String,
    },
    MarkdownLinkCheck {
        path: String,
    },
    EditMarkdown {
        path: String,
        expected: String,
        replacement: String,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolExecutionError {
    #[error("tool is not registered: {0}")]
    NotRegistered(String),
    #[error("tool is not executable in this slice: {0}")]
    NotImplemented(String),
    #[error("tool arguments must be an object")]
    InvalidArguments,
    #[error("tool profile is not a safe read-only profile: {0}")]
    UnsafeToolProfile(String),
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceToolExecutionError {
    #[error("tool is not registered: {0}")]
    NotRegistered(String),
    #[error("tool arguments must be an object with the exact expected fields")]
    InvalidArguments,
    #[error("tool is not a workspace-write profile: {0}")]
    UnsafeToolProfile(String),
    #[error("workspace edit failed: {0}")]
    Edit(#[from] WorkspaceEditError),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WorkspaceToolExecutor;

impl ReadOnlyToolExecutor {
    /// Map a validated model action to one of the fixed read-only tool shapes.
    /// This performs no process execution and never accepts a shell command.
    pub fn map_action(
        &self,
        registry: &ToolRegistry,
        workspace: &Path,
        action: &ModelActionEnvelope,
    ) -> Result<StructuredToolAction, ToolExecutionError> {
        action
            .validate()
            .map_err(|_| ToolExecutionError::InvalidArguments)?;
        if !workspace.is_dir() {
            return Err(ToolExecutionError::InvalidWorkspace(
                workspace.display().to_string(),
            ));
        }
        if action.path_scope.root != workspace.display().to_string() {
            return Err(ToolExecutionError::OutsideWorkspace(
                action.path_scope.root.clone(),
            ));
        }
        let profile = registry
            .get(&action.tool_id)
            .ok_or_else(|| ToolExecutionError::NotRegistered(action.tool_id.clone()))?;
        ensure_read_only_profile(profile)?;
        match profile.recipe_ref.as_deref() {
            Some("git-status-readonly") => {
                require_exact_keys(&action.parameters, &["scope"])?;
                if action.parameters["scope"] != "workspace" {
                    return Err(ToolExecutionError::InvalidArguments);
                }
                Ok(StructuredToolAction::GitStatus)
            }
            Some("git-diff-readonly") => {
                require_exact_keys(&action.parameters, &["scope"])?;
                if action.parameters["scope"] != "workspace" {
                    return Err(ToolExecutionError::InvalidArguments);
                }
                Ok(StructuredToolAction::GitDiff)
            }
            Some("rg-fixed-search") => {
                require_exact_keys(&action.parameters, &["path", "query"])?;
                let path = required_string(&action.parameters, "path")?;
                let query = required_string(&action.parameters, "query")?;
                Ok(StructuredToolAction::RgFixed { query, path })
            }
            Some("read-file-readonly") => {
                require_exact_keys(&action.parameters, &["path"])?;
                let path = required_string(&action.parameters, "path")?;
                Ok(StructuredToolAction::ReadFile { path })
            }
            Some("markdown-link-check") => {
                require_exact_keys(&action.parameters, &["path"])?;
                let path = required_string(&action.parameters, "path")?;
                Ok(StructuredToolAction::MarkdownLinkCheck { path })
            }
            _ => Err(ToolExecutionError::NotImplemented(action.tool_id.clone())),
        }
    }

    pub fn execute(
        &self,
        registry: &ToolRegistry,
        workspace: &Path,
        input: ToolCallInput,
    ) -> Result<ToolCallOutput, ToolExecutionError> {
        let profile = registry
            .get(&input.tool_id)
            .ok_or_else(|| ToolExecutionError::NotRegistered(input.tool_id.clone()))?;
        ensure_read_only_profile(profile)?;
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
        if profile.recipe_ref.as_deref() == Some("git-diff-readonly") {
            require_exact_keys(&input.arguments, &["scope"])?;
            if input.arguments["scope"] != "workspace" {
                return Err(ToolExecutionError::InvalidArguments);
            }
            let mut command = Command::new("git");
            command
                .args(["diff", "--no-ext-diff", "--no-color", "--"])
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
                    "scope": "workspace",
                    "executed": true,
                    "command": ["git", "diff", "--no-ext-diff", "--no-color", "--"],
                    "exit_code": output.status.code(),
                    "stdout": summary,
                    "stdout_truncated": output.stdout.truncated,
                    "stderr": output.stderr.text,
                    "stderr_truncated": output.stderr.truncated,
                }),
            });
        }
        if profile.recipe_ref.as_deref() == Some("rg-fixed-search") {
            require_exact_keys(&input.arguments, &["path", "query"])?;
            let path = required_string(&input.arguments, "path")?;
            let query = required_string(&input.arguments, "query")?;
            let path = resolve_workspace_path(workspace, &path)?;
            let mut command = Command::new("rg");
            command
                .args([
                    "--fixed-strings",
                    "--line-number",
                    "--no-heading",
                    "--color",
                    "never",
                    "--",
                ])
                .arg(&query)
                .arg(&path)
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
                    "path": path,
                    "query": query,
                    "executed": true,
                    "exit_code": output.status.code(),
                    "stdout": summary,
                    "stdout_truncated": output.stdout.truncated,
                    "stderr": output.stderr.text,
                    "stderr_truncated": output.stderr.truncated,
                }),
            });
        }
        if profile.recipe_ref.as_deref() == Some("read-file-readonly") {
            require_exact_keys(&input.arguments, &["path"])?;
            let relative_path = required_string(&input.arguments, "path")?;
            let path = resolve_workspace_path(workspace, &relative_path)?;
            let content = read_bounded_file(&path)?;
            return Ok(ToolCallOutput {
                summary: content.clone(),
                result: serde_json::json!({
                    "tool_id": input.tool_id,
                    "workspace": workspace,
                    "path": relative_path,
                    "executed": true,
                    "content": content,
                }),
            });
        }
        if profile.recipe_ref.as_deref() == Some("markdown-link-check") {
            require_exact_keys(&input.arguments, &["path"])?;
            let relative_path = required_string(&input.arguments, "path")?;
            let path = resolve_workspace_path(workspace, &relative_path)?;
            let content = read_bounded_file(&path)?;
            let missing = markdown_missing_links(workspace, &relative_path, &content);
            let summary = if missing.is_empty() {
                format!("checked {relative_path}: no missing local links")
            } else {
                format!(
                    "checked {relative_path}: {} missing local link(s)",
                    missing.len()
                )
            };
            return Ok(ToolCallOutput {
                summary,
                result: serde_json::json!({
                    "tool_id": input.tool_id,
                    "workspace": workspace,
                    "path": relative_path,
                    "executed": true,
                    "missing_local_links": missing,
                }),
            });
        }
        Err(ToolExecutionError::NotImplemented(input.tool_id))
    }
}

impl WorkspaceToolExecutor {
    /// Map a validated write action without executing it. The caller still
    /// needs a policy decision and explicit approval before `execute`.
    pub fn map_action(
        &self,
        registry: &ToolRegistry,
        workspace: &Path,
        action: &ModelActionEnvelope,
    ) -> Result<StructuredToolAction, WorkspaceToolExecutionError> {
        action
            .validate()
            .map_err(|_| WorkspaceToolExecutionError::InvalidArguments)?;
        if !workspace.is_dir() || action.path_scope.root != workspace.display().to_string() {
            return Err(WorkspaceToolExecutionError::InvalidArguments);
        }
        let profile = registry
            .get(&action.tool_id)
            .ok_or_else(|| WorkspaceToolExecutionError::NotRegistered(action.tool_id.clone()))?;
        ensure_workspace_write_profile(profile)?;
        if profile.recipe_ref.as_deref() != Some("markdown-edit-workspace") {
            return Err(WorkspaceToolExecutionError::UnsafeToolProfile(
                profile.tool_id.clone(),
            ));
        }
        require_exact_keys_write(&action.parameters, &["expected", "path", "replacement"])?;
        Ok(StructuredToolAction::EditMarkdown {
            path: required_write_string(&action.parameters, "path")?,
            expected: required_string_allow_empty(&action.parameters, "expected")?,
            replacement: required_string_allow_empty(&action.parameters, "replacement")?,
        })
    }

    /// Execute only the registered Markdown edit recipe. `approved` must be
    /// supplied by a trusted policy/interaction boundary; the helper itself
    /// remains atomic and workspace-scoped.
    pub fn execute(
        &self,
        registry: &ToolRegistry,
        workspace: &Path,
        goal: &GoalContract,
        input: ToolCallInput,
        approved: bool,
    ) -> Result<EditResult, WorkspaceToolExecutionError> {
        let profile = registry
            .get(&input.tool_id)
            .ok_or_else(|| WorkspaceToolExecutionError::NotRegistered(input.tool_id.clone()))?;
        ensure_workspace_write_profile(profile)?;
        if profile.recipe_ref.as_deref() != Some("markdown-edit-workspace") {
            return Err(WorkspaceToolExecutionError::UnsafeToolProfile(
                profile.tool_id.clone(),
            ));
        }
        require_exact_keys_write(&input.arguments, &["expected", "path", "replacement"])?;
        edit_markdown(
            workspace,
            goal,
            &required_write_string(&input.arguments, "path")?,
            &required_string_allow_empty(&input.arguments, "expected")?,
            &required_string_allow_empty(&input.arguments, "replacement")?,
            approved,
        )
        .map_err(WorkspaceToolExecutionError::from)
    }
}

fn ensure_workspace_write_profile(
    profile: &crate::tool_registry::ToolProfile,
) -> Result<(), WorkspaceToolExecutionError> {
    if profile.side_effect_class != SideEffectClass::WorkspaceWrite {
        return Err(WorkspaceToolExecutionError::UnsafeToolProfile(
            profile.tool_id.clone(),
        ));
    }
    Ok(())
}

fn ensure_read_only_profile(
    profile: &crate::tool_registry::ToolProfile,
) -> Result<(), ToolExecutionError> {
    if profile.side_effect_class != SideEffectClass::None || profile.risk_level != RiskLevel::Low {
        return Err(ToolExecutionError::UnsafeToolProfile(
            profile.tool_id.clone(),
        ));
    }
    Ok(())
}

fn require_exact_keys(value: &Value, expected: &[&str]) -> Result<(), ToolExecutionError> {
    let object = value
        .as_object()
        .ok_or(ToolExecutionError::InvalidArguments)?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(ToolExecutionError::InvalidArguments);
    }
    Ok(())
}

fn require_exact_keys_write(
    value: &Value,
    expected: &[&str],
) -> Result<(), WorkspaceToolExecutionError> {
    let object = value
        .as_object()
        .ok_or(WorkspaceToolExecutionError::InvalidArguments)?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(WorkspaceToolExecutionError::InvalidArguments);
    }
    Ok(())
}

fn required_string(value: &Value, key: &'static str) -> Result<String, ToolExecutionError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or(ToolExecutionError::InvalidArguments)
}

fn required_string_allow_empty(
    value: &Value,
    key: &'static str,
) -> Result<String, WorkspaceToolExecutionError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(WorkspaceToolExecutionError::InvalidArguments)
}

fn required_write_string(
    value: &Value,
    key: &'static str,
) -> Result<String, WorkspaceToolExecutionError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or(WorkspaceToolExecutionError::InvalidArguments)
}

fn resolve_workspace_path(workspace: &Path, relative: &str) -> Result<PathBuf, ToolExecutionError> {
    if relative.trim().is_empty()
        || relative.starts_with('/')
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
    {
        return Err(ToolExecutionError::OutsideWorkspace(relative.to_owned()));
    }
    let relative_path = Path::new(relative);
    if relative_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(ToolExecutionError::OutsideWorkspace(relative.to_owned()));
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|_| ToolExecutionError::InvalidWorkspace(workspace.display().to_string()))?;
    let candidate = workspace.join(relative_path);
    if candidate.exists() {
        let canonical = candidate
            .canonicalize()
            .map_err(|_| ToolExecutionError::OutsideWorkspace(relative.to_owned()))?;
        if !canonical.starts_with(&workspace) {
            return Err(ToolExecutionError::OutsideWorkspace(relative.to_owned()));
        }
        Ok(canonical)
    } else {
        Ok(candidate)
    }
}

fn read_bounded_file(path: &Path) -> Result<String, ToolExecutionError> {
    let file = fs::File::open(path).map_err(|_| ToolExecutionError::OutputCollectionFailed)?;
    let mut bytes = Vec::new();
    file.take((MAX_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ToolExecutionError::OutputCollectionFailed)?;
    let truncated = bytes.len() > MAX_OUTPUT_BYTES;
    bytes.truncate(MAX_OUTPUT_BYTES);
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    while text.len() > MAX_OUTPUT_BYTES {
        text.pop();
    }
    if truncated {
        text.push_str("\n[truncated]");
    }
    Ok(text)
}

fn markdown_missing_links(workspace: &Path, markdown_path: &str, content: &str) -> Vec<String> {
    let base = Path::new(markdown_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let mut missing = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("](") {
        let target_start = start + 2;
        let Some(end) = remaining[target_start..].find(')') else {
            break;
        };
        let target = remaining[target_start..target_start + end].trim();
        let target = target.split_whitespace().next().unwrap_or_default();
        if !target.is_empty()
            && !target.starts_with('#')
            && !target.contains("://")
            && !target.starts_with("mailto:")
        {
            let target_path = target.split('#').next().unwrap_or_default();
            if !target_path.is_empty() {
                let candidate = base.join(target_path);
                let candidate_text = candidate.to_string_lossy();
                if resolve_workspace_path(workspace, &candidate_text).is_err()
                    || !workspace.join(&candidate).exists()
                {
                    missing.push(target_path.to_owned());
                }
            }
        }
        remaining = &remaining[target_start + end + 1..];
    }
    missing.sort();
    missing.dedup();
    missing
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
