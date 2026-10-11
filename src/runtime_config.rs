use crate::anthropic_adapter::{AnthropicAdapter, AnthropicTransport};
use crate::http_transport::{CredentialResolver, HttpAnthropicTransport, HttpProviderTransport};
use crate::provider_model_adapter::ProviderModelAdapter;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const DATA_ROOT_PREFIX: &str = "/temp/steerbox";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("configuration file read failed: {message}")]
    Io { message: String },
    #[error("configuration JSON is invalid: {message}")]
    InvalidJson { message: String },
    #[error("configuration contains an unsupported field: {field}")]
    UnknownField { field: String },
    #[error("project_ref must be a safe non-empty path segment")]
    InvalidProjectRef,
    #[error("unsupported configuration schema_version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("data_root must be an absolute path below /temp/steerbox")]
    InvalidDataRoot,
    #[error("model profile auth_ref cannot be empty")]
    EmptyAuthRef,
    #[error("model profile not found: {model_id}")]
    ModelProfileNotFound { model_id: String },
    #[error("HTTP provider transport could not be constructed")]
    HttpTransportBuild,
    #[error("Anthropic adapter requires provider `anthropic`, got `{provider}`")]
    UnsupportedAnthropicProvider { provider: String },
    #[error("Anthropic adapter requires capability `{capability}`")]
    UnsupportedAnthropicCapability { capability: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub project_ref: String,
    pub data_root: PathBuf,
    #[serde(default)]
    pub model_profiles: Vec<ModelProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileConfig {
    pub model_id: String,
    pub provider: String,
    pub endpoint: String,
    #[serde(default)]
    pub auth_ref: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl RuntimeConfig {
    pub fn default_for_project(project_ref: impl Into<String>) -> Result<Self, ConfigError> {
        let project_ref = project_ref.into();
        let data_root = PathBuf::from(DATA_ROOT_PREFIX).join(&project_ref);
        let config = Self {
            schema_version: 1,
            project_ref,
            data_root,
            model_profiles: Vec::new(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn from_json_str(contents: &str) -> Result<Self, ConfigError> {
        let config = serde_json::from_str::<Self>(contents).map_err(|error| {
            let message = error.to_string();
            if let Some(field) = unknown_field_name(&message) {
                ConfigError::UnknownField { field }
            } else {
                ConfigError::InvalidJson { message }
            }
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|error| ConfigError::Io {
            message: error.to_string(),
        })?;
        Self::from_json_str(&contents)
    }

    pub fn events_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("events.jsonl")
    }

    pub fn model_calls_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("model-calls.jsonl")
    }

    pub fn commands_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("commands.jsonl")
    }

    pub fn state_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("state.json")
    }

    pub fn traces_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("traces.jsonl")
    }

    pub fn checkpoints_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("checkpoints.jsonl")
    }

    pub fn side_effects_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("side-effects.jsonl")
    }

    pub fn run_summary_path(&self, loop_run_id: &str) -> PathBuf {
        self.data_root
            .join(".steerbox")
            .join("runs")
            .join(loop_run_id)
            .join("summary.json")
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_root.join("config.json")
    }

    pub fn build_http_provider_adapter<R: CredentialResolver>(
        &self,
        model_id: &str,
        resolver: R,
    ) -> Result<ProviderModelAdapter<HttpProviderTransport<R>>, ConfigError> {
        let profile = self
            .model_profiles
            .iter()
            .find(|profile| profile.model_id == model_id)
            .ok_or_else(|| ConfigError::ModelProfileNotFound {
                model_id: model_id.to_owned(),
            })?;
        let transport =
            HttpProviderTransport::new(resolver).map_err(|_| ConfigError::HttpTransportBuild)?;
        let provider = crate::provider_adapter::OpenAiCompatibleAdapter::new(
            profile.provider.clone(),
            profile.endpoint.clone(),
            transport,
        );
        Ok(ProviderModelAdapter::new(provider, profile.auth_ref.clone()).with_model_id(model_id))
    }

    pub fn build_anthropic_adapter<T: AnthropicTransport>(
        &self,
        model_id: &str,
        transport: T,
    ) -> Result<AnthropicAdapter<T>, ConfigError> {
        let profile = self
            .model_profiles
            .iter()
            .find(|profile| profile.model_id == model_id)
            .ok_or_else(|| ConfigError::ModelProfileNotFound {
                model_id: model_id.to_owned(),
            })?;
        if profile.provider != "anthropic" {
            return Err(ConfigError::UnsupportedAnthropicProvider {
                provider: profile.provider.clone(),
            });
        }
        if !profile
            .capabilities
            .iter()
            .any(|capability| capability == "text")
        {
            return Err(ConfigError::UnsupportedAnthropicCapability {
                capability: "text".to_owned(),
            });
        }

        Ok(AnthropicAdapter::new(
            profile.provider.clone(),
            profile.endpoint.clone(),
            transport,
            profile.auth_ref.clone(),
        )
        .with_model_id(model_id))
    }

    pub fn build_http_anthropic_adapter<R: CredentialResolver>(
        &self,
        model_id: &str,
        resolver: R,
    ) -> Result<AnthropicAdapter<HttpAnthropicTransport<R>>, ConfigError> {
        let transport =
            HttpAnthropicTransport::new(resolver).map_err(|_| ConfigError::HttpTransportBuild)?;
        self.build_anthropic_adapter(model_id, transport)
    }

    pub fn write_json(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ConfigError::Io {
                message: error.to_string(),
            })?;
        }
        let encoded =
            serde_json::to_vec_pretty(self).map_err(|error| ConfigError::InvalidJson {
                message: error.to_string(),
            })?;
        fs::write(path, encoded).map_err(|error| ConfigError::Io {
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 {
            return Err(ConfigError::UnsupportedSchemaVersion(self.schema_version));
        }
        if !is_safe_segment(&self.project_ref) {
            return Err(ConfigError::InvalidProjectRef);
        }
        if !is_valid_data_root(&self.data_root, &self.project_ref) {
            return Err(ConfigError::InvalidDataRoot);
        }
        if self
            .model_profiles
            .iter()
            .any(|profile| profile.auth_ref.as_deref() == Some(""))
        {
            return Err(ConfigError::EmptyAuthRef);
        }
        Ok(())
    }
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn is_valid_data_root(path: &Path, project_ref: &str) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let prefix = Path::new(DATA_ROOT_PREFIX);
    path == prefix.join(project_ref)
        && path.starts_with(prefix)
        && path
            .strip_prefix(prefix)
            .map(|relative| {
                relative.components().all(|component| {
                    !matches!(
                        component,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
            })
            .unwrap_or(false)
}

fn unknown_field_name(message: &str) -> Option<String> {
    let remainder = message.strip_prefix("unknown field `")?;
    Some(remainder.split('`').next()?.to_owned())
}
