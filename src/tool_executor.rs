use crate::tool_registry::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
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
        if profile.recipe_ref.as_deref() == Some("git-status-readonly") {
            return Ok(ToolCallOutput {
                summary: format!("git status recipe selected for {}", workspace.display()),
                result: serde_json::json!({
                    "tool_id": input.tool_id,
                    "workspace": workspace,
                    "executed": false,
                    "reason": "executor remains fixture-only"
                }),
            });
        }
        Err(ToolExecutionError::NotImplemented(input.tool_id))
    }
}
