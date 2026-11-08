use super::*;
use crate::domain::RiskLevel;
use crate::http_transport::HttpAnthropicTransport;
use crate::loop_controller::{ModelActionRuntime, ResolvedModelLoopOutcome};
use crate::model_contracts::ModelActionEnvelope;
use crate::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};
use serde_json::{json, Value};
use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedReadOnlyRunConfig {
    pub workspace: PathBuf,
    pub allowed_paths: Vec<String>,
    pub max_iterations: u32,
}

pub struct BoundedReadOnlyProviderRun {
    inner: ForegroundProviderRun,
    run: BoundedReadOnlyRunConfig,
    workspace_identity: WorkspaceIdentity,
}

/// This synchronous outcome does not imply a background worker or durable resume.
/// RequestHuman and ContinueObservationWithoutAction retain the controller's status.
#[derive(Debug, PartialEq, Eq)]
pub struct BoundedReadOnlyProviderRunOutcome {
    pub outcome: ResolvedModelLoopOutcome,
    pub loop_status: LoopStatus,
    budget: LoopBudget,
    reported_token_budget: Option<ReportedTokenBudget>,
}

impl BoundedReadOnlyProviderRunOutcome {
    /// Snapshot of this synchronous run's TaskRun budget, not resumable state.
    pub fn budget(&self) -> &LoopBudget {
        &self.budget
    }

