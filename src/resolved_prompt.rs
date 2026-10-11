//! Pure prompt resolution. No transport, credential access, fallback, or mutation.
//!
//! Fragment bodies and policy/tool summaries are caller-governed instructions.
//! Objective, observation, and context are always untrusted JSON user data.
//! The caller must verify a skill's governance evidence before passing it here;
//! a governance reference is provenance, not an authorization grant.

use crate::model_adapter::DECISION_SYSTEM_PROMPT;
pub use crate::model_adapter::{DECISION_OUTPUT_CONTRACT_ID, DECISION_OUTPUT_CONTRACT_VERSION};
use crate::model_contracts::{ModelContractError, RouteDecision};
use crate::model_registry::ModelRegistry;
use crate::prompt_registry::PromptRegistry;
use crate::provider_adapter::ProviderMessage;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedPromptFragment {
    pub id: String,
    pub version: u32,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPromptFragment {
    pub provider_id: String,
    pub fragment: VersionedPromptFragment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPromptFragment {
    pub provider_id: String,
    pub model_id: String,
    pub fragment: VersionedPromptFragment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRolePrompt {
    pub role: String,
    pub fragment: VersionedPromptFragment,
}

/// References the registered task template, rather than accepting a replacement
/// body that could silently bypass the route's pinned prompt version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPromptPack {
    pub prompt_id: String,
    pub version: u32,
    pub task_type: String,
}

/// Explicitly supplied by a trusted governance boundary. The resolver checks
/// presence and uniqueness, but does not resolve or grant governance approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernedSkillPrompt {
    pub fragment: VersionedPromptFragment,
    pub governance_ref: String,
}

pub struct ResolvePromptInput<'a> {
    pub route: &'a RouteDecision,
    pub base: &'a VersionedPromptFragment,
    pub provider: &'a ProviderPromptFragment,
    pub model: &'a ModelPromptFragment,
    pub role: &'a AgentRolePrompt,
    pub task: &'a TaskPromptPack,
    pub skills: &'a [GovernedSkillPrompt],
    pub policy_summary: &'a Value,
    pub tool_summary: &'a Value,
    pub context_package: &'a Value,
    pub objective: &'a str,
    pub observation: &'a str,
    pub resolution_reason: &'a str,
    pub created_at: &'a str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PromptVersion {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillPromptVersion {
    pub prompt: PromptVersion,
    pub governance_ref: String,
}

/// Body-free audit metadata, not a DLP filter. Callers must authorize and
/// sanitize free-text provenance such as resolution_reason before persistence.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResolvedPromptMetadata {
    pub prompt_pack_id: String,
    pub route_decision_id: String,
    pub task_id: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub endpoint_protocol: String,
    pub model_id: String,
    pub agent_role: String,
    pub task_type: String,
    pub base_prompt: PromptVersion,
    pub provider_prompt: PromptVersion,
    pub model_prompt: PromptVersion,
    pub role_prompt: PromptVersion,
    pub task_prompt: PromptVersion,
    pub output_contract: PromptVersion,
    pub skills: Vec<SkillPromptVersion>,
    pub policy_summary_hash: String,
    pub tool_summary_hash: String,
    pub context_package_hash: String,
    /// Messages-only digest. The final wire-body hash comes from preparation.
    pub rendered_messages_hash: String,
    pub resolution_reason: String,
    pub created_at: String,
}

/// Can only be constructed by the resolver. No serde implementation or mutable
/// getters: deserialization cannot bypass the registry and compatibility checks.
#[derive(Clone)]
pub struct ResolvedPromptPack {
    provider_id: String,
    endpoint_id: String,
    endpoint_base_url: String,
    model_id: String,
    model_name: String,
    objective: String,
    observation: String,
    messages: Vec<ProviderMessage>,
    metadata: ResolvedPromptMetadata,
}

impl std::fmt::Debug for ResolvedPromptPack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedPromptPack")
            .field("prompt_pack_id", &self.metadata.prompt_pack_id)
            .field(
                "rendered_messages_hash",
                &self.metadata.rendered_messages_hash,
            )
            .finish_non_exhaustive()
    }
}

impl ResolvedPromptPack {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    pub fn endpoint_protocol(&self) -> &str {
        &self.metadata.endpoint_protocol
    }

