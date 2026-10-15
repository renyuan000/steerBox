//! A bounded foreground provider call. Credentials and storage are supplied by
//! the trusted caller; this module does not load secrets or execute model tools.

use crate::anthropic_adapter::{AnthropicAdapter, AnthropicTransport};
use crate::domain::{GoalContract, LoopBudget, LoopStatus};
use crate::event_log::EventLog;
use crate::http_transport::{CredentialResolver, HttpProviderTransport};
use crate::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use crate::model_adapter::{ModelDecision, ModelRequest};
use crate::model_call_store::ModelCallStore;
use crate::model_contracts::RouteDecision;
use crate::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use crate::prompt_registry::{PromptProfile, PromptRegistry};
use crate::provider_adapter::{OpenAiCompatibleAdapter, ProviderTransport};
use crate::provider_model_adapter::ProviderModelAdapter;
use crate::resolved_prompt::{
    resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
    ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
};
use crate::runtime_config::{ConfigError, ModelProfileConfig, RuntimeConfig};
use crate::stream_reporter::ProgressSink;
use thiserror::Error;

const ROLE: &str = "observer";
const TASK: &str = "foreground_observation";
const PROMPT: &str = "foreground-observation";

#[derive(Debug, Error)]
pub enum ProviderRunError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("foreground provider run requires an opaque auth_ref and a caller-supplied resolver")]
    AuthRefRequired,
    #[error("foreground provider transport does not match the selected provider protocol")]
    UnsupportedProtocol,
    #[error("foreground provider request or profile is invalid")]
    InvalidRequest,
    #[error("provider transport could not be constructed")]
    TransportBuild,
    #[error(transparent)]
    Controller(#[from] LoopControllerError),
}

/// These are caller-supplied objective and metadata, never credential bodies.
pub struct ProviderRunRequest {
    pub project_id: String,
    pub goal_id: String,
    pub loop_run_id: String,
    pub task_run_id: String,
    pub model_id: String,
    pub objective: String,
    pub observation: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProviderRunOutcome {
    pub decision: ModelDecision,
    pub loop_status: LoopStatus,
}

/// Preparation performs registry/prompt binding before a controller or a
/// credential resolver is invoked. There is no automatic route fallback.
pub struct ForegroundProviderRun {
    request: ProviderRunRequest,
    profile: ModelProfileConfig,
    route: RouteDecision,
    prompt: ResolvedPromptPack,
}

impl ForegroundProviderRun {
    pub fn prepare(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
    ) -> Result<Self, ProviderRunError> {
        // RuntimeConfig fields are public; revalidate caller-mutated configs
        // before accepting endpoint/auth metadata for a provider invocation.
        let encoded =
            serde_json::to_string(config).map_err(|_| ProviderRunError::InvalidRequest)?;
        let config = RuntimeConfig::from_json_str(&encoded)?;
        let mut profiles = config
            .model_profiles
            .iter()
            .filter(|p| p.model_id == request.model_id);
        let profile = profiles
            .next()
            .ok_or_else(|| ConfigError::ModelProfileNotFound {
                model_id: request.model_id.clone(),
            })?
            .clone();
        if profiles.next().is_some() {
            return Err(ProviderRunError::InvalidRequest);
        }
        if profile.auth_ref.is_none() {
            return Err(ProviderRunError::AuthRefRequired);
        }
        if !profile
            .capabilities
            .iter()
            .any(|capability| capability == "text")
            || [
                &request.project_id,
                &request.goal_id,
                &request.loop_run_id,
                &request.task_run_id,
                &request.objective,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(ProviderRunError::InvalidRequest);
        }
        let mut models = ModelRegistry::default();
        let endpoint_id = format!("foreground:{}", profile.model_id);
        let endpoint_protocol = if profile.provider == "anthropic" {
            "anthropic"
        } else {
            "openai-compatible"
        };
        models
            .register_provider(
                ProviderProfile::new(
                    &profile.provider,
                    &profile.provider,
                    profile.auth_ref.clone(),
                )
                .map_err(|_| ProviderRunError::InvalidRequest)?,
            )
            .map_err(|_| ProviderRunError::InvalidRequest)?;
        models
            .register_endpoint(
                EndpointProfile::new(
                    &endpoint_id,
                    &profile.provider,
                    &profile.endpoint,
                    endpoint_protocol,
                )
                .map_err(|_| ProviderRunError::InvalidRequest)?,
            )
            .map_err(|_| ProviderRunError::InvalidRequest)?;
        models
            .register_model(
                ModelProfile::new(
                    &profile.model_id,
                    &profile.provider,
                    &endpoint_id,
                    &profile.model_id,
                    profile.capabilities.clone(),
                )
                .map_err(|_| ProviderRunError::InvalidRequest)?,
            )
            .map_err(|_| ProviderRunError::InvalidRequest)?;
        let mut prompts = PromptRegistry::default();
        prompts.register(PromptProfile::new(PROMPT, 1, ROLE, TASK,
            "Evaluate only the supplied objective and observation. Do not propose tool actions.")
            .map_err(|_| ProviderRunError::InvalidRequest)?)
            .map_err(|_| ProviderRunError::InvalidRequest)?;
        let now = chrono::Utc::now().to_rfc3339();
        let route = RouteDecision {
            route_decision_id: format!("{}:route:step-1", request.loop_run_id),
            task_id: request.task_run_id.clone(),
            task_type: TASK.into(),
            agent_role: ROLE.into(),
            candidate_models: vec![profile.model_id.clone()],
            selected_provider: profile.provider.clone(),
            selected_model: profile.model_id.clone(),
            selected_endpoint: endpoint_id,
            selection_reason: "explicit caller-selected foreground model profile".into(),
            rejected_candidates: Vec::new(),
            cost_estimate: None,
            latency_estimate_ms: None,
            risk_level: "low".into(),
            privacy_requirement: "caller_supplied_observation_only".into(),
            fallback_chain: Vec::new(),
            prompt_profile_id: PROMPT.into(),
            prompt_profile_version: "1".into(),
            approval_required: false,
            created_at: now.clone(),
        };
        let fragment = |id: &str, content: &str| VersionedPromptFragment {
            id: id.into(),
            version: 1,
            content: content.into(),
        };
        let prompt = resolve_prompt(&models, &prompts, ResolvePromptInput {
            route: &route,
            base: &fragment("foreground-base", "A bounded, read-only foreground observation. Never request or reveal credentials."),
            provider: &ProviderPromptFragment { provider_id: profile.provider.clone(),
                fragment: fragment("foreground-provider", "Use the pinned decision-output contract.") },
            model: &ModelPromptFragment { provider_id: profile.provider.clone(), model_id: profile.model_id.clone(),
                fragment: fragment("foreground-model", "Do not assume any tool execution capability.") },
            role: &AgentRolePrompt { role: ROLE.into(), fragment: fragment("foreground-role", "Observe; do not mutate.") },
            task: &TaskPromptPack { prompt_id: PROMPT.into(), version: 1, task_type: TASK.into() },
            skills: &[], policy_summary: &serde_json::json!({"mode":"read_only","tool_execution":false}),
            tool_summary: &serde_json::json!({"tools":[]}), context_package: &serde_json::json!({}),
            objective: &request.objective, observation: &request.observation,
            resolution_reason: "explicit caller-selected foreground model profile", created_at: &now,
        }).map_err(|_| ProviderRunError::InvalidRequest)?;
        Ok(Self {
            request,
            profile,
            route,
            prompt,
        })
    }

