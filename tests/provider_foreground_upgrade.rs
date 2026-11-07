use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use steerbox::adaptive_routing::{EffectLevel, QualityObservation};
use steerbox::domain::LoopStatus;
use steerbox::event_log::EventLog;
use steerbox::execution_query::ExecutionQuery;
use steerbox::http_transport::{CredentialError, CredentialResolver, ResolvedCredential};
use steerbox::model_adapter::{ModelDecision, ModelResponse};
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{
    ProviderTransport, ReasoningEffort, TransportError, TransportResponse,
};
use steerbox::provider_runner::{
    ForegroundProviderRun, ProviderRunError, ProviderRunOutcome, ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::{
    ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject, StreamReporter,
};

#[derive(Clone)]
struct SequenceTransport {
    bodies: Rc<RefCell<Vec<Value>>>,
    raw_bodies: Rc<RefCell<Vec<String>>>,
    responses: Rc<RefCell<VecDeque<TransportResponse>>>,
    on_send: Option<Rc<dyn Fn(usize)>>,
}

impl SequenceTransport {
    fn new(responses: Vec<TransportResponse>) -> Self {
        Self {
            bodies: Rc::new(RefCell::new(Vec::new())),
            raw_bodies: Rc::new(RefCell::new(Vec::new())),
            responses: Rc::new(RefCell::new(responses.into())),
            on_send: None,
        }
    }
}

impl ProviderTransport for SequenceTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert_eq!(endpoint, "https://upgrade.example.test/v1/chat/completions");
        assert_eq!(auth_ref, Some("secret-ref:upgrade"));
        assert!(!streaming);
        self.raw_bodies.borrow_mut().push(body.to_owned());
        self.bodies
            .borrow_mut()
            .push(serde_json::from_str(body).unwrap());
        if let Some(on_send) = &self.on_send {
            on_send(self.bodies.borrow().len());
        }
        Ok(self
            .responses
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra send"))
    }
}

fn response(decision: &str, usage: bool) -> TransportResponse {
    let mut body = json!({
        "id": "synthetic-upgrade", "model": "upgrade-model",
        "choices": [{"message": {"content": json!({
            "decision": decision, "summary": "synthetic observation"
        }).to_string()}, "finish_reason": "stop"}]
    });
    if usage {
        body["usage"] = json!({"prompt_tokens": 2, "completion_tokens": 3});
    }
    TransportResponse::body(200, body.to_string())
}

fn config() -> RuntimeConfig {
    RuntimeConfig::from_json_str(
        &json!({
            "schema_version": 1, "project_ref": "foreground-upgrade-test",
            "data_root": "/temp/steerbox/foreground-upgrade-test",
            "reported_token_limit": 20,
            "model_profiles": [{
                "model_id": "upgrade-model", "provider": "synthetic-openai",
                "endpoint": "https://upgrade.example.test/v1", "auth_ref": "secret-ref:upgrade",
                "endpoint_protocol": "openai-compatible-chat", "capabilities": ["text"],
                "effect": {"selected_level": "fast", "automatic_upgrade": true,
                    "maximum_level": "strong",
                    "mapping": {"fast": "low", "balanced": "medium", "strong": "high"}}
            }]
        })
        .to_string(),
    )
    .unwrap()
}

fn request() -> ProviderRunRequest {
    ProviderRunRequest {
        project_id: "project-upgrade".into(),
        goal_id: "goal-upgrade".into(),
        loop_run_id: "loop-upgrade".into(),
        task_run_id: "task-upgrade".into(),
        model_id: "upgrade-model".into(),
        objective: "Evaluate only supplied evidence".into(),
        observation: "Synthetic bounded input".into(),
    }
}

struct Harness {
    directory: tempfile::TempDir,
    transport: SequenceTransport,
    calls: ModelCallStore,
}

impl Harness {
    fn new(responses: Vec<TransportResponse>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        Self {
            directory,
            transport: SequenceTransport::new(responses),
            calls,
        }
    }

    fn events(&self) -> EventLog {
        EventLog::open(self.directory.path().join("events.jsonl")).unwrap()
    }

    fn run(
        &self,
        config: &RuntimeConfig,
        evaluate: impl FnMut(&ModelResponse) -> QualityObservation,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        ForegroundProviderRun::prepare_upgrade(config, request())?.run_with_transport(
            self.transport.clone(),
            self.events(),
            self.calls.clone(),
            StreamReporter::new(Vec::new()),
            evaluate,
        )
    }

