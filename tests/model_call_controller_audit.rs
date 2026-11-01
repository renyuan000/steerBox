use serde_json::json;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steerbox::anthropic_adapter::{
    AnthropicAdapter, AnthropicHeaders, AnthropicTransport, AnthropicTransportError,
};
use steerbox::domain::{BudgetKind, GoalContract, HumanSteeringMode, LoopBudget};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{
    LoopControllerError, ModelActionRuntime, ReadOnlyLoopController, ResolvedModelLoopStopReason,
    WorkspaceEditActionRuntime,
};
use steerbox::model_adapter::{
    DecisionOutputRejection, ModelAdapterError, ModelDecision, ModelRequest,
};
use steerbox::model_call_store::ModelCallStore;
use steerbox::model_contracts::RouteDecision;
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::policy_gate::PolicyGate;
use steerbox::prompt_registry::{PromptProfile, PromptRegistry};
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderTransport, TransportError, TransportResponse,
};
use steerbox::provider_model_adapter::ProviderModelAdapter;
use steerbox::resolved_prompt::{
    resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
    ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
};
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallOutput, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;

const BASE_URL: &str = "https://fixture.example.test/v1";

fn token_budget_request(pack: &ResolvedPromptPack, step_id: &str) -> ModelRequest {
    ModelRequest {
        model_id: pack.model_id().into(),
        goal_id: "goal-model-audit".into(),
        loop_run_id: "loop-model-audit".into(),
        step_id: step_id.into(),
        objective: pack.objective().into(),
        observation: pack.observation().into(),
    }
}

fn token_usage_transport(input: Option<u64>, output: Option<u64>, valid: bool) -> FakeTransport {
    FakeTransport {
        result: Ok(TransportResponse::body(
            200,
            json!({
                "id": "fixture-response", "model": "wire-model",
                "choices": [{"message": {"role": "assistant", "content": if valid {
                    r#"{"decision":"continue_observation","summary":"fixture observed"}"#
                } else { "not decision JSON" }}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": input, "completion_tokens": output, "total_tokens": 999}
            })
            .to_string(),
        )),
        sends: Arc::new(Mutex::new(Vec::new())),
    }
}

#[test]
fn reported_token_budget_zero_limit_stops_before_count_budgets_and_send() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller(&directory)
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(0).unwrap();
    assert!(matches!(
        controller.run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "zero"),
            &pack,
            &route,
            "zero"
        ),
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(controller.budget().steps_used, 0);
    assert_eq!(controller.budget().model_calls_used, 0);
    assert!(sends.lock().unwrap().is_empty());
    assert!(controller
        .task_run()
        .reported_token_budget()
        .unwrap()
        .records()
        .is_empty());
}

#[test]
fn reported_token_budget_settles_before_followup_and_rejects_duplicate_send() {
    for usage in [(Some(2), Some(3)), (None, Some(3)), (Some(0), Some(0))] {
        let directory = TestDirectory::new();
        let (pack, route) = resolved_fixture();
        let transport = token_usage_transport(usage.0, usage.1, true);
        let sends = transport.sends.clone();
        let adapter = ProviderModelAdapter::new(
            OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
            None,
        );
        let mut controller = controller_with_budget(
            &directory,
            vec![".".into()],
            LoopBudget::new(6, 0, 6, 1, 0, 6, 6, 1),
        )
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        controller.enable_reported_token_budget(5).unwrap();
        controller
            .run_resolved_model_step(
                &adapter,
                token_budget_request(&pack, "first"),
                &pack,
                &route,
                "first",
            )
            .unwrap();
        let before = controller.budget().clone();
        assert!(matches!(
            controller.run_resolved_model_step(
                &adapter,
                token_budget_request(&pack, "first"),
                &pack,
                &route,
                "first"
            ),
            Err(LoopControllerError::ReportedTokenBudget(_))
        ));
        assert_eq!(controller.budget(), &before);
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert!(controller.enable_reported_token_budget(100).is_err());
        let budget = controller.task_run().reported_token_budget().unwrap();
        assert_eq!(budget.records()[0].input_tokens(), usage.0);
        assert_eq!(budget.records()[0].output_tokens(), usage.1);
        assert_eq!(
            budget.known_total_tokens(),
            usage.0.unwrap_or(0) + usage.1.unwrap_or(0)
        );
        let next = controller.run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "next"),
            &pack,
            &route,
            "next",
        );
        if usage == (Some(0), Some(0)) {
            next.unwrap();
            assert_eq!(sends.lock().unwrap().len(), 2);
        } else {
            assert!(matches!(
                next,
                Err(LoopControllerError::ReportedTokenBudget(_))
            ));
            assert_eq!(sends.lock().unwrap().len(), 1);
        }
    }
}

#[test]
fn reported_token_budget_failure_preserves_original_error_and_usage() {
    for usage in [
        (Some(4), Some(3)),
        (Some(0), None),
        (None, Some(2)),
        (None, None),
    ] {
        let directory = TestDirectory::new();
        let (pack, route) = resolved_fixture();
        let transport = token_usage_transport(usage.0, usage.1, false);
        let adapter = ProviderModelAdapter::new(
            OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
            None,
        );
        let mut controller = controller(&directory)
            .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        controller.enable_reported_token_budget(20).unwrap();
        assert!(matches!(
            controller.run_resolved_model_step(
                &adapter,
                token_budget_request(&pack, "failure"),
                &pack,
                &route,
                "failure"
            ),
            Err(LoopControllerError::ModelAdapter(_))
        ));
        let budget = controller.task_run().reported_token_budget().unwrap();
        assert_eq!(
            budget.known_total_tokens(),
            usage.0.unwrap_or(0) + usage.1.unwrap_or(0)
        );
        assert_eq!(budget.records()[0].input_tokens(), usage.0);
        assert_eq!(budget.records()[0].output_tokens(), usage.1);
        assert_eq!(
            budget.blocked().is_some(),
            usage.0.is_none() || usage.1.is_none()
        );
    }
}

#[test]
fn reported_token_budget_legacy_api_is_rejected_without_send_or_usage() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller(&directory);
    controller.enable_reported_token_budget(100).unwrap();
    let before = controller.budget().clone();
    assert!(matches!(
        controller.run_model_step(&adapter, "logical-model", "legacy", "observation"),
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(controller.budget(), &before);
    assert!(sends.lock().unwrap().is_empty());
    assert!(controller
        .task_run()
        .reported_token_budget()
        .unwrap()
        .records()
        .is_empty());
}

#[test]
fn reported_token_budget_success_and_failure_share_one_total() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let good = token_usage_transport(Some(2), Some(3), true);
    let bad = token_usage_transport(Some(4), Some(1), false);
    let sends_good = good.sends.clone();
    let sends_bad = bad.sends.clone();
    let good = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, good),
        None,
    );
    let bad = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, bad),
        None,
    );
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(6, 0, 6, 1, 0, 6, 6, 1),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(10).unwrap();
    controller
        .run_resolved_model_step(
            &good,
            token_budget_request(&pack, "success"),
            &pack,
            &route,
            "success",
        )
        .unwrap();
    assert!(matches!(
        controller.run_resolved_model_step(
            &bad,
            token_budget_request(&pack, "failed"),
            &pack,
            &route,
            "failed"
        ),
        Err(LoopControllerError::ModelAdapter(_))
    ));
    assert_eq!(
        controller
            .task_run()
            .reported_token_budget()
            .unwrap()
            .known_total_tokens(),
        10
    );
    let before = controller.budget().clone();
    assert!(matches!(
        controller.run_resolved_model_step(
            &good,
            token_budget_request(&pack, "after"),
            &pack,
            &route,
            "after"
        ),
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(controller.budget(), &before);
    assert_eq!(sends_good.lock().unwrap().len(), 1);
    assert_eq!(sends_bad.lock().unwrap().len(), 1);
}

#[test]
fn reported_token_budget_transport_failure_keeps_unknown_and_original_error() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::failing();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(6, 0, 6, 1, 0, 6, 6, 1),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(100).unwrap();
    assert!(matches!(
        controller.run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "failed"),
            &pack,
            &route,
            "failed"
        ),
        Err(LoopControllerError::ModelAdapter(_))
    ));
    let record = &controller
        .task_run()
        .reported_token_budget()
        .unwrap()
        .records()[0];
    assert_eq!(record.input_tokens(), None);
    assert_eq!(record.output_tokens(), None);
    assert!(matches!(
        controller.run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "after"),
            &pack,
            &route,
            "after"
        ),
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
}

#[test]
fn reported_token_budget_task_run_roundtrip_preserves_state_and_old_snapshots_disable() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = token_usage_transport(Some(2), Some(3), true);
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller(&directory)
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let old = serde_json::to_value(controller.task_run()).unwrap();
    assert!(old.get("reported_token_budget").is_none());
    let restored: steerbox::domain::TaskRun = serde_json::from_value(old).unwrap();
    assert!(restored.reported_token_budget().is_none());
    controller.enable_reported_token_budget(5).unwrap();
    controller
        .run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "first"),
            &pack,
            &route,
            "first",
        )
        .unwrap();
    let mut value = serde_json::to_value(controller.task_run()).unwrap();
    let restored: steerbox::domain::TaskRun = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(&restored, controller.task_run());
    value["reported_token_budget"]["known_total_tokens"] = json!(0);
    assert!(serde_json::from_value::<steerbox::domain::TaskRun>(value).is_err());
}

#[test]
fn reported_token_budget_cannot_be_enabled_after_an_unmetered_model_call() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller(&directory)
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller
        .run_resolved_model_step(
            &adapter,
            token_budget_request(&pack, "first"),
            &pack,
            &route,
            "first",
        )
        .unwrap();
    assert!(controller.enable_reported_token_budget(100).is_err());
    assert!(controller.task_run().reported_token_budget().is_none());
}

