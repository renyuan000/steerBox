use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const MODEL_ACTION_SCHEMA_VERSION: u32 = 1;
const MAX_ACTION_IDENTIFIER_LENGTH: usize = 128;
const MAX_PATH_LENGTH: usize = 4096;
const MAX_ALLOWED_PATHS: usize = 256;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelContractError {
    #[error("invalid model contract field {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("model contract contains forbidden sensitive field: {field}")]
    SensitiveField { field: String },
    #[error("model contract serialization failed: {message}")]
    Serialization { message: String },
}

/// A versioned, structured action proposal kept separate from `ModelDecision`.
/// It describes what a tool may receive; it does not execute the action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelActionEnvelope {
    pub schema_version: u32,
    pub action_id: String,
    pub tool_id: String,
    pub parameters: Value,
    pub path_scope: ActionPathScope,
    pub verification: VerificationContract,
}

/// Lexical workspace boundary for paths carried by an action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActionPathScope {
    pub root: String,
    pub allowed_paths: Vec<String>,
}

/// Required verification step for a proposed action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerificationContract {
    pub verifier_tool_id: String,
    pub parameters: Value,
}

impl ModelActionEnvelope {
    pub fn parse_json(contents: &str) -> Result<Self, ModelContractError> {
        let action = serde_json::from_str::<Self>(contents).map_err(|error| {
            ModelContractError::Serialization {
                message: error.to_string(),
            }
        })?;
        action.validate()?;
        Ok(action)
    }

    pub fn to_json(&self) -> Result<String, ModelContractError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| ModelContractError::Serialization {
            message: error.to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), ModelContractError> {
        if self.schema_version != MODEL_ACTION_SCHEMA_VERSION {
            return Err(ModelContractError::InvalidField {
                field: "schema_version",
                reason: "unsupported action schema version",
            });
        }
        validate_identifier("action_id", &self.action_id)?;
        validate_identifier("tool_id", &self.tool_id)?;
        validate_structured_parameters("parameters", &self.parameters)?;
        validate_no_shell_fields(&self.parameters, "parameters")?;
        self.path_scope.validate()?;
        validate_parameter_paths(&self.parameters, &self.path_scope)?;
        self.verification.validate()?;
        validate_no_shell_fields(&self.verification.parameters, "verification")?;
        validate_parameter_paths(&self.verification.parameters, &self.path_scope)?;
        self.validate_secret_free()
    }

    pub fn validate_secret_free(&self) -> Result<(), ModelContractError> {
        validate_secret_free(self)
    }
}

impl ActionPathScope {
    fn validate(&self) -> Result<(), ModelContractError> {
        if self.root.trim().is_empty()
            || self.root.len() > MAX_PATH_LENGTH
            || !self.root.starts_with('/')
            || self.root == "/"
            || contains_root_path_escape(&self.root)
        {
            return Err(ModelContractError::InvalidField {
                field: "path_scope",
                reason: "root must be an absolute, bounded path without traversal",
            });
        }
        if self.allowed_paths.is_empty() || self.allowed_paths.len() > MAX_ALLOWED_PATHS {
            return Err(ModelContractError::InvalidField {
                field: "path_scope",
                reason: "allowed_paths must be a bounded non-empty list",
            });
        }
        for path in &self.allowed_paths {
            if !is_relative_path(path) {
                return Err(ModelContractError::InvalidField {
                    field: "path_scope",
                    reason: "allowed paths must remain below root",
                });
            }
        }
        Ok(())
    }

    fn contains(&self, path: &str) -> bool {
        self.allowed_paths.iter().any(|allowed| {
            let allowed = allowed.trim_end_matches('/');
            path == allowed || (!allowed.is_empty() && path.starts_with(&format!("{allowed}/")))
        })
    }
}

impl VerificationContract {
    fn validate(&self) -> Result<(), ModelContractError> {
        validate_identifier("verification", &self.verifier_tool_id)?;
        validate_structured_parameters("verification", &self.parameters)
    }
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
    /// Validate audit bindings and routing semantics before any serialization.
    /// This does not inspect free-text content or resolve credentials.
    pub fn validate(&self) -> Result<(), ModelContractError> {
        for (field, value) in [
            ("route_decision_id", self.route_decision_id.as_str()),
            ("task_id", self.task_id.as_str()),
            ("task_type", self.task_type.as_str()),
            ("agent_role", self.agent_role.as_str()),
            ("selected_provider", self.selected_provider.as_str()),
            ("selected_model", self.selected_model.as_str()),
            ("selected_endpoint", self.selected_endpoint.as_str()),
            ("prompt_profile_id", self.prompt_profile_id.as_str()),
            (
                "prompt_profile_version",
                self.prompt_profile_version.as_str(),
            ),
        ] {
            require_nonempty(field, value)?;
        }
        if !self.candidate_models.contains(&self.selected_model) {
            return Err(ModelContractError::InvalidField {
                field: "selected_model",
                reason: "must be present in candidate_models",
            });
        }
        let mut fallback_models = std::collections::HashSet::new();
        for model in &self.fallback_chain {
            require_nonempty("fallback_chain", model)?;
            if model == &self.selected_model {
                return Err(ModelContractError::InvalidField {
                    field: "fallback_chain",
                    reason: "must not contain the selected model",
                });
            }
            if !fallback_models.insert(model) {
                return Err(ModelContractError::InvalidField {
                    field: "fallback_chain",
                    reason: "must not contain duplicate models",
                });
            }
        }
        validate_cost("cost_estimate", self.cost_estimate)?;
        self.validate_secret_free()
    }

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
    /// Validate audit identifiers and costs without changing serialized fields.
    pub fn validate(&self) -> Result<(), ModelContractError> {
        for (field, value) in [
            ("model_call_id", self.model_call_id.as_str()),
            ("task_id", self.task_id.as_str()),
            ("task_run_id", self.task_run_id.as_str()),
            ("agent_role", self.agent_role.as_str()),
            ("provider_id", self.provider_id.as_str()),
            ("endpoint_id", self.endpoint_id.as_str()),
            ("model_id", self.model_id.as_str()),
            ("route_decision_id", self.route_decision_id.as_str()),
            (
                "resolved_prompt_pack_id",
                self.resolved_prompt_pack_id.as_str(),
            ),
            ("input_context_hash", self.input_context_hash.as_str()),
            ("output_artifact_ref", self.output_artifact_ref.as_str()),
        ] {
            require_nonempty(field, value)?;
        }
        validate_cost("estimated_cost", self.estimated_cost)?;
        self.validate_secret_free()
    }

