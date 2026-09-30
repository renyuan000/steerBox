use crate::tool_registry::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

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
            let output = Command::new("git")
                .args(["status", "--short", "--branch"])
                .current_dir(workspace)
                .output()
                .map_err(|_| ToolExecutionError::NotImplemented(input.tool_id.clone()))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let max_output_bytes = 16 * 1024;
            let summary = truncate_output(&stdout, max_output_bytes);
            return Ok(ToolCallOutput {
                summary: summary.clone(),
                result: serde_json::json!({
                    "tool_id": input.tool_id,
                    "workspace": workspace,
                    "executed": true,
                    "exit_code": output.status.code(),
                    "stdout": summary,
                    "stderr": truncate_output(&stderr, max_output_bytes),
                }),
            });
        }
        Err(ToolExecutionError::NotImplemented(input.tool_id))
    }
}

fn truncate_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.into();
    }
    let boundary = output
        .char_indices()
        .take_while(|(index, _)| *index < max_bytes)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    let mut truncated = output[..boundary].to_owned();
    truncated.push_str("\n[OUTPUT_TRUNCATED]");
    truncated
}