#[test]
fn reported_token_budget_fallback_attempt_does_not_clear_unknown_usage() {
    let directory = TestDirectory::new();
    let (primary_pack, primary_route) = resolved_fixture_for(
        "logical-model",
        "route-primary",
        vec!["backup-model".into()],
    );
    let (fallback_pack, fallback_route) =
        resolved_fixture_for("backup-model", "route-backup", vec![]);
    let transport = FakeTransport::failing();
    let sends = transport.sends.clone();
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(100).unwrap();
    let result =
        controller.run_resolved_routed_model_step(&primary_route, "fallback", |route, model| {
            let (pack, route) = if model == "logical-model" {
                (primary_pack.clone(), route.clone())
            } else {
                (fallback_pack.clone(), fallback_route.clone())
            };
            Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                request: token_budget_request(&pack, "fallback"),
                pack,
                route,
                adapter: Box::new(ProviderModelAdapter::new(
                    OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport.clone()),
                    None,
                )),
            })
        });
    assert!(matches!(
        result,
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(controller.budget().model_calls_used, 1);
    assert_eq!(controller.budget().steps_used, 1);
    assert_eq!(controller.task_run().attempts.len(), 2);
    assert_eq!(
        controller
            .task_run()
            .reported_token_budget()
            .unwrap()
            .records()
            .len(),
        1
    );
}

#[test]
fn reported_token_budget_upgrade_attempt_does_not_replenish_known_usage() {
    use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let transport = token_usage_transport(Some(3), Some(2), true);
    let sends = transport.sends.clone();
    let mut controller = upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(5).unwrap();
    let result = controller.run_resolved_upgrade_model_step(
        primary_upgrade_candidate(&config, transport.clone()),
        "step-upgrade",
        |_| QualityObservation {
            low_quality: true,
            ..QualityObservation::default()
        },
        |child_route, level| {
            assert_eq!(level, EffectLevel::Balanced);
            let (pack, route) =
                resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
            Ok(config
                .build_effect_bound_candidate(
                    "wire-model",
                    level,
                    transport,
                    upgrade_request(&pack),
                    pack,
                    route,
                )
                .unwrap())
        },
    );
    assert!(matches!(
        result,
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(controller.budget().model_calls_used, 1);
    assert_eq!(controller.budget().steps_used, 1);
    assert_eq!(
        controller
            .task_run()
            .reported_token_budget()
            .unwrap()
            .known_total_tokens(),
        5
    );
}

#[test]
fn reported_token_budget_continuation_stops_after_verified_action_without_refill() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().unwrap();
    let first = json!({"decision":"continue_observation", "summary":"fixture action", "action": {
        "schema_version": 1, "action_id": "token-action", "tool_id": "git_status", "parameters": {"scope":"workspace"},
        "path_scope": {"root":workspace.display().to_string(), "allowed_paths":["README.md"]},
        "verification": {"verifier_tool_id":"markdown_link_check", "parameters":{"path":"README.md"}}
    }}).to_string();
    let mut wire: serde_json::Value = serde_json::from_str(&openai_response(first)).unwrap();
    wire["usage"] = json!({"prompt_tokens":3, "completion_tokens":2});
    let transport = SequenceTransport::new(vec![wire.to_string()]);
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_budget(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
        LoopBudget::new(4, 4, 4, 1, 0, 4, 4, 1),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    controller.enable_reported_token_budget(5).unwrap();
    let registry = ToolRegistry::baseline().unwrap();
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);
    let result = controller.run_resolved_model_loop(
        &adapter,
        token_budget_request(&pack, "continuation"),
        &pack,
        &route,
        &runtime,
        3,
    );
    assert!(matches!(
        result,
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(controller.budget().steps_used, 1);
    assert_eq!(controller.budget().model_calls_used, 1);
    let records = ModelCallStore::open(directory.model_calls())
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool_call_count, 2);
    assert_eq!(records[0].verification_status, "passed");
    assert_eq!(
        controller
            .task_run()
            .reported_token_budget()
            .unwrap()
            .known_total_tokens(),
        5
    );
}

#[test]
fn reported_token_budget_wrapped_snapshot_audit_failure_blocks_later_send() {
    use steerbox::reported_token_budget::ReportedTokenBudgetError;
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    fs::write(directory.path.join("README.md"), "# Before\n").unwrap();
    let mut goal = GoalContract::new(
        "goal-model-audit",
        "project-model-audit",
        pack.objective(),
        vec!["README.md".into()],
        vec![],
        vec!["verified".into()],
    );
    goal.allowed_tools = vec!["edit_markdown".into(), "markdown_link_check".into()];
    goal.human_steering_mode = HumanSteeringMode::AutonomousLoop;
    let mut wire: serde_json::Value = serde_json::from_str(&openai_response(json!({
        "decision":"continue_observation", "summary":"bounded edit", "action": {
            "schema_version":1, "action_id":"snapshot-failure-action", "tool_id":"edit_markdown",
            "parameters":{"path":"README.md", "expected":"# Before\n", "replacement":"# After\n"},
            "path_scope":{"root":directory.path.display().to_string(), "allowed_paths":["README.md"]},
            "verification":{"verifier_tool_id":"markdown_link_check", "parameters":{"path":"README.md"}}
        }
    }).to_string())).unwrap();
    wire["usage"] = json!({"prompt_tokens":3, "completion_tokens":2});
    let transport = FakeTransport {
        result: Ok(TransportResponse::body(200, wire.to_string())),
        sends: Arc::new(Mutex::new(vec![])),
    };
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.events()).unwrap(),
        goal,
        "loop-model-audit",
        "task-run-model-audit",
        LoopBudget::new(4, 4, 4, 1, 0, 4, 4, 1),
    )
    .unwrap()
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap())
    .with_checkpoint_store(directory.path.join("checkpoints.jsonl"))
    .with_audit_stores(
        directory.path.join("traces.jsonl"),
        directory.path.join("side-effects.jsonl"),
    )
    .with_snapshot_store(directory.path.join("invalid-nested-snapshots"));
    controller.enable_reported_token_budget(100).unwrap();
    let registry = ToolRegistry::baseline().unwrap();
    let gate = PolicyGate::new();
    let workspace_executor = WorkspaceToolExecutor;
    let executor = ReadOnlyToolExecutor;
    let runtime = WorkspaceEditActionRuntime::new(
        &registry,
        &gate,
        &workspace_executor,
        &executor,
        &directory.path,
        false,
    );
    let result = controller.run_resolved_model_step_with_workspace_edit(
        &adapter,
        token_budget_request(&pack, "snapshot-failed"),
        &pack,
        &route,
        &runtime,
        "snapshot-failed",
    );
    assert!(matches!(
        result,
        Err(LoopControllerError::WorkspaceSnapshot(_))
    ));
    let budget = controller.task_run().reported_token_budget().unwrap();
    assert_eq!(budget.known_total_tokens(), 5);
    assert_eq!(
        budget.blocked(),
        Some(ReportedTokenBudgetError::AuditFailure)
    );
    assert!(matches!(
        controller.run_resolved_model_step_with_workspace_edit(
            &adapter,
            token_budget_request(&pack, "after-snapshot-failed"),
            &pack,
            &route,
            &runtime,
            "after-snapshot-failed"
        ),
        Err(LoopControllerError::ReportedTokenBudget(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# Before\n"
    );
}

#[derive(Clone)]
struct RecordingProgressSink(Arc<Mutex<Vec<ProgressEvent>>>);

impl ProgressSink for RecordingProgressSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        self.0.lock().expect("progress capture lock").push(event);
        Ok(())
    }
}

#[derive(Clone)]
struct FakeTransport {
    result: Result<TransportResponse, TransportError>,
    sends: Arc<Mutex<Vec<String>>>,
}

impl FakeTransport {
    fn valid() -> Self {
        Self {
            result: Ok(TransportResponse::body(
                200,
                r#"{"id":"fixture-response","model":"wire-model","choices":[{"message":{"role":"assistant","content":"{\"decision\":\"continue_observation\",\"summary\":\"fixture observed\"}"},"finish_reason":"stop"}]}"#,
            )),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing() -> Self {
        Self {
            result: Err(TransportError::Network),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_decision_content(content: String) -> Self {
        Self {
            result: Ok(TransportResponse::body(
                200,
                serde_json::json!({
                    "id": "fixture-response",
                    "model": "wire-model",
                    "choices": [{
                        "message": {"role": "assistant", "content": content},
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ProviderTransport for FakeTransport {
    fn send(
        &self,
        _endpoint: &str,
        _auth_ref: Option<&str>,
        request_body: &str,
        _timeout: Duration,
        _streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.sends
            .lock()
            .expect("send capture lock")
            .push(request_body.into());
        self.result.clone()
    }
}

impl AnthropicTransport for FakeTransport {
    fn send(
        &self,
        _endpoint: &str,
        _headers: &AnthropicHeaders,
        request_body: &str,
        _timeout: Duration,
        _streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        self.sends.lock().unwrap().push(request_body.into());
        self.result
            .clone()
            .map_err(|_| AnthropicTransportError::Network)
    }
}

#[derive(Clone)]
struct SequenceTransport {
    responses: Arc<Mutex<VecDeque<String>>>,
    sends: Arc<Mutex<Vec<String>>>,
}

impl SequenceTransport {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ProviderTransport for SequenceTransport {
    fn send(
        &self,
        _endpoint: &str,
        _auth_ref: Option<&str>,
        request_body: &str,
        _timeout: Duration,
        _streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.sends
            .lock()
            .expect("sequence send capture lock")
            .push(request_body.into());
        self.responses
            .lock()
            .expect("sequence response lock")
            .pop_front()
            .map(|body| TransportResponse::body(200, body))
            .ok_or(TransportError::Network)
    }
}

fn openai_response(content: String) -> String {
    serde_json::json!({
        "id": "fixture-response",
        "model": "wire-model",
        "choices": [{
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }]
    })
    .to_string()
}

fn fragment(id: &str, content: &str) -> VersionedPromptFragment {
    VersionedPromptFragment {
        id: id.into(),
        version: 1,
        content: content.into(),
    }
}

fn resolved_fixture() -> (ResolvedPromptPack, RouteDecision) {
    resolved_fixture_for("logical-model", "route-model-audit", vec![])
}

fn resolved_fixture_for(
    model_id: &str,
    route_decision_id: &str,
    fallback_chain: Vec<String>,
) -> (ResolvedPromptPack, RouteDecision) {
    resolved_fixture_for_protocol(model_id, route_decision_id, fallback_chain, "openai")
}

fn resolved_fixture_for_protocol(
    model_id: &str,
    route_decision_id: &str,
    fallback_chain: Vec<String>,
    protocol: &str,
) -> (ResolvedPromptPack, RouteDecision) {
    resolved_fixture_with_boundary(model_id, route_decision_id, fallback_chain, protocol, "")
}

fn resolved_fixture_with_boundary(
    model_id: &str,
    route_decision_id: &str,
    fallback_chain: Vec<String>,
    protocol: &str,
    boundary: &str,
) -> (ResolvedPromptPack, RouteDecision) {
    let mut models = ModelRegistry::default();
    models
        .register_provider(ProviderProfile::new("fixture-provider", "Fixture", None).unwrap())
        .unwrap();
    models
        .register_endpoint(
            EndpointProfile::new("fixture-endpoint", "fixture-provider", BASE_URL, protocol)
                .unwrap(),
        )
        .unwrap();
    models
        .register_model(
            ModelProfile::new(
                model_id,
                "fixture-provider",
                "fixture-endpoint",
                "wire-model",
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let mut prompts = PromptRegistry::default();
    prompts
        .register(PromptProfile::new("inspect", 1, "reviewer", "review", "TASK_BODY").unwrap())
        .unwrap();
    let mut route = RouteDecision {
        route_decision_id: route_decision_id.into(),
        task_id: "task-model-audit".into(),
        task_type: "review".into(),
        agent_role: "reviewer".into(),
        candidate_models: std::iter::once(model_id.to_owned())
            .chain(fallback_chain.iter().cloned())
            .collect(),
        selected_provider: "fixture-provider".into(),
        selected_model: model_id.into(),
        selected_endpoint: "fixture-endpoint".into(),
        selection_reason: "explicit fixture".into(),
        rejected_candidates: vec![],
        cost_estimate: None,
        latency_estimate_ms: None,
        risk_level: "low".into(),
        privacy_requirement: "local".into(),
        fallback_chain,
        prompt_profile_id: "inspect".into(),
        prompt_profile_version: "1".into(),
        approval_required: false,
        created_at: "2026-10-11T00:00:00Z".into(),
    };
    if boundary == "privacy" {
        route.privacy_requirement = "restricted".into();
    }
    if boundary == "risk" {
        route.risk_level = "medium".into();
    }
    if boundary == "task" {
        route.task_id = "another-task".into();
    }
    let pack = resolve_prompt(
        &models,
        &prompts,
        ResolvePromptInput {
            route: &route,
            base: &fragment(
                "base",
                if boundary == "prompt" {
                    "OTHER_BASE"
                } else {
                    "BASE"
                },
            ),
            provider: &ProviderPromptFragment {
                provider_id: "fixture-provider".into(),
                fragment: fragment("provider", "PROVIDER"),
            },
            model: &ModelPromptFragment {
                provider_id: "fixture-provider".into(),
                model_id: model_id.into(),
                fragment: fragment("model", "MODEL"),
            },
            role: &AgentRolePrompt {
                role: "reviewer".into(),
                fragment: fragment("role", "ROLE"),
            },
            task: &TaskPromptPack {
                prompt_id: "inspect".into(),
                version: 1,
                task_type: "review".into(),
            },
            skills: &[],
            policy_summary: &json!({"mode":if boundary == "policy" {"other"} else {"read_only"}}),
            tool_summary: &if boundary == "tool" {
                json!({"tools":["git_status"]})
            } else {
                json!({"tools":[]})
            },
            context_package: &json!({"fixture":boundary != "context"}),
            objective: if boundary == "objective" {
                "another objective"
            } else {
                "inspect the repository"
            },
            observation: if boundary == "observation" {
                "another observation"
            } else {
                "repository observed"
            },
            resolution_reason: "explicit fixture",
            created_at: "2026-10-11T00:00:00Z",
        },
    )
    .expect("resolved fixture builds");
    (pack, route)
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        for attempt in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "steerbox-model-call-controller-audit-{}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("test fixture directory creation failed: {error}"),
            }
        }
        panic!("could not allocate an isolated test fixture directory");
    }

    fn events(&self) -> PathBuf {
        self.path.join("events.jsonl")
    }

    fn model_calls(&self) -> PathBuf {
        self.path.join("model-calls.jsonl")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn controller(directory: &TestDirectory) -> ReadOnlyLoopController {
    controller_with_allowed_tools(directory, vec![".".into()])
}

fn controller_with_allowed_tools(
    directory: &TestDirectory,
    allowed_tools: Vec<String>,
) -> ReadOnlyLoopController {
    controller_with_budget(
        directory,
        allowed_tools,
        LoopBudget::new(2, 0, 1, 1, 0, 1, 1, 1),
    )
}

fn effect_upgrade_config(automatic_upgrade: bool) -> steerbox::runtime_config::RuntimeConfig {
    let mut effect = json!({
        "selected_level": "fast",
        "mapping": {"fast": "low", "balanced": "medium", "strong": "high"},
    });
    if automatic_upgrade {
        effect["automatic_upgrade"] = json!(true);
        effect["maximum_level"] = json!("strong");
    }
    steerbox::runtime_config::RuntimeConfig::from_json_str(&json!({
        "schema_version": 1, "project_ref": "upgrade-fixture",
        "data_root": "/temp/steerbox/upgrade-fixture",
        "model_profiles": [{"model_id": "wire-model", "provider": "fixture-provider",
            "endpoint": BASE_URL, "endpoint_protocol": "openai-compatible-chat", "effect": effect}],
    }).to_string()).unwrap()
}

fn upgrade_controller(directory: &TestDirectory, budget: LoopBudget) -> ReadOnlyLoopController {
    upgrade_controller_with_models(directory, budget, vec![])
}

fn upgrade_controller_with_models(
    directory: &TestDirectory,
    budget: LoopBudget,
    allowed_models: Vec<String>,
) -> ReadOnlyLoopController {
    let mut goal = GoalContract::new(
        "goal-model-audit",
        "project-model-audit",
        "inspect the repository",
        vec![".".into()],
        Vec::new(),
        vec!["model_result_recorded".into()],
    );
    goal.allowed_models = allowed_models;
    ReadOnlyLoopController::start_with_task_run(
        EventLog::open(directory.events()).unwrap(),
        goal,
        "loop-model-audit",
        steerbox::domain::TaskRun::new("task-run-model-audit", "task-model-audit", budget).unwrap(),
    )
    .unwrap()
}

fn upgrade_request(pack: &ResolvedPromptPack) -> ModelRequest {
    ModelRequest {
        model_id: pack.model_id().into(),
        goal_id: "goal-model-audit".into(),
        loop_run_id: "loop-model-audit".into(),
        step_id: "step-upgrade".into(),
        objective: pack.objective().into(),
        observation: pack.observation().into(),
    }
}

fn primary_upgrade_candidate(
    config: &steerbox::runtime_config::RuntimeConfig,
    transport: FakeTransport,
) -> steerbox::runtime_config::EffectBoundResolvedCandidate {
    let (pack, route) = resolved_fixture_for("wire-model", "route-upgrade-primary", vec![]);
    config
        .build_effect_bound_candidate(
            "wire-model",
            steerbox::adaptive_routing::EffectLevel::Fast,
            transport,
            upgrade_request(&pack),
            pack,
            route,
        )
        .unwrap()
}

#[test]
fn resolved_effect_upgrade_is_off_by_default_and_accepted_quality_sends_once() {
    use steerbox::adaptive_routing::QualityObservation;
    for opt_in in [false, true] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(opt_in);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let mut evaluations = 0;
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| {
                evaluations += 1;
                QualityObservation {
                    accepted: true,
                    ..QualityObservation::default()
                }
            },
            |_, _| panic!("opt-out or accepted quality cannot resolve a second candidate"),
        );
        assert!(result.is_ok());
        assert_eq!(evaluations, usize::from(opt_in));
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert_eq!(controller.task_run().attempts.len(), 1);
    }
}

#[test]
fn resolved_effect_upgrade_rejects_non_whitelisted_host_observations() {
    use steerbox::adaptive_routing::QualityObservation;
    let baseline = QualityObservation {
        no_progress: true,
        ..QualityObservation::default()
    };
    for observation in [
        QualityObservation {
            security_boundary: true,
            ..baseline
        },
        QualityObservation {
            provider_timeout: true,
            ..baseline
        },
        QualityObservation {
            provider_unavailable: true,
            ..baseline
        },
        QualityObservation {
            structured_output_valid: Some(false),
            ..baseline
        },
        QualityObservation {
            tool_result_failed: true,
            ..baseline
        },
        QualityObservation {
            verification_failed: true,
            ..baseline
        },
        QualityObservation {
            verification_passed: true,
            ..baseline
        },
        QualityObservation {
            usage_budget_exceeded: true,
            ..baseline
        },
        QualityObservation {
            output_truncated: true,
            ..baseline
        },
        QualityObservation {
            latency_budget_exceeded: true,
            ..baseline
        },
    ] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| observation,
            |_, _| panic!("forbidden observation cannot resolve another candidate"),
        );
        assert!(result.is_err());
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert_eq!(controller.task_run().attempts.len(), 1);
        assert_eq!(controller.status(), steerbox::domain::LoopStatus::Failed);
    }
}

#[test]
fn resolved_effect_upgrade_does_not_evaluate_parser_provider_or_action_failure() {
    use steerbox::adaptive_routing::QualityObservation;
    for transport in [
        FakeTransport::with_decision_content("fixture-private-not-json".into()),
        FakeTransport::with_decision_content(r#"{"decision":"complete_loop","summary":" "}"#.into()),
        FakeTransport::with_decision_content(json!({"decision":"complete_loop","summary":"fixture",
            "action":{"schema_version":1,"action_id":"action-1","tool_id":"git_status",
            "parameters":{"path":"README.md"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},
            "verification":{"verifier_tool_id":"verify","parameters":{"path":"README.md"}}}}).to_string()),
        FakeTransport::failing(),
    ] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let sends = transport.sends.clone();
        let mut controller = upgrade_controller(&directory,
            LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
            .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport), "step-upgrade",
            |_| -> QualityObservation { panic!("failed/actionful response must not reach evaluator") },
            |_, _| panic!("failed response cannot resolve another candidate"),
        );
        assert!(result.is_err());
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert_eq!(controller.task_run().attempts.len(), 1);
    }
}

#[test]
fn resolved_effect_upgrade_requires_positive_host_acceptance_before_completion() {
    use steerbox::adaptive_routing::QualityObservation;
    use steerbox::domain::LoopStatus;
    for accepted in [false, true] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::with_decision_content(
            json!({"decision":"complete_loop","summary":"fixture complete"}).to_string(),
        );
        let sends = transport.sends.clone();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(2, 0, 2, 2, 0, 2, 2, 2))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| QualityObservation {
                accepted,
                ..QualityObservation::default()
            },
            |_, _| panic!("neither acceptance nor missing evidence may upgrade"),
        );
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(
            controller.status(),
            if accepted {
                LoopStatus::Completed
            } else {
                LoopStatus::Failed
            }
        );
        assert_eq!(sends.lock().unwrap().len(), 1);
    }
}

#[test]
fn resolved_effect_upgrade_exhausted_budgets_stop_before_second_resolution() {
    use steerbox::adaptive_routing::QualityObservation;
    for (steps, calls, attempts) in [(1, 2, 2), (2, 1, 2), (2, 2, 1)] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut controller = upgrade_controller(
            &directory,
            LoopBudget::new(steps, 0, calls, attempts, 0, 4, 4, 4),
        )
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| QualityObservation {
                low_quality: true,
                ..QualityObservation::default()
            },
            |_, _| panic!("exhausted budget cannot resolve a second candidate"),
        );
        assert!(matches!(
            result,
            Err(LoopControllerError::BudgetExhausted(_))
        ));
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert_eq!(controller.budget().steps_used, 1);
        assert_eq!(controller.budget().model_calls_used, 1);
        assert_eq!(controller.budget().attempts_used, 1);
    }
}

#[test]
fn resolved_effect_upgrade_enforces_goal_model_allowlist_before_send() {
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let mut controller = upgrade_controller_with_models(
        &directory,
        LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4),
        vec!["other-allowed-model".into()],
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let before = controller.budget().clone();
    assert!(controller
        .run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| panic!("disallowed model cannot evaluate"),
            |_, _| panic!("disallowed model cannot resolve")
        )
        .is_err());
    assert!(sends.lock().unwrap().is_empty());
    assert_eq!(controller.budget(), &before);
}

#[test]
fn resolved_effect_upgrade_cannot_upgrade_twice_after_legacy_interleaving() {
    use steerbox::adaptive_routing::QualityObservation;
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let mut controller = upgrade_controller(&directory, LoopBudget::new(6, 0, 6, 6, 0, 6, 6, 6))
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let mut evaluations = 0;
    controller
        .run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport.clone()),
            "step-upgrade",
            |_| {
                evaluations += 1;
                QualityObservation {
                    no_progress: evaluations == 1,
                    accepted: evaluations == 2,
                    ..QualityObservation::default()
                }
            },
            |child_route, level| {
                let (pack, route) =
                    resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
                Ok(config
                    .build_effect_bound_candidate(
                        "wire-model",
                        level,
                        transport.clone(),
                        upgrade_request(&pack),
                        pack,
                        route,
                    )
                    .unwrap())
            },
        )
        .unwrap();
    let (pack, route) = resolved_fixture_for("wire-model", "route-fixed-interleave", vec![]);
    // The ordinary API remains available, but it cannot reset TaskRun lineage.
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport.clone()),
        None,
    );
    controller
        .run_resolved_model_step(
            &adapter,
            upgrade_request(&pack),
            &pack,
            &route,
            "legacy-interleave",
        )
        .unwrap();
    assert_eq!(sends.lock().unwrap().len(), 3);
    assert!(controller
        .run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade-again",
            |_| panic!("one upgrade per TaskRun"),
            |_, _| panic!("one upgrade per TaskRun")
        )
        .is_err());
    assert_eq!(sends.lock().unwrap().len(), 3);
    assert_eq!(controller.task_run().attempts.len(), 2);
}

