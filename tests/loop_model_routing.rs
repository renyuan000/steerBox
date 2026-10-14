use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use steerbox::domain::{
    AttemptLineageError, AttemptReason, AttemptStatus, EventEnvelope, GoalContract, LoopBudget,
    LoopStatus, LoopStepStatus, TaskRunStatus,
};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_adapter::{
    LocalFixtureModelAdapter, ModelAdapter, ModelAdapterError, ModelAdapterRegistry, ModelDecision,
    ModelRequest, ModelResponse,
};
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::model_router::{
    ModelRouteProfile, ModelRouter, PrivacyRequirement, RiskLevel, RouteRequest,
};
use steerbox::provider_adapter::ProviderErrorClass;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use steerbox::user_command_inbox::{CommandKind, UserCommand, UserCommandInbox};

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/steerbox-loop-routing-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("unique test directory creates");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CountingAdapter {
    calls: Arc<AtomicUsize>,
    fail: bool,
}

impl ModelAdapter for CountingAdapter {
    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(ModelAdapterError::EmptyObjective);
        }
        Ok(ModelResponse {
            decision: ModelDecision::CompleteLoop,
            summary: "fake model completed".into(),
        })
    }
}

struct RetryableAdapter {
    calls: Arc<AtomicUsize>,
    class: ProviderErrorClass,
}

struct ObservingAdapter {
    calls: Arc<AtomicUsize>,
}

impl ModelAdapter for ObservingAdapter {
    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelResponse {
            decision: ModelDecision::ContinueObservation,
            summary: "keep observing".into(),
        })
    }
}

impl ModelAdapter for RetryableAdapter {
    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ModelAdapterError::Provider { class: self.class })
    }
}

struct FailOnceProgressSink {
    subject: ProgressSubject,
    status: ProgressStatus,
    failed: bool,
}

impl ProgressSink for FailOnceProgressSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if !self.failed && event.subject == self.subject && event.status == self.status {
            self.failed = true;
            return Err(std::io::Error::other("injected routed progress failure"));
        }
        Ok(())
    }
}

struct BlockAuditAfterStepFailure {
    path: PathBuf,
    blocked: bool,
}

impl ProgressSink for BlockAuditAfterStepFailure {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if !self.blocked
            && event.subject == ProgressSubject::Step
            && event.status == ProgressStatus::Failed
        {
            fs::rename(&self.path, self.path.with_extension("saved"))?;
            fs::create_dir(&self.path)?;
            self.blocked = true;
        }
        Ok(())
    }
}

fn restore_blocked_event_log(path: &Path) {
    fs::remove_dir(path).expect("injected log blocker removes");
    fs::rename(path.with_extension("saved"), path).expect("original event log restores");
}

fn read_events(path: &Path) -> Vec<EventEnvelope> {
    EventLog::open(path)
        .expect("event log reopens")
        .read_all()
        .expect("events read")
}

fn registry() -> ModelRegistry {
    let mut registry = ModelRegistry::default();
    registry
        .register_provider(ProviderProfile::new("fixture", "Fixture", None).unwrap())
        .unwrap();
    registry
        .register_endpoint(
            EndpointProfile::new("fixture-endpoint", "fixture", "local://fixture", "fixture")
                .unwrap(),
        )
        .unwrap();
    registry
        .register_model(
            ModelProfile::new(
                "fixture-model",
                "fixture",
                "fixture-endpoint",
                "fixture-model",
                vec!["text".into()],
            )
            .unwrap(),
        )
        .unwrap();
    registry
}

fn fallback_registry() -> ModelRegistry {
    let mut registry = registry();
    registry
        .register_model(
            ModelProfile::new(
                "backup-model",
                "fixture",
                "fixture-endpoint",
                "backup-model",
                vec!["text".into()],
            )
            .unwrap(),
        )
        .unwrap();
    registry
        .register_model(
            ModelProfile::new(
                "last-model",
                "fixture",
                "fixture-endpoint",
                "last-model",
                vec!["text".into()],
            )
            .unwrap(),
        )
        .unwrap();
    registry
}

fn route_request() -> RouteRequest {
    RouteRequest {
        task_id: "task-routing".into(),
        task_type: "code_change".into(),
        agent_role: "implementer".into(),
        risk_level: RiskLevel::Low,
        required_capabilities: vec!["text".into()],
        privacy_requirement: PrivacyRequirement::Internal,
        budget_limit: None,
        latency_target_ms: None,
        prompt_profile_id: "coding-default".into(),
        prompt_profile_version: "1".into(),
    }
}