    pub fn reported_token_budget(&self) -> Option<&ReportedTokenBudget> {
        self.reported_token_budget.as_ref()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct WorkspaceIdentity {
    canonical: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

/// Frozen filesystem scope for rechecking destinations after the prepared run
/// is consumed. Checks create no files or directories and inspect the current
/// filesystem; they are not an immutable permit or protection from OS races.
pub struct ReadOnlyRuntimePathCheck {
    run: BoundedReadOnlyRunConfig,
    workspace_identity: WorkspaceIdentity,
    events: PathBuf,
    model_calls: PathBuf,
    projection: Option<PathBuf>,
}

impl ReadOnlyRuntimePathCheck {
    pub fn revalidate(&self) -> Result<(), ProviderRunError> {
        if validate_run(&self.run)? != self.workspace_identity {
            return Err(ProviderRunError::InvalidRequest);
        }
        validate_stores(
            &self.run,
            &self.events,
            &self.model_calls,
            self.projection.as_deref(),
        )
    }
}

impl ForegroundProviderRun {
    pub fn prepare_readonly(
        config: &RuntimeConfig,
        request: ProviderRunRequest,
        run: BoundedReadOnlyRunConfig,
    ) -> Result<BoundedReadOnlyProviderRun, ProviderRunError> {
        let workspace_identity = validate_run(&run)?;
        let inner = Self::prepare_internal(config, request, None, None, Some(&run))?;
        Ok(BoundedReadOnlyProviderRun {
            inner,
            run,
            workspace_identity,
        })
    }
}

impl BoundedReadOnlyProviderRun {
    pub fn validate_runtime_paths(
        &self,
        events: &Path,
        model_calls: &Path,
        projection: Option<&Path>,
    ) -> Result<ReadOnlyRuntimePathCheck, ProviderRunError> {
        let check = ReadOnlyRuntimePathCheck {
            run: self.run.clone(),
            workspace_identity: self.workspace_identity.clone(),
            events: events.to_owned(),
            model_calls: model_calls.to_owned(),
            projection: projection.map(Path::to_owned),
        };
        check.revalidate()?;
        Ok(check)
    }

    pub fn run_with_transport<T: ProviderTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<BoundedReadOnlyProviderRunOutcome, ProviderRunError> {
        if self.inner.profile.endpoint_protocol != OPENAI_COMPATIBLE_CHAT_PROTOCOL {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        self.preflight(&events, &model_calls)?;
        let adapter = self.inner.profile.build_openai_adapter(transport)?;
        self.execute(adapter, events, model_calls, sink)
    }

    pub fn run_with_anthropic_transport<T: AnthropicTransport, S: ProgressSink + 'static>(
        self,
        transport: T,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<BoundedReadOnlyProviderRunOutcome, ProviderRunError> {
        if self.inner.profile.endpoint_protocol != ANTHROPIC_MESSAGES_PROTOCOL {
            return Err(ProviderRunError::UnsupportedProtocol);
        }
        self.preflight(&events, &model_calls)?;
        let adapter = AnthropicAdapter::new(
            self.inner.profile.provider.clone(),
            self.inner.profile.endpoint.clone(),
            transport,
            self.inner.profile.auth_ref.clone(),
        )
        .with_model_id(&self.inner.request.model_id);
        self.execute(adapter, events, model_calls, sink)
    }

    pub fn run_with_resolver<R: CredentialResolver, S: ProgressSink + 'static>(
        self,
        resolver: R,
        events: EventLog,
        model_calls: ModelCallStore,
        sink: S,
    ) -> Result<BoundedReadOnlyProviderRunOutcome, ProviderRunError> {
        self.preflight(&events, &model_calls)?;
        match self.inner.profile.endpoint_protocol.as_str() {
            OPENAI_COMPATIBLE_CHAT_PROTOCOL => {
                let transport = HttpProviderTransport::new(resolver)
                    .map_err(|_| ProviderRunError::TransportBuild)?;
                self.run_with_transport(transport, events, model_calls, sink)
            }
            ANTHROPIC_MESSAGES_PROTOCOL => {
                let transport = HttpAnthropicTransport::new(resolver)
                    .map_err(|_| ProviderRunError::TransportBuild)?;
                self.run_with_anthropic_transport(transport, events, model_calls, sink)
            }
            _ => Err(ProviderRunError::UnsupportedProtocol),
        }
    }

    fn preflight(&self, events: &EventLog, calls: &ModelCallStore) -> Result<(), ProviderRunError> {
        self.validate_runtime_paths(events.path(), &calls.storage_paths()[0], None)?;
        reject_reused_run_identity(events, calls, &self.inner.request)
    }

    fn execute<A: ResolvedModelAdapter, S: ProgressSink + 'static>(
        self,
        adapter: A,
        events: EventLog,
        calls: ModelCallStore,
        sink: S,
    ) -> Result<BoundedReadOnlyProviderRunOutcome, ProviderRunError> {
        let tools = ToolCallRecordStore::open(tool_call_store_path(events.path()))?;
        let request = &self.inner.request;
        let mut goal = GoalContract::new(
            &request.goal_id,
            &request.project_id,
            &request.objective,
            self.run.allowed_paths.clone(),
            Vec::new(),
            vec!["read_only_verification".into()],
        );
        goal.allowed_tools = vec!["git_status".into(), "markdown_link_check".into()];
        goal.allowed_models = vec![request.model_id.clone()];
        let limit = self.run.max_iterations;
        let budget = LoopBudget::new(limit, 2 * limit, limit, 1, 0, limit + 1, 2, 1);
        let mut controller = ReadOnlyLoopController::start_with_progress_sink(
            events,
            goal,
            &request.loop_run_id,
            &request.task_run_id,
            budget.clone(),
            sink,
        )?
        .with_model_call_store(calls)
        .with_tool_call_store(tools);
        let result = (|| {
            controller.select_strategy(
                TaskFeatures {
                    estimated_steps: limit,
                    uncertainty_level: 0.0,
                    interdependency_count: 1,
                    risk_level: RiskLevel::Low,
                    reversible: true,
                    blast_radius: BlastRadius::Local,
                    domain: ProfessionalDomain::SoftwareDevelopment,
                    requires_human_approval: false,
                    max_budget_multiplier: 1.5,
                },
                &budget,
            )?;
            configure_reported_token_budget(&mut controller, self.inner.reported_token_limit)?;
            let registry =
                ToolRegistry::baseline().map_err(|_| ProviderRunError::InvalidRequest)?;
            let gate = PolicyGate::new();
            let executor = ReadOnlyToolExecutor;
            let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &self.run.workspace);
            let adapter = ReadOnlyBoundAdapter {
                inner: adapter,
                config: &self.run,
                workspace_identity: &self.workspace_identity,
            };
            let outcome = controller.run_resolved_model_loop(
                &adapter,
                ModelRequest {
                    model_id: request.model_id.clone(),
                    goal_id: request.goal_id.clone(),
                    loop_run_id: request.loop_run_id.clone(),
                    step_id: "step-1".into(),
                    objective: request.objective.clone(),
                    observation: request.observation.clone(),
                },
                &self.inner.prompt,
                &self.inner.route,
                &runtime,
                limit,
            )?;
            Ok(BoundedReadOnlyProviderRunOutcome {
                outcome,
                loop_status: controller.status(),
                budget: controller.task_run().budget.clone(),
                reported_token_budget: controller.task_run().reported_token_budget.clone(),
            })
        })();
        result.or_else(|error| {
            controller.close_failed_foreground_run()?;
            Err(error)
        })
    }
}

struct ReadOnlyBoundAdapter<'a, A> {
    inner: A,
    config: &'a BoundedReadOnlyRunConfig,
    workspace_identity: &'a WorkspaceIdentity,
}