#[test]
fn resolved_effect_upgrade_rejects_controller_identity_before_send_and_budget() {
    use steerbox::adaptive_routing::EffectLevel;
    for boundary in ["task", "goal", "loop", "objective"] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let (pack, route) = resolved_fixture_with_boundary(
            "wire-model",
            "route-primary",
            vec![],
            "openai",
            boundary,
        );
        let mut request = upgrade_request(&pack);
        if boundary == "goal" {
            request.goal_id = "other-goal".into();
        }
        if boundary == "loop" {
            request.loop_run_id = "other-loop".into();
        }
        let candidate = config
            .build_effect_bound_candidate(
                "wire-model",
                EffectLevel::Fast,
                transport,
                request,
                pack,
                route,
            )
            .unwrap();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let before = controller.budget().clone();
        assert!(controller
            .run_resolved_upgrade_model_step(
                candidate,
                "step-upgrade",
                |_| panic!("identity rejected before response"),
                |_, _| panic!("identity rejected before resolution")
            )
            .is_err());
        assert!(sends.lock().unwrap().is_empty(), "boundary: {boundary}");
        assert_eq!(controller.budget(), &before);
        assert!(controller.task_run().attempts[0]
            .route_decision_id
            .is_none());
    }
}

#[test]
fn resolved_effect_upgrade_rejects_each_child_boundary_before_second_send() {
    use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
    for boundary in [
        "auth",
        "maximum",
        "mapping",
        "level",
        "privacy",
        "risk",
        "task",
        "prompt",
        "policy",
        "tool",
        "context",
        "objective",
        "observation",
    ] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut child_config = config.clone();
        if boundary == "auth" {
            child_config.model_profiles[0].auth_ref = Some("env:SYNTHETIC_OTHER_REF".into());
        }
        if boundary == "maximum" {
            child_config.model_profiles[0]
                .effect
                .as_mut()
                .unwrap()
                .maximum_level = Some(EffectLevel::Balanced);
        }
        if boundary == "mapping" {
            use steerbox::provider_adapter::ReasoningEffort;
            let effect = child_config.model_profiles[0].effect.as_mut().unwrap();
            effect
                .mapping
                .insert(EffectLevel::Balanced, ReasoningEffort::High);
            effect
                .mapping
                .insert(EffectLevel::Strong, ReasoningEffort::XHigh);
        }
        let primary = primary_upgrade_candidate(&config, transport.clone());
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_upgrade_model_step(
            primary,
            "step-upgrade",
            |_| QualityObservation {
                low_quality: true,
                ..QualityObservation::default()
            },
            |child_route, level| {
                let (pack, route) = resolved_fixture_with_boundary(
                    "wire-model",
                    &child_route.route_decision_id,
                    vec![],
                    "openai",
                    boundary,
                );
                Ok(child_config
                    .build_effect_bound_candidate(
                        "wire-model",
                        if boundary == "level" {
                            EffectLevel::Fast
                        } else {
                            level
                        },
                        transport,
                        upgrade_request(&pack),
                        pack,
                        route,
                    )
                    .unwrap())
            },
        );
        assert!(result.is_err(), "boundary: {boundary}");
        assert_eq!(sends.lock().unwrap().len(), 1, "boundary: {boundary}");
        assert_eq!(controller.task_run().attempts.len(), 1);
        assert_eq!(controller.status(), steerbox::domain::LoopStatus::Failed);
    }
}

