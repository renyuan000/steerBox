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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLifecycleState {
    Configured,
    Enabled,
    Running,
    Invoked,
}

impl CapabilityLifecycleState {
    fn display_label(self) -> &'static str {
        match self {
            Self::Configured => "⏸️ 配置",
            Self::Enabled => "✅ 启用",
            Self::Running => "▶️ 运行",
            Self::Invoked => "↗️ 已调用",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCapabilityView {
    pub tool_id: String,
    pub name: String,
    pub version: u32,
    pub configured: bool,
    pub enabled: bool,
    pub lifecycle_state: CapabilityLifecycleState,
    pub startup_mode: String,
    pub resource_profile: String,
    pub use_case: String,
    pub side_effect_class: SideEffectClass,
    pub risk_level: RiskLevel,
    pub allowed_in_modes: Vec<HumanSteeringMode>,
    pub recipe_ref: Option<String>,
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

    pub fn capability_views(&self) -> Vec<ToolCapabilityView> {
        self.profiles
            .values()
            .map(|profile| {
                let enabled = !profile.allowed_in_modes.is_empty();
                let (resource_profile, use_case) = capability_metadata(&profile.tool_id);
                ToolCapabilityView {
                    tool_id: profile.tool_id.clone(),
                    name: profile.name.clone(),
                    version: profile.version,
                    configured: true,
                    enabled,
                    lifecycle_state: if enabled {
                        CapabilityLifecycleState::Enabled
                    } else {
                        CapabilityLifecycleState::Configured
                    },
                    startup_mode: "on_demand".into(),
                    resource_profile: resource_profile.into(),
                    use_case: use_case.into(),
                    side_effect_class: profile.side_effect_class,
                    risk_level: profile.risk_level,
                    allowed_in_modes: profile.allowed_in_modes.clone(),
                    recipe_ref: profile.recipe_ref.clone(),
                }
            })
            .collect()
    }

    pub fn render_capability_table(&self) -> String {
        render_capability_table(&self.capability_views())
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
                "git_diff",
                "git diff",
                "inspect tracked workspace changes without changing files",
                "git-diff-readonly",
            ),
            readonly_profile(
                "file_write_verification",
                "file write verification",
                "compare a bounded workspace file SHA-256 without returning file content",
                "file-write-verification",
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

fn capability_metadata(tool_id: &str) -> (&'static str, &'static str) {
    match tool_id {
        "git_status" => ("极低/按需", "查看仓库状态"),
        "git_diff" => ("极低/按需", "查看受限工作区变更"),
        "file_write_verification" => ("极低/按需", "验证受控文件写入结果"),
        "rg_fixed" => ("极低/按需", "搜索受限工作区文本"),
        "read_file" => ("极低/按需", "读取受限工作区文件"),
        "edit_markdown" => ("极低/按需", "审批后修改 Markdown"),
        "markdown_link_check" => ("极低/按需", "检查 Markdown 链接"),
        _ => ("未测/未知", "未定义使用场景"),
    }
}

pub fn render_capability_table(views: &[ToolCapabilityView]) -> String {
    let headers = ["工具/能力", "默认状态", "运行资源", "使用场景"];
    let rows: Vec<[String; 4]> = views
        .iter()
        .map(|view| {
            [
                view.tool_id.clone(),
                view.lifecycle_state.display_label().into(),
                view.resource_profile.clone(),
                view.use_case.clone(),
            ]
        })
        .collect();
    let widths = (0..headers.len())
        .map(|index| {
            rows.iter()
                .map(|row| display_width(&row[index]))
                .chain(std::iter::once(display_width(headers[index])))
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();

    let border = |left: char, middle: char, right: char| {
        let mut line = String::new();
        line.push(left);
        for (index, width) in widths.iter().enumerate() {
            line.push_str(&"─".repeat(width + 2));
            line.push(if index + 1 == widths.len() {
                right
            } else {
                middle
            });
        }
        line
    };
    let row = |cells: &[String]| {
        let mut line = String::from("│");
        for (index, cell) in cells.iter().enumerate() {
            line.push(' ');
            line.push_str(cell);
            line.push_str(&" ".repeat(widths[index] - display_width(cell) + 1));
            line.push('│');
        }
        line
    };

    let mut output = vec![border('┌', '┬', '┐')];
    output.push(row(&headers
        .iter()
        .map(|header| (*header).to_owned())
        .collect::<Vec<_>>()));
    output.push(border('├', '┼', '┤'));
    for capability in rows {
        output.push(row(&capability));
    }
    output.push(border('└', '┴', '┘'));
    output.join("\n")
}

fn display_width(value: &str) -> usize {
    value
        .chars()
        .map(|character| if character.is_ascii() { 1 } else { 2 })
        .sum()
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
