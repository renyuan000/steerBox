use crate::adaptive_routing::EffectLevel;
use crate::anthropic_adapter::{AnthropicAdapter, AnthropicTransport};
use crate::http_transport::{CredentialResolver, HttpAnthropicTransport, HttpProviderTransport};
use crate::model_adapter::ModelRequest;
use crate::model_contracts::RouteDecision;
use crate::model_registry::{contains_url_authentication_material, is_opaque_auth_ref};
use crate::provider_adapter::{
    ModelEffectPolicy, OpenAiCompatibleAdapter, ProviderTransport, ReasoningEffort,
};
use crate::provider_model_adapter::{
    EffectBoundResolvedAdapter, EffectProfileBinding, ProviderModelAdapter, ResolvedModelAdapter,
};
use crate::resolved_prompt::ResolvedPromptPack;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const DATA_ROOT_PREFIX: &str = "/temp/steerbox";

pub const OPENAI_COMPATIBLE_CHAT_PROTOCOL: &str = "openai-compatible-chat";
pub const ANTHROPIC_MESSAGES_PROTOCOL: &str = "anthropic-messages";

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
    #[error("model profile auth_ref must be an opaque credential reference")]
    InvalidAuthRef,
    #[error("model profile endpoint must not contain authentication material")]
    SensitiveEndpoint,
    #[error("model profile not found: {model_id}")]
    ModelProfileNotFound { model_id: String },
    #[error("HTTP provider transport could not be constructed")]
    HttpTransportBuild,
    #[error("Anthropic adapter requires provider `anthropic`, got `{provider}`")]
    UnsupportedAnthropicProvider { provider: String },
    #[error("Anthropic adapter requires capability `{capability}`")]
    UnsupportedAnthropicCapability { capability: String },
    #[error("unsupported model profile endpoint_protocol: {protocol}")]
    UnsupportedEndpointProtocol { protocol: String },
    #[error("invalid model profile effect: {reason}")]
    InvalidModelEffect { reason: &'static str },
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileConfig {
    pub model_id: String,
    pub provider: String,
    pub endpoint: String,
    /// Explicit wire protocol. Missing values are normalized only while
    /// reading legacy JSON; runners use this field and never infer a
    /// protocol from provider names.
    pub endpoint_protocol: String,
    #[serde(default)]
    pub auth_ref: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional schema-v1 extension; older binaries reject this new field.
    /// Host declaration bound to this profile's exact wire model ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<ModelEffectConfig>,
}

/// Effect selection and the optional, explicitly enabled task-local upgrade bound.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelEffectConfig {
    pub selected_level: EffectLevel,
    #[serde(default, skip_serializing_if = "is_false")]
    pub automatic_upgrade: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_level: Option<EffectLevel>,
    #[serde(deserialize_with = "deserialize_effect_mapping")]
    pub mapping: BTreeMap<EffectLevel, ReasoningEffort>,
}

/// Can only be built by RuntimeConfig after validating the profile and binding
/// the requested effect to the actual provider adapter. No mutable/public
/// fields or deserialization path can replace its authorization boundary.
pub struct EffectBoundResolvedCandidate {
    pub(crate) adapter: Box<dyn ResolvedModelAdapter>,
    pub(crate) request: ModelRequest,
    pub(crate) pack: ResolvedPromptPack,
    pub(crate) route: RouteDecision,
    pub(crate) binding: EffectProfileBinding,
    pub(crate) automatic_upgrade: bool,
    pub(crate) level: EffectLevel,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn deserialize_effect_mapping<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<EffectLevel, ReasoningEffort>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct MappingVisitor;

    impl<'de> serde::de::Visitor<'de> for MappingVisitor {
        type Value = BTreeMap<EffectLevel, ReasoningEffort>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mapping with unique effect levels")
        }

        fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut mapping = BTreeMap::new();
            while let Some((level, effort)) =
                entries.next_entry::<EffectLevel, ReasoningEffort>()?
            {
                if mapping.insert(level, effort).is_some() {
                    return Err(serde::de::Error::custom("duplicate effect mapping level"));
                }
            }
            Ok(mapping)
        }
    }

    deserializer.deserialize_map(MappingVisitor)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelProfileConfigWire {
    model_id: String,
    provider: String,
    endpoint: String,
    #[serde(default)]
    endpoint_protocol: Option<String>,
    #[serde(default)]
    auth_ref: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    effect: Option<ModelEffectConfig>,
}

impl<'de> Deserialize<'de> for ModelProfileConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ModelProfileConfigWire::deserialize(deserializer)?;
        let endpoint_protocol = wire.endpoint_protocol.unwrap_or_else(|| {
            // Compatibility-only normalization for profiles written before
            // endpoint_protocol existed. The resulting struct is explicit;
            // provider runners do not repeat this inference.
            if wire.provider == "anthropic" {
                ANTHROPIC_MESSAGES_PROTOCOL.to_owned()
            } else {
                OPENAI_COMPATIBLE_CHAT_PROTOCOL.to_owned()
            }
        });
        Ok(Self {
            model_id: wire.model_id,
            provider: wire.provider,
            endpoint: wire.endpoint,
            endpoint_protocol,
            auth_ref: wire.auth_ref,
            capabilities: wire.capabilities,
            effect: wire.effect,
        })
    }
}

impl ModelProfileConfig {
    fn effect_policy(&self) -> Result<Option<(ModelEffectPolicy, EffectLevel)>, ConfigError> {
        let Some(effect) = &self.effect else {
            return Ok(None);
        };
        if self.endpoint_protocol != OPENAI_COMPATIBLE_CHAT_PROTOCOL {
            return Err(ConfigError::InvalidModelEffect {
                reason: "effect requires openai-compatible-chat",
            });
        }
        let policy = ModelEffectPolicy::new(&self.model_id, effect.mapping.clone()).map_err(|_| {
            ConfigError::InvalidModelEffect {
                reason: "effect policy requires a nonempty model and strictly increasing declared levels",
            }
        })?;
        if !effect.mapping.contains_key(&effect.selected_level) {
            return Err(ConfigError::InvalidModelEffect {
                reason: "selected effect level is not declared",
            });
        }
        if effect.automatic_upgrade != effect.maximum_level.is_some() {
            return Err(ConfigError::InvalidModelEffect {
                reason: "automatic upgrade requires an explicit maximum effect level",
            });
        }
        let maximum_level = effect.maximum_level.unwrap_or(effect.selected_level);
        if maximum_level < effect.selected_level {
            return Err(ConfigError::InvalidModelEffect {
                reason: "maximum effect level must not be below selected level",
            });
        }
        if effect.automatic_upgrade {
            if maximum_level == effect.selected_level {
                return Err(ConfigError::InvalidModelEffect {
                    reason: "automatic upgrade maximum must allow at least one adjacent level",
                });
            }
            let mut level = effect.selected_level;
            while let Some(next) = level.next() {
                if next > maximum_level {
                    break;
                }
                if !effect.mapping.contains_key(&next) {
                    return Err(ConfigError::InvalidModelEffect {
                        reason: "automatic upgrade requires every adjacent declared level",
                    });
                }
                level = next;
            }
        }
        Ok(Some((policy, effect.selected_level)))
    }

    fn validate_effect(&self) -> Result<(), ConfigError> {
        self.effect_policy().map(|_| ())
    }

    pub(crate) fn build_openai_adapter<T: ProviderTransport>(
        &self,
        transport: T,
    ) -> Result<ProviderModelAdapter<T>, ConfigError> {
        if self
            .effect
            .as_ref()
            .is_some_and(|effect| effect.automatic_upgrade)
        {
            return Err(ConfigError::InvalidModelEffect {
                reason: "automatic upgrade requires the explicit resolved upgrade API",
            });
        }
        let policy = self.effect_policy()?;
        let mut provider = OpenAiCompatibleAdapter::new(&self.provider, &self.endpoint, transport);
        if let Some((policy, selected_level)) = policy {
            provider = provider
                .with_effect_policy(policy, selected_level)
                .map_err(|_| ConfigError::InvalidModelEffect {
                    reason: "selected effect level is not declared",
                })?;
        }
        Ok(
            ProviderModelAdapter::new(provider, self.auth_ref.clone())
                .with_model_id(&self.model_id),
        )
    }
}