    pub fn validate_secret_free(&self) -> Result<(), ModelContractError> {
        validate_secret_free(self)
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ModelContractError> {
    if value.trim().is_empty() {
        return Err(ModelContractError::InvalidField {
            field,
            reason: "must not be empty",
        });
    }
    if value.len() > MAX_ACTION_IDENTIFIER_LENGTH
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':'))
        })
    {
        return Err(ModelContractError::InvalidField {
            field,
            reason: "must be a bounded identifier",
        });
    }
    Ok(())
}

fn validate_structured_parameters(
    field: &'static str,
    value: &Value,
) -> Result<(), ModelContractError> {
    match value {
        Value::Object(fields) if !fields.is_empty() => Ok(()),
        Value::Object(_) => Err(ModelContractError::InvalidField {
            field,
            reason: "must not be empty",
        }),
        _ => Err(ModelContractError::InvalidField {
            field,
            reason: "must be a non-empty structured object",
        }),
    }
}

fn validate_no_shell_fields(value: &Value, field: &'static str) -> Result<(), ModelContractError> {
    const FORBIDDEN_SHELL_FIELDS: &[&str] = &[
        "shell",
        "command",
        "command_line",
        "shell_command",
        "script",
    ];
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.to_ascii_lowercase();
                if FORBIDDEN_SHELL_FIELDS
                    .iter()
                    .any(|forbidden| *forbidden == normalized)
                {
                    return Err(ModelContractError::InvalidField {
                        field,
                        reason: "free shell command fields are not allowed",
                    });
                }
                validate_no_shell_fields(value, field)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_no_shell_fields(value, field)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

fn validate_parameter_paths(
    value: &Value,
    scope: &ActionPathScope,
) -> Result<(), ModelContractError> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.to_ascii_lowercase();
                if normalized == "path" || normalized.ends_with("_path") {
                    validate_path_value(value, scope)?;
                } else if normalized == "paths" || normalized.ends_with("_paths") {
                    match value {
                        Value::Array(paths) => {
                            for path in paths {
                                validate_path_value(path, scope)?;
                            }
                        }
                        _ => {
                            return Err(ModelContractError::InvalidField {
                                field: "path_scope",
                                reason: "path collections must be arrays",
                            });
                        }
                    }
                } else {
                    validate_parameter_paths(value, scope)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_parameter_paths(value, scope)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

fn validate_path_value(value: &Value, scope: &ActionPathScope) -> Result<(), ModelContractError> {
    let path = value.as_str().ok_or(ModelContractError::InvalidField {
        field: "path_scope",
        reason: "path values must be strings",
    })?;
    if !is_relative_path(path) || !scope.contains(path) {
        return Err(ModelContractError::InvalidField {
            field: "path_scope",
            reason: "action path is outside the declared scope",
        });
    }
    Ok(())
}

fn is_relative_path(path: &str) -> bool {
    !path.trim().is_empty()
        && path.len() <= MAX_PATH_LENGTH
        && !path.starts_with('/')
        && !path.contains('\\')
        && !contains_path_escape(path)
        && !path.chars().any(|character| character.is_control())
}

fn contains_path_escape(path: &str) -> bool {
    path.split('/')
        .any(|component| component == ".." || component.is_empty())
}

fn contains_root_path_escape(path: &str) -> bool {
    path.split('/')
        .skip(1)
        .any(|component| component == ".." || component.is_empty())
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), ModelContractError> {
    if value.trim().is_empty() {
        return Err(ModelContractError::InvalidField {
            field,
            reason: "must not be empty",
        });
    }
    Ok(())
}

fn validate_cost(field: &'static str, value: Option<f64>) -> Result<(), ModelContractError> {
    if value.is_some_and(|cost| !cost.is_finite() || cost < 0.0) {
        return Err(ModelContractError::InvalidField {
            field,
            reason: "must be finite and nonnegative",
        });
    }
    Ok(())
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