fn route_profile() -> ModelRouteProfile {
    ModelRouteProfile {
        model_id: "fixture-model".into(),
        supported_task_types: vec!["code_change".into()],
        supported_agent_roles: vec!["implementer".into()],
        max_risk_level: RiskLevel::Critical,
        privacy_clearance: PrivacyRequirement::Restricted,
        estimated_cost: 0.01,
        estimated_latency_ms: 50,
        quality_score: 80,
        reliability_score: 90,
        preferred_for_high_risk: false,
        fallback_priority: 0,
    }
}

fn fallback_profiles() -> Vec<ModelRouteProfile> {
    vec![
        route_profile(),
        ModelRouteProfile {
            model_id: "backup-model".into(),
            supported_task_types: vec!["code_change".into()],
            supported_agent_roles: vec!["implementer".into()],
            max_risk_level: RiskLevel::Critical,
            privacy_clearance: PrivacyRequirement::Restricted,
            estimated_cost: 0.02,
            estimated_latency_ms: 80,
            quality_score: 70,
            reliability_score: 80,
            preferred_for_high_risk: false,
            fallback_priority: -1,
        },
        ModelRouteProfile {
            model_id: "last-model".into(),
            supported_task_types: vec!["code_change".into()],
            supported_agent_roles: vec!["implementer".into()],
            max_risk_level: RiskLevel::Critical,
            privacy_clearance: PrivacyRequirement::Restricted,
            estimated_cost: 0.03,
            estimated_latency_ms: 90,
            quality_score: 60,
            reliability_score: 70,
            preferred_for_high_risk: false,
            fallback_priority: -2,
        },
    ]
}

fn controller(events_path: &PathBuf) -> ReadOnlyLoopController {
    ReadOnlyLoopController::start(
        EventLog::open(events_path).expect("event log opens"),
        GoalContract::new(
            "goal-routing",
            "project-routing",
            "finish the task",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
}

#[test]
fn routed_model_step_records_route_decision_before_model_call() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let goal = GoalContract::new(
        "goal-routing",
        "project-routing",
        "finish the task",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::CompleteLoop,
                "routed complete",
            )),
        )
        .expect("adapter registers");
    let registry = registry();
    let router = ModelRouter::new(
        &registry,
        vec![ModelRouteProfile {
            model_id: "fixture-model".into(),
            supported_task_types: vec!["code_change".into()],
            supported_agent_roles: vec!["implementer".into()],
            max_risk_level: RiskLevel::Critical,
            privacy_clearance: PrivacyRequirement::Restricted,
            estimated_cost: 0.01,
            estimated_latency_ms: 50,
            quality_score: 80,
            reliability_score: 90,
            preferred_for_high_risk: false,
            fallback_priority: 0,
        }],
    )
    .expect("router builds");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");

    let decision = controller
        .run_routed_model_step(&router, &adapters, route_request(), "step-routing", "ready")
        .expect("routed step succeeds");

    assert_eq!(decision, ModelDecision::CompleteLoop);
    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    let route_index = events
        .iter()
        .position(|event| event.event_type == "model_route_decided")
        .expect("route event exists");
    let call_index = events
        .iter()
        .position(|event| event.event_type == "model_call_started")
        .expect("model call event exists");
    assert!(route_index < call_index);
    assert_eq!(
        events[route_index].payload["selected_model"],
        json!("fixture-model")
    );
    let route_id = events[route_index].payload["route_decision_id"]
        .as_str()
        .expect("route decision id exists");
    assert_eq!(
        controller.task_run().attempts[0]
            .route_decision_id
            .as_deref(),
        Some(route_id)
    );
    for event_type in ["model_call_started", "model_call_completed"] {
        let call = events
            .iter()
            .find(|event| event.event_type == event_type)
            .expect("model call lifecycle event exists");
        assert_eq!(
            call.payload["route_decision_id"].as_str(),
            Some(route_id),
            "{event_type} must reference its route decision"
        );
    }
    assert!(!serde_json::to_string(&events[route_index].payload)
        .unwrap()
        .contains("auth_ref"));
}

#[test]
fn incompatible_route_does_not_emit_model_call_started() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let adapters = ModelAdapterRegistry::default();
    let mut request = route_request();
    request.required_capabilities = vec!["vision".into()];
    let mut controller = controller(&events_path);

    let error = controller
        .run_routed_model_step(&router, &adapters, request, "step-routing", "ready")
        .expect_err("incompatible route must fail before model call");
    assert!(matches!(
        error,
        steerbox::loop_controller::LoopControllerError::ModelRouter(_)
    ));

    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_call_started"));
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_route_decided"));
}