impl RuntimeConfig {
    /// Explicit, transport-injected seam for pure-model automatic upgrade.
    /// No credentials are loaded here. The target must be the selected level
    /// or its single declared successor, and successors require profile opt-in.
    #[allow(clippy::too_many_arguments)]
    pub fn build_effect_bound_candidate<T: ProviderTransport + 'static>(
        &self,
        model_id: &str,
        level: EffectLevel,
        transport: T,
        request: ModelRequest,
        pack: ResolvedPromptPack,
        route: RouteDecision,
    ) -> Result<EffectBoundResolvedCandidate, ConfigError> {
        self.validate()?;
        let profile = self
            .model_profiles
            .iter()
            .find(|profile| profile.model_id == model_id)
            .ok_or_else(|| ConfigError::ModelProfileNotFound {
                model_id: model_id.into(),
            })?;
        let effect = profile
            .effect
            .as_ref()
            .ok_or(ConfigError::InvalidModelEffect {
                reason: "effect-bound candidate requires a declared effect",
            })?;
        let (policy, _) = profile
            .effect_policy()?
            .ok_or(ConfigError::InvalidModelEffect {
                reason: "effect-bound candidate requires a declared effect",
            })?;
        let maximum_level = effect.maximum_level.unwrap_or(effect.selected_level);
        if level != effect.selected_level
            && (!effect.automatic_upgrade
                || effect.selected_level.next() != Some(level)
                || level > maximum_level)
        {
            return Err(ConfigError::InvalidModelEffect {
                reason: "effect target is not the permitted adjacent level",
            });
        }
        if profile.provider != pack.provider_id()
            || profile.endpoint != pack.endpoint_base_url()
            || profile.model_id != pack.model_name()
            || profile.model_id != pack.model_id()
            || request.model_id != profile.model_id
            || route.selected_model != profile.model_id
            || route.selected_provider != profile.provider
            || route.selected_endpoint != pack.endpoint_id()
            || route.route_decision_id != pack.metadata().route_decision_id
            || route.task_id != pack.metadata().task_id
            || request.objective != pack.objective()
            || request.observation != pack.observation()
            || route.approval_required
            || !matches!(pack.endpoint_protocol(), "openai" | "openai-compatible")
            || route.validate().is_err()
        {
            return Err(ConfigError::InvalidModelEffect {
                reason: "effect candidate profile or prompt binding mismatch",
            });
        }
        let binding = EffectProfileBinding {
            provider_id: profile.provider.clone(),
            endpoint: profile.endpoint.clone(),
            model_id: profile.model_id.clone(),
            auth_ref: profile.auth_ref.clone(),
            selected_level: effect.selected_level,
            maximum_level,
            mapping: effect.mapping.clone(),
        };
        let provider =
            OpenAiCompatibleAdapter::new(&profile.provider, &profile.endpoint, transport)
                .with_effect_policy(policy, level)
                .map_err(|_| ConfigError::InvalidModelEffect {
                    reason: "effect candidate target is not declared",
                })?;
        let inner = ProviderModelAdapter::new(provider, profile.auth_ref.clone())
            .with_model_id(&profile.model_id);
        let adapter = EffectBoundResolvedAdapter::from_profile(inner, binding.clone());
        debug_assert!(adapter.binding() == &binding);
        Ok(EffectBoundResolvedCandidate {
            adapter: Box::new(adapter),
            request,
            pack,
            route,
            binding,
            automatic_upgrade: effect.automatic_upgrade,
            level,
        })
    }

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

    pub fn tool_calls_path(&self) -> PathBuf {
        self.data_root.join(".steerbox").join("tool-calls.jsonl")
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
        if profile.endpoint_protocol != OPENAI_COMPATIBLE_CHAT_PROTOCOL {
            return Err(ConfigError::UnsupportedEndpointProtocol {
                protocol: profile.endpoint_protocol.clone(),
            });
        }
        profile.validate_effect()?;
        let transport =
            HttpProviderTransport::new(resolver).map_err(|_| ConfigError::HttpTransportBuild)?;
        profile.build_openai_adapter(transport)
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
        profile.validate_effect()?;
        if profile.provider != "anthropic" {
            return Err(ConfigError::UnsupportedAnthropicProvider {
                provider: profile.provider.clone(),
            });
        }
        if profile.endpoint_protocol != ANTHROPIC_MESSAGES_PROTOCOL {
            return Err(ConfigError::UnsupportedEndpointProtocol {
                protocol: profile.endpoint_protocol.clone(),
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
        let profile = self
            .model_profiles
            .iter()
            .find(|profile| profile.model_id == model_id)
            .ok_or_else(|| ConfigError::ModelProfileNotFound {
                model_id: model_id.to_owned(),
            })?;
        profile.validate_effect()?;
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
        for profile in &self.model_profiles {
            if !matches!(
                profile.endpoint_protocol.as_str(),
                OPENAI_COMPATIBLE_CHAT_PROTOCOL | ANTHROPIC_MESSAGES_PROTOCOL
            ) {
                return Err(ConfigError::UnsupportedEndpointProtocol {
                    protocol: profile.endpoint_protocol.clone(),
                });
            }
            profile.validate_effect()?;
            if let Some(auth_ref) = profile.auth_ref.as_deref() {
                if auth_ref.is_empty() {
                    return Err(ConfigError::EmptyAuthRef);
                }
                if !is_opaque_auth_ref(auth_ref) {
                    return Err(ConfigError::InvalidAuthRef);
                }
            }
            if contains_url_authentication_material(&profile.endpoint) {
                return Err(ConfigError::SensitiveEndpoint);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_transport::{CredentialError, ResolvedCredential};
    use crate::model_adapter::{ModelDecision, ModelRequest};
    use crate::model_contracts::RouteDecision;
    use crate::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
    use crate::prompt_registry::{PromptProfile, PromptRegistry};
    use crate::resolved_prompt::{
        resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
        ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    struct NoCredentials;

    impl CredentialResolver for NoCredentials {
        fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
            panic!("unauthenticated localhost fixture must not resolve credentials")
        }
    }

    fn fragment(id: &str) -> VersionedPromptFragment {
        VersionedPromptFragment {
            id: id.into(),
            version: 1,
            content: "fixture".into(),
        }
    }

    fn resolved_pack(profile: &ModelProfileConfig) -> ResolvedPromptPack {
        let mut models = ModelRegistry::default();
        models
            .register_provider(ProviderProfile::new(&profile.provider, "Fixture", None).unwrap())
            .unwrap();
        models
            .register_endpoint(
                EndpointProfile::new(
                    "fixture-api",
                    &profile.provider,
                    &profile.endpoint,
                    "openai",
                )
                .unwrap(),
            )
            .unwrap();
        models
            .register_model(
                ModelProfile::new(
                    &profile.model_id,
                    &profile.provider,
                    "fixture-api",
                    &profile.model_id,
                    vec![],
                )
                .unwrap(),
            )
            .unwrap();
        let mut prompts = PromptRegistry::default();
        prompts
            .register(PromptProfile::new("inspect", 1, "reviewer", "review", "fixture").unwrap())
            .unwrap();
        let route = RouteDecision {
            route_decision_id: "route-fixture".into(),
            task_id: "task-fixture".into(),
            task_type: "review".into(),
            agent_role: "reviewer".into(),
            candidate_models: vec![profile.model_id.clone()],
            selected_provider: profile.provider.clone(),
            selected_model: profile.model_id.clone(),
            selected_endpoint: "fixture-api".into(),
            selection_reason: "localhost fixture".into(),
            rejected_candidates: vec![],
            cost_estimate: None,
            latency_estimate_ms: None,
            risk_level: "low".into(),
            privacy_requirement: "local".into(),
            fallback_chain: vec![],
            prompt_profile_id: "inspect".into(),
            prompt_profile_version: "1".into(),
            approval_required: false,
            created_at: "2026-10-31T00:00:00Z".into(),
        };
        resolve_prompt(
            &models,
            &prompts,
            ResolvePromptInput {
                route: &route,
                base: &fragment("base"),
                provider: &ProviderPromptFragment {
                    provider_id: profile.provider.clone(),
                    fragment: fragment("provider"),
                },
                model: &ModelPromptFragment {
                    provider_id: profile.provider.clone(),
                    model_id: profile.model_id.clone(),
                    fragment: fragment("model"),
                },
                role: &AgentRolePrompt {
                    role: "reviewer".into(),
                    fragment: fragment("role"),
                },
                task: &TaskPromptPack {
                    prompt_id: "inspect".into(),
                    version: 1,
                    task_type: "review".into(),
                },
                skills: &[],
                policy_summary: &json!({"mode":"read_only"}),
                tool_summary: &json!({"tools":[]}),
                context_package: &json!({}),
                objective: "Inspect fixture",
                observation: "Fixture ready",
                resolution_reason: "localhost fixture",
                created_at: "2026-10-31T00:00:00Z",
            },
        )
        .unwrap()
    }

    #[test]
    fn profile_helper_sends_effect_or_legacy_body_and_exact_hash_over_loopback() {
        for configured in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("localhost fixture accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, "POST /v1/chat/completions HTTP/1.1\r\n");
                let mut length = None;
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    assert!(!line.to_ascii_lowercase().starts_with("authorization:"));
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let length = length.expect("bounded HTTP request length");
                assert!(length < 64 * 1024);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                drop(reader);
                let response = json!({"id":"fixture-call","model":"fixture-model","choices":[{
                    "message":{"content":"{\"decision\":\"complete_loop\",\"summary\":\"done\"}"},"finish_reason":"stop"
                }]}).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
                String::from_utf8(body).unwrap()
            });
            let mut encoded = json!({"schema_version":1,"project_ref":"profile-fixture",
                "data_root":"/temp/steerbox/profile-fixture","model_profiles":[{
                    "model_id":"fixture-model","provider":"fixture-provider","endpoint":endpoint,
                    "capabilities":["text"]}]});
            if configured {
                encoded["model_profiles"][0]["effect"] =
                    json!({"selected_level":"strong","mapping":{"strong":"high"}});
            }
            let config = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap();
            let profile = &config.model_profiles[0];
            let transport = HttpProviderTransport::new(NoCredentials)
                .unwrap()
                .allow_loopback_http_for_tests();
            // The public HTTP factory delegates here, but deliberately cannot opt in to plain HTTP.
            let adapter = profile
                .build_openai_adapter(transport)
                .unwrap()
                .with_timeout_ms(2_000);
            let result = adapter.complete_resolved(
                ModelRequest {
                    model_id: profile.model_id.clone(),
                    goal_id: "goal".into(),
                    loop_run_id: "loop".into(),
                    step_id: "step".into(),
                    objective: "Inspect fixture".into(),
                    observation: "Fixture ready".into(),
                },
                &resolved_pack(profile),
            );
            let body = server.join().unwrap();
            let result = result.unwrap();
            assert_eq!(result.response.decision, ModelDecision::CompleteLoop);
            let wire: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(wire["model"], "fixture-model");
            if configured {
                assert_eq!(wire["reasoning_effort"], "high");
            } else {
                assert!(wire.get("reasoning_effort").is_none());
            }
            assert_eq!(
                result.input_context_hash,
                format!("sha256:{:x}", Sha256::digest(body.as_bytes()))
            );
            assert!(!body.contains("auth_ref"));
        }
    }
}