#[test]
fn resolved_effect_upgrade_preserves_strategy_approval_and_terminal_rejections() {
    use steerbox::domain::{LoopStatus, RiskLevel};
    use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};
    for state in ["approval", "completed", "paused", "cancelled"] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let budget = LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4);
        let mut controller = upgrade_controller(&directory, budget.clone())
            .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let expected = match state {
            "approval" => {
                controller
                    .select_strategy(
                        TaskFeatures {
                            estimated_steps: 1,
                            uncertainty_level: 0.0,
                            interdependency_count: 0,
                            risk_level: RiskLevel::Low,
                            reversible: false,
                            blast_radius: BlastRadius::Local,
                            domain: ProfessionalDomain::SoftwareDevelopment,
                            requires_human_approval: false,
                            max_budget_multiplier: 4.0,
                        },
                        &budget,
                    )
                    .unwrap();
                LoopStatus::Running
            }
            "completed" => {
                controller.complete().unwrap();
                LoopStatus::Completed
            }
            _ => {
                use steerbox::user_command_inbox::{CommandKind, UserCommand, UserCommandInbox};
                let inbox = UserCommandInbox::open(directory.path.join("upgrade-commands.jsonl"));
                inbox
                    .submit(
                        UserCommand::new(
                            "state-command",
                            "fixture-user",
                            "project-model-audit",
                            "goal-model-audit",
                            "task-run-model-audit",
                            "loop-model-audit",
                            if state == "paused" {
                                CommandKind::Pause
                            } else {
                                CommandKind::Cancel
                            },
                            "fixture-correlation",
                            json!({}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                controller = controller.with_command_inbox(inbox);
                // The next invocation processes this safe-boundary command.
                if state == "paused" {
                    LoopStatus::Paused
                } else {
                    LoopStatus::Cancelled
                }
            }
        };
        let result = controller.run_resolved_upgrade_model_step(
            primary_upgrade_candidate(&config, transport),
            "step-upgrade",
            |_| panic!("blocked state cannot evaluate"),
            |_, _| panic!("blocked state cannot resolve"),
        );
        assert!(result.is_err(), "state: {state}");
        assert_eq!(controller.status(), expected, "state: {state}");
        assert!(sends.lock().unwrap().is_empty());
        let events = EventLog::open(directory.events())
            .unwrap()
            .read_all()
            .unwrap();
        assert!(!events.iter().any(|event| event.event_type == "loop_failed"));
    }
}

#[test]
fn resolved_effect_upgrade_request_human_never_evaluates_or_upgrades() {
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let transport = FakeTransport::with_decision_content(
        json!({"decision":"request_human","summary":"host review needed"}).to_string(),
    );
    let sends = transport.sends.clone();
    let mut controller = upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    assert_eq!(
        controller
            .run_resolved_upgrade_model_step(
                primary_upgrade_candidate(&config, transport),
                "step-upgrade",
                |_| panic!("request-human is not host quality"),
                |_, _| panic!("request-human cannot upgrade")
            )
            .unwrap(),
        ModelDecision::RequestHuman
    );
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(controller.task_run().attempts.len(), 1);
}

#[test]
fn resolved_effect_upgrade_child_request_human_stops_without_quality_or_completion() {
    use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let transport = SequenceTransport::new(vec![
        openai_response(
            json!({"decision":"complete_loop","summary":"fixture incomplete"}).to_string(),
        ),
        openai_response(
            json!({"decision":"request_human","summary":"host review needed"}).to_string(),
        ),
    ]);
    let sends = transport.sends.clone();
    let (pack, route) = resolved_fixture_for("wire-model", "route-primary", vec![]);
    let primary = config
        .build_effect_bound_candidate(
            "wire-model",
            EffectLevel::Fast,
            transport.clone(),
            upgrade_request(&pack),
            pack,
            route,
        )
        .unwrap();
    let mut controller = upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let mut evaluations = 0;
    let mut resolutions = 0;
    let result = controller
        .run_resolved_upgrade_model_step(
            primary,
            "step-upgrade",
            |response| {
                evaluations += 1;
                assert_eq!(response.summary, "fixture incomplete");
                QualityObservation {
                    low_quality: true,
                    ..QualityObservation::default()
                }
            },
            |child_route, level| {
                resolutions += 1;
                assert_eq!(level, EffectLevel::Balanced);
                let (pack, route) =
                    resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
                Ok(config
                    .build_effect_bound_candidate(
                        "wire-model",
                        level,
                        transport,
                        upgrade_request(&pack),
                        pack,
                        route,
                    )
                    .unwrap())
            },
        )
        .unwrap();
    assert_eq!(result, ModelDecision::RequestHuman);
    assert_eq!(sends.lock().unwrap().len(), 2);
    assert_eq!(evaluations, 1);
    assert_eq!(resolutions, 1);
    assert_eq!(controller.task_run().attempts.len(), 2);
    assert_ne!(controller.status(), steerbox::domain::LoopStatus::Completed);
    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
}

#[test]
fn resolved_effect_upgrade_failure_closes_before_followup_send() {
    use steerbox::adaptive_routing::QualityObservation;
    struct FailCompleted;
    impl ProgressSink for FailCompleted {
        fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
            if event.subject == ProgressSubject::Model && event.status == ProgressStatus::Completed
            {
                return Err(std::io::Error::other("synthetic progress failure"));
            }
            Ok(())
        }
    }
    for audit_failure in [false, true] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        if audit_failure {
            fs::create_dir(directory.model_calls()).unwrap();
        } else {
            controller = controller.with_progress_sink(FailCompleted);
        }
        assert!(controller
            .run_resolved_upgrade_model_step(
                primary_upgrade_candidate(&config, transport.clone()),
                "step-upgrade",
                |_| QualityObservation {
                    accepted: true,
                    ..QualityObservation::default()
                },
                |_, _| panic!("audit/progress failure cannot upgrade")
            )
            .is_err());
        assert_eq!(controller.status(), steerbox::domain::LoopStatus::Failed);
        let sent = sends.lock().unwrap().len();
        assert!(controller
            .run_resolved_upgrade_model_step(
                primary_upgrade_candidate(&config, transport),
                "step-after-failure",
                |_| panic!("failed controller cannot evaluate"),
                |_, _| panic!("failed controller cannot resolve")
            )
            .is_err());
        assert_eq!(sends.lock().unwrap().len(), sent);
    }
}

#[test]
fn resolved_effect_upgrade_route_and_host_audit_failures_close_before_retry() {
    use steerbox::adaptive_routing::QualityObservation;
    fn block_log(directory: &TestDirectory) {
        fs::rename(
            directory.events(),
            directory.path.join("saved-events.jsonl"),
        )
        .unwrap();
        fs::create_dir(directory.events()).unwrap();
    }
    for stage in ["primary_route", "child_route", "host_quality"] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        if stage == "primary_route" {
            block_log(&directory);
        }
        assert!(controller
            .run_resolved_upgrade_model_step(
                primary_upgrade_candidate(&config, transport.clone()),
                "step-upgrade",
                |_| {
                    if stage == "host_quality" {
                        block_log(&directory);
                    }
                    QualityObservation {
                        no_progress: true,
                        ..QualityObservation::default()
                    }
                },
                |child_route, level| {
                    let (pack, route) =
                        resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
                    let child = config
                        .build_effect_bound_candidate(
                            "wire-model",
                            level,
                            transport.clone(),
                            upgrade_request(&pack),
                            pack,
                            route,
                        )
                        .unwrap();
                    block_log(&directory);
                    Ok(child)
                }
            )
            .is_err());
        assert_eq!(controller.status(), steerbox::domain::LoopStatus::Failed);
        let sent = sends.lock().unwrap().len();
        assert_eq!(sent, usize::from(stage != "primary_route"));
        assert!(controller
            .run_resolved_upgrade_model_step(
                primary_upgrade_candidate(&config, transport),
                "retry-blocked",
                |_| panic!("closed state cannot evaluate"),
                |_, _| panic!("closed state cannot resolve")
            )
            .is_err());
        assert_eq!(sends.lock().unwrap().len(), sent);
    }
}

#[test]
fn resolved_effect_upgrade_factory_rejects_profile_and_skipped_target() {
    use steerbox::adaptive_routing::EffectLevel;
    for boundary in ["provider", "endpoint", "model", "protocol", "skip"] {
        let mut config = effect_upgrade_config(true);
        if boundary == "provider" {
            config.model_profiles[0].provider = "other-provider".into();
        }
        if boundary == "endpoint" {
            config.model_profiles[0].endpoint = "https://other.example.test/v1".into();
        }
        if boundary == "model" {
            config.model_profiles[0].model_id = "other-model".into();
        }
        let transport = FakeTransport::valid();
        let sends = transport.sends.clone();
        let (pack, route) = resolved_fixture_for_protocol(
            "wire-model",
            "route-primary",
            vec![],
            if boundary == "protocol" {
                "anthropic-messages"
            } else {
                "openai"
            },
        );
        assert!(
            config
                .build_effect_bound_candidate(
                    "wire-model",
                    if boundary == "skip" {
                        EffectLevel::Strong
                    } else {
                        EffectLevel::Fast
                    },
                    transport,
                    upgrade_request(&pack),
                    pack,
                    route
                )
                .is_err(),
            "boundary: {boundary}"
        );
        assert!(sends.lock().unwrap().is_empty());
    }
}

#[test]
fn resolved_effect_upgrade_child_requires_acceptance_and_never_attempts_a_third_call() {
    use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
    for child_quality in [
        "accepted",
        "low_quality",
        "no_progress",
        "empty",
        "provider_failure",
    ] {
        let directory = TestDirectory::new();
        let config = effect_upgrade_config(true);
        let transport = SequenceTransport::new(if child_quality == "provider_failure" {
            vec![openai_response(
                json!({"decision":"complete_loop","summary":"first"}).to_string(),
            )]
        } else {
            vec![
                openai_response(json!({"decision":"complete_loop","summary":"first"}).to_string()),
                openai_response(json!({"decision":"complete_loop","summary":"second"}).to_string()),
            ]
        });
        let sends = transport.sends.clone();
        let (pack, route) = resolved_fixture_for("wire-model", "route-primary", vec![]);
        let primary = config
            .build_effect_bound_candidate(
                "wire-model",
                EffectLevel::Fast,
                transport.clone(),
                upgrade_request(&pack),
                pack,
                route,
            )
            .unwrap();
        let mut controller =
            upgrade_controller(&directory, LoopBudget::new(4, 0, 4, 4, 0, 4, 4, 4))
                .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let mut evaluations = 0;
        let result = controller.run_resolved_upgrade_model_step(
            primary,
            "step-upgrade",
            |_| {
                evaluations += 1;
                if evaluations == 1 || child_quality == "low_quality" {
                    QualityObservation {
                        low_quality: true,
                        ..QualityObservation::default()
                    }
                } else if child_quality == "no_progress" {
                    QualityObservation {
                        no_progress: true,
                        ..QualityObservation::default()
                    }
                } else {
                    QualityObservation {
                        accepted: child_quality == "accepted",
                        ..QualityObservation::default()
                    }
                }
            },
            |child_route, level| {
                let (pack, route) =
                    resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
                Ok(config
                    .build_effect_bound_candidate(
                        "wire-model",
                        level,
                        transport,
                        upgrade_request(&pack),
                        pack,
                        route,
                    )
                    .unwrap())
            },
        );
        assert_eq!(
            result.is_ok(),
            child_quality == "accepted",
            "quality: {child_quality}"
        );
        assert_eq!(
            evaluations,
            if child_quality == "provider_failure" {
                1
            } else {
                2
            }
        );
        assert_eq!(sends.lock().unwrap().len(), 2);
        assert_eq!(controller.task_run().attempts.len(), 2);
        assert_eq!(
            controller.status(),
            if child_quality == "accepted" {
                steerbox::domain::LoopStatus::Completed
            } else {
                steerbox::domain::LoopStatus::Failed
            }
        );
    }
}