    fn assert_no_completed_loop(&self) {
        let events = self.events().read_all().unwrap();
        assert!(!events
            .iter()
            .any(|event| event.event_type == "loop_completed"));
        let state = StateStore::rebuild(&events).unwrap();
        assert_ne!(
            state.loop_run("loop-upgrade").unwrap().status,
            LoopStatus::Completed
        );
    }
}

fn accepted(_: &ModelResponse) -> QualityObservation {
    QualityObservation {
        accepted: true,
        ..QualityObservation::default()
    }
}

fn low_quality(_: &ModelResponse) -> QualityObservation {
    QualityObservation {
        low_quality: true,
        ..QualityObservation::default()
    }
}

#[test]
fn opt_in_foreground_upgrades_once_and_preserves_wire_audit_and_projection() {
    let directory = tempfile::tempdir().unwrap();
    let event_path = directory.path().join("events.jsonl");
    let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
    let transport = SequenceTransport::new(vec![
        response("complete_loop", true),
        response("complete_loop", true),
    ]);
    let mut evaluations = 0;
    let outcome = ForegroundProviderRun::prepare_upgrade(&config(), request())
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(&event_path).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
            |_| {
                evaluations += 1;
                QualityObservation {
                    accepted: evaluations == 2,
                    low_quality: evaluations == 1,
                    ..QualityObservation::default()
                }
            },
        )
        .expect("opt-in foreground executes a bounded adjacent upgrade");
    assert_eq!(outcome.decision, ModelDecision::CompleteLoop);
    assert_eq!(outcome.loop_status, LoopStatus::Completed);
    assert_eq!(evaluations, 2);
    let bodies = transport.bodies.borrow();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["reasoning_effort"], "low");
    assert_eq!(bodies[1]["reasoning_effort"], "medium");
    assert_eq!(bodies[0]["messages"], bodies[1]["messages"]);
    assert_eq!(bodies[0]["model"], bodies[1]["model"]);
    let records = calls.read_all().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].model_call_id, "call-loop-upgrade-step-1");
    assert_eq!(
        records[1].model_call_id,
        "call-loop-upgrade-step-1:upgrade:1"
    );
    assert_ne!(records[0].route_decision_id, records[1].route_decision_id);
    assert_ne!(
        records[0].resolved_prompt_pack_id,
        records[1].resolved_prompt_pack_id
    );
    assert_ne!(records[0].input_context_hash, records[1].input_context_hash);
    for (record, body) in records.iter().zip(transport.raw_bodies.borrow().iter()) {
        assert_eq!(
            record.input_context_hash,
            format!("sha256:{:x}", Sha256::digest(body.as_bytes()))
        );
        assert!(!body.contains("secret-ref:"));
        assert!(!body.contains("auth_ref"));
    }
    let ledger = outcome.reported_token_budget().unwrap();
    assert_eq!(ledger.known_total_tokens(), 10);
    assert_eq!(ledger.known_input_tokens(), 4);
    assert_eq!(ledger.known_output_tokens(), 6);
    assert_eq!(ledger.records().len(), 2);
    let events = EventLog::open(&event_path).unwrap().read_all().unwrap();
    let routes: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "model_route_decided")
        .collect();
    assert_eq!(routes.len(), 2);
    assert_eq!(
        routes[0].attempt_id.as_deref(),
        Some("loop-upgrade:attempt:1")
    );
    assert_eq!(
        routes[1].attempt_id.as_deref(),
        Some("loop-upgrade:attempt:upgrade:1")
    );
    assert_eq!(
        routes[0].payload["route_decision_id"],
        records[0].route_decision_id
    );
    assert_eq!(
        routes[1].payload["route_decision_id"],
        records[1].route_decision_id
    );
    let upgraded = events
        .iter()
        .find(|event| event.event_type == "model_effect_upgraded")
        .unwrap();
    assert_eq!(upgraded.payload["attempt_no"], 2);
    assert_eq!(
        upgraded.payload["from_route_decision_id"],
        records[0].route_decision_id
    );
    assert_eq!(
        upgraded.payload["to_route_decision_id"],
        records[1].route_decision_id
    );
    assert_eq!(upgraded.payload["from_effect_level"], "fast");
    assert_eq!(upgraded.payload["to_effect_level"], "balanced");
    assert_eq!(upgraded.task_run_id.as_deref(), Some("task-upgrade"));
    assert_eq!(
        upgraded.attempt_id.as_deref(),
        Some("loop-upgrade:attempt:upgrade:1")
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "model_call_started")
            .count(),
        2
    );
    let step_numbers: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "step_started")
        .map(|event| event.payload["step_no"].clone())
        .collect();
    assert_eq!(step_numbers, vec![json!(1), json!(2)]);
    let state = StateStore::rebuild(&events).unwrap();
    assert_eq!(state.steps().count(), 2);
    assert_eq!(
        state.loop_run("loop-upgrade").unwrap().status,
        LoopStatus::Completed
    );
    let query = ExecutionQuery::from_events(&events).unwrap();
    assert_eq!(query.list().len(), 1);
    assert!(query.surface_snapshot("loop-upgrade").is_some());
}

