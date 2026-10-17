use serde_json::json;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steerbox::domain::{BudgetKind, GoalContract, HumanSteeringMode, LoopBudget};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{
    LoopControllerError, ModelActionRuntime, ReadOnlyLoopController, WorkspaceEditActionRuntime,
};
use steerbox::model_adapter::ModelRequest;
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
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallOutput, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;

const BASE_URL: &str = "https://fixture.example.test/v1";

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
    let mut models = ModelRegistry::default();
    models
        .register_provider(ProviderProfile::new("fixture-provider", "Fixture", None).unwrap())
        .unwrap();
    models
        .register_endpoint(
            EndpointProfile::new("fixture-endpoint", "fixture-provider", BASE_URL, "openai")
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
    let route = RouteDecision {
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
    let pack = resolve_prompt(
        &models,
        &prompts,
        ResolvePromptInput {
            route: &route,
            base: &fragment("base", "BASE"),
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
            policy_summary: &json!({"mode":"read_only"}),
            tool_summary: &json!({"tools":[]}),
            context_package: &json!({"fixture":true}),
            objective: "inspect the repository",
            observation: "repository observed",
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