#[test]
fn resolved_effect_upgrade_changes_wire_and_preserves_attempt_lineage_and_budget() {
    use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
    let directory = TestDirectory::new();
    let config = effect_upgrade_config(true);
    let (pack, route) = resolved_fixture_for("wire-model", "route-upgrade-primary", vec![]);
    let transport = SequenceTransport::new(vec![
        openai_response(r#"{"decision":"complete_loop","summary":"fixture incomplete"}"#.into()),
        openai_response(r#"{"decision":"complete_loop","summary":"fixture done"}"#.into()),
    ]);
    let sends = transport.sends.clone();
    let primary = config
        .build_effect_bound_candidate(
            "wire-model",
            EffectLevel::Fast,
            transport.clone(),
            upgrade_request(&pack),
            pack,
            route,
        )
        .unwrap();
    let mut controller = upgrade_controller(&directory, LoopBudget::new(2, 0, 2, 2, 0, 2, 2, 2))
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let mut evaluations = 0;
    let result = controller
        .run_resolved_upgrade_model_step(
            primary,
            "step-upgrade",
            |response| {
                evaluations += 1;
                if evaluations == 1 {
                    assert_eq!(response.summary, "fixture incomplete");
                    QualityObservation {
                        no_progress: true,
                        ..QualityObservation::default()
                    }
                } else {
                    assert_eq!(response.summary, "fixture done");
                    QualityObservation {
                        accepted: true,
                        ..QualityObservation::default()
                    }
                }
            },
            |child_route, level| {
                assert_eq!(level, EffectLevel::Balanced);
                let (pack, route) =
                    resolved_fixture_for("wire-model", &child_route.route_decision_id, vec![]);
                Ok(config
                    .build_effect_bound_candidate(
                        "wire-model",
                        level,
                        transport,
                        upgrade_request(&pack),
                        pack,
                        route,
                    )
                    .unwrap())
            },
        )
        .unwrap();
    assert_eq!(result, ModelDecision::CompleteLoop);
    assert_eq!(evaluations, 2);
    let bodies = sends.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&bodies[0]).unwrap()["reasoning_effort"],
        "low"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&bodies[1]).unwrap()["reasoning_effort"],
        "medium"
    );
    let attempts = &controller.task_run().attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].reason, steerbox::domain::AttemptReason::Upgrade);
    assert_eq!(
        attempts[1].parent_attempt_id.as_deref(),
        Some(attempts[0].attempt_id.as_str())
    );
    assert!(attempts[0].finished_at.is_some());
    assert!(!attempts[0].evidence_refs.is_empty());
    assert!(attempts[0]
        .evidence_refs
        .iter()
        .all(|reference| attempts[1].evidence_refs.contains(reference)));
    assert_eq!(controller.budget().steps_used, 2);
    assert_eq!(controller.budget().model_calls_used, 2);
    assert_eq!(controller.budget().attempts_used, 2);
    let records = ModelCallStore::open(directory.model_calls())
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0].route_decision_id, records[1].route_decision_id);
    assert_ne!(
        records[0].resolved_prompt_pack_id,
        records[1].resolved_prompt_pack_id
    );
    assert_ne!(records[0].input_context_hash, records[1].input_context_hash);
    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    let routes: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "model_route_decided")
        .collect();
    assert_eq!(routes.len(), 2);
    for (index, attempt) in attempts.iter().enumerate() {
        assert_eq!(
            attempt.route_decision_id.as_ref(),
            Some(&records[index].route_decision_id)
        );
        assert_eq!(
            attempt.resolved_prompt_pack_id.as_ref(),
            Some(&records[index].resolved_prompt_pack_id)
        );
        assert_eq!(
            routes[index].payload["route_decision_id"],
            records[index].route_decision_id
        );
        assert_eq!(
            routes[index].attempt_id.as_deref(),
            Some(attempt.attempt_id.as_str())
        );
        assert!(attempt
            .evidence_refs
            .iter()
            .all(|reference| events.iter().any(|event| event.event_id == *reference)));
    }
    let query = steerbox::execution_query::ExecutionQuery::from_events(&events).unwrap();
    let page = query
        .events_page("loop-model-audit", 0, events.len())
        .unwrap();
    for event in page.events.iter().filter(|event| {
        matches!(
            event.event_type.as_str(),
            "model_quality_assessed" | "model_effect_upgraded"
        )
    }) {
        assert_eq!(
            event.surface,
            steerbox::execution_query::ExecutionEventSurface::Current
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "model_quality_assessed"
                && event.payload["evidence_kind"] == "host_quality")
            .count(),
        2
    );
}

#[test]
fn resolved_handoff_uses_derived_pack_and_redacts_tool_results() {
    let directory = TestDirectory::new();
    let workspace = tempfile::tempdir().expect("workspace creates");
    let (pack, route) = resolved_fixture();
    let original_pack_id = pack.metadata().prompt_pack_id.clone();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let action_output = ToolCallOutput {
        summary: "action completed".into(),
        result: json!({
            "api_key": "synthetic-action-secret",
            "safe": "action metadata"
        }),
    };
    let verification_output = ToolCallOutput {
        summary: "verification completed".into(),
        result: json!({
            "password": "synthetic-verification-secret",
            "safe": "verification metadata"
        }),
    };

    controller
        .run_resolved_model_step_with_tool_results(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-before-handoff".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "action-1",
            "git_status",
            &action_output,
            &verification_output,
            "step-after-handoff",
        )
        .expect("resolved handoff step succeeds");

    assert_eq!(pack.metadata().prompt_pack_id, original_pack_id);
    let request_body = sends
        .lock()
        .expect("send capture lock")
        .first()
        .cloned()
        .expect("one provider request");
    assert!(request_body.contains("action metadata"));
    assert!(request_body.contains("verification metadata"));
    for secret in [
        "synthetic-action-secret",
        "synthetic-action-token",
        "synthetic-verification-secret",
        "synthetic-verification-password",
    ] {
        assert!(!request_body.contains(secret), "secret leaked: {secret}");
    }
}

#[test]
fn resolved_handoff_rejects_oversized_tool_result_before_provider_call() {
    let directory = TestDirectory::new();
    let workspace = tempfile::tempdir().expect("workspace creates");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let action_output = ToolCallOutput {
        summary: "x".repeat(steerbox::redaction::DEFAULT_MAX_INPUT_BYTES + 1),
        result: json!({"safe": "action"}),
    };
    let verification_output = ToolCallOutput {
        summary: "verification".into(),
        result: json!({"safe": "verification"}),
    };

    assert!(matches!(
        controller.run_resolved_model_step_with_tool_results(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-before-handoff-limit".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "action-oversized",
            "git_status",
            &action_output,
            &verification_output,
            "step-after-handoff-limit",
        ),
        Err(LoopControllerError::ModelToolContextRedaction(_))
    ));
    assert!(sends.lock().expect("send capture lock").is_empty());
}

#[test]
fn resolved_model_loop_runs_only_after_verified_action_and_stops_on_completion() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().expect("workspace exists");
    let action = json!({
        "schema_version": 1,
        "action_id": "action-resolved-loop",
        "tool_id": "git_status",
        "parameters": {"scope": "workspace"},
        "path_scope": {
            "root": workspace.display().to_string(),
            "allowed_paths": ["README.md"]
        },
        "verification": {
            "verifier_tool_id": "markdown_link_check",
            "parameters": {"path": "README.md"}
        }
    });
    let first = json!({
        "decision": "continue_observation",
        "summary": "fixture action observed",
        "action": action
    })
    .to_string();
    let second = json!({
        "decision": "complete_loop",
        "summary": "fixture loop completed"
    })
    .to_string();
    let transport = SequenceTransport::new(vec![openai_response(first), openai_response(second)]);
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_budget(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
        LoopBudget::new(3, 4, 3, 1, 0, 3, 2, 1),
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    let outcome = controller
        .run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-resolved-loop".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            3,
        )
        .expect("bounded resolved loop succeeds");

    assert_eq!(
        outcome.decision,
        steerbox::model_adapter::ModelDecision::CompleteLoop
    );
    assert_eq!(
        outcome.stop_reason,
        ResolvedModelLoopStopReason::CompleteLoop
    );
    assert_eq!(outcome.iterations, 2);
    assert_eq!(sends.lock().expect("sequence send capture lock").len(), 2);
    let records = ModelCallStore::open(directory.model_calls())
        .expect("model call store reopens")
        .read_all()
        .expect("model call records read");
    assert_eq!(records.len(), 2);
    assert_ne!(
        records[0].resolved_prompt_pack_id,
        records[1].resolved_prompt_pack_id
    );
    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let stopped = events
        .iter()
        .find(|event| event.event_type == "resolved_loop_stopped")
        .expect("resolved loop stop event exists");
    assert_eq!(stopped.payload["reason"], "complete_loop");
}

#[test]
fn resolved_model_loop_reports_request_human_stop_without_spinning() {
    let directory = TestDirectory::new();
    let workspace = std::env::current_dir().expect("workspace exists");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::with_decision_content(
        json!({"decision":"request_human","summary":"fixture needs review"}).to_string(),
    );
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);
    let outcome = controller
        .run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-request-human".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            3,
        )
        .expect("request human is a normal bounded stop");

    assert_eq!(
        outcome.stop_reason,
        ResolvedModelLoopStopReason::RequestHuman
    );
    assert_eq!(sends.lock().expect("send capture lock").len(), 1);
    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "resolved_loop_stopped" && event.payload["reason"] == "request_human"
    }));
}

#[test]
fn resolved_model_loop_reports_no_action_stop_without_spinning() {
    let directory = TestDirectory::new();
    let workspace = std::env::current_dir().expect("workspace exists");
    let (pack, route) = resolved_fixture();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let transport = FakeTransport::with_decision_content(
        json!({"decision":"continue_observation","summary":"fixture awaits evidence"}).to_string(),
    );
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_progress_sink(RecordingProgressSink(progress.clone()))
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);
    let outcome = controller
        .run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-no-action".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            3,
        )
        .expect("no action is a normal bounded stop");

    assert_eq!(
        outcome.stop_reason,
        ResolvedModelLoopStopReason::ContinueObservationWithoutAction
    );
    assert_eq!(sends.lock().expect("send capture lock").len(), 1);
    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "resolved_loop_stopped"
            && event.payload["reason"] == "continue_observation_without_action"
    }));
    let progress_events = progress.lock().expect("progress capture lock");
    assert!(progress_events.iter().any(|event| {
        event.subject == ProgressSubject::Loop
            && event.subject_id == "loop-model-audit"
            && event.status == ProgressStatus::Progress
            && event.summary.as_deref() == Some("bounded resolved loop stopped: no verified action")
    }));
}

#[test]
fn resolved_model_loop_records_budget_stop_before_returning_error() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().expect("workspace exists");
    let action = json!({
        "schema_version": 1,
        "action_id": "action-budget-stop",
        "tool_id": "git_status",
        "parameters": {"scope": "workspace"},
        "path_scope": {
            "root": workspace.display().to_string(),
            "allowed_paths": ["README.md"]
        },
        "verification": {
            "verifier_tool_id": "markdown_link_check",
            "parameters": {"path": "README.md"}
        }
    });
    let transport = SequenceTransport::new(vec![openai_response(
        json!({
            "decision": "continue_observation",
            "summary": "fixture action observed",
            "action": action
        })
        .to_string(),
    )]);
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_budget(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
        LoopBudget::new(2, 4, 2, 1, 0, 3, 2, 1),
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);
    assert!(matches!(
        controller.run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-budget-stop".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            1,
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::Steps))
    ));
    assert_eq!(sends.lock().expect("send capture lock").len(), 1);
    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "resolved_loop_stopped"
            && event.payload["reason"] == "budget_exhausted"
            && event.payload["iterations"] == 1
    }));
}

#[test]
fn resolved_handoff_rejects_unsafe_identifier_before_provider_call() {
    let directory = TestDirectory::new();
    let workspace = tempfile::tempdir().expect("workspace creates");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, workspace.path());
    let output = ToolCallOutput {
        summary: "safe".into(),
        result: json!({"safe": true}),
    };
    assert!(matches!(
        controller.run_resolved_model_step_with_tool_results(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-unsafe-id".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "token=synthetic-secret",
            "git_status",
            &output,
            &output,
            "step-unsafe-id-next",
        ),
        Err(LoopControllerError::ModelToolContextIdentifierInvalid { .. })
    ));
    assert!(sends.lock().expect("send capture lock").is_empty());
}

#[test]
fn resolved_model_loop_rejects_request_lineage_before_provider_call() {
    let directory = TestDirectory::new();
    let workspace = std::env::current_dir().expect("workspace exists");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    let error = controller
        .run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "wrong-goal".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-lineage-mismatch".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            1,
        )
        .expect_err("lineage mismatch must fail before provider call");

    assert!(matches!(
        error,
        LoopControllerError::ModelAdapter(
            steerbox::model_adapter::ModelAdapterError::ResolvedBindingMismatch {
                field: "goal_id"
            }
        )
    ));
    assert!(sends.lock().expect("send capture lock").is_empty());
}

#[test]
fn resolved_model_loop_zero_iterations_fails_closed_before_provider_call() {
    let directory = TestDirectory::new();
    let workspace = std::env::current_dir().expect("workspace exists");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    assert!(matches!(
        controller.run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-zero-iterations".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            0,
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::Steps))
    ));
    assert!(sends.lock().expect("send capture lock").is_empty());
    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let stopped = events
        .iter()
        .find(|event| event.event_type == "resolved_loop_stopped")
        .expect("zero-iteration stop event exists");
    assert_eq!(stopped.payload["reason"], "budget_exhausted");
    assert_eq!(stopped.payload["iterations"], 0);
    assert_eq!(stopped.payload["last_step_id"], "step-zero-iterations");
}