#[test]
fn missing_adapter_leaves_route_decided_but_not_model_call_started() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let adapters = ModelAdapterRegistry::default();
    let mut controller = controller(&events_path);

    let error = controller
        .run_routed_model_step(&router, &adapters, route_request(), "step-routing", "ready")
        .expect_err("missing adapter must fail after route decision");
    assert!(matches!(
        error,
        steerbox::loop_controller::LoopControllerError::ModelAdapterNotFound(ref model)
            if model == "fixture-model"
    ));

    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_route_decided"));
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_call_started"));
}

fn assert_unapproved_route_is_blocked(request: RouteRequest) {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: false,
            }),
        )
        .expect("adapter registers");
    let mut controller = controller(&events_path);
    let result =
        controller.run_routed_model_step(&router, &adapters, request, "step-routing", "ready");
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(controller.task_run().status, TaskRunStatus::Running);
    let events = read_events(&events_path);
    let route_index = events
        .iter()
        .position(|event| event.event_type == "model_route_decided")
        .expect("approval-gated route is audited before refusal");
    let route = &events[route_index];
    assert_eq!(route.payload["approval_required"], json!(true));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "unapproved route invoked adapter"
    );
    let error = result.expect_err("unapproved route must fail closed");
    let route_decision_id = route.payload["route_decision_id"]
        .as_str()
        .expect("route decision id exists");
    match error {
        steerbox::loop_controller::LoopControllerError::ModelApprovalRequired {
            route_decision_id: rejected_route_id,
        } => assert_eq!(rejected_route_id, route_decision_id),
        other => panic!("expected ModelApprovalRequired, got {other:?}"),
    }
    let human_index = events
        .iter()
        .position(|event| event.event_type == "human_intervention_required")
        .expect("approval requirement is audited");
    assert!(route_index < human_index);
    for forbidden in ["step_started", "model_call_started", "loop_completed"] {
        assert!(
            events.iter().all(|event| event.event_type != forbidden),
            "unapproved route emitted {forbidden}"
        );
    }
}

#[test]
fn high_risk_route_requires_approval_without_invoking_adapter() {
    let mut request = route_request();
    request.risk_level = RiskLevel::High;
    assert_unapproved_route_is_blocked(request);
}

#[test]
fn restricted_privacy_route_requires_approval_without_invoking_adapter() {
    let mut request = route_request();
    request.privacy_requirement = PrivacyRequirement::Restricted;
    assert_unapproved_route_is_blocked(request);
}

#[test]
fn goal_model_allowlist_rejects_excluded_selected_model_without_invoking_adapter() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: false,
            }),
        )
        .expect("adapter registers");
    let mut goal = GoalContract::new(
        "goal-routing",
        "project-routing",
        "finish the task",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    goal.allowed_models = vec!["allowed-model-only".into()];
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    let result = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "step-routing",
        "ready",
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(result.is_err(), "excluded model route must be rejected");
    let events = read_events(&events_path);
    for forbidden in ["step_started", "model_call_started", "loop_completed"] {
        assert!(events.iter().all(|event| event.event_type != forbidden));
    }
}

#[test]
fn goal_high_risk_cannot_be_downgraded_by_low_route_request() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: false,
            }),
        )
        .expect("adapter registers");
    let mut goal = GoalContract::new(
        "goal-routing",
        "project-routing",
        "finish the task",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    goal.risk_level = steerbox::domain::RiskLevel::High;
    goal.allowed_models = vec!["fixture-model".into()];
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts");
    let request = route_request();
    assert_eq!(request.risk_level, RiskLevel::Low);
    let result =
        controller.run_routed_model_step(&router, &adapters, request, "step-routing", "ready");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let events = read_events(&events_path);
    let route = events
        .iter()
        .find(|event| event.event_type == "model_route_decided")
        .expect("goal-risk route decision is audited");
    assert_eq!(route.payload["approval_required"], json!(true));
    let route_id = route.payload["route_decision_id"]
        .as_str()
        .expect("route decision id exists");
    assert!(matches!(
        result,
        Err(steerbox::loop_controller::LoopControllerError::ModelApprovalRequired {
            route_decision_id,
        }) if route_decision_id == route_id
    ));
    assert!(events.iter().any(|event| {
        event.event_type == "human_intervention_required"
            && event.payload["route_decision_id"].as_str() == Some(route_id)
    }));
    for forbidden in ["step_started", "model_call_started", "loop_completed"] {
        assert!(events.iter().all(|event| event.event_type != forbidden));
    }
}

