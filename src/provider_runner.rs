//! A bounded foreground provider call. Credentials and storage are supplied by
//! the trusted caller; this module does not load secrets. Markdown mutations
//! use only the controller's explicit policy/checkpoint/audit/verification path.

use crate::anthropic_adapter::{AnthropicAdapter, AnthropicTransport};
use crate::domain::{GoalContract, HumanSteeringMode, LoopBudget, LoopStatus};
use crate::event_log::EventLog;
use crate::http_transport::{CredentialResolver, HttpProviderTransport};
use crate::loop_controller::{
    LoopControllerError, ReadOnlyLoopController, WorkspaceEditActionRuntime,
};
use crate::model_adapter::{ModelAdapterError, ModelDecision, ModelRequest};
use crate::model_call_store::ModelCallStore;
use crate::model_contracts::RouteDecision;
use crate::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use crate::policy_gate::PolicyGate;
use crate::prompt_registry::{PromptProfile, PromptRegistry};
use crate::provider_adapter::{OpenAiCompatibleAdapter, ProviderTransport};
use crate::provider_model_adapter::{
    ProviderModelAdapter, ResolvedModelAdapter, ResolvedModelResponseWithAction,
};
use crate::resolved_prompt::{
    resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
    ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
};
use crate::runtime_config::{
    ConfigError, ModelProfileConfig, RuntimeConfig, ANTHROPIC_MESSAGES_PROTOCOL,
    OPENAI_COMPATIBLE_CHAT_PROTOCOL,
};
use crate::stream_reporter::ProgressSink;
use crate::tool_executor::{ReadOnlyToolExecutor, WorkspaceToolExecutor};
use crate::tool_registry::ToolRegistry;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

const ROLE: &str = "observer";
const TASK: &str = "foreground_observation";
const PROMPT: &str = "foreground-observation";
const MARKDOWN_ROLE: &str = "software_developer";
const MARKDOWN_TASK: &str = "bounded_markdown_edit";
const MARKDOWN_PROMPT: &str = "bounded-markdown-edit";

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
    #[error("bounded Markdown run did not reach a completed decision")]
    IncompleteMarkdownRun,
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

/// Explicit caller-owned resources for the bounded Markdown write path.
/// Authentication remains an opaque profile reference; these paths are only
/// used by the controller's existing checkpoint and audit stores.
/// The trusted caller must keep the opaque ModelCallStore (including its
/// failure/lock files) separate from these stores and all editable files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedMarkdownRunConfig {
    pub workspace: PathBuf,
    pub allowed_paths: Vec<String>,
    pub checkpoint_path: PathBuf,
    pub trace_path: PathBuf,
    pub side_effect_path: PathBuf,
    pub approved: bool,
    pub human_steering_mode: HumanSteeringMode,
}

/// Preparation performs registry/prompt binding before a controller or a
/// credential resolver is invoked. There is no automatic route fallback.
pub struct ForegroundProviderRun {
    request: ProviderRunRequest,
    profile: ModelProfileConfig,
    route: RouteDecision,
    prompt: ResolvedPromptPack,
}

/// Provider-side Markdown runner. The controller remains the only mutation
/// path; this type merely binds the Markdown prompt and explicit audit paths.
pub struct BoundedMarkdownProviderRun {
    inner: ForegroundProviderRun,
    run: BoundedMarkdownRunConfig,
}

impl ForegroundProviderRun {
    pub fn prepare(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
    ) -> Result<Self, ProviderRunError> {
        Self::prepare_internal(config, request, None)
    }

    pub fn prepare_markdown(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
        run: BoundedMarkdownRunConfig,
    ) -> Result<BoundedMarkdownProviderRun, ProviderRunError> {
        if !run.workspace.is_absolute()
            || !run.workspace.is_dir()
            || run.workspace.parent().is_none()
            || run.allowed_paths.is_empty()
            || run.allowed_paths.iter().any(|path| {
                path.is_empty()
                    || !(path.ends_with(".md") || path.ends_with(".markdown"))
                    || !std::path::Path::new(path)
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
            })
            || [&run.checkpoint_path, &run.trace_path, &run.side_effect_path]
                .iter()
                .any(|path| !path.is_absolute() || path.parent().is_none())
        {
            return Err(ProviderRunError::InvalidRequest);
        }
        reject_symlink_components(&run.workspace)?;
        let stores = [&run.checkpoint_path, &run.trace_path, &run.side_effect_path];
        for (index, path) in stores.iter().enumerate() {
            if path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
                || stores[..index].contains(path)
                || run
                    .allowed_paths
                    .iter()
                    .any(|allowed| **path == run.workspace.join(allowed))
            {
                return Err(ProviderRunError::InvalidRequest);
            }
        }
        Ok(BoundedMarkdownProviderRun {
            inner: Self::prepare_internal(config, request, Some(&run))?,
            run,
        })
    }