#[test]
fn resolved_model_loop_reports_safe_stop_progress_without_model_content() {
    let directory = TestDirectory::new();
    let workspace = std::env::current_dir().expect("workspace exists");
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::with_decision_content(
        json!({"decision":"request_human","summary":"do not expose this"}).to_string(),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let progress = Arc::new(Mutex::new(Vec::new()));
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_progress_sink(RecordingProgressSink(progress.clone()))
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    controller
        .run_resolved_model_loop(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-progress-stop".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            1,
        )
        .expect("request-human stop is a successful bounded invocation");

    let events = progress.lock().expect("progress capture lock").clone();
    let stop = events
        .iter()
        .find(|event| {
            event.subject == ProgressSubject::Loop
                && event.subject_id == "loop-model-audit"
                && event.status == ProgressStatus::Progress
        })
        .expect("safe stop progress is observable");
    assert_eq!(
        stop.summary.as_deref(),
        Some("bounded resolved loop stopped: request human")
    );
    assert!(!stop
        .summary
        .as_deref()
        .unwrap_or_default()
        .contains("do not expose this"));
}

fn controller_with_budget(
    directory: &TestDirectory,
    allowed_tools: Vec<String>,
    budget: LoopBudget,
) -> ReadOnlyLoopController {
    let goal = GoalContract::new(
        "goal-model-audit",
        "project-model-audit",
        "inspect the repository",
        allowed_tools,
        Vec::new(),
        vec!["model_result_recorded".into()],
    );
    ReadOnlyLoopController::start(
        EventLog::open(directory.events()).expect("event log opens"),
        goal,
        "loop-model-audit",
        "task-run-model-audit",
        budget,
    )
    .expect("controller starts")
}

fn repeat_controller(directory: &TestDirectory) -> ReadOnlyLoopController {
    let goal = GoalContract::new(
        "goal-model-audit",
        "project-model-audit",
        "inspect the repository",
        vec![".".into()],
        Vec::new(),
        vec!["model_result_recorded".into()],
    );
    ReadOnlyLoopController::start(
        EventLog::open(directory.events()).expect("event log opens"),
        goal,
        "loop-model-audit-repeat",
        "task-run-model-audit-repeat",
        LoopBudget::new(2, 0, 2, 1, 0, 1, 2, 1),
    )
    .expect("controller starts")
}

#[test]
fn successful_model_step_persists_complete_audit_identity_and_actual_output_reference() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-success".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-success",
        )
        .expect("model step succeeds");

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let completed = events
        .iter()
        .find(|event| event.event_type == "model_call_completed")
        .expect("completed model event exists");
    let quality = events
        .iter()
        .find(|event| event.event_type == "model_quality_assessed")
        .expect("quality assessment event exists");
    assert_eq!(quality.payload["status"], json!("verified"));
    assert_eq!(quality.payload["signal"], json!("verification_passed"));
    assert!(!quality.payload.to_string().contains("fixture response"));
    for field in [
        "model_call_id",
        "provider_id",
        "endpoint_id",
        "model_id",
        "route_decision_id",
        "resolved_prompt_pack_id",
        "input_context_hash",
        "output_artifact_ref",
    ] {
        assert!(
            completed.payload.get(field).is_some(),
            "completed audit event must contain {field}"
        );
    }

    let records = ModelCallStore::open(directory.model_calls())
        .expect("model call store opens")
        .read_all()
        .expect("model call records read");
    assert_eq!(records.len(), 1, "successful model calls must be persisted");
    let record = &records[0];
    assert_eq!(record.model_id, "logical-model");
    assert_eq!(record.task_run_id, "task-run-model-audit");
    assert!(record.model_call_id.starts_with("call-"));
    assert!(!record.provider_id.trim().is_empty());
    assert!(!record.endpoint_id.trim().is_empty());
    assert!(!record.route_decision_id.trim().is_empty());
    assert!(!record.resolved_prompt_pack_id.trim().is_empty());
    assert!(record.input_context_hash.starts_with("sha256:"));
    assert!(record.output_artifact_ref.starts_with("trace://"));
    assert_eq!(record.verification_status, "not_run");
}

#[test]
fn provider_failure_persists_only_failure_contract_and_keeps_success_log_empty() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, FakeTransport::failing()),
        None,
    );

    let error = controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-provider-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-provider-failure",
        )
        .expect_err("provider failure must fail the model step");
    assert!(matches!(error, LoopControllerError::ModelAdapter(_)));

    let store = ModelCallStore::open(directory.model_calls()).expect("model call store reopens");
    assert!(store.read_all().expect("success records read").is_empty());
    let failures = store.read_failures().expect("failure records read");
    assert_eq!(failures.len(), 1);
    let failure = &failures[0];
    assert_eq!(failure.failure_stage, "provider_invocation");
    assert_eq!(failure.failure_class, "transport");
    assert!(failure.input_context_hash.is_none());
    assert!(failure.output_artifact_ref.is_none());
    assert!(failure.finish_reason.is_none());
    assert!(failure.input_tokens.is_none());
    assert!(failure.output_tokens.is_none());
    assert!(failure.estimated_cost.is_none());

    let failure_json = fs::read_to_string(directory.model_calls().with_extension("failures.jsonl"))
        .expect("failure JSONL reads");
    assert!(!failure_json.contains("network"));
    assert!(!failure_json.contains("error"));
}

#[test]
fn parsed_wire_failure_keeps_exact_input_hash_and_known_usage() {
    use sha2::{Digest, Sha256};

    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport {
        result: Ok(TransportResponse::body(200, json!({
            "id": "fixture-response-body-id",
            "model": "fixture-response-body-model",
            "choices": [{"message": {"content": "fixture-response-body-not-json"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 17, "completion_tokens": 0, "total_tokens": 17},
        }).to_string())),
        sends: Arc::new(Mutex::new(Vec::new())),
    };
    let sends = Arc::clone(&transport.sends);
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let mut controller = controller(&directory)
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
    let result = controller.run_resolved_model_step(
        &adapter,
        ModelRequest {
            model_id: pack.model_id().into(),
            goal_id: "goal-model-audit".into(),
            loop_run_id: "loop-model-audit".into(),
            step_id: "step-wire-failure".into(),
            objective: pack.objective().into(),
            observation: pack.observation().into(),
        },
        &pack,
        &route,
        "step-wire-failure",
    );
    assert!(result.is_err());
    let bodies = sends.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    let exact_hash = format!("sha256:{:x}", Sha256::digest(bodies[0].as_bytes()));
    let store = ModelCallStore::open(directory.model_calls()).unwrap();
    let failures = store.read_failures().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].input_context_hash.as_deref(),
        Some(exact_hash.as_str())
    );
    assert_eq!(failures[0].input_tokens, Some(17));
    assert_eq!(failures[0].output_tokens, Some(0));
    assert_eq!(failures[0].finish_reason.as_deref(), Some("stop"));
    assert!(failures[0].estimated_cost.is_none());
    assert!(failures[0].output_artifact_ref.is_none());
    assert!(store.read_all().unwrap().is_empty());
    assert_eq!(controller.budget().model_calls_used, 1);
    assert_eq!(controller.budget().steps_used, 1);
    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    assert!(!format!("{result:?}").contains("fixture-response-body"));
    assert!(!serde_json::to_string(&failures)
        .unwrap()
        .contains("fixture-response-body"));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("fixture-response-body"));
}

#[test]
fn anthropic_parse_failure_preserves_partial_usage_and_filters_finish_before_audit() {
    use sha2::{Digest, Sha256};

    let private_finish =
        "https://fixture-private.invalid/finish\n\u{1b}[31mfixture-synthetic-secret-marker";
    for finish in ["max_tokens", private_finish] {
        let directory = TestDirectory::new();
        let (pack, route) =
            resolved_fixture_for_protocol("logical-model", "route-fixture", vec![], "anthropic");
        let transport = FakeTransport {
            result: Ok(TransportResponse::body(
                200,
                json!({
                    "type": "message", "role": "assistant",
                    "id": "fixture-response-private-id", "model": "fixture-response-private-model",
                    "content": [{"type": "text", "text": "fixture-response-private-body"}],
                    "stop_reason": finish, "usage": {"input_tokens": 0},
                })
                .to_string(),
            )),
            sends: Arc::new(Mutex::new(Vec::new())),
        };
        let sends = Arc::clone(&transport.sends);
        let adapter = AnthropicAdapter::new("fixture-provider", BASE_URL, transport, None);
        let mut controller = controller(&directory)
            .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let result = controller.run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-anthropic-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-anthropic-failure",
        );
        assert!(matches!(
            result,
            Err(LoopControllerError::ModelAdapter(
                ModelAdapterError::DecisionOutputRejected { .. }
            ))
        ));
        let bodies = sends.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(controller.budget().model_calls_used, 1);
        assert_eq!(controller.budget().steps_used, 1);
        let failures = ModelCallStore::open(directory.model_calls())
            .unwrap()
            .read_failures()
            .unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].input_context_hash,
            Some(format!("sha256:{:x}", Sha256::digest(bodies[0].as_bytes())))
        );
        assert_eq!(failures[0].input_tokens, Some(0));
        assert_eq!(failures[0].output_tokens, None);
        assert_eq!(
            failures[0].finish_reason.as_deref(),
            (finish == "max_tokens").then_some("max_tokens")
        );
        let events = EventLog::open(directory.events())
            .unwrap()
            .read_all()
            .unwrap();
        let error = result.as_ref().unwrap_err();
        for diagnostic in [
            format!("{error}\n{error:?}"),
            serde_json::to_string(&failures).unwrap(),
            serde_json::to_string(&events).unwrap(),
        ] {
            for marker in [
                "fixture-response-private",
                "https://fixture-private.invalid",
                "fixture-synthetic-secret-marker",
            ] {
                assert!(!diagnostic.contains(marker));
            }
        }
    }
}

#[test]
fn parsed_wire_failure_audit_write_error_cannot_trigger_a_second_call() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture_for(
        "logical-model",
        "route-fixture",
        vec!["backup-model".into()],
    );
    let store = ModelCallStore::open(directory.model_calls()).unwrap();
    fs::create_dir(directory.model_calls().with_extension("failures.jsonl")).unwrap();
    let transport = FakeTransport::with_decision_content("fixture-private-body-not-json".into());
    let sends = Arc::clone(&transport.sends);
    let mut controller = controller(&directory).with_model_call_store(store);
    let result = controller.run_resolved_routed_model_step(
        &route,
        "step-audit-failure",
        |candidate_route, model_id| {
            assert_eq!(model_id, "logical-model");
            Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                adapter: Box::new(ProviderModelAdapter::new(
                    OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport.clone()),
                    None,
                )),
                request: ModelRequest {
                    model_id: pack.model_id().into(),
                    goal_id: "goal-model-audit".into(),
                    loop_run_id: "loop-model-audit".into(),
                    step_id: "step-audit-failure".into(),
                    objective: pack.objective().into(),
                    observation: pack.observation().into(),
                },
                pack: pack.clone(),
                route: candidate_route.clone(),
            })
        },
    );
    assert!(matches!(
        result,
        Err(LoopControllerError::ModelCallStore(_))
    ));
    assert_eq!(sends.lock().unwrap().len(), 1);
    assert_eq!(controller.task_run().attempts.len(), 1);
    assert_eq!(controller.budget().model_calls_used, 1);
    assert_eq!(controller.budget().steps_used, 1);
    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_call_failed"
            && event.payload["audit_failure"] == true));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_route_fallback"
            || event.event_type == "model_call_completed"));
}

#[test]
fn provider_failure_audit_write_failure_is_fail_closed() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let model_call_path = directory.model_calls();
    fs::create_dir(model_call_path.with_extension("failures.jsonl"))
        .expect("failure log directory fixture creates");
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(&model_call_path).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, FakeTransport::failing()),
        None,
    );

    let error = controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-provider-failure-store-error".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-provider-failure-store-error",
        )
        .expect_err("failure audit write error must fail closed");
    assert!(matches!(error, LoopControllerError::ModelCallStore(_)));

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| {
        event.event_type == "model_call_failed" && event.payload["audit_failure"] == true
    }));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
}