#[test]
fn failed_routed_adapter_records_step_failure_and_rebuilds_failed_state() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).expect("router builds");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: true,
            }),
        )
        .expect("failing adapter registers");
    let error = controller(&events_path)
        .run_routed_model_step(&router, &adapters, route_request(), "step-routing", "ready")
        .expect_err("fake adapter failure is propagated");
    assert!(matches!(
        error,
        steerbox::loop_controller::LoopControllerError::ModelAdapter(
            ModelAdapterError::EmptyObjective
        )
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = read_events(&events_path);
    let model_failure_index = events
        .iter()
        .position(|event| event.event_type == "model_call_failed")
        .expect("model failure is audited");
    let route = events
        .iter()
        .find(|event| event.event_type == "model_route_decided")
        .expect("failed call route was audited");
    let route_id = route.payload["route_decision_id"]
        .as_str()
        .expect("route decision id exists");
    assert_eq!(
        events[model_failure_index].payload["route_decision_id"].as_str(),
        Some(route_id),
        "model_call_failed must reference its route decision"
    );
    let state = StateStore::rebuild(&events).expect("failed step projection rebuilds");
    assert_eq!(
        state.step("step-routing").expect("step exists").status,
        LoopStepStatus::Failed,
        "adapter failure left the step projection running"
    );
    let step_failure_index = events
        .iter()
        .position(|event| event.event_type == "step_failed")
        .expect("step failure is audited");
    assert!(model_failure_index < step_failure_index);
    for forbidden in ["step_completed", "loop_completed"] {
        assert!(events.iter().all(|event| event.event_type != forbidden));
    }
}

#[test]
fn retryable_routed_failure_uses_declared_fallback_and_records_switch() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).expect("router builds");
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(RetryableAdapter {
                calls: Arc::clone(&primary_calls),
                class: ProviderErrorClass::RateLimit,
            }),
        )
        .expect("primary adapter registers");
    adapters
        .register(
            "backup-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&backup_calls),
                fail: false,
            }),
        )
        .expect("fallback adapter registers");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        GoalContract::new(
            "goal-routing",
            "project-routing",
            "finish the task",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(2, 0, 2, 2, 0, 2, 2, 2),
    )
    .expect("loop starts");

    let decision = controller
        .run_routed_model_step(&router, &adapters, route_request(), "step-routing", "ready")
        .expect("fallback succeeds");

    assert_eq!(decision, ModelDecision::CompleteLoop);
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 1);
    let events = read_events(&events_path);
    let fallback = events
        .iter()
        .find(|event| event.event_type == "model_route_fallback")
        .expect("fallback switch is audited");
    assert_eq!(fallback.payload["from_model"], json!("fixture-model"));
    assert_eq!(fallback.payload["to_model"], json!("backup-model"));
    assert_eq!(fallback.payload["reason"], json!("retryable_model_failure"));
    let fallback_route_id = fallback.payload["fallback_route_decision_id"]
        .as_str()
        .expect("fallback route decision id exists");
    let primary_failure = events
        .iter()
        .find(|event| event.event_type == "model_call_failed")
        .expect("primary failure is audited");
    let backup_started = events
        .iter()
        .find(|event| {
            event.event_type == "model_call_started"
                && event.payload["model_id"] == json!("backup-model")
        })
        .expect("fallback model call is audited");
    assert_eq!(controller.task_run().attempts.len(), 2);
    let initial_attempt = &controller.task_run().attempts[0];
    let fallback_attempt = &controller.task_run().attempts[1];
    assert_eq!(initial_attempt.reason, AttemptReason::Initial);
    assert_eq!(initial_attempt.status, AttemptStatus::Failed);
    assert_eq!(fallback_attempt.reason, AttemptReason::Fallback);
    assert_eq!(fallback_attempt.status, AttemptStatus::Succeeded);
    assert_eq!(
        fallback_attempt.parent_attempt_id.as_deref(),
        Some(initial_attempt.attempt_id.as_str())
    );
    assert_eq!(
        fallback_attempt.route_decision_id.as_deref(),
        Some(fallback_route_id)
    );
    assert!(initial_attempt
        .evidence_refs
        .iter()
        .any(|reference| reference == &primary_failure.event_id));
    assert!(fallback_attempt
        .evidence_refs
        .iter()
        .any(|reference| reference == &fallback.event_id));
    assert!(fallback_attempt
        .evidence_refs
        .iter()
        .any(|reference| reference == &backup_started.event_id));
    assert!(events.iter().any(|event| {
        event.event_type == "model_call_started"
            && event.payload["model_id"] == json!("backup-model")
            && event.attempt_id.as_deref() == Some(fallback_attempt.attempt_id.as_str())
    }));
    assert_eq!(
        primary_failure.attempt_id.as_deref(),
        Some(initial_attempt.attempt_id.as_str())
    );
}