#[test]
fn ordinary_foreground_keeps_rejecting_automatic_upgrade_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let transport = SequenceTransport::new(Vec::new());
    let result = ForegroundProviderRun::prepare(&config(), request())
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(directory.path().join("events.jsonl")).unwrap(),
            ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        );
    assert!(result.is_err());
    assert!(transport.bodies.borrow().is_empty());
}

#[test]
fn upgrade_prepare_requires_effect_opt_in_and_openai_protocol() {
    let mut absent = config();
    absent.model_profiles[0].effect = None;
    assert!(ForegroundProviderRun::prepare_upgrade(&absent, request()).is_err());
    let mut disabled = config();
    disabled.model_profiles[0]
        .effect
        .as_mut()
        .unwrap()
        .automatic_upgrade = false;
    disabled.model_profiles[0]
        .effect
        .as_mut()
        .unwrap()
        .maximum_level = None;
    assert!(RuntimeConfig::from_json_str(&serde_json::to_string(&disabled).unwrap()).is_ok());
    assert!(ForegroundProviderRun::prepare_upgrade(&disabled, request()).is_err());
    let mut anthropic = config();
    anthropic.model_profiles[0].provider = "anthropic".into();
    anthropic.model_profiles[0].endpoint_protocol = "anthropic-messages".into();
    assert!(ForegroundProviderRun::prepare_upgrade(&anthropic, request()).is_err());
    let mut invalid = config();
    invalid.model_profiles[0].auth_ref = Some("secret-ref:invalid reference".into());
    assert!(ForegroundProviderRun::prepare_upgrade(&invalid, request()).is_err());
}

#[test]
fn accepted_first_response_uses_one_call() {
    let harness = Harness::new(vec![response("complete_loop", true)]);
    let outcome = harness.run(&config(), accepted).unwrap();
    assert_eq!(outcome.loop_status, LoopStatus::Completed);
    assert_eq!(harness.transport.bodies.borrow().len(), 1);
    assert_eq!(harness.calls.read_all().unwrap().len(), 1);
    assert_eq!(
        outcome
            .reported_token_budget()
            .unwrap()
            .known_total_tokens(),
        5
    );
}