impl<A: ResolvedModelAdapter> ResolvedModelAdapter for ReadOnlyBoundAdapter<'_, A> {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure> {
        let response = self.inner.complete_resolved_with_action(request, pack)?;
        let valid = validate_run(self.config)
            .is_ok_and(|identity| &identity == self.workspace_identity)
            && response
                .response
                .action
                .as_ref()
                .is_none_or(|action| authorized_action(self.config, action));
        if !valid {
            return Err(ResolvedCallFailure::after_response(
                ModelAdapterError::ProviderTextUnmappable,
                &response.input_context_hash,
                &response.provider_response,
            ));
        }
        Ok(response)
    }
}

fn authorized_action(run: &BoundedReadOnlyRunConfig, action: &ModelActionEnvelope) -> bool {
    action.validate().is_ok()
        && action.path_scope.root == run.workspace.to_string_lossy()
        && action
            .path_scope
            .allowed_paths
            .iter()
            .all(|path| run.allowed_paths.contains(path))
        && action
            .path_scope
            .allowed_paths
            .iter()
            .enumerate()
            .all(|(index, path)| !action.path_scope.allowed_paths[..index].contains(path))
        && authorized_parameters(run, action, &action.tool_id, &action.parameters)
        && authorized_parameters(
            run,
            action,
            &action.verification.verifier_tool_id,
            &action.verification.parameters,
        )
}

fn authorized_parameters(
    run: &BoundedReadOnlyRunConfig,
    action: &ModelActionEnvelope,
    tool: &str,
    parameters: &Value,
) -> bool {
    match tool {
        "git_status" => parameters == &json!({"scope":"workspace"}),
        "markdown_link_check" => {
            parameters
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| {
                    run.allowed_paths.iter().any(|allowed| allowed == path)
                        && action
                            .path_scope
                            .allowed_paths
                            .iter()
                            .any(|allowed| allowed == path)
                        && parameters == &json!({"path":path})
                })
        }
        _ => false,
    }
}

#[cfg(not(unix))]
fn validate_run(_: &BoundedReadOnlyRunConfig) -> Result<WorkspaceIdentity, ProviderRunError> {
    Err(ProviderRunError::InvalidRequest)
}