#[test]
fn retryable_fallback_chain_is_finite_and_next_loop_step_hits_no_progress_guard() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).expect("router builds");
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let last_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    for (model_id, calls) in [
        ("fixture-model", Arc::clone(&primary_calls)),
        ("backup-model", Arc::clone(&backup_calls)),
    ] {
        adapters
            .register(
                model_id,
                Box::new(RetryableAdapter {
                    calls,
                    class: ProviderErrorClass::Upstream,
                }),
            )
            .expect("retryable adapter registers");
    }
    adapters
        .register(
            "last-model",
            Box::new(ObservingAdapter {
                calls: Arc::clone(&last_calls),
            }),
        )
        .expect("observing adapter registers");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        GoalContract::new(
            "goal-routing",
            "project-routing",
            "finish the task",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(6, 0, 6, 3, 0, 1, 3, 1),
    )
    .expect("loop starts");

    let first = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "step-routing",
        "ready",
    );
    assert!(matches!(first, Ok(ModelDecision::ContinueObservation)));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(last_calls.load(Ordering::SeqCst), 1);

    let second = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "step-routing-again",
        "ready",
    );
    assert!(matches!(
        second,
        Err(
            steerbox::loop_controller::LoopControllerError::BudgetExhausted(
                steerbox::domain::BudgetKind::NoProgressSteps
            )
        )
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(last_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        read_events(&events_path)
            .iter()
            .filter(|event| event.event_type == "model_route_fallback")
            .count(),
        2
    );
}

#[test]
fn non_retryable_routed_failure_does_not_use_fallback() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).expect("router builds");
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&primary_calls),
                fail: true,
            }),
        )
        .expect("primary adapter registers");
    adapters
        .register(
            "backup-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&backup_calls),
                fail: false,
            }),
        )
        .expect("fallback adapter registers");
    let result = controller(&events_path).run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "step-routing",
        "ready",
    );

    assert!(matches!(
        result,
        Err(
            steerbox::loop_controller::LoopControllerError::ModelAdapter(
                ModelAdapterError::EmptyObjective
            )
        )
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
    assert!(read_events(&events_path)
        .iter()
        .all(|event| event.event_type != "model_route_fallback"));
}

#[test]
fn fallback_cannot_escape_goal_allowed_models() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).expect("router builds");
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(RetryableAdapter {
                calls: Arc::clone(&primary_calls),
                class: ProviderErrorClass::Transport,
            }),
        )
        .expect("primary adapter registers");
    adapters
        .register(
            "backup-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&backup_calls),
                fail: false,
            }),
        )
        .expect("fallback adapter registers");
    let mut goal = GoalContract::new(
        "goal-routing",
        "project-routing",
        "finish the task",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    goal.allowed_models = vec!["fixture-model".into()];
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-routing",
        "task-run-routing",
        LoopBudget::new(2, 0, 2, 1, 0, 2, 2, 2),
    )
    .expect("loop starts");

    let result = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "step-routing",
        "ready",
    );

    assert!(matches!(
        result,
        Err(
            steerbox::loop_controller::LoopControllerError::ModelAdapter(
                ModelAdapterError::Provider {
                    class: ProviderErrorClass::Transport
                }
            )
        )
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
    assert!(read_events(&events_path)
        .iter()
        .all(|event| event.event_type != "model_route_fallback"));
}

fn fallback_controller(events_path: &Path, budget: LoopBudget) -> ReadOnlyLoopController {
    ReadOnlyLoopController::start(
        EventLog::open(events_path).expect("event log opens"),
        GoalContract::new(
            "goal-routing",
            "project-routing",
            "finish the task",
            vec![".".into()],
            Vec::new(),
            vec!["manual_review".into()],
        ),
        "loop-routing",
        "task-run-routing",
        budget,
    )
    .expect("loop starts")
}

#[test]
fn attempt_budget_exhaustion_stops_before_fallback_and_closes_failed_run() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(RetryableAdapter {
                calls: Arc::clone(&primary_calls),
                class: ProviderErrorClass::RateLimit,
            }),
        )
        .unwrap();
    adapters
        .register(
            "backup-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&backup_calls),
                fail: false,
            }),
        )
        .unwrap();
    let mut controller = fallback_controller(&events_path, LoopBudget::new(3, 0, 3, 1, 0, 3, 3, 1));

    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "first", "ready"),
        Err(LoopControllerError::TaskRun(
            AttemptLineageError::AttemptBudgetExhausted
        ))
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
    assert_eq!(controller.budget().attempts_used, 1);
    assert_eq!(controller.task_run().attempts.len(), 1);
    assert_eq!(
        controller.task_run().attempts[0].status,
        AttemptStatus::Failed
    );
    assert_eq!(controller.task_run().status, TaskRunStatus::Failed);
    assert_eq!(controller.status(), LoopStatus::Failed);
    let events = read_events(&events_path);
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_route_fallback"));
    assert!(controller.task_run().attempts[0]
        .evidence_refs
        .iter()
        .all(|reference| events.iter().any(|event| &event.event_id == reference)));
    assert!(matches!(
        controller.complete(),
        Err(LoopControllerError::Terminal)
    ));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "again", "ready"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_fallbacks_keep_only_real_events_for_each_generation() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    for model in ["fixture-model", "backup-model", "last-model"] {
        adapters
            .register(
                model,
                Box::new(RetryableAdapter {
                    calls: Arc::clone(&calls),
                    class: ProviderErrorClass::Upstream,
                }),
            )
            .unwrap();
    }
    let mut controller = fallback_controller(&events_path, LoopBudget::new(3, 0, 3, 3, 0, 3, 3, 1));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "failed", "ready"),
        Err(LoopControllerError::ModelAdapter(_))
    ));
    let events = read_events(&events_path);
    assert_eq!(controller.task_run().attempts.len(), 3);
    assert_eq!(controller.budget().attempts_used, 3);
    for attempt in &controller.task_run().attempts {
        assert!(
            attempt
                .evidence_refs
                .iter()
                .all(|reference| events.iter().any(|event| &event.event_id == reference)),
            "all refs must identify appended events: {:?}",
            attempt.evidence_refs
        );
        assert_eq!(attempt.status, AttemptStatus::Failed);
        assert!(attempt
            .evidence_refs
            .iter()
            .any(
                |reference| events.iter().any(|event| &event.event_id == reference
                    && event.event_type == "model_call_failed"
                    && event.attempt_id.as_deref() == Some(attempt.attempt_id.as_str()))
            ));
        assert!(attempt
            .evidence_refs
            .iter()
            .all(|reference| !reference.ends_with(":model_completed")));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(controller.status(), LoopStatus::Failed);
    assert_eq!(controller.task_run().status, TaskRunStatus::Failed);
    assert!(matches!(
        controller.complete(),
        Err(LoopControllerError::Terminal)
    ));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "again", "ready"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn fallback_budget_rejection_does_not_invent_model_lifecycle_events() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    for model in ["fixture-model", "backup-model"] {
        adapters
            .register(
                model,
                Box::new(RetryableAdapter {
                    calls: Arc::clone(&calls),
                    class: ProviderErrorClass::Upstream,
                }),
            )
            .unwrap();
    }
    let mut controller = fallback_controller(&events_path, LoopBudget::new(2, 0, 1, 2, 0, 3, 3, 1));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "budget", "ready"),
        Err(LoopControllerError::BudgetExhausted(
            steerbox::domain::BudgetKind::ModelCalls
        ))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = read_events(&events_path);
    let child = &controller.task_run().attempts[1];
    assert!(child
        .evidence_refs
        .iter()
        .all(|reference| events.iter().any(|event| &event.event_id == reference)));
    assert!(child
        .evidence_refs
        .iter()
        .all(|reference| !reference.ends_with(":model_started")
            && !reference.ends_with(":model_completed")));
    assert_eq!(child.status, AttemptStatus::Failed);
    assert_eq!(controller.status(), LoopStatus::Failed);
    assert!(matches!(
        controller.complete(),
        Err(LoopControllerError::Terminal)
    ));
}

