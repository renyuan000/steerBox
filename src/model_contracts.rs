use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelContractError {
    #[error("model contract contains forbidden sensitive field: {field}")]
    SensitiveField { field: String },
    #[error("model contract serialization failed: {message}")]
    Serialization { message: String },
}

/// A persisted, secret-free explanation of how a model was selected for a task.
///
/// Provider authentication is intentionally absent. A provider adapter may resolve
/// credentials from an external `auth_ref`, but the reference and credential body
/// do not belong in this audit contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteDecision {
    pub route_decision_id: String,
    pub task_id: String,
    pub task_type: String,
    pub agent_role: String,
    pub candidate_models: Vec<String>,
    pub selected_provider: String,
    pub selected_model: String,
    pub selected_endpoint: String,
    pub selection_reason: String,
    pub rejected_candidates: Vec<String>,
    pub cost_estimate: Option<f64>,
    pub latency_estimate_ms: Option<u64>,
    pub risk_level: String,
    pub privacy_requirement: String,
    pub fallback_chain: Vec<String>,
    pub prompt_profile_id: String,
    pub prompt_profile_version: String,
    pub approval_required: bool,
    pub created_at: String,
}

impl RouteDecision {
    pub fn validate_secret_free(&self) -> Result<(), ModelContractError> {
        validate_secret_free(self)
    }
}

/// A persisted audit record for one model invocation.
///
/// This records references, measurements, and status only. It deliberately does
/// not contain request prompts, response bodies, auth references, or credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelCallRecord {
    pub model_call_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub agent_role: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub model_id: String,
    pub route_decision_id: String,
    pub resolved_prompt_pack_id: String,
    pub input_context_hash: String,
    pub output_artifact_ref: String,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated_cost: Option<f64>,
    pub streaming: bool,
    pub finish_reason: String,
    pub tool_call_count: u32,
    pub verification_status: String,
    pub error_class: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
}

impl ModelCallRecord {
    pub fn validate_secret_free(&self) -> Result<(), ModelContractError> {
        validate_secret_free(self)
    }
}

fn validate_secret_free<T: Serialize>(record: &T) -> Result<(), ModelContractError> {
    let value =
        serde_json::to_value(record).map_err(|error| ModelContractError::Serialization {
            message: error.to_string(),
        })?;
    if let Some(field) = forbidden_field_name(&value) {
        return Err(ModelContractError::SensitiveField { field });
    }
    Ok(())
}

fn forbidden_field_name(value: &serde_json::Value) -> Option<String> {
    const FORBIDDEN_FIELDS: &[&str] = &[
        "api_key",
        "apikey",
        "access_token",
        "authorization",
        "bearer_token",
        "client_secret",
        "cookie",
        "credential",
        "password",
        "private_key",
        "refresh_token",
        "secret",
        "session_token",
        "ssh_key",
        "token",
    ];

    match value {
        serde_json::Value::Object(fields) => fields.iter().find_map(|(key, value)| {
            let normalized = key.to_ascii_lowercase();
            if FORBIDDEN_FIELDS
                .iter()
                .any(|forbidden| *forbidden == normalized)
            {
                Some(key.clone())
            } else {
                forbidden_field_name(value)
            }
        }),
        serde_json::Value::Array(values) => values.iter().find_map(forbidden_field_name),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => None,
    }
}