    fn prepare_internal(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
        markdown: Option<&BoundedMarkdownRunConfig>,
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
        let (
            role,
            task,
            prompt_id,
            prompt_text,
            provider_text,
            model_text,
            role_text,
            policy_summary,
            tool_summary,
            privacy_requirement,
            risk_level,
            resolution_reason,
        ) = if markdown.is_some() {
            (MARKDOWN_ROLE, MARKDOWN_TASK, MARKDOWN_PROMPT,
                 "Prepare only a bounded Markdown edit and its required markdown_link_check verification.",
                 "Use the pinned decision-output contract and bounded Markdown action schema.",
                 "The only writable capability is the explicitly scoped edit_markdown recipe; never request shell commands or credentials.",
                 "Prepare a minimal exact Markdown change; do not broaden path scope.",
                 serde_json::json!({"mode":"bounded_markdown","tool_execution":true}),
                 serde_json::json!({"allowed":["edit_markdown","markdown_link_check"]}),
                 "caller_supplied_workspace_edit", "medium",
                 "explicit caller-selected Markdown model profile")
        } else {
            (ROLE, TASK, PROMPT,
                 "Evaluate only the supplied objective and observation. Do not propose tool actions.",
                 "Use the pinned decision-output contract.", "Do not assume any tool execution capability.",
                 "Observe; do not mutate.", serde_json::json!({"mode":"read_only","tool_execution":false}),
                 serde_json::json!({"tools":[]}), "caller_supplied_observation_only", "low",
                 "explicit caller-selected foreground model profile")
        };
        let mut models = ModelRegistry::default();
        let endpoint_id = format!("foreground:{}", profile.model_id);
        let endpoint_protocol = match profile.endpoint_protocol.as_str() {
            OPENAI_COMPATIBLE_CHAT_PROTOCOL => "openai-compatible",
            ANTHROPIC_MESSAGES_PROTOCOL => "anthropic",
            _ => return Err(ProviderRunError::UnsupportedProtocol),
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
        prompts
            .register(
                PromptProfile::new(prompt_id, 1, role, task, prompt_text)
                    .map_err(|_| ProviderRunError::InvalidRequest)?,
            )
            .map_err(|_| ProviderRunError::InvalidRequest)?;
        let now = chrono::Utc::now().to_rfc3339();
        let route = RouteDecision {
            route_decision_id: format!("{}:route:step-1", request.loop_run_id),
            task_id: request.task_run_id.clone(),
            task_type: task.into(),
            agent_role: role.into(),
            candidate_models: vec![profile.model_id.clone()],
            selected_provider: profile.provider.clone(),
            selected_model: profile.model_id.clone(),
            selected_endpoint: endpoint_id,
            selection_reason: resolution_reason.into(),
            rejected_candidates: Vec::new(),
            cost_estimate: None,
            latency_estimate_ms: None,
            risk_level: risk_level.into(),
            privacy_requirement: privacy_requirement.into(),
            fallback_chain: Vec::new(),
            prompt_profile_id: prompt_id.into(),
            prompt_profile_version: "1".into(),
            approval_required: false,
            created_at: now.clone(),
        };
        let fragment = |id: &str, content: &str| VersionedPromptFragment {
            id: id.into(),
            version: 1,
            content: content.into(),
        };
        let context = markdown.map_or_else(
            || serde_json::json!({}),
            |run| serde_json::json!({"workspace":run.workspace,"allowed_paths":run.allowed_paths}),
        );
        let base_text = if markdown.is_some() {
            prompt_text
        } else {
            "A bounded, read-only foreground observation. Never request or reveal credentials."
        };
        let prompt = resolve_prompt(
            &models,
            &prompts,
            ResolvePromptInput {
                route: &route,
                base: &fragment("foreground-base", base_text),
                provider: &ProviderPromptFragment {
                    provider_id: profile.provider.clone(),
                    fragment: fragment("foreground-provider", provider_text),
                },
                model: &ModelPromptFragment {
                    provider_id: profile.provider.clone(),
                    model_id: profile.model_id.clone(),
                    fragment: fragment("foreground-model", model_text),
                },
                role: &AgentRolePrompt {
                    role: role.into(),
                    fragment: fragment("foreground-role", role_text),
                },
                task: &TaskPromptPack {
                    prompt_id: prompt_id.into(),
                    version: 1,
                    task_type: task.into(),
                },
                skills: &[],
                policy_summary: &policy_summary,
                tool_summary: &tool_summary,
                context_package: &context,
                objective: &request.objective,
                observation: &request.observation,
                resolution_reason,
                created_at: &now,
            },
        )
        .map_err(|_| ProviderRunError::InvalidRequest)?;
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
        if self.profile.endpoint_protocol != OPENAI_COMPATIBLE_CHAT_PROTOCOL {
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
        if self.profile.endpoint_protocol != ANTHROPIC_MESSAGES_PROTOCOL {
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

/// Enforce caller authority before the controller sees a model proposal.
struct MarkdownBoundAdapter<'a, A: ResolvedModelAdapter> {
    inner: A,
    config: &'a BoundedMarkdownRunConfig,
}

impl<A: ResolvedModelAdapter> ResolvedModelAdapter for MarkdownBoundAdapter<'_, A> {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ModelAdapterError> {
        let response = self.inner.complete_resolved_with_action(request, pack)?;
        if response.response.decision != ModelDecision::CompleteLoop {
            return Err(ModelAdapterError::ProviderTextUnmappable);
        }
        let action = response
            .response
            .action
            .as_ref()
            .ok_or(ModelAdapterError::ProviderTextUnmappable)?;
        let path = action
            .parameters
            .get("path")
            .and_then(|value| value.as_str());
        if action.tool_id != "edit_markdown"
            || action.verification.verifier_tool_id != "markdown_link_check"
            || action.path_scope.root != self.config.workspace.display().to_string()
            || !action
                .path_scope
                .allowed_paths
                .iter()
                .all(|path| self.config.allowed_paths.contains(path))
            || !path.is_some_and(|path| {
                self.config
                    .allowed_paths
                    .iter()
                    .any(|allowed| allowed == path)
            })
            || action.verification.parameters != serde_json::json!({"path":path})
        {
            return Err(ModelAdapterError::ProviderTextUnmappable);
        }
        Ok(response)
    }
}

impl BoundedMarkdownProviderRun {
    pub fn run_with_transport<T: ProviderTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        let Self { inner, run } = self;
        if inner.profile.endpoint_protocol != OPENAI_COMPATIBLE_CHAT_PROTOCOL {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let adapter = ProviderModelAdapter::new(
            OpenAiCompatibleAdapter::new(
                inner.profile.provider.clone(),
                inner.profile.endpoint.clone(),
                transport,
            ),
            inner.profile.auth_ref.clone(),
        )
        .with_model_id(&inner.request.model_id);
        Self::run_with_resolved_adapter(inner, run, adapter, events, model_calls, sink)
    }

    pub fn run_with_anthropic_transport<T: AnthropicTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        let Self { inner, run } = self;
        if inner.profile.endpoint_protocol != ANTHROPIC_MESSAGES_PROTOCOL {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        let adapter = AnthropicAdapter::new(
            inner.profile.provider.clone(),
            inner.profile.endpoint.clone(),
            transport,
            inner.profile.auth_ref.clone(),
        )
        .with_model_id(&inner.request.model_id);
        Self::run_with_resolved_adapter(inner, run, adapter, events, model_calls, sink)
    }

    fn run_with_resolved_adapter<A: ResolvedModelAdapter, S: ProgressSink + 'static>(
        inner: ForegroundProviderRun,
        run: BoundedMarkdownRunConfig,
        adapter: A,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        validate_markdown_storage_paths(&run, events.path(), &model_calls)?;
        let request = inner.request;
        let mut goal = GoalContract::new(
            &request.goal_id,
            &request.project_id,
            &request.objective,
            run.allowed_paths.clone(),
            Vec::new(),
            vec!["markdown_link_check".into()],
        );
        goal.allowed_tools = vec!["edit_markdown".into(), "markdown_link_check".into()];
        goal.human_steering_mode = run.human_steering_mode;
        let mut controller = ReadOnlyLoopController::start_with_progress_sink(
            events,
            goal,
            &request.loop_run_id,
            &request.task_run_id,
            LoopBudget::new(1, 2, 1, 1, 0, 1, 1, 1),
            sink,
        )?
        .with_model_call_store(model_calls)
        .with_checkpoint_store(&run.checkpoint_path)
        .with_audit_stores(&run.trace_path, &run.side_effect_path);
        let adapter = MarkdownBoundAdapter {
            inner: adapter,
            config: &run,
        };
        let registry = ToolRegistry::baseline().map_err(|_| ProviderRunError::InvalidRequest)?;
        let gate = PolicyGate::new();
        let runtime = WorkspaceEditActionRuntime::new(
            &registry,
            &gate,
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &run.workspace,
            run.approved,
        );
        let decision = controller.run_resolved_model_step_with_workspace_edit(
            &adapter,
            ModelRequest {
                model_id: request.model_id,
                goal_id: request.goal_id,
                loop_run_id: request.loop_run_id,
                step_id: "step-1".into(),
                objective: request.objective,
                observation: request.observation,
            },
            &inner.prompt,
            &inner.route,
            &runtime,
            "step-1",
        );
        let decision = match decision {
            Ok(ModelDecision::CompleteLoop) => ModelDecision::CompleteLoop,
            Ok(_) => {
                if controller.status() == LoopStatus::Running {
                    controller.fail("bounded Markdown run ended without a completed decision")?;
                }
                return Err(ProviderRunError::IncompleteMarkdownRun);
            }
            Err(error) => {
                if controller.status() == LoopStatus::Running {
                    controller.fail("bounded Markdown run failed; see model audit metadata")?;
                }
                return Err(error.into());
            }
        };
        Ok(ProviderRunOutcome {
            decision,
            loop_status: controller.status(),
        })
    }
}

fn validate_markdown_storage_paths(
    run: &BoundedMarkdownRunConfig,
    event_path: &std::path::Path,
    model_calls: &ModelCallStore,
) -> Result<(), ProviderRunError> {
    let mut protected = vec![event_path.to_path_buf()];
    protected.extend([
        run.checkpoint_path.clone(),
        run.trace_path.clone(),
        run.side_effect_path.clone(),
    ]);
    protected.extend(model_calls.storage_paths());

    for path in &protected {
        validate_storage_path(path)?;
    }
    for allowed in &run.allowed_paths {
        let target = run.workspace.join(allowed);
        validate_workspace_target(&target)?;
        let target_key = normalized_path_key(&target)?;
        if protected
            .iter()
            .any(|path| normalized_path_key(path).ok().as_ref() == Some(&target_key))
        {
            return Err(ProviderRunError::InvalidRequest);
        }
    }

    let mut normalized = Vec::with_capacity(protected.len());
    for path in &protected {
        let key = normalized_path_key(path)?;
        if normalized.iter().any(|existing| existing == &key) {
            return Err(ProviderRunError::InvalidRequest);
        }
        normalized.push(key);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut identities = Vec::new();
        for path in protected.iter().filter(|path| path.exists()) {
            let metadata = fs::metadata(path).map_err(|_| ProviderRunError::InvalidRequest)?;
            let identity = (metadata.dev(), metadata.ino());
            if identities.contains(&identity) {
                return Err(ProviderRunError::InvalidRequest);
            }
            identities.push(identity);
        }
        for allowed in &run.allowed_paths {
            let target = run.workspace.join(allowed);
            if !target.exists() {
                continue;
            }
            let metadata = fs::metadata(&target).map_err(|_| ProviderRunError::InvalidRequest)?;
            let identity = (metadata.dev(), metadata.ino());
            if identities.contains(&identity) {
                return Err(ProviderRunError::InvalidRequest);
            }
            identities.push(identity);
        }
    }

    Ok(())
}

fn validate_storage_path(path: &std::path::Path) -> Result<(), ProviderRunError> {
    if !path.is_absolute() {
        return Err(ProviderRunError::InvalidRequest);
    }
    reject_symlink_components(path)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() {
            return Err(ProviderRunError::InvalidRequest);
        }
    }
    normalized_path_key(path).map(|_| ())
}

fn validate_workspace_target(path: &std::path::Path) -> Result<(), ProviderRunError> {
    reject_symlink_components(path)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() {
            return Err(ProviderRunError::InvalidRequest);
        }
    }
    normalized_path_key(path).map(|_| ())
}

fn reject_symlink_components(path: &std::path::Path) -> Result<(), ProviderRunError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(ProviderRunError::InvalidRequest);
            }
        }
    }
    Ok(())
}

fn normalized_path_key(path: &std::path::Path) -> Result<PathBuf, ProviderRunError> {
    let parent = path.parent().ok_or(ProviderRunError::InvalidRequest)?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|_| ProviderRunError::InvalidRequest)?;
    let name = path.file_name().ok_or(ProviderRunError::InvalidRequest)?;
    Ok(canonical_parent.join(name))
}