#[test]
fn fallback_ids_and_attempt_budget_are_shared_across_external_steps() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(RetryableAdapter {
                calls: Arc::clone(&calls),
                class: ProviderErrorClass::RateLimit,
            }),
        )
        .unwrap();
    adapters
        .register(
            "backup-model",
            Box::new(LocalFixtureModelAdapter::new(
                ModelDecision::ContinueObservation,
                "keep observing",
            )),
        )
        .unwrap();
    let mut controller = fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1));
    for step in ["first", "second"] {
        assert_eq!(
            controller
                .run_routed_model_step(&router, &adapters, route_request(), step, "ready")
                .unwrap(),
            ModelDecision::ContinueObservation
        );
    }
    let attempts = &controller.task_run().attempts;
    assert_eq!(attempts.len(), 3);
    assert_ne!(attempts[1].attempt_id, attempts[2].attempt_id);
    assert_eq!(
        attempts[2].parent_attempt_id.as_deref(),
        Some(attempts[1].attempt_id.as_str())
    );
    assert_eq!(controller.budget().attempts_used, 3);
    assert_eq!(controller.task_run().budget.attempts_used, 3);
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "third", "ready"),
        Err(LoopControllerError::TaskRun(
            AttemptLineageError::AttemptBudgetExhausted
        ))
    ));
    assert_eq!(controller.task_run().attempts.len(), 3);
    assert_eq!(controller.budget().attempts_used, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(controller.status(), LoopStatus::Failed);
}