#[test]
fn continue_and_human_decisions_preserve_nonterminal_status() {
    for (decision, expected, status) in [
        (
            "continue_observation",
            ModelDecision::ContinueObservation,
            LoopStatus::Running,
        ),
        (
            "request_human",
            ModelDecision::RequestHuman,
            LoopStatus::Running,
        ),
    ] {
        let harness = Harness::new(vec![response(decision, true)]);
        let mut evaluations = 0;
        let outcome = harness
            .run(&config(), |result| {
                evaluations += 1;
                accepted(result)
            })
            .unwrap();
        assert_eq!(outcome.decision, expected);
        assert_eq!(outcome.loop_status, status);
        assert_eq!(evaluations, usize::from(decision != "request_human"));
        assert_eq!(harness.transport.bodies.borrow().len(), 1);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn child_continue_and_human_decisions_do_not_claim_completion() {
    for decision in ["continue_observation", "request_human"] {
        let harness = Harness::new(vec![
            response("complete_loop", true),
            response(decision, true),
        ]);
        let mut evaluations = 0;
        let outcome = harness
            .run(&config(), |result| {
                evaluations += 1;
                if evaluations == 1 {
                    low_quality(result)
                } else {
                    accepted(result)
                }
            })
            .unwrap();
        assert_ne!(outcome.loop_status, LoopStatus::Completed);
        assert_eq!(evaluations, if decision == "request_human" { 1 } else { 2 });
        assert_eq!(harness.transport.bodies.borrow().len(), 2);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn child_low_quality_or_missing_evidence_never_causes_a_third_call() {
    for missing_evidence in [false, true] {
        let harness = Harness::new(vec![
            response("complete_loop", true),
            response("complete_loop", true),
        ]);
        let mut evaluations = 0;
        let result = harness.run(&config(), |result| {
            evaluations += 1;
            if evaluations == 2 && missing_evidence {
                QualityObservation::default()
            } else {
                low_quality(result)
            }
        });
        assert!(result.is_err());
        assert_eq!(evaluations, 2);
        assert_eq!(harness.transport.bodies.borrow().len(), 2);
        assert_eq!(harness.calls.read_all().unwrap().len(), 2);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn security_or_missing_host_evidence_does_not_upgrade() {
    for observation in [
        QualityObservation::default(),
        QualityObservation {
            security_boundary: true,
            low_quality: true,
            ..QualityObservation::default()
        },
    ] {
        let harness = Harness::new(vec![response("complete_loop", true)]);
        assert!(harness.run(&config(), |_| observation).is_err());
        assert_eq!(harness.transport.bodies.borrow().len(), 1);
        harness.assert_no_completed_loop();
    }
}

fn response_with_content(content: Value) -> TransportResponse {
    TransportResponse::body(
        200,
        json!({
            "id":"synthetic-upgrade", "model":"upgrade-model",
            "choices":[{"message":{"content":content.to_string()},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":3}
        })
        .to_string(),
    )
}

#[test]
fn parser_provider_and_action_rejections_do_not_evaluate_or_upgrade() {
    let responses = vec![
        TransportResponse::body(503, "synthetic provider failure"),
        TransportResponse::body(200, "invalid JSON"),
        response_with_content(json!({"decision":"not_a_decision", "summary":"synthetic"})),
        response_with_content(
            json!({"decision":"complete_loop", "summary":"synthetic", "action":{
                "schema_version":1,"action_id":"synthetic-action","tool_id":"edit_markdown",
                "parameters":{"path":"README.md","expected":"before","replacement":"after"},
                "path_scope":{"root":"/tmp/synthetic-workspace","allowed_paths":["README.md"]},
                "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}
            }}),
        ),
    ];
    for response in responses {
        let harness = Harness::new(vec![response]);
        let mut evaluations = 0;
        let result = harness.run(&config(), |result| {
            evaluations += 1;
            low_quality(result)
        });
        assert!(result.is_err());
        assert_eq!(evaluations, 0);
        assert_eq!(harness.transport.bodies.borrow().len(), 1);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn disabled_token_budget_accepts_missing_usage_for_both_calls() {
    let mut config = config();
    config.reported_token_limit = None;
    let harness = Harness::new(vec![
        response("complete_loop", false),
        response("complete_loop", false),
    ]);
    let mut evaluations = 0;
    let outcome = harness
        .run(&config, |result| {
            evaluations += 1;
            if evaluations == 1 {
                low_quality(result)
            } else {
                accepted(result)
            }
        })
        .unwrap();
    assert!(outcome.reported_token_budget().is_none());
    assert_eq!(harness.transport.bodies.borrow().len(), 2);
}

#[test]
fn known_zero_usage_is_known_and_does_not_exhaust_positive_limit() {
    let mut config = config();
    config.reported_token_limit = Some(1);
    let zero = || {
        TransportResponse::body(200, json!({
        "id":"synthetic-zero", "model":"upgrade-model",
        "choices":[{"message":{"content":json!({"decision":"complete_loop","summary":"synthetic"}).to_string()},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":0,"completion_tokens":0}
    }).to_string())
    };
    let harness = Harness::new(vec![zero(), zero()]);
    let mut evaluations = 0;
    let outcome = harness
        .run(&config, |result| {
            evaluations += 1;
            if evaluations == 1 {
                low_quality(result)
            } else {
                accepted(result)
            }
        })
        .unwrap();
    let ledger = outcome.reported_token_budget().unwrap();
    assert_eq!(ledger.known_total_tokens(), 0);
    assert_eq!(ledger.records().len(), 2);
    assert_eq!(ledger.blocked(), None);
    assert_eq!(harness.transport.bodies.borrow().len(), 2);
}

#[test]
fn zero_token_limit_blocks_before_any_send_or_evaluation() {
    let mut config = config();
    config.reported_token_limit = Some(0);
    let harness = Harness::new(Vec::new());
    let mut evaluations = 0;
    assert!(harness
        .run(&config, |result| {
            evaluations += 1;
            accepted(result)
        })
        .is_err());
    assert_eq!(evaluations, 0);
    assert!(harness.transport.bodies.borrow().is_empty());
    assert!(harness.calls.read_all().unwrap().is_empty());
    harness.assert_no_completed_loop();
}

#[test]
fn unknown_or_exhausted_first_usage_blocks_the_child_send() {
    for (usage, limit) in [(false, 20), (true, 5), (true, 4)] {
        let mut config = config();
        config.reported_token_limit = Some(limit);
        let harness = Harness::new(vec![response("complete_loop", usage)]);
        assert!(harness.run(&config, low_quality).is_err());
        assert_eq!(harness.transport.bodies.borrow().len(), 1);
        assert_eq!(harness.calls.read_all().unwrap().len(), 1);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn prepare_freezes_profile_mapping_and_limit_before_delayed_child() {
    let harness = Harness::new(vec![
        response("complete_loop", true),
        response("complete_loop", true),
    ]);
    let mut config = config();
    let prepared = ForegroundProviderRun::prepare_upgrade(&config, request()).unwrap();
    let mut evaluations = 0;
    let outcome = prepared
        .run_with_transport(
            harness.transport.clone(),
            harness.events(),
            harness.calls.clone(),
            StreamReporter::new(Vec::new()),
            |result| {
                evaluations += 1;
                config.reported_token_limit = Some(0);
                config.model_profiles[0].endpoint = "https://changed.example.test/v1".into();
                config.model_profiles[0].auth_ref = Some("secret-ref:changed".into());
                config.model_profiles[0].model_id = "changed-model".into();
                let effect = config.model_profiles[0].effect.as_mut().unwrap();
                effect.automatic_upgrade = false;
                effect.selected_level = EffectLevel::Strong;
                effect.maximum_level = Some(EffectLevel::Strong);
                effect
                    .mapping
                    .insert(EffectLevel::Balanced, ReasoningEffort::High);
                if evaluations == 1 {
                    low_quality(result)
                } else {
                    accepted(result)
                }
            },
        )
        .unwrap();
    assert_eq!(outcome.reported_token_budget().unwrap().limit(), 20);
    assert_eq!(
        outcome
            .reported_token_budget()
            .unwrap()
            .known_total_tokens(),
        10
    );
    assert_eq!(
        harness.transport.bodies.borrow()[1]["reasoning_effort"],
        "medium"
    );
    assert_eq!(
        harness.transport.bodies.borrow()[1]["model"],
        "upgrade-model"
    );
}

struct NonCloneResolver(Arc<AtomicUsize>);

impl CredentialResolver for NonCloneResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CredentialError::Missing)
    }
}

#[test]
fn owned_non_clone_resolver_builds_and_zero_limit_never_resolves() {
    let mut config = config();
    config.reported_token_limit = Some(0);
    let harness = Harness::new(Vec::new());
    let resolutions = Arc::new(AtomicUsize::new(0));
    let result = ForegroundProviderRun::prepare_upgrade(&config, request())
        .unwrap()
        .run_with_resolver(
            NonCloneResolver(resolutions.clone()),
            harness.events(),
            harness.calls.clone(),
            StreamReporter::new(Vec::new()),
            accepted,
        );
    assert!(result.is_err());
    assert_eq!(resolutions.load(Ordering::SeqCst), 0);
    assert!(harness.calls.read_all().unwrap().is_empty());
}

#[test]
fn duplicate_identity_blocks_transport_and_non_clone_resolver_before_mutation() {
    let harness = Harness::new(vec![response("complete_loop", true)]);
    harness.run(&config(), accepted).unwrap();
    let event_snapshot = fs::read(harness.directory.path().join("events.jsonl")).unwrap();
    let call_snapshot = fs::read(harness.directory.path().join("calls.jsonl")).unwrap();
    assert!(matches!(
        harness.run(&config(), accepted),
        Err(ProviderRunError::DuplicateRunIdentity)
    ));
    let resolutions = Arc::new(AtomicUsize::new(0));
    let result = ForegroundProviderRun::prepare_upgrade(&config(), request())
        .unwrap()
        .run_with_resolver(
            NonCloneResolver(resolutions.clone()),
            harness.events(),
            harness.calls.clone(),
            StreamReporter::new(Vec::new()),
            accepted,
        );
    assert!(matches!(
        result,
        Err(ProviderRunError::DuplicateRunIdentity)
    ));
    assert_eq!(resolutions.load(Ordering::SeqCst), 0);
    assert_eq!(harness.transport.bodies.borrow().len(), 1);
    assert_eq!(
        fs::read(harness.directory.path().join("events.jsonl")).unwrap(),
        event_snapshot
    );
    assert_eq!(
        fs::read(harness.directory.path().join("calls.jsonl")).unwrap(),
        call_snapshot
    );
}

#[test]
fn corrupt_event_or_call_history_rejects_before_transport_and_resolver() {
    for corrupt_events in [false, true] {
        for use_resolver in [false, true] {
            let harness = Harness::new(Vec::new());
            let events = harness.events();
            let path = harness.directory.path().join(if corrupt_events {
                "events.jsonl"
            } else {
                "calls.jsonl"
            });
            fs::write(&path, "invalid synthetic history\n").unwrap();
            let prepared = ForegroundProviderRun::prepare_upgrade(&config(), request()).unwrap();
            let resolutions = Arc::new(AtomicUsize::new(0));
            let result = if use_resolver {
                prepared.run_with_resolver(
                    NonCloneResolver(resolutions.clone()),
                    events,
                    harness.calls.clone(),
                    StreamReporter::new(Vec::new()),
                    accepted,
                )
            } else {
                prepared.run_with_transport(
                    harness.transport.clone(),
                    events,
                    harness.calls.clone(),
                    StreamReporter::new(Vec::new()),
                    accepted,
                )
            };
            assert!(matches!(
                result,
                Err(ProviderRunError::RunHistoryUnavailable)
            ));
            assert!(harness.transport.bodies.borrow().is_empty());
            assert_eq!(resolutions.load(Ordering::SeqCst), 0);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                "invalid synthetic history\n"
            );
        }
    }
}

struct FailingProgress {
    fail_at: usize,
    completed: usize,
}

impl ProgressSink for FailingProgress {
    fn report(&mut self, event: ProgressEvent) -> io::Result<()> {
        if event.subject == ProgressSubject::Model && event.status == ProgressStatus::Completed {
            self.completed += 1;
        }
        if self.fail_at == 0 || self.completed == self.fail_at {
            Err(io::Error::other("synthetic progress failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn progress_failure_before_first_or_after_either_response_stops_the_run() {
    for fail_at in [0, 1, 2] {
        let harness = Harness::new(vec![
            response("complete_loop", true),
            response("complete_loop", true),
        ]);
        let mut evaluations = 0;
        let result = ForegroundProviderRun::prepare_upgrade(&config(), request())
            .unwrap()
            .run_with_transport(
                harness.transport.clone(),
                harness.events(),
                harness.calls.clone(),
                FailingProgress {
                    fail_at,
                    completed: 0,
                },
                |result| {
                    evaluations += 1;
                    low_quality(result)
                },
            );
        assert!(result.is_err());
        assert_eq!(evaluations, fail_at.saturating_sub(1));
        assert_eq!(harness.transport.bodies.borrow().len(), fail_at);
        harness.assert_no_completed_loop();
    }
}

#[test]
fn first_audit_failure_after_response_prevents_evaluation_and_upgrade() {
    let mut harness = Harness::new(vec![response("complete_loop", true)]);
    let call_path = harness.directory.path().join("calls.jsonl");
    harness.transport.on_send = Some(Rc::new(move |_| fs::create_dir(&call_path).unwrap()));
    let mut evaluations = 0;
    let result = harness.run(&config(), |result| {
        evaluations += 1;
        low_quality(result)
    });
    assert!(result.is_err());
    assert_eq!(evaluations, 0);
    assert_eq!(harness.transport.bodies.borrow().len(), 1);
    harness.assert_no_completed_loop();
}

#[test]
fn child_audit_failure_records_no_child_success_or_third_send() {
    let mut harness = Harness::new(vec![
        response("complete_loop", true),
        response("complete_loop", true),
    ]);
    let call_path = harness.directory.path().join("calls.jsonl");
    let archived_path = harness.directory.path().join("calls-archived.jsonl");
    harness.transport.on_send = Some(Rc::new(move |send_number| {
        if send_number == 2 {
            fs::rename(&call_path, &archived_path).unwrap();
            fs::create_dir(&call_path).unwrap();
        }
    }));
    let mut evaluations = 0;
    let result = harness.run(&config(), |response| {
        evaluations += 1;
        low_quality(response)
    });
    assert!(result.is_err());
    assert_eq!(evaluations, 1);
    assert_eq!(harness.transport.bodies.borrow().len(), 2);
    let archived_calls =
        ModelCallStore::open(harness.directory.path().join("calls-archived.jsonl")).unwrap();
    assert_eq!(archived_calls.read_all().unwrap().len(), 1);
    let events = harness.events().read_all().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "model_call_completed")
            .count(),
        1
    );
    harness.assert_no_completed_loop();
}

#[test]
fn event_failure_during_either_host_evaluation_stops_without_extra_send() {
    for fail_at in [1, 2] {
        let harness = Harness::new(vec![
            response("complete_loop", true),
            response("complete_loop", true),
        ]);
        let event_path = harness.directory.path().join("events.jsonl");
        let archived_path = harness.directory.path().join("events-archived.jsonl");
        let mut evaluations = 0;
        let result = harness.run(&config(), |response| {
            evaluations += 1;
            if evaluations == fail_at {
                fs::rename(&event_path, &archived_path).unwrap();
                fs::create_dir(&event_path).unwrap();
            }
            low_quality(response)
        });
        assert!(result.is_err());
        assert_eq!(evaluations, fail_at);
        assert_eq!(harness.transport.bodies.borrow().len(), fail_at);
        let archived_events = EventLog::open(&archived_path).unwrap().read_all().unwrap();
        assert!(!archived_events
            .iter()
            .any(|event| event.event_type == "loop_completed"));
    }
}

struct NonCloneTransport(SequenceTransport);

impl ProviderTransport for NonCloneTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.0.send(endpoint, auth_ref, body, timeout, streaming)
    }
}

#[test]
fn owned_non_clone_transport_is_shared_between_both_calls() {
    let harness = Harness::new(vec![
        response("complete_loop", true),
        response("complete_loop", true),
    ]);
    let mut evaluations = 0;
    let outcome = ForegroundProviderRun::prepare_upgrade(&config(), request())
        .unwrap()
        .run_with_transport(
            NonCloneTransport(harness.transport.clone()),
            harness.events(),
            harness.calls.clone(),
            StreamReporter::new(Vec::new()),
            |response| {
                evaluations += 1;
                if evaluations == 1 {
                    low_quality(response)
                } else {
                    accepted(response)
                }
            },
        )
        .unwrap();
    assert_eq!(outcome.loop_status, LoopStatus::Completed);
    assert_eq!(harness.transport.bodies.borrow().len(), 2);
}

#[test]
fn reported_usage_accumulates_without_adding_to_the_original_limit() {
    for limit in [6, 10, 20] {
        let mut config = config();
        config.reported_token_limit = Some(limit);
        let harness = Harness::new(vec![
            response("complete_loop", true),
            response("complete_loop", true),
        ]);
        let mut evaluations = 0;
        let outcome = harness
            .run(&config, |response| {
                evaluations += 1;
                if evaluations == 1 {
                    low_quality(response)
                } else {
                    accepted(response)
                }
            })
            .unwrap();
        let ledger = outcome.reported_token_budget().unwrap();
        assert_eq!(ledger.limit(), limit);
        assert_eq!(ledger.known_total_tokens(), 10);
        assert_eq!(ledger.records().len(), 2);
        assert_eq!(harness.transport.bodies.borrow().len(), 2);
    }
}

#[test]
fn invalid_audit_contract_rejects_before_another_send() {
    let harness = Harness::new(vec![response("complete_loop", true)]);
    harness.run(&config(), accepted).unwrap();
    let mut record = serde_json::to_value(&harness.calls.read_all().unwrap()[0]).unwrap();
    record["input_context_hash"] = json!("");
    let path = harness.directory.path().join("calls.jsonl");
    fs::write(&path, format!("{record}\n")).unwrap();
    assert!(matches!(
        harness.run(&config(), accepted),
        Err(ProviderRunError::RunHistoryUnavailable)
    ));
    assert_eq!(harness.transport.bodies.borrow().len(), 1);
}