#[cfg(unix)]
fn validate_run(run: &BoundedReadOnlyRunConfig) -> Result<WorkspaceIdentity, ProviderRunError> {
    run.workspace
        .to_str()
        .ok_or(ProviderRunError::InvalidRequest)?;
    if !run.workspace.is_absolute()
        || run.workspace.parent().is_none()
        || run
            .workspace
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || !(1..=16).contains(&run.max_iterations)
        || run.allowed_paths.is_empty()
        || run.allowed_paths.len() > 256
    {
        return Err(ProviderRunError::InvalidRequest);
    }
    reject_symlink_components(&run.workspace)?;
    let metadata = fs::metadata(&run.workspace).map_err(|_| ProviderRunError::InvalidRequest)?;
    if !metadata.is_dir() {
        return Err(ProviderRunError::InvalidRequest);
    }
    for (index, path) in run.allowed_paths.iter().enumerate() {
        let relative = Path::new(path);
        let normal: PathBuf = relative.components().collect();
        if path.is_empty()
            || !(path.ends_with(".md") || path.ends_with(".markdown"))
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || normal.as_os_str() != relative.as_os_str()
            || run.allowed_paths[..index].contains(path)
        {
            return Err(ProviderRunError::InvalidRequest);
        }
        let target = run.workspace.join(path);
        reject_symlink_components(&target)?;
        let target_metadata = fs::metadata(target).map_err(|_| ProviderRunError::InvalidRequest)?;
        if !target_metadata.is_file() {
            return Err(ProviderRunError::InvalidRequest);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if target_metadata.nlink() > 1 {
                return Err(ProviderRunError::InvalidRequest);
            }
        }
    }
    let canonical =
        fs::canonicalize(&run.workspace).map_err(|_| ProviderRunError::InvalidRequest)?;
    if canonical.parent().is_none()
        || serde_json::from_value::<ModelActionEnvelope>(action_example(run)["action"].clone())
            .map_err(|_| ProviderRunError::InvalidRequest)?
            .validate()
            .is_err()
    {
        return Err(ProviderRunError::InvalidRequest);
    }
    Ok(WorkspaceIdentity {
        canonical,
        #[cfg(unix)]
        device: {
            use std::os::unix::fs::MetadataExt;
            metadata.dev()
        },
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        },
    })
}

fn validate_stores(
    run: &BoundedReadOnlyRunConfig,
    event_path: &Path,
    calls_path: &Path,
    projection: Option<&Path>,
) -> Result<(), ProviderRunError> {
    let tools = tool_call_store_path(event_path);
    let mut tools_lock = tools.as_os_str().to_os_string();
    tools_lock.push(".lock");
    let mut paths = vec![event_path.to_path_buf(), tools, PathBuf::from(tools_lock)];
    paths.extend(ModelCallStore::storage_paths_for(calls_path));
    paths.extend(projection.map(Path::to_path_buf));
    let workspace =
        fs::canonicalize(&run.workspace).map_err(|_| ProviderRunError::InvalidRequest)?;
    let mut normalized = Vec::new();
    for path in &paths {
        if path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(ProviderRunError::InvalidRequest);
        }
        validate_storage_path(path)?;
        let key = normalized_path_key(path)?;
        if key.starts_with(&workspace) || normalized.contains(&key) {
            return Err(ProviderRunError::InvalidRequest);
        }
        normalized.push(key);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut identities = Vec::new();
        for path in paths.iter().cloned().chain(
            run.allowed_paths
                .iter()
                .map(|path| run.workspace.join(path)),
        ) {
            match fs::metadata(path) {
                Ok(metadata) => {
                    let identity = (metadata.dev(), metadata.ino());
                    if metadata.nlink() > 1 || identities.contains(&identity) {
                        return Err(ProviderRunError::InvalidRequest);
                    }
                    identities.push(identity);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err(ProviderRunError::InvalidRequest),
            }
        }
    }
    Ok(())
}

pub(super) fn tool_summary() -> Value {
    json!({
        "allowed":["git_status","markdown_link_check"],
        "parameters":{"git_status":{"scope":"workspace"},"markdown_link_check":{"path":"an explicitly authorized relative .md or .markdown path"}},
        "action_schema":{"schema_version":1,"action_id":"unique identifier","tool_id":"one allowed tool","parameters":"exact parameters for tool_id","path_scope":{"root":"authorized workspace","allowed_paths":"nonempty subset of caller paths"},"verification":{"verifier_tool_id":"one allowed tool","parameters":"exact parameters for verifier_tool_id"}},
        "decisions":["continue_observation","complete_loop","request_human"]
    })
}

pub(super) fn action_example(run: &BoundedReadOnlyRunConfig) -> Value {
    json!({"decision":"continue_observation","summary":"Inspect workspace metadata and verify authorized Markdown links","action":{
        "schema_version":1,"action_id":"inspect-1","tool_id":"git_status","parameters":{"scope":"workspace"},
        "path_scope":{"root":run.workspace,"allowed_paths":run.allowed_paths},
        "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":run.allowed_paths[0]}}
    }})
}