#[test]
fn fallback_event_failure_finishes_the_child_and_blocks_further_calls() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = fallback_registry();
    let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let backup_calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(RetryableAdapter {
                calls: Arc::clone(&primary_calls),
                class: ProviderErrorClass::Upstream,
            }),
        )
        .unwrap();
    adapters
        .register(
            "backup-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&backup_calls),
                fail: false,
            }),
        )
        .unwrap();
    let mut controller = fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1))
        .with_progress_sink(BlockAuditAfterStepFailure {
            path: events_path.clone(),
            blocked: false,
        });

    let result = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "fallback-audit-fault",
        "ready",
    );
    restore_blocked_event_log(&events_path);

    assert!(matches!(result, Err(LoopControllerError::EventLog(_))));
    assert_eq!(controller.status(), LoopStatus::Failed);
    assert_eq!(controller.task_run().status, TaskRunStatus::Failed);
    assert_eq!(controller.task_run().attempts.len(), 2);
    assert_eq!(controller.budget().attempts_used, 2);
    for attempt in &controller.task_run().attempts {
        assert_eq!(attempt.status, AttemptStatus::Failed);
    }
    assert!(controller.task_run().attempts[1].evidence_refs.is_empty());
    let events = read_events(&events_path);
    assert!(events
        .iter()
        .all(|event| event.event_type != "model_route_fallback"));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "again", "fresh"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn loop_failed_event_failure_keeps_the_run_terminal_without_fake_evidence() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: true,
            }),
        )
        .unwrap();
    let mut controller = fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1))
        .with_progress_sink(BlockAuditAfterStepFailure {
            path: events_path.clone(),
            blocked: false,
        });

    let result = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "close-audit-fault",
        "ready",
    );
    restore_blocked_event_log(&events_path);

    assert!(matches!(result, Err(LoopControllerError::EventLog(_))));
    assert_eq!(controller.status(), LoopStatus::Failed);
    assert_eq!(controller.task_run().status, TaskRunStatus::Failed);
    let attempt = &controller.task_run().attempts[0];
    assert_eq!(attempt.status, AttemptStatus::Failed);
    let events = read_events(&events_path);
    assert!(events.iter().all(|event| event.event_type != "loop_failed"));
    assert!(attempt
        .evidence_refs
        .iter()
        .all(|reference| events.iter().any(|event| &event.event_id == reference)));
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "again", "fresh"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn routed_progress_failures_never_retry_models_or_overwrite_completed_runs() {
    for (subject, status, retryable, expected_calls, expected_status) in [
        (
            ProgressSubject::Step,
            ProgressStatus::Started,
            false,
            0,
            LoopStatus::Failed,
        ),
        (
            ProgressSubject::Model,
            ProgressStatus::Started,
            false,
            0,
            LoopStatus::Failed,
        ),
        (
            ProgressSubject::Model,
            ProgressStatus::Failed,
            true,
            1,
            LoopStatus::Failed,
        ),
        (
            ProgressSubject::Model,
            ProgressStatus::Completed,
            false,
            1,
            LoopStatus::Failed,
        ),
        (
            ProgressSubject::Step,
            ProgressStatus::Completed,
            false,
            1,
            LoopStatus::Failed,
        ),
        (
            ProgressSubject::Loop,
            ProgressStatus::Completed,
            false,
            1,
            LoopStatus::Completed,
        ),
    ] {
        let directory = TestDirectory::new();
        let events_path = directory.join("events.jsonl");
        let registry = fallback_registry();
        let router = ModelRouter::new(&registry, fallback_profiles()).unwrap();
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let backup_calls = Arc::new(AtomicUsize::new(0));
        let primary: Box<dyn ModelAdapter> = if retryable {
            Box::new(RetryableAdapter {
                calls: Arc::clone(&primary_calls),
                class: ProviderErrorClass::Upstream,
            })
        } else {
            Box::new(CountingAdapter {
                calls: Arc::clone(&primary_calls),
                fail: false,
            })
        };
        let mut adapters = ModelAdapterRegistry::default();
        adapters.register("fixture-model", primary).unwrap();
        adapters
            .register(
                "backup-model",
                Box::new(CountingAdapter {
                    calls: Arc::clone(&backup_calls),
                    fail: false,
                }),
            )
            .unwrap();
        let mut controller =
            fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1))
                .with_progress_sink(FailOnceProgressSink {
                    subject,
                    status,
                    failed: false,
                });

        let error = controller
            .run_routed_model_step(
                &router,
                &adapters,
                route_request(),
                "progress-fault",
                "ready",
            )
            .expect_err("injected progress error must be preserved");

        assert!(
            matches!(error, LoopControllerError::ProgressReport(ref message) if message.contains("injected routed progress failure"))
        );
        assert_eq!(
            controller.status(),
            expected_status,
            "{subject:?}/{status:?}"
        );
        let expected_task_status = if expected_status == LoopStatus::Completed {
            TaskRunStatus::Succeeded
        } else {
            TaskRunStatus::Failed
        };
        assert_eq!(controller.task_run().status, expected_task_status);
        assert_eq!(controller.task_run().attempts.len(), 1);
        assert!(controller.task_run().attempts[0].status.is_terminal());
        assert!(matches!(
            controller.run_routed_model_step(&router, &adapters, route_request(), "again", "fresh"),
            Err(LoopControllerError::Terminal)
        ));
        assert_eq!(primary_calls.load(Ordering::SeqCst), expected_calls);
        assert_eq!(backup_calls.load(Ordering::SeqCst), 0);
        let events = read_events(&events_path);
        assert!(events
            .iter()
            .all(|event| event.event_type != "model_route_fallback"));
        if expected_status == LoopStatus::Completed {
            assert!(events.iter().all(|event| event.event_type != "loop_failed"));
        }
    }
}

