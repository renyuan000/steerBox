use crate::domain::{HumanSteeringMode, RiskLevel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    None,
    WorkspaceRead,
    WorkspaceWrite,
    ExternalNetwork,
    ExternalSystem,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectDisposition {
    ReadOnly,
    Reversible,
    Compensatable,
    Irreversible,
    ManualRequired,
}

impl SideEffectClass {
    pub const fn disposition(self) -> SideEffectDisposition {
        match self {
            Self::None | Self::WorkspaceRead => SideEffectDisposition::ReadOnly,
            Self::WorkspaceWrite => SideEffectDisposition::Reversible,
            Self::ExternalNetwork | Self::ExternalSystem | Self::Unknown => {
                SideEffectDisposition::ManualRequired
            }
        }
    }

    pub const fn automatic_execution_allowed(self) -> bool {
        matches!(
            self.disposition(),
            SideEffectDisposition::ReadOnly
                | SideEffectDisposition::Reversible
                | SideEffectDisposition::Compensatable
        )
    }

    pub const fn audit_label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WorkspaceRead => "workspace_read",
            Self::WorkspaceWrite => "workspace_write",
            Self::ExternalNetwork => "external_network",
            Self::ExternalSystem => "external_system",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_audit_label(label: &str) -> Self {
        match label.trim().to_ascii_lowercase().as_str() {
            "none" | "read_only" | "readonly" => Self::None,
            "workspace_read" | "workspaceread" => Self::WorkspaceRead,
            "workspace_write" | "workspacewrite" | "reversible_local_change" => {
                Self::WorkspaceWrite
            }
            "external_network" | "externalnetwork" => Self::ExternalNetwork,
            "external_system" | "externalsystem" => Self::ExternalSystem,
            "unknown" => Self::Unknown,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolProfile {
    pub tool_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub side_effect_class: SideEffectClass,
    pub risk_level: RiskLevel,
    pub allowed_in_modes: Vec<HumanSteeringMode>,
    pub recipe_ref: Option<String>,
    pub version: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolRegistryError {
    #[error("tool id must not be empty")]
    EmptyToolId,
    #[error("tool is already registered: {0}")]
    DuplicateTool(String),
}

#[derive(Debug, Default, Clone)]
pub struct ToolRegistry {
    profiles: std::collections::BTreeMap<String, ToolProfile>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, profile: ToolProfile) -> Result<(), ToolRegistryError> {
        if profile.tool_id.trim().is_empty() {
            return Err(ToolRegistryError::EmptyToolId);
        }
        if self.profiles.contains_key(&profile.tool_id) {
            return Err(ToolRegistryError::DuplicateTool(profile.tool_id));
        }
        self.profiles.insert(profile.tool_id.clone(), profile);
        Ok(())
    }

    pub fn get(&self, tool_id: &str) -> Option<&ToolProfile> {
        self.profiles.get(tool_id)
    }

    pub fn baseline() -> Result<Self, ToolRegistryError> {
        let mut registry = Self::new();
        for profile in [
            readonly_profile(
                "git_status",
                "git status",
                "read repository status without changing files",
                "git-status-readonly",
            ),
            readonly_profile(
                "rg_fixed",
                "fixed-string search",
                "search repository text with fixed-string semantics",
                "rg-fixed-search",
            ),
            readonly_profile(
                "read_file",
                "read file",
                "read a bounded regular file within the workspace",
                "read-file-readonly",
            ),
            workspace_write_profile(
                "edit_markdown",
                "edit Markdown",
                "replace exact Markdown content after explicit approval",
                "markdown-edit-workspace",
            ),
            readonly_profile(
                "markdown_link_check",
                "Markdown link check",
                "check Markdown links without changing files",
                "markdown-link-check",
            ),
        ] {
            registry.register(profile)?;
        }
        Ok(registry)
    }
}

fn readonly_profile(tool_id: &str, name: &str, description: &str, recipe_ref: &str) -> ToolProfile {
    ToolProfile {
        tool_id: tool_id.into(),
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({"arguments": "object"}),
        output_schema: serde_json::json!({"summary": "string"}),
        side_effect_class: SideEffectClass::None,
        risk_level: RiskLevel::Low,
        allowed_in_modes: vec![
            HumanSteeringMode::AutonomousLoop,
            HumanSteeringMode::HumanSteerableLoop,
            HumanSteeringMode::HumanInTheLoop,
            HumanSteeringMode::ManualTakeover,
        ],
        recipe_ref: Some(recipe_ref.into()),
        version: 1,
    }
}

fn workspace_write_profile(
    tool_id: &str,
    name: &str,
    description: &str,
    recipe_ref: &str,
) -> ToolProfile {
    ToolProfile {
        tool_id: tool_id.into(),
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({"arguments": "object"}),
        output_schema: serde_json::json!({"relative_path": "string", "changed": "boolean"}),
        side_effect_class: SideEffectClass::WorkspaceWrite,
        risk_level: RiskLevel::Medium,
        allowed_in_modes: vec![
            HumanSteeringMode::AutonomousLoop,
            HumanSteeringMode::HumanSteerableLoop,
            HumanSteeringMode::HumanInTheLoop,
            HumanSteeringMode::ManualTakeover,
        ],
        recipe_ref: Some(recipe_ref.into()),
        version: 1,
    }
}