    pub fn run_with_resolver<R: CredentialResolver, S: ProgressSink + 'static>(
        self,
        resolver: R,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        let transport =
            HttpProviderTransport::new(resolver).map_err(|_| ProviderRunError::TransportBuild)?;
        self.run_with_transport(transport, events, model_calls, sink)
    }

    /// Injection seam for controlled transports, including local HTTP fixture
    /// servers. It is not permission to bypass a credential or endpoint policy.
    pub fn run_with_transport<T: ProviderTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        if self.profile.provider == "anthropic" {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let request = self.request;
        let goal = GoalContract::new(
            &request.goal_id,
            &request.project_id,
            &request.objective,
            Vec::new(),
            Vec::new(),
            vec!["model_result_recorded".into()],
        );
        let mut controller = ReadOnlyLoopController::start_with_progress_sink(
            events,
            goal,
            &request.loop_run_id,
            &request.task_run_id,
            LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
            sink,
        )?
        .with_model_call_store(model_calls);
        let provider =
            OpenAiCompatibleAdapter::new(self.profile.provider, self.profile.endpoint, transport);
        let adapter = ProviderModelAdapter::new(provider, self.profile.auth_ref)
            .with_model_id(&request.model_id);
        let decision = controller.run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: request.model_id,
                goal_id: request.goal_id,
                loop_run_id: request.loop_run_id,
                step_id: "step-1".into(),
                objective: request.objective,
                observation: request.observation,
            },
            &self.prompt,
            &self.route,
            "step-1",
        )?;
        Ok(ProviderRunOutcome {
            decision,
            loop_status: controller.status(),
        })
    }

    /// Library-only injection seam for Anthropic's independent wire protocol.
    /// The standalone CLI intentionally has no credential resolver and remains
    /// fail-closed.
    pub fn run_anthropic_with_transport<T: AnthropicTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        if self.profile.provider != "anthropic" {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let request = self.request;
        let goal = GoalContract::new(
            &request.goal_id,
            &request.project_id,
            &request.objective,
            Vec::new(),
            Vec::new(),
            vec!["model_result_recorded".into()],
        );
        let mut controller = ReadOnlyLoopController::start_with_progress_sink(
            events,
            goal,
            &request.loop_run_id,
            &request.task_run_id,
            LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
            sink,
        )?
        .with_model_call_store(model_calls);
        let adapter = AnthropicAdapter::new(
            self.profile.provider,
            self.profile.endpoint,
            transport,
            self.profile.auth_ref,
        )
        .with_model_id(&request.model_id);
        let decision = controller.run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: request.model_id,
                goal_id: request.goal_id,
                loop_run_id: request.loop_run_id,
                step_id: "step-1".into(),
                objective: request.objective,
                observation: request.observation,
            },
            &self.prompt,
            &self.route,
            "step-1",
        )?;
        Ok(ProviderRunOutcome {
            decision,
            loop_status: controller.status(),
        })
    }
}
