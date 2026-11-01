use serde_json::json;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
use steerbox::domain::{AttemptReason, GoalContract, LoopBudget, LoopStatus, TaskRun};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ModelActionRuntime, ReadOnlyLoopController};
use steerbox::model_adapter::{ModelDecision, ModelRequest, ModelResponseWithAction};
use steerbox::model_call_store::ModelCallStore;
use steerbox::model_contracts::RouteDecision;
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::policy_gate::PolicyGate;
use steerbox::prompt_registry::{PromptProfile, PromptRegistry};
use steerbox::provider_adapter::{
    ProviderResponse, ProviderTransport, ProviderUsage, TransportError, TransportResponse,
};
use steerbox::provider_model_adapter::{
    ResolvedCallFailure, ResolvedModelAdapter, ResolvedModelResponseWithAction,
};
use steerbox::reported_token_budget::ReportedTokenBudgetError;
use steerbox::resolved_prompt::{
    resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
    ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use steerbox::tool_executor::ReadOnlyToolExecutor;
use steerbox::tool_registry::ToolRegistry;

const LOOP_ID: &str = "loop-token-boundary";
const LIMIT: u64 = 1_000;

#[derive(Default)]
struct KnownUsageAdapter(AtomicUsize);

impl ResolvedModelAdapter for KnownUsageAdapter {
    fn complete_resolved_with_action(
        &self,
        _request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ResolvedModelResponseWithAction {
            response: ModelResponseWithAction {
                decision: ModelDecision::ContinueObservation,
                summary: "fixture observation recorded".into(),
                action: None,
            },
            provider_response: ProviderResponse {
                request_id: "fixture-response".into(),
                model: "fixture-model".into(),
                text: String::new(),
                finish_reason: Some("stop".into()),
                usage: Some(ProviderUsage {
                    prompt_tokens: Some(7),
                    completion_tokens: Some(3),
                    total_tokens: Some(10),
                }),
            },
            input_context_hash: pack.metadata().rendered_messages_hash.clone(),
        })
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        for index in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "steerbox-token-boundary-{}-{index}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("fixture directory failed: {error}"),
            }
        }
        panic!("fixture directory allocation exhausted");
    }

    fn store(&self) -> ModelCallStore {
        ModelCallStore::open(self.0.join("calls.jsonl")).unwrap()
    }

    fn start(&self, restored: Option<TaskRun>) -> ReadOnlyLoopController {
        let goal = GoalContract::new(
            "goal-token-boundary",
            "project-token-boundary",
            "inspect fixture repository",
            vec![".".into()],
            vec![],
            vec!["observation recorded".into()],
        );
        let events = EventLog::open(self.0.join("events.jsonl")).unwrap();
        let controller = if let Some(task_run) = restored {
            ReadOnlyLoopController::start_with_task_run(events, goal, LOOP_ID, task_run)
        } else {
            ReadOnlyLoopController::start_with_task_run(
                events,
                goal,
                LOOP_ID,
                TaskRun::new(
                    "run-token-boundary",
                    "task-token-boundary",
                    LoopBudget::new(16, 0, 16, 8, 0, 16, 16, 8),
                )
                .unwrap(),
            )
        };
        controller.unwrap().with_model_call_store(self.store())
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (ResolvedPromptPack, RouteDecision) {
    fixture_for_route("route-token-boundary")
}

fn fixture_for_route(route_id: &str) -> (ResolvedPromptPack, RouteDecision) {
    let mut models = ModelRegistry::default();
    models
        .register_provider(ProviderProfile::new("fixture-provider", "Fixture", None).unwrap())
        .unwrap();
    models
        .register_endpoint(
            EndpointProfile::new(
                "fixture-endpoint",
                "fixture-provider",
                "https://fixture.example.test/v1",
                "openai",
            )
            .unwrap(),
        )
        .unwrap();
    models
        .register_model(
            ModelProfile::new(
                "fixture-model",
                "fixture-provider",
                "fixture-endpoint",
                "fixture-model",
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let mut prompts = PromptRegistry::default();
    prompts
        .register(PromptProfile::new("inspect", 1, "reviewer", "review", "inspect").unwrap())
        .unwrap();
    let route = RouteDecision {
        route_decision_id: route_id.into(),
        task_id: "task-token-boundary".into(),
        task_type: "review".into(),
        agent_role: "reviewer".into(),
        candidate_models: vec!["fixture-model".into()],
        selected_provider: "fixture-provider".into(),
        selected_model: "fixture-model".into(),
        selected_endpoint: "fixture-endpoint".into(),
        selection_reason: "boundary fixture".into(),
        rejected_candidates: vec![],
        cost_estimate: None,
        latency_estimate_ms: None,
        risk_level: "low".into(),
        privacy_requirement: "local".into(),
        fallback_chain: vec![],
        prompt_profile_id: "inspect".into(),
        prompt_profile_version: "1".into(),
        approval_required: false,
        created_at: "2026-11-01T00:00:00Z".into(),
    };
    let fragment = |name: &str| VersionedPromptFragment {
        id: name.into(),
        version: 1,
        content: format!("fixture {name}"),
    };
    let pack = resolve_prompt(
        &models,
        &prompts,
        ResolvePromptInput {
            route: &route,
            base: &fragment("base"),
            provider: &ProviderPromptFragment {
                provider_id: "fixture-provider".into(),
                fragment: fragment("provider"),
            },
            model: &ModelPromptFragment {
                provider_id: "fixture-provider".into(),
                model_id: "fixture-model".into(),
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
            policy_summary: &json!({"mode": "read_only"}),
            tool_summary: &json!({"tools": []}),
            context_package: &json!({"fixture": true}),
            objective: "inspect fixture repository",
            observation: "fixture repository observed",
            resolution_reason: "boundary fixture",
            created_at: "2026-11-01T00:00:00Z",
        },
    )
    .unwrap();
    (pack, route)
}

fn request(pack: &ResolvedPromptPack, step: &str) -> ModelRequest {
    ModelRequest {
        model_id: pack.model_id().into(),
        goal_id: "goal-token-boundary".into(),
        loop_run_id: LOOP_ID.into(),
        step_id: step.into(),
        objective: pack.objective().into(),
        observation: pack.observation().into(),
    }
}

fn assert_usage(controller: &ReadOnlyLoopController, calls: usize, blocked: bool) {
    let budget = controller.task_run().reported_token_budget().unwrap();
    assert_eq!(budget.limit(), LIMIT);
    assert_eq!(budget.known_input_tokens(), 7 * calls as u64);
    assert_eq!(budget.known_output_tokens(), 3 * calls as u64);
    assert_eq!(budget.known_total_tokens(), 10 * calls as u64);
    assert!(budget.known_total_tokens() < budget.limit());
    assert_eq!(budget.records().len(), calls);
    for usage in budget.records() {
        assert_eq!(usage.input_tokens(), Some(7));
        assert_eq!(usage.output_tokens(), Some(3));
    }
    assert_eq!(
        budget.blocked(),
        blocked.then_some(ReportedTokenBudgetError::AuditFailure)
    );
    assert_eq!(controller.status(), LoopStatus::Running);
}

fn assert_cross_entry_blocked(
    controller: &mut ReadOnlyLoopController,
    directory: &TestDirectory,
    adapter: &KnownUsageAdapter,
    pack: &ResolvedPromptPack,
    route: &RouteDecision,
) {
    let registry = ToolRegistry::baseline().unwrap();
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &directory.0);
    let before = controller.budget().clone();
    assert!(matches!(
        controller.run_resolved_model_step_with_action(
            adapter,
            request(pack, "cross-entry"),
            pack,
            route,
            &runtime,
            "cross-entry"
        ),
        Err(LoopControllerError::ReportedTokenBudget(
            ReportedTokenBudgetError::AuditFailure
        ))
    ));
    assert_eq!(controller.budget(), &before);
    assert_eq!(adapter.0.load(Ordering::SeqCst), 1);
    assert_usage(controller, 1, true);
}

#[test]
fn known_nonzero_usage_allows_new_ids_but_never_replays_settled_ids() {
    let directory = TestDirectory::new();
    let (pack, route) = fixture();
    let adapter = KnownUsageAdapter::default();
    let mut controller = directory.start(None);
    controller.enable_reported_token_budget(LIMIT).unwrap();
    for step in ["first", "second"] {
        assert_eq!(
            controller
                .run_resolved_model_step(&adapter, request(&pack, step), &pack, &route, step)
                .unwrap(),
            ModelDecision::ContinueObservation
        );
    }
    assert_usage(&controller, 2, false);
    let before = controller.budget().clone();
    for step in ["first", "second"] {
        assert!(matches!(
            controller.run_resolved_model_step(&adapter, request(&pack, step), &pack, &route, step),
            Err(LoopControllerError::ReportedTokenBudget(
                ReportedTokenBudgetError::CallAlreadySettled
            ))
        ));
    }
    assert_eq!(controller.budget(), &before);
    assert_eq!(adapter.0.load(Ordering::SeqCst), 2);
    assert_eq!(directory.store().read_all().unwrap().len(), 2);
}

#[test]
fn repaired_ordinary_audit_failure_blocks_other_entry_and_survives_restart() {
    let directory = TestDirectory::new();
    let (pack, route) = fixture();
    let adapter = KnownUsageAdapter::default();
    let mut controller = directory.start(None);
    controller.enable_reported_token_budget(LIMIT).unwrap();
    let path = directory.0.join("calls.jsonl");
    fs::create_dir(&path).unwrap();
    assert!(matches!(
        controller.run_resolved_model_step(
            &adapter,
            request(&pack, "audit-failed"),
            &pack,
            &route,
            "audit-failed"
        ),
        Err(LoopControllerError::ModelCallStore(_))
    ));
    assert_usage(&controller, 1, true);
    fs::rename(&path, directory.0.join("retired-audit-obstruction")).unwrap();
    fs::write(&path, "").unwrap();
    assert!(directory.store().read_all().unwrap().is_empty());
    assert_cross_entry_blocked(&mut controller, &directory, &adapter, &pack, &route);
    let snapshot = serde_json::to_string(controller.task_run()).unwrap();
    let restored: TaskRun = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(&restored, controller.task_run());
    let original_attempts = restored.attempts.clone();
    let original_current = restored.current_attempt_id.clone();
    drop(controller);
    let mut resumed = directory.start(Some(restored));
    assert_eq!(resumed.task_run().attempts, original_attempts);
    assert_eq!(resumed.task_run().current_attempt_id, original_current);
    assert_cross_entry_blocked(&mut resumed, &directory, &adapter, &pack, &route);
    assert!(directory.store().read_all().unwrap().is_empty());
}

#[derive(Clone)]
struct OneShotProgressFailure(Arc<AtomicBool>);

impl ProgressSink for OneShotProgressFailure {
    fn report(&mut self, event: ProgressEvent) -> io::Result<()> {
        if event.subject == ProgressSubject::Model
            && event.status == ProgressStatus::Completed
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(io::Error::other("fixture progress failure"));
        }
        Ok(())
    }
}

#[test]
fn repaired_post_call_progress_failure_blocks_other_entry_with_known_usage() {
    let directory = TestDirectory::new();
    let (pack, route) = fixture();
    let adapter = KnownUsageAdapter::default();
    let armed = Arc::new(AtomicBool::new(true));
    let mut sink = OneShotProgressFailure(armed.clone());
    let mut controller = directory.start(None).with_progress_sink(sink.clone());
    controller.enable_reported_token_budget(LIMIT).unwrap();
    assert!(matches!(
        controller.run_resolved_model_step(
            &adapter,
            request(&pack, "progress-failed"),
            &pack,
            &route,
            "progress-failed"
        ),
        Err(LoopControllerError::ProgressReport(_))
    ));
    assert!(!armed.load(Ordering::SeqCst));
    sink.report(ProgressEvent {
        subject: ProgressSubject::Model,
        subject_id: "repair-probe".into(),
        status: ProgressStatus::Completed,
        summary: None,
        progress: None,
    })
    .unwrap();
    assert_eq!(directory.store().read_all().unwrap().len(), 1);
    assert_cross_entry_blocked(&mut controller, &directory, &adapter, &pack, &route);
}

#[test]
fn task_run_roundtrip_resumes_used_ids_and_totals_without_initial_attempt_reset() {
    let directory = TestDirectory::new();
    let (pack, route) = fixture();
    let adapter = KnownUsageAdapter::default();
    let mut controller = directory.start(None);
    controller.enable_reported_token_budget(LIMIT).unwrap();
    controller
        .run_resolved_model_step(&adapter, request(&pack, "first"), &pack, &route, "first")
        .unwrap();
    let restored: TaskRun =
        serde_json::from_str(&serde_json::to_string(controller.task_run()).unwrap()).unwrap();
    assert_eq!(&restored, controller.task_run());
    let original_attempts = restored.attempts.clone();
    let original_current = restored.current_attempt_id.clone();
    let original_budget = controller.budget().clone();
    drop(controller);
    let mut resumed = directory.start(Some(restored));
    assert_eq!(resumed.task_run().attempts, original_attempts);
    assert_eq!(resumed.task_run().current_attempt_id, original_current);
    assert_eq!(resumed.budget(), &original_budget);
    assert_usage(&resumed, 1, false);
    assert!(matches!(
        resumed.run_resolved_model_step(&adapter, request(&pack, "first"), &pack, &route, "first"),
        Err(LoopControllerError::ReportedTokenBudget(
            ReportedTokenBudgetError::CallAlreadySettled
        ))
    ));
    assert_eq!(adapter.0.load(Ordering::SeqCst), 1);
    assert_eq!(resumed.budget(), &original_budget);
    assert_eq!(
        resumed
            .run_resolved_model_step(&adapter, request(&pack, "next"), &pack, &route, "next")
            .unwrap(),
        ModelDecision::ContinueObservation
    );
    assert_usage(&resumed, 2, false);
    assert_eq!(resumed.task_run().attempts.len(), original_attempts.len());
    assert_eq!(resumed.task_run().current_attempt_id, original_current);
    assert_eq!(adapter.0.load(Ordering::SeqCst), 2);
}

#[derive(Clone)]
struct KnownUsageTransport(Arc<AtomicUsize>);

impl ProviderTransport for KnownUsageTransport {
    fn send(
        &self,
        _endpoint: &str,
        _auth_ref: Option<&str>,
        _request_body: &str,
        _timeout: Duration,
        _streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(TransportResponse::body(
            200,
            json!({
                "id": "fixture-response", "model": "fixture-model",
                "choices": [{"message": {"role": "assistant", "content":
                    r#"{"decision":"continue_observation","summary":"fixture observed"}"#
                }, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
            })
            .to_string(),
        ))
    }
}

#[test]
fn actual_upgrade_child_send_accumulates_nonzero_usage_in_the_same_task_run() {
    let directory = TestDirectory::new();
    let (pack, route) = fixture();
    let config = RuntimeConfig::from_json_str(
        &json!({
            "schema_version": 1,
            "project_ref": "token-boundary-fixture",
            "data_root": "/temp/steerbox/token-boundary-fixture",
            "model_profiles": [{
                "model_id": "fixture-model", "provider": "fixture-provider",
                "endpoint": "https://fixture.example.test/v1",
                "endpoint_protocol": "openai-compatible-chat",
                "effect": {"selected_level": "fast", "maximum_level": "strong",
                    "automatic_upgrade": true,
                    "mapping": {"fast": "low", "balanced": "medium", "strong": "high"}}
            }]
        })
        .to_string(),
    )
    .unwrap();
    let sends = Arc::new(AtomicUsize::new(0));
    let transport = KnownUsageTransport(sends.clone());
    let candidate = config
        .build_effect_bound_candidate(
            "fixture-model",
            EffectLevel::Fast,
            transport.clone(),
            request(&pack, "upgrade"),
            pack,
            route,
        )
        .unwrap();
    let mut controller = directory.start(None);
    controller.enable_reported_token_budget(LIMIT).unwrap();
    let mut evaluations = 0;
    assert_eq!(
        controller
            .run_resolved_upgrade_model_step(
                candidate,
                "upgrade",
                |_| {
                    evaluations += 1;
                    QualityObservation {
                        low_quality: evaluations == 1,
                        accepted: evaluations == 2,
                        ..QualityObservation::default()
                    }
                },
                |child_route, level| {
                    assert_eq!(level, EffectLevel::Balanced);
                    let (pack, route) = fixture_for_route(&child_route.route_decision_id);
                    assert_eq!(&route, child_route);
                    Ok(config
                        .build_effect_bound_candidate(
                            "fixture-model",
                            level,
                            transport,
                            request(&pack, "upgrade"),
                            pack,
                            route,
                        )
                        .unwrap())
                },
            )
            .unwrap(),
        ModelDecision::ContinueObservation
    );
    assert_eq!(evaluations, 2);
    assert_eq!(sends.load(Ordering::SeqCst), 2);
    assert_usage(&controller, 2, false);
    assert_eq!(controller.budget().model_calls_used, 2);
    assert_eq!(controller.task_run().attempts.len(), 2);
    let parent = &controller.task_run().attempts[0];
    let child = &controller.task_run().attempts[1];
    assert_eq!(parent.reason, AttemptReason::Initial);
    assert_eq!(child.reason, AttemptReason::Upgrade);
    assert_eq!(
        child.parent_attempt_id.as_deref(),
        Some(parent.attempt_id.as_str())
    );
    assert_eq!(
        controller.task_run().current_attempt_id.as_deref(),
        Some(child.attempt_id.as_str())
    );
    let budget = controller.task_run().reported_token_budget().unwrap();
    assert_ne!(
        budget.records()[0].model_call_id(),
        budget.records()[1].model_call_id()
    );
    assert_eq!(directory.store().read_all().unwrap().len(), 2);
}