#[test]
fn resolved_model_repeat_is_rejected_before_second_provider_call() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let sends = transport.sends.clone();
    let mut controller = repeat_controller(&directory).with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let request = |step_id: &str| ModelRequest {
        model_id: pack.model_id().into(),
        goal_id: "goal-model-audit".into(),
        loop_run_id: "loop-model-audit-repeat".into(),
        step_id: step_id.into(),
        objective: pack.objective().into(),
        observation: pack.observation().into(),
    };

    controller
        .run_resolved_model_step(
            &adapter,
            request("step-repeat-1"),
            &pack,
            &route,
            "step-repeat-1",
        )
        .expect("first resolved call succeeds");
    let error = controller
        .run_resolved_model_step(
            &adapter,
            request("step-repeat-2"),
            &pack,
            &route,
            "step-repeat-2",
        )
        .expect_err("repeated resolved observation must stop before provider call");

    assert!(matches!(
        error,
        LoopControllerError::BudgetExhausted(BudgetKind::NoProgressSteps)
            | LoopControllerError::BudgetExhausted(BudgetKind::SameActionRepetitions)
    ));
    assert_eq!(
        sends.lock().expect("send capture lock").len(),
        1,
        "no-progress rejection must not repeat the provider side effect"
    );
}

#[test]
fn model_call_completed_and_step_completed_are_not_claimed_after_adapter_failure() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::failing();
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    assert!(controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-failure",
        )
        .is_err());

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(!events.iter().any(|event| {
        event.event_type == "step_completed"
            && event.step_id.as_deref() == Some("step-audit-failure")
    }));
    let quality = events
        .iter()
        .find(|event| event.event_type == "model_quality_assessed")
        .expect("provider failure quality event exists");
    assert_eq!(quality.payload["status"], json!("retryable"));
    assert_eq!(quality.payload["signal"], json!("provider_unavailable"));
    assert!(!events.iter().any(|event| {
        event.event_type == "model_call_failed"
            && event
                .payload
                .get("output_artifact_ref")
                .is_some_and(Value::is_string)
    }));
    assert!(ModelCallStore::open(directory.model_calls())
        .expect("model call store opens")
        .read_all()
        .expect("model call records read")
        .is_empty());
}

#[test]
fn model_call_audit_write_failure_is_fail_closed_before_completion_events() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let model_call_path = directory.model_calls();
    fs::create_dir(&model_call_path).expect("directory-backed audit failure fixture creates");
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(&model_call_path).expect("directory-backed store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );

    assert!(controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-store-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-store-failure",
        )
        .is_err());

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(!events.iter().any(|event| {
        event.event_type == "step_completed"
            && event.step_id.as_deref() == Some("step-audit-store-failure")
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "model_call_failed" && event.payload["audit_failure"] == true
    }));
}

#[test]
fn completed_audit_input_hash_must_be_the_actual_resolved_wire_hash() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let transport = FakeTransport::valid();
    let mut controller = controller(&directory).with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    controller
        .run_resolved_model_step(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-audit-hash".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            "step-audit-hash",
        )
        .expect("model step succeeds");

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let completed = events
        .iter()
        .find(|event| event.event_type == "model_call_completed")
        .expect("completed model event exists");
    let hash = completed
        .payload
        .get("input_context_hash")
        .and_then(Value::as_str)
        .expect("completed event must contain the actual wire hash");
    assert!(hash.starts_with("sha256:"));
    assert_ne!(hash, "sha256:fixture");
}

#[test]
fn resolved_action_executes_verification_and_persists_final_audit_fields() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().expect("workspace exists");
    let action = json!({
        "schema_version": 1,
        "action_id": "action-resolved-audit",
        "tool_id": "git_status",
        "parameters": {"scope": "workspace"},
        "path_scope": {
            "root": workspace.display().to_string(),
            "allowed_paths": ["README.md"]
        },
        "verification": {
            "verifier_tool_id": "markdown_link_check",
            "parameters": {"path": "README.md"}
        }
    });
    let content = json!({
        "decision": "continue_observation",
        "summary": "fixture action observed",
        "action": action
    })
    .to_string();
    let transport = FakeTransport::with_decision_content(content);
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    controller
        .run_resolved_model_step_with_action(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-resolved-action".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "step-resolved-action",
        )
        .expect("resolved action step succeeds");

    let record = ModelCallStore::open(directory.model_calls())
        .expect("model call store reopens")
        .read_all()
        .expect("model call records read")
        .pop()
        .expect("resolved action record exists");
    assert_eq!(record.tool_call_count, 2);
    assert_eq!(record.verification_status, "passed");
    assert_eq!(record.error_class, None);

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    let event_index = |event_type: &str| {
        events
            .iter()
            .position(|event| event.event_type == event_type)
            .expect("event exists for ordering assertion")
    };
    assert!(event_index("model_action_started") < event_index("model_action_verified"));
    assert!(event_index("model_action_verified") < event_index("model_call_completed"));
    assert!(event_index("model_call_completed") < event_index("step_completed"));
    assert!(!events.iter().any(|event| {
        event.event_type == "step_failed"
            && event.step_id.as_deref() == Some("step-resolved-action")
    }));
}

#[test]
fn resolved_action_verification_failure_is_recorded_before_step_failure() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().expect("workspace exists");
    let content = json!({
        "decision": "continue_observation",
        "summary": "fixture verification failure",
        "action": {
            "schema_version": 1,
            "action_id": "action-resolved-verification-failure",
            "tool_id": "git_status",
            "parameters": {"scope": "workspace"},
            "path_scope": {
                "root": workspace.display().to_string(),
                "allowed_paths": ["README.md", "missing-verification.md"]
            },
            "verification": {
                "verifier_tool_id": "markdown_link_check",
                "parameters": {"path": "missing-verification.md"}
            }
        }
    })
    .to_string();
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(directory.model_calls()).expect("model call store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new(
            "fixture-provider",
            BASE_URL,
            FakeTransport::with_decision_content(content),
        ),
        None,
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    assert!(controller
        .run_resolved_model_step_with_action(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-resolved-verification-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "step-resolved-verification-failure",
        )
        .is_err());

    let record = ModelCallStore::open(directory.model_calls())
        .expect("model call store reopens")
        .read_all()
        .expect("model call records read")
        .pop()
        .expect("failed action record exists");
    assert_eq!(record.tool_call_count, 2);
    assert_eq!(record.verification_status, "failed");
    assert_eq!(record.error_class.as_deref(), Some("verification_failed"));

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_failed"));
    let quality = events
        .iter()
        .find(|event| event.event_type == "model_quality_assessed")
        .expect("verification quality event exists");
    assert_eq!(quality.payload["status"], json!("needs_upgrade"));
    assert_eq!(quality.payload["signal"], json!("verification_failed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(!events.iter().any(|event| {
        event.event_type == "step_completed"
            && event.step_id.as_deref() == Some("step-resolved-verification-failure")
    }));
}

#[test]
fn resolved_action_audit_write_failure_keeps_evidence_without_claiming_completion() {
    let directory = TestDirectory::new();
    let (pack, route) = resolved_fixture();
    let workspace = std::env::current_dir().expect("workspace exists");
    let content = json!({
        "decision": "continue_observation",
        "summary": "fixture action audit failure",
        "action": {
            "schema_version": 1,
            "action_id": "action-resolved-audit-failure",
            "tool_id": "git_status",
            "parameters": {"scope": "workspace"},
            "path_scope": {
                "root": workspace.display().to_string(),
                "allowed_paths": ["README.md"]
            },
            "verification": {
                "verifier_tool_id": "markdown_link_check",
                "parameters": {"path": "README.md"}
            }
        }
    })
    .to_string();
    let model_call_path = directory.model_calls();
    fs::create_dir(&model_call_path).expect("directory-backed audit failure fixture creates");
    let mut controller = controller_with_allowed_tools(
        &directory,
        vec!["git_status".into(), "markdown_link_check".into()],
    )
    .with_model_call_store(
        ModelCallStore::open(&model_call_path).expect("directory-backed store opens"),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new(
            "fixture-provider",
            BASE_URL,
            FakeTransport::with_decision_content(content),
        ),
        None,
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let executor = ReadOnlyToolExecutor;
    let runtime = ModelActionRuntime::new(&registry, &gate, &executor, &workspace);

    assert!(controller
        .run_resolved_model_step_with_action(
            &adapter,
            ModelRequest {
                model_id: pack.model_id().into(),
                goal_id: "goal-model-audit".into(),
                loop_run_id: "loop-model-audit".into(),
                step_id: "step-resolved-audit-failure".into(),
                objective: pack.objective().into(),
                observation: pack.observation().into(),
            },
            &pack,
            &route,
            &runtime,
            "step-resolved-audit-failure",
        )
        .is_err());

    let events = EventLog::open(directory.events())
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_action_verified"));
    assert!(events.iter().any(|event| {
        event.event_type == "model_call_failed" && event.payload["audit_failure"] == true
    }));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(!events.iter().any(|event| {
        event.event_type == "step_completed"
            && event.step_id.as_deref() == Some("step-resolved-audit-failure")
    }));
}

fn resolved_workspace_edit(
    directory: &TestDirectory,
    max_tool_calls: u32,
    checkpoint: bool,
    ledger: bool,
    permit_edit: bool,
    expected: &str,
) -> (
    ReadOnlyLoopController,
    Result<steerbox::model_adapter::ModelDecision, LoopControllerError>,
) {
    let (pack, route) = resolved_fixture();
    fs::write(directory.path.join("README.md"), "# Before\n").unwrap();
    let mut goal = GoalContract::new(
        "goal-model-audit",
        "project-model-audit",
        pack.objective(),
        vec!["README.md".into()],
        Vec::new(),
        vec!["verified".into()],
    );
    goal.allowed_tools = vec!["markdown_link_check".into()];
    if permit_edit {
        goal.allowed_tools.push("edit_markdown".into());
    }
    goal.human_steering_mode = HumanSteeringMode::AutonomousLoop;
    let calls = ModelCallStore::open(directory.model_calls()).unwrap();
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(directory.events()).unwrap(),
        goal,
        "loop-model-audit",
        "task-run-model-audit",
        LoopBudget::new(2, max_tool_calls, 1, 1, 0, 2, 2, 1),
    )
    .unwrap()
    .with_model_call_store(calls);
    if checkpoint {
        controller = controller.with_checkpoint_store(directory.path.join("checkpoints.jsonl"));
    }
    if ledger {
        controller = controller.with_audit_stores(
            directory.path.join("traces.jsonl"),
            directory.path.join("side-effects.jsonl"),
        );
    }
    let transport = FakeTransport::with_decision_content(
        json!({
            "decision": "continue_observation",
            "summary": "bounded fixture edit",
            "action": {
                "schema_version": 1,
                "action_id": "workspace-audit-action",
                "tool_id": "edit_markdown",
                "parameters": {
                    "path": "README.md",
                    "expected": expected,
                    "replacement": "# After\n"
                },
                "path_scope": {
                    "root": directory.path.display().to_string(),
                    "allowed_paths": ["README.md"]
                },
                "verification": {
                    "verifier_tool_id": "markdown_link_check",
                    "parameters": {"path": "README.md"}
                }
            }
        })
        .to_string(),
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    );
    let registry = ToolRegistry::baseline().unwrap();
    let gate = PolicyGate::new();
    let workspace_executor = WorkspaceToolExecutor;
    let read_only_executor = ReadOnlyToolExecutor;
    let runtime = WorkspaceEditActionRuntime::new(
        &registry,
        &gate,
        &workspace_executor,
        &read_only_executor,
        &directory.path,
        false,
    );
    let result = controller.run_resolved_model_step_with_workspace_edit(
        &adapter,
        ModelRequest {
            model_id: pack.model_id().into(),
            goal_id: "goal-model-audit".into(),
            loop_run_id: "loop-model-audit".into(),
            step_id: "workspace-audit-step".into(),
            objective: pack.objective().into(),
            observation: pack.observation().into(),
        },
        &pack,
        &route,
        &runtime,
        "workspace-audit-step",
    );
    (controller, result)
}

fn assert_workspace_audit(directory: &TestDirectory, tool_count: u32, verification_status: &str) {
    let records = ModelCallStore::open(directory.model_calls())
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool_call_count, tool_count);
    assert_eq!(records[0].verification_status, verification_status);
}

#[test]
fn resolved_workspace_missing_checkpoint_records_no_tool_attempt() {
    let directory = TestDirectory::new();
    let (_, result) = resolved_workspace_edit(&directory, 2, false, true, true, "# Before\n");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# Before\n"
    );
    assert_workspace_audit(&directory, 0, "not_run");
}

#[test]
fn resolved_workspace_missing_ledger_records_no_tool_attempt() {
    let directory = TestDirectory::new();
    let (_, result) = resolved_workspace_edit(&directory, 2, true, false, true, "# Before\n");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# Before\n"
    );
    assert_workspace_audit(&directory, 0, "not_run");
}

#[test]
fn resolved_workspace_policy_denial_records_no_tool_attempt() {
    let directory = TestDirectory::new();
    let (_, result) = resolved_workspace_edit(&directory, 2, true, true, false, "# Before\n");
    assert!(matches!(
        result,
        Err(LoopControllerError::PolicyDenied { .. })
    ));
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# Before\n"
    );
    assert_workspace_audit(&directory, 0, "not_run");
}