#[test]
fn route_audit_failure_stops_before_the_model_and_prevents_later_calls() {
    let directory = TestDirectory::new();
    let events_path = directory.join("events.jsonl");
    let registry = registry();
    let router = ModelRouter::new(&registry, vec![route_profile()]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = ModelAdapterRegistry::default();
    adapters
        .register(
            "fixture-model",
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
                fail: false,
            }),
        )
        .unwrap();
    let mut controller = fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1));
    fs::rename(&events_path, events_path.with_extension("saved")).unwrap();
    fs::create_dir(&events_path).unwrap();

    let result = controller.run_routed_model_step(
        &router,
        &adapters,
        route_request(),
        "route-audit-fault",
        "ready",
    );
    restore_blocked_event_log(&events_path);

    assert!(matches!(result, Err(LoopControllerError::EventLog(_))));
    assert_eq!(controller.status(), LoopStatus::Failed);
    assert_eq!(
        controller.task_run().attempts[0].status,
        AttemptStatus::Failed
    );
    assert_eq!(controller.budget().steps_used, 0);
    assert_eq!(controller.budget().model_calls_used, 0);
    assert!(matches!(
        controller.run_routed_model_step(&router, &adapters, route_request(), "again", "fresh"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn routed_preflight_pause_and_cancel_are_not_reclassified_as_failed() {
    for (kind, expected_status, progress_status) in [
        (
            CommandKind::Pause,
            LoopStatus::Paused,
            ProgressStatus::Paused,
        ),
        (
            CommandKind::Cancel,
            LoopStatus::Cancelled,
            ProgressStatus::Cancelled,
        ),
    ] {
        for progress_fault in [false, true] {
            let directory = TestDirectory::new();
            let events_path = directory.join("events.jsonl");
            let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
            inbox
                .submit(
                    UserCommand::new(
                        "steer",
                        "operator",
                        "project-routing",
                        "goal-routing",
                        "task-run-routing",
                        "loop-routing",
                        kind,
                        "corr-steer",
                        json!({}),
                    )
                    .unwrap(),
                )
                .unwrap();
            let registry = registry();
            let router = ModelRouter::new(&registry, vec![route_profile()]).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let mut adapters = ModelAdapterRegistry::default();
            adapters
                .register(
                    "fixture-model",
                    Box::new(CountingAdapter {
                        calls: Arc::clone(&calls),
                        fail: false,
                    }),
                )
                .unwrap();
            let mut controller =
                fallback_controller(&events_path, LoopBudget::new(6, 0, 6, 3, 0, 6, 6, 1))
                    .with_command_inbox(inbox);
            if progress_fault {
                controller = controller.with_progress_sink(FailOnceProgressSink {
                    subject: ProgressSubject::Loop,
                    status: progress_status,
                    failed: false,
                });
            }

            let error = controller
                .run_routed_model_step(&router, &adapters, route_request(), "preflight", "ready")
                .expect_err("steering prevents execution");

            if progress_fault {
                assert!(matches!(error, LoopControllerError::ProgressReport(_)));
            } else if expected_status == LoopStatus::Paused {
                assert!(matches!(error, LoopControllerError::Paused));
            } else {
                assert!(matches!(error, LoopControllerError::Terminal));
            }
            assert_eq!(controller.status(), expected_status);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(read_events(&events_path)
                .iter()
                .all(|event| event.event_type != "loop_failed"));
        }
    }
}