    pub fn endpoint_base_url(&self) -> &str {
        &self.endpoint_base_url
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    pub fn objective(&self) -> &str {
        &self.objective
    }

    pub fn observation(&self) -> &str {
        &self.observation
    }

    pub fn messages(&self) -> &[ProviderMessage] {
        &self.messages
    }

    pub fn metadata(&self) -> &ResolvedPromptMetadata {
        &self.metadata
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptResolutionError {
    #[error("route validation failed: {0}")]
    InvalidRoute(#[from] ModelContractError),
    #[error("route requires human approval before prompt resolution for execution")]
    ApprovalRequired,
    #[error("missing registered {entity}: {id}")]
    MissingBinding { entity: &'static str, id: String },
    #[error("prompt binding mismatch: {field}")]
    BindingMismatch { field: &'static str },
    #[error("invalid prompt field: {field}")]
    InvalidField { field: &'static str },
    #[error("duplicate skill prompt id: {id}")]
    DuplicateSkill { id: String },
    #[error("prompt serialization failed")]
    Serialization,
}

pub fn resolve_prompt(
    models: &ModelRegistry,
    prompts: &PromptRegistry,
    input: ResolvePromptInput<'_>,
) -> Result<ResolvedPromptPack, PromptResolutionError> {
    let route = input.route;
    route.validate()?;
    if route.approval_required {
        return Err(PromptResolutionError::ApprovalRequired);
    }
    require_nonempty(input.objective, "objective")?;
    require_nonempty(input.resolution_reason, "resolution_reason")?;
    chrono::DateTime::parse_from_rfc3339(input.created_at).map_err(|_| {
        PromptResolutionError::InvalidField {
            field: "created_at",
        }
    })?;

    let provider = models.provider(&route.selected_provider).ok_or_else(|| {
        PromptResolutionError::MissingBinding {
            entity: "provider",
            id: route.selected_provider.clone(),
        }
    })?;
    let endpoint = models.endpoint(&route.selected_endpoint).ok_or_else(|| {
        PromptResolutionError::MissingBinding {
            entity: "endpoint",
            id: route.selected_endpoint.clone(),
        }
    })?;
    let model = models.model(&route.selected_model).ok_or_else(|| {
        PromptResolutionError::MissingBinding {
            entity: "model",
            id: route.selected_model.clone(),
        }
    })?;
    require_match(
        &endpoint.provider_id,
        &provider.provider_id,
        "endpoint.provider_id",
    )?;
    require_match(
        &model.provider_id,
        &provider.provider_id,
        "model.provider_id",
    )?;
    require_match(
        &model.endpoint_id,
        &endpoint.endpoint_id,
        "model.endpoint_id",
    )?;
    require_match(
        &input.provider.provider_id,
        &provider.provider_id,
        "provider_prompt.provider_id",
    )?;
    require_match(
        &input.model.provider_id,
        &provider.provider_id,
        "model_prompt.provider_id",
    )?;
    require_match(
        &input.model.model_id,
        &model.model_id,
        "model_prompt.model_id",
    )?;

    let version = route
        .prompt_profile_version
        .parse::<u32>()
        .ok()
        .filter(|version| *version > 0)
        .ok_or(PromptResolutionError::InvalidField {
            field: "prompt_profile_version",
        })?;
    require_match(
        &input.task.prompt_id,
        &route.prompt_profile_id,
        "task_prompt.prompt_id",
    )?;
    if input.task.version != version {
        return Err(PromptResolutionError::BindingMismatch {
            field: "task_prompt.version",
        });
    }
    let profile = prompts
        .get(&route.prompt_profile_id, version)
        .ok_or_else(|| PromptResolutionError::MissingBinding {
            entity: "prompt",
            id: format!("{} v{version}", route.prompt_profile_id),
        })?;
    require_match(&profile.binding.role, &route.agent_role, "prompt.role")?;
    require_match(&profile.binding.task, &route.task_type, "prompt.task")?;
    require_match(&input.role.role, &route.agent_role, "role_prompt.role")?;
    require_match(
        &input.task.task_type,
        &route.task_type,
        "task_prompt.task_type",
    )?;

    for fragment in [
        input.base,
        &input.provider.fragment,
        &input.model.fragment,
        &input.role.fragment,
    ] {
        validate_fragment(fragment)?;
    }
    let mut skill_ids = BTreeSet::new();
    for skill in input.skills {
        validate_fragment(&skill.fragment)?;
        require_nonempty(&skill.governance_ref, "skill.governance_ref")?;
        if !skill_ids.insert(&skill.fragment.id) {
            return Err(PromptResolutionError::DuplicateSkill {
                id: skill.fragment.id.clone(),
            });
        }
    }

    // All instruction bodies come from explicit trusted configuration. There is
    // no interpolation of user-controlled objective, observation, or context.
    let system = format!(
        "Base harness instructions:\n{}\n\nProvider instructions:\n{}\n\nModel instructions:\n{}\n\nFixed decision-output contract:\n{}",
        input.base.content, input.provider.fragment.content, input.model.fragment.content,
        DECISION_SYSTEM_PROMPT,
    );
    let skills = input
        .skills
        .iter()
        .map(|skill| {
            serde_json::json!({
                "id": skill.fragment.id,
                "version": skill.fragment.version,
                "content": skill.fragment.content,
            })
        })
        .collect::<Vec<_>>();
    let developer = serde_json::json!({
        "role_instructions": input.role.fragment.content,
        "task_instructions": profile.template,
        "governed_skill_instructions": skills,
        "policy_summary": input.policy_summary,
        "tool_summary": input.tool_summary,
    })
    .to_string();
    let user = serde_json::json!({
        "objective": input.objective,
        "observation": input.observation,
        "context_package": input.context_package,
    })
    .to_string();
    let messages = vec![
        ProviderMessage::system(system),
        ProviderMessage {
            role: "developer".into(),
            content: developer,
        },
        ProviderMessage::user(user),
    ];
    let mut metadata = ResolvedPromptMetadata {
        prompt_pack_id: String::new(),
        route_decision_id: route.route_decision_id.clone(),
        task_id: route.task_id.clone(),
        provider_id: provider.provider_id.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        endpoint_protocol: endpoint.protocol.clone(),
        model_id: model.model_id.clone(),
        agent_role: route.agent_role.clone(),
        task_type: route.task_type.clone(),
        base_prompt: fragment_version(input.base),
        provider_prompt: fragment_version(&input.provider.fragment),
        model_prompt: fragment_version(&input.model.fragment),
        role_prompt: fragment_version(&input.role.fragment),
        task_prompt: PromptVersion {
            id: profile.prompt_id.clone(),
            version: profile.version,
        },
        output_contract: PromptVersion {
            id: DECISION_OUTPUT_CONTRACT_ID.into(),
            version: DECISION_OUTPUT_CONTRACT_VERSION,
        },
        skills: input
            .skills
            .iter()
            .map(|skill| SkillPromptVersion {
                prompt: fragment_version(&skill.fragment),
                governance_ref: skill.governance_ref.clone(),
            })
            .collect(),
        policy_summary_hash: hash_json(input.policy_summary)?,
        tool_summary_hash: hash_json(input.tool_summary)?,
        context_package_hash: hash_json(input.context_package)?,
        rendered_messages_hash: hash_json(&messages)?,
        resolution_reason: input.resolution_reason.into(),
        created_at: input.created_at.into(),
    };
    // The identity includes message bytes through rendered_messages_hash, not
    // just claimed fragment versions. Endpoint/model wire bindings are covered.
    metadata.prompt_pack_id = format!(
        "prompt-pack:{}",
        hash_json(&serde_json::json!({
            "metadata": metadata,
            "endpoint_base_url": endpoint.base_url,
            "model_name": model.model_name,
        }))?
    );
    Ok(ResolvedPromptPack {
        provider_id: provider.provider_id.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        endpoint_base_url: endpoint.base_url.clone(),
        model_id: model.model_id.clone(),
        model_name: model.model_name.clone(),
        objective: input.objective.into(),
        observation: input.observation.into(),
        messages,
        metadata,
    })
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), PromptResolutionError> {
    if value.trim().is_empty() {
        Err(PromptResolutionError::InvalidField { field })
    } else {
        Ok(())
    }
}

fn require_match(
    actual: &str,
    expected: &str,
    field: &'static str,
) -> Result<(), PromptResolutionError> {
    if actual == expected {
        Ok(())
    } else {
        Err(PromptResolutionError::BindingMismatch { field })
    }
}

fn validate_fragment(fragment: &VersionedPromptFragment) -> Result<(), PromptResolutionError> {
    require_nonempty(&fragment.id, "fragment.id")?;
    require_nonempty(&fragment.content, "fragment.content")?;
    if fragment.version == 0 {
        return Err(PromptResolutionError::InvalidField {
            field: "fragment.version",
        });
    }
    Ok(())
}

fn fragment_version(fragment: &VersionedPromptFragment) -> PromptVersion {
    PromptVersion {
        id: fragment.id.clone(),
        version: fragment.version,
    }
}

fn hash_json(value: &impl Serialize) -> Result<String, PromptResolutionError> {
    let encoded = serde_json::to_vec(value).map_err(|_| PromptResolutionError::Serialization)?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}