#[test]
fn resolved_workspace_expected_mismatch_records_only_edit_attempt() {
    let directory = TestDirectory::new();
    let (_, result) =
        resolved_workspace_edit(&directory, 2, true, true, true, "not current content");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# Before\n"
    );
    assert_workspace_audit(&directory, 1, "not_run");
}

#[test]
fn resolved_workspace_success_consumes_both_tool_calls() {
    let directory = TestDirectory::new();
    let (controller, result) =
        resolved_workspace_edit(&directory, 2, true, true, true, "# Before\n");
    result.unwrap();
    assert_eq!(
        fs::read_to_string(directory.path.join("README.md")).unwrap(),
        "# After\n"
    );
    assert_eq!(controller.budget().tool_calls_used, 2);
    assert_workspace_audit(&directory, 2, "passed");
}

#[test]
fn resolved_workspace_insufficient_tool_budget_prevents_mutation() {
    for max_tool_calls in [0, 1] {
        let directory = TestDirectory::new();
        let (_, result) =
            resolved_workspace_edit(&directory, max_tool_calls, true, true, true, "# Before\n");
        assert!(matches!(
            result,
            Err(LoopControllerError::BudgetExhausted(BudgetKind::ToolCalls))
        ));
        assert_eq!(
            fs::read_to_string(directory.path.join("README.md")).unwrap(),
            "# Before\n",
            "budget {max_tool_calls} must reject before mutation"
        );
        assert_workspace_audit(&directory, 0, "not_run");
    }
}

#[test]
fn resolved_routed_retryable_failure_creates_child_and_keeps_failure_jsonl() {
    let directory = TestDirectory::new();
    let (primary_pack, primary_route) = resolved_fixture_for(
        "logical-model",
        "route-primary",
        vec!["backup-model".into()],
    );
    let (fallback_pack, fallback_route) =
        resolved_fixture_for("backup-model", "route-backup", vec![]);
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(4, 0, 2, 2, 0, 2, 2, 2),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());

    let result = controller.run_resolved_routed_model_step(
        &primary_route,
        "step-resolved-fallback",
        |route, model_id| {
            let (pack, candidate_route, transport) = if model_id == "logical-model" {
                (
                    primary_pack.clone(),
                    route.clone(),
                    FakeTransport::failing(),
                )
            } else {
                (
                    fallback_pack.clone(),
                    fallback_route.clone(),
                    FakeTransport::with_decision_content(
                        r#"{"decision":"complete_loop","summary":"fallback done"}"#.into(),
                    ),
                )
            };
            Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                adapter: Box::new(ProviderModelAdapter::new(
                    OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
                    None,
                )),
                request: ModelRequest {
                    model_id: pack.model_id().into(),
                    goal_id: "goal-model-audit".into(),
                    loop_run_id: "loop-model-audit".into(),
                    step_id: "step-resolved-fallback".into(),
                    objective: pack.objective().into(),
                    observation: pack.observation().into(),
                },
                pack,
                route: candidate_route,
            })
        },
    );

    assert!(
        result.is_ok(),
        "fallback candidate should complete: {result:?}"
    );
    assert_eq!(controller.task_run().attempts.len(), 2);
    assert_eq!(
        controller.task_run().attempts[0].status,
        steerbox::domain::AttemptStatus::Failed
    );
    assert_eq!(
        controller.task_run().attempts[1].reason,
        steerbox::domain::AttemptReason::Fallback
    );
    assert_eq!(
        controller.task_run().attempts[1].status,
        steerbox::domain::AttemptStatus::Succeeded
    );
    assert_eq!(
        controller.task_run().attempts[1]
            .parent_attempt_id
            .as_deref(),
        Some(controller.task_run().attempts[0].attempt_id.as_str())
    );

    let store = ModelCallStore::open(directory.model_calls()).unwrap();
    assert_eq!(store.read_all().unwrap().len(), 1);
    let failures = store.read_failures().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].model_id, "logical-model");
    assert_eq!(failures[0].failure_class, "transport");

    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    let fallback = events
        .iter()
        .find(|event| event.event_type == "model_route_fallback")
        .expect("fallback event is recorded");
    assert_eq!(fallback.payload["from_route_decision_id"], "route-primary");
    assert_eq!(fallback.payload["to_route_decision_id"], "route-backup");
    assert_eq!(fallback.payload["error_class"], "transport");
    assert!(events
        .iter()
        .all(|event| !event.payload.to_string().contains("auth")));
}

#[test]
fn resolved_routed_decision_rejection_keeps_failure_audit_without_fallback_or_upgrade() {
    for (content, reason) in [
        (
            "fixture-response-body-not-json",
            DecisionOutputRejection::JsonSyntaxInvalid,
        ),
        (
            r#"{"decision":"complete_loop","summary":"fixture-response-body","action":{"schema_version":1,"action_id":"action-1","tool_id":"git_status","parameters":{"shell":"fixture-response-body"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},"verification":{"verifier_tool_id":"verify","parameters":{"path":"README.md"}}}}"#,
            DecisionOutputRejection::ActionRejected,
        ),
    ] {
        let directory = TestDirectory::new();
        let (pack, route) = resolved_fixture_for(
            "logical-model",
            "route-primary",
            vec!["backup-model".into()],
        );
        let transport = FakeTransport::with_decision_content(content.into());
        let sends = Arc::clone(&transport.sends);
        let mut controller = controller_with_budget(
            &directory,
            vec![".".into()],
            LoopBudget::new(4, 0, 2, 2, 0, 2, 2, 2),
        )
        .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());
        let mut resolutions = 0;
        let result = controller.run_resolved_routed_model_step(
            &route,
            "step-decision-rejection",
            |candidate_route, model_id| {
                resolutions += 1;
                assert_eq!(model_id, "logical-model");
                Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                    adapter: Box::new(ProviderModelAdapter::new(
                        OpenAiCompatibleAdapter::new(
                            "fixture-provider",
                            BASE_URL,
                            transport.clone(),
                        ),
                        None,
                    )),
                    request: ModelRequest {
                        model_id: pack.model_id().into(),
                        goal_id: "goal-model-audit".into(),
                        loop_run_id: "loop-model-audit".into(),
                        step_id: "step-decision-rejection".into(),
                        objective: pack.objective().into(),
                        observation: pack.observation().into(),
                    },
                    pack: pack.clone(),
                    route: candidate_route.clone(),
                })
            },
        );
        assert!(matches!(
            result,
            Err(LoopControllerError::ModelAdapter(
                ModelAdapterError::DecisionOutputRejected { reason: actual }
            )) if actual == reason
        ));
        assert_eq!(resolutions, 1);
        assert_eq!(sends.lock().unwrap().len(), 1);
        assert_eq!(controller.task_run().attempts.len(), 1);
        let store = ModelCallStore::open(directory.model_calls()).unwrap();
        assert!(store.read_all().unwrap().is_empty());
        let failures = store.read_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_stage, "provider_response");
        assert_eq!(failures[0].failure_class, "response_unmappable");
        let events = EventLog::open(directory.events())
            .unwrap()
            .read_all()
            .unwrap();
        let quality = events
            .iter()
            .find(|event| event.event_type == "model_quality_assessed")
            .expect("failure quality assessment exists");
        assert_eq!(quality.payload["status"], "failed");
        assert_eq!(quality.payload["signal"], "insufficient_evidence");
        assert!(!events
            .iter()
            .any(|event| event.event_type == "model_route_fallback"));
        assert!(!serde_json::to_string(&failures)
            .unwrap()
            .contains("fixture-response-body"));
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("fixture-response-body"));
    }
}

#[test]
fn resolved_routed_binding_failure_does_not_create_child_attempt() {
    let directory = TestDirectory::new();
    let (primary_pack, primary_route) = resolved_fixture_for(
        "logical-model",
        "route-primary",
        vec!["backup-model".into()],
    );
    let (fallback_pack, fallback_route) =
        resolved_fixture_for("backup-model", "route-backup", vec![]);
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(4, 0, 2, 2, 0, 2, 2, 2),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());

    let result = controller.run_resolved_routed_model_step(
        &primary_route,
        "step-resolved-binding-failure",
        |route, model_id| {
            let pack = if model_id == "logical-model" {
                primary_pack.clone()
            } else {
                fallback_pack.clone()
            };
            let mut candidate_route = if model_id == "logical-model" {
                route.clone()
            } else {
                fallback_route.clone()
            };
            if model_id == "backup-model" {
                candidate_route.selected_model = "wrong-model".into();
            }
            Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                adapter: Box::new(ProviderModelAdapter::new(
                    OpenAiCompatibleAdapter::new(
                        "fixture-provider",
                        BASE_URL,
                        if model_id == "logical-model" {
                            FakeTransport::failing()
                        } else {
                            FakeTransport::valid()
                        },
                    ),
                    None,
                )),
                request: ModelRequest {
                    model_id: pack.model_id().into(),
                    goal_id: "goal-model-audit".into(),
                    loop_run_id: "loop-model-audit".into(),
                    step_id: "step-resolved-binding-failure".into(),
                    objective: pack.objective().into(),
                    observation: pack.observation().into(),
                },
                pack,
                route: candidate_route,
            })
        },
    );

    assert!(result.is_err(), "invalid fallback binding must be rejected");
    assert_eq!(controller.task_run().attempts.len(), 1);
    assert_eq!(
        controller.task_run().attempts[0].status,
        steerbox::domain::AttemptStatus::Running
    );
    assert!(ModelCallStore::open(directory.model_calls())
        .unwrap()
        .read_all()
        .unwrap()
        .is_empty());
}

#[test]
fn resolved_routed_action_is_not_retried_or_executed_without_action_runtime() {
    let directory = TestDirectory::new();
    let workspace = directory.path.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let file = workspace.join("README.md");
    fs::write(&file, "# Before\n").unwrap();
    let before = fs::read_to_string(&file).unwrap();
    let (primary_pack, primary_route) = resolved_fixture_for(
        "logical-model",
        "route-action-primary",
        vec!["backup-model".into()],
    );
    let (fallback_pack, fallback_route) =
        resolved_fixture_for("backup-model", "route-action-backup", vec![]);
    let action = json!({
        "schema_version": 1,
        "action_id": "action-routed-no-runtime",
        "tool_id": "edit_markdown",
        "parameters": {
            "path": "README.md",
            "expected": "# Before\n",
            "replacement": "# After\n"
        },
        "path_scope": {
            "root": workspace.display().to_string(),
            "allowed_paths": ["README.md"]
        },
        "verification": {
            "verifier_tool_id": "markdown_link_check",
            "parameters": {"path": "README.md"}
        }
    });
    let action_content = json!({
        "decision": "continue_observation",
        "summary": "action requires an explicit runtime",
        "action": action
    })
    .to_string();
    let mut controller = controller_with_budget(
        &directory,
        vec![".".into()],
        LoopBudget::new(4, 4, 2, 2, 0, 2, 2, 2),
    )
    .with_model_call_store(ModelCallStore::open(directory.model_calls()).unwrap());

    let result = controller.run_resolved_routed_model_step(
        &primary_route,
        "step-routed-action-no-runtime",
        |route, model_id| {
            let (pack, candidate_route, transport) = if model_id == "logical-model" {
                (
                    primary_pack.clone(),
                    route.clone(),
                    FakeTransport::with_decision_content(action_content.clone()),
                )
            } else {
                (
                    fallback_pack.clone(),
                    fallback_route.clone(),
                    FakeTransport::valid(),
                )
            };
            Ok(steerbox::loop_controller::ResolvedProviderCandidate {
                adapter: Box::new(ProviderModelAdapter::new(
                    OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
                    None,
                )),
                request: ModelRequest {
                    model_id: pack.model_id().into(),
                    goal_id: "goal-model-audit".into(),
                    loop_run_id: "loop-model-audit".into(),
                    step_id: "step-routed-action-no-runtime".into(),
                    objective: pack.objective().into(),
                    observation: pack.observation().into(),
                },
                pack,
                route: candidate_route,
            })
        },
    );

    assert!(result.is_err(), "action without runtime must fail closed");
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
    assert_eq!(controller.task_run().attempts.len(), 1);
    assert!(controller.task_run().attempts[0].status.is_terminal());

    let records = ModelCallStore::open(directory.model_calls())
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool_call_count, 0);
    assert_eq!(records[0].verification_status, "not_run");
    assert_eq!(records[0].error_class.as_deref(), Some("model_adapter"));

    let events = EventLog::open(directory.events())
        .unwrap()
        .read_all()
        .unwrap();
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_route_fallback"));
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_action_started"));
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_action_verified"));
}
