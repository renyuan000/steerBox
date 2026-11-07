use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::LoopStatus;
use steerbox::event_log::EventLog;
use steerbox::http_transport::{
    CredentialError, CredentialResolver, HttpAnthropicTransport, ResolvedCredential,
};
use steerbox::loop_controller::LoopControllerError;
use steerbox::model_adapter::{DecisionOutputRejection, ModelAdapterError, ModelDecision};
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{
    ProviderErrorClass, ProviderTransport, TransportError, TransportResponse,
};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunError, ProviderRunRequest};
use steerbox::reported_token_budget::ReportedTokenBudgetError;
use steerbox::runtime_config::{ConfigError, RuntimeConfig};
use steerbox::stream_reporter::{
    ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject, StreamReporter,
};
use steerbox::tool_call_store::ToolCallRecordStore;

struct TestTransport {
    calls: Cell<usize>,
    status: u16,
}

impl ProviderTransport for &TestTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(
            endpoint,
            "https://provider.example.test/v1/chat/completions"
        );
        assert_eq!(auth_ref, Some("secret-ref:provider-fixture"));
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["model"], "test-model");
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["role"], "developer");
        assert_eq!(payload["messages"][2]["role"], "user");
        assert!(!streaming);
        if self.status != 200 {
            return Ok(TransportResponse::body(self.status, "RAW_UPSTREAM_BODY"));
        }
        Ok(TransportResponse::body(
            200,
            json!({
                "id": "provider-request-1", "model": "test-model",
                "choices": [{"message": {"content": json!({
                    "decision": "complete_loop", "summary": "review complete"
                }).to_string()}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 21, "completion_tokens": 8, "total_tokens": 29}
            })
            .to_string(),
        ))
    }
}

fn config(auth_ref: Option<&str>, provider: &str) -> RuntimeConfig {
    RuntimeConfig::from_json_str(
        &json!({
            "schema_version": 1, "project_ref": "provider-runner-test",
            "data_root": "/temp/steerbox/provider-runner-test",
            "model_profiles": [{ "model_id": "test-model", "provider": provider,
                "endpoint": "https://provider.example.test/v1", "auth_ref": auth_ref,
                "capabilities": ["text"] }]
        })
        .to_string(),
    )
    .unwrap()
}

fn request() -> ProviderRunRequest {
    ProviderRunRequest {
        project_id: "project-provider".into(),
        goal_id: "goal-provider".into(),
        loop_run_id: "loop-provider".into(),
        task_run_id: "task-run-provider".into(),
        model_id: "test-model".into(),
        objective: "Review the supplied observation only".into(),
        observation: "The fixture repository is clean".into(),
    }
}

struct AnthropicTestTransport {
    calls: Cell<usize>,
}

impl AnthropicTransport for &AnthropicTestTransport {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(endpoint, "https://provider.example.test/v1/messages");
        assert_eq!(
            headers.api_key_ref.as_deref(),
            Some("secret-ref:provider-fixture")
        );
        assert!(!streaming);
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["model"], "test-model");
        assert_eq!(payload["messages"][0]["role"], "user");
        Ok(TransportResponse::body(200, json!({
            "id": "message-1", "type": "message", "role": "assistant",
            "model": "test-model",
            "content": [{"type": "text", "text": "{\"decision\":\"complete_loop\",\"summary\":\"anthropic done\"}"}],
            "stop_reason": "end_turn", "usage": {"input_tokens": 17, "output_tokens": 5}
        }).to_string()))
    }
}

#[test]
fn anthropic_runner_uses_injected_transport_and_persists_common_audit() {
    let directory = tempfile::tempdir().unwrap();
    let calls = ModelCallStore::open(directory.path().join("model-calls.jsonl")).unwrap();
    let transport = AnthropicTestTransport {
        calls: Cell::new(0),
    };
    let mut config = config(Some("secret-ref:provider-fixture"), "anthropic");
    config.model_profiles[0].endpoint = "https://provider.example.test/v1".into();
    let result = ForegroundProviderRun::prepare(&config, request())
        .unwrap()
        .run_anthropic_with_transport(
            &transport,
            EventLog::open(directory.path().join("events.jsonl")).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(result.decision, ModelDecision::CompleteLoop);
    assert_eq!(result.loop_status, LoopStatus::Completed);
    assert_eq!(transport.calls.get(), 1);
    let records = calls.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_id, "anthropic");
    assert_eq!(records[0].input_tokens, Some(17));
    assert_eq!(records[0].output_tokens, Some(5));
}

#[test]
fn runner_wires_resolved_prompt_controller_and_model_call_audit() {
    let directory = tempfile::tempdir().unwrap();
    let events_path = directory.path().join("events.jsonl");
    let calls = ModelCallStore::open(directory.path().join("model-calls.jsonl")).unwrap();
    let transport = TestTransport {
        calls: Cell::new(0),
        status: 200,
    };
    let prepared = ForegroundProviderRun::prepare(
        &config(Some("secret-ref:provider-fixture"), "openai-compatible"),
        request(),
    )
    .unwrap();
    let result = prepared
        .run_with_transport(
            &transport,
            EventLog::open(&events_path).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(result.decision, ModelDecision::CompleteLoop);
    assert_eq!(result.loop_status, LoopStatus::Completed);
    assert_eq!(transport.calls.get(), 1);
    let records = calls.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_id, "openai-compatible");
    assert_eq!(records[0].input_tokens, Some(21));
    assert!(records[0].input_context_hash.starts_with("sha256:"));
    assert!(records[0]
        .resolved_prompt_pack_id
        .starts_with("prompt-pack:"));
    let events = EventLog::open(events_path).unwrap().read_all().unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
    let metadata = serde_json::to_string(&records).unwrap();
    assert!(!metadata.contains("The fixture repository is clean"));
    assert!(!metadata.contains("secret-ref:provider-fixture"));
}

#[test]
fn missing_profile_fails_before_any_runtime_is_started() {
    let mut input = request();
    input.model_id = "missing".into();
    assert!(matches!(
        ForegroundProviderRun::prepare(
            &config(Some("secret-ref:provider-fixture"), "openai"),
            input
        ),
        Err(ProviderRunError::Config(_))
    ));
}

#[test]
fn missing_auth_ref_is_not_silently_treated_as_unauthenticated_access() {
    assert!(matches!(
        ForegroundProviderRun::prepare(&config(None, "openai"), request()),
        Err(ProviderRunError::AuthRefRequired)
    ));
}

#[test]
fn anthropic_is_not_silently_sent_to_an_openai_wire_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let calls = ModelCallStore::open(directory.path().join("model-calls.jsonl")).unwrap();
    let transport = TestTransport {
        calls: Cell::new(0),
        status: 200,
    };
    let result = ForegroundProviderRun::prepare(
        &config(Some("secret-ref:provider-fixture"), "anthropic"),
        request(),
    )
    .unwrap()
    .run_with_transport(
        &transport,
        EventLog::open(directory.path().join("events.jsonl")).unwrap(),
        calls,
        StreamReporter::new(Vec::new()),
    );
    assert!(matches!(result, Err(ProviderRunError::UnsupportedProtocol)));
    assert_eq!(transport.calls.get(), 0);
}

#[test]
fn unsupported_explicit_protocol_fails_before_chat_completions_transport() {
    let mut config = config(Some("secret-ref:provider-fixture"), "openai");
    config.model_profiles[0].endpoint_protocol = "openai-responses".into();
    let result = ForegroundProviderRun::prepare(&config, request());
    assert!(matches!(
        result,
        Err(ProviderRunError::Config(
            ConfigError::UnsupportedEndpointProtocol { protocol }
        )) if protocol == "openai-responses"
    ));
}

#[test]
fn failure_writes_metadata_without_raw_response_body() {
    let directory = tempfile::tempdir().unwrap();
    let events_path = directory.path().join("events.jsonl");
    let calls = ModelCallStore::open(directory.path().join("model-calls.jsonl")).unwrap();
    let transport = TestTransport {
        calls: Cell::new(0),
        status: 401,
    };
    let prepared = ForegroundProviderRun::prepare(
        &config(Some("secret-ref:provider-fixture"), "openai-compatible"),
        request(),
    )
    .unwrap();
    let error = prepared
        .run_with_transport(
            &transport,
            EventLog::open(&events_path).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap_err();
    assert_eq!(transport.calls.get(), 1);
    assert!(calls.read_all().unwrap().is_empty());
    let failures = calls.read_failures().unwrap();
    assert_eq!(failures.len(), 1);
    let events = EventLog::open(&events_path).unwrap().read_all().unwrap();
    let audit = format!(
        "{error} {} {}",
        serde_json::to_string(&failures).unwrap(),
        serde_json::to_string(&events).unwrap()
    );
    assert!(!audit.contains("RAW_UPSTREAM_BODY"));
    assert!(!audit.contains("secret-ref:provider-fixture"));
}

#[test]
fn blank_objective_is_rejected_during_preparation() {
    let mut input = request();
    input.objective = "   ".into();
    assert!(ForegroundProviderRun::prepare(
        &config(Some("secret-ref:provider-fixture"), "openai"),
        input
    )
    .is_err());
}

struct EffectCaptureTransport {
    inner: TestTransport,
    body: RefCell<String>,
}

impl ProviderTransport for &EffectCaptureTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        *self.body.borrow_mut() = body.into();
        (&self.inner).send(endpoint, auth_ref, body, timeout, streaming)
    }
}

#[test]
fn configured_effect_binds_foreground_wire_and_exact_audit_hash() {
    for (level, effort) in [
        ("legacy", None),
        ("fast", Some("low")),
        ("balanced", Some("medium")),
        ("strong", Some("high")),
        ("x_high", Some("xhigh")),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut encoded =
            serde_json::to_value(config(Some("secret-ref:provider-fixture"), "openai")).unwrap();
        if effort.is_some() {
            encoded["model_profiles"][0]["effect"] = json!({
                "selected_level":level,
                "mapping":{"fast":"low","balanced":"medium","strong":"high","x_high":"xhigh"}
            });
        }
        let config = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap();
        let transport = EffectCaptureTransport {
            inner: TestTransport {
                calls: Cell::new(0),
                status: 200,
            },
            body: RefCell::new(String::new()),
        };
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        ForegroundProviderRun::prepare(&config, request())
            .unwrap()
            .run_with_transport(
                &transport,
                EventLog::open(directory.path().join("events.jsonl")).unwrap(),
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
            .unwrap();
        let body = transport.body.borrow();
        let wire: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(wire["model"], "test-model");
        if effort.is_none() {
            assert!(wire.get("reasoning_effort").is_none());
        }
        assert_eq!(
            wire.get("reasoning_effort")
                .and_then(serde_json::Value::as_str),
            effort
        );
        assert_eq!(
            calls.read_all().unwrap()[0].input_context_hash,
            format!("sha256:{:x}", Sha256::digest(body.as_bytes()))
        );
        assert_eq!(transport.inner.calls.get(), 1);
        assert!(!body.contains("secret-ref:"));
    }
}

#[test]
fn invalid_effect_is_rejected_during_prepare_before_missing_auth() {
    let mut config = config(None, "openai");
    let mut profile = serde_json::to_value(&config.model_profiles[0]).unwrap();
    profile["effect"] = json!({"selected_level":"strong", "mapping":{"fast":"low"}});
    config.model_profiles[0] = serde_json::from_value(profile).unwrap();
    let error = ForegroundProviderRun::prepare(&config, request())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ProviderRunError::Config(ConfigError::InvalidModelEffect { .. })
    ));
}

#[test]
fn zero_reported_token_limit_stops_both_foreground_transports_before_any_call() {
    for provider in ["openai-compatible", "anthropic"] {
        let directory = tempfile::tempdir().unwrap();
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let mut config = config(Some("secret-ref:provider-fixture"), provider);
        config.reported_token_limit = Some(0);
        let prepared = ForegroundProviderRun::prepare(&config, request()).unwrap();
        config.reported_token_limit = None;
        let openai = TestTransport {
            calls: Cell::new(0),
            status: 200,
        };
        let anthropic = AnthropicTestTransport {
            calls: Cell::new(0),
        };
        let events = EventLog::open(directory.path().join("events.jsonl")).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_anthropic_with_transport(
                &anthropic,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_transport(
                &openai,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        };
        assert_eq!(openai.calls.get() + anthropic.calls.get(), 0, "{provider}");
        assert!(
            matches!(
                result,
                Err(ProviderRunError::Controller(
                    LoopControllerError::ReportedTokenBudget(ReportedTokenBudgetError::Exhausted)
                ))
            ),
            "{result:?}"
        );
        assert!(calls.read_all().unwrap().is_empty());
        assert!(calls.read_failures().unwrap().is_empty());
        assert_failed_loop(&directory.path().join("events.jsonl"));
        assert!(
            ToolCallRecordStore::open(directory.path().join("tool-calls.jsonl"))
                .unwrap()
                .read_page(0, 16)
                .unwrap()
                .is_empty()
        );
    }
}

struct UnavailableCountingResolver(Arc<AtomicUsize>);

impl CredentialResolver for UnavailableCountingResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }
}

#[test]
fn zero_reported_token_limit_stops_both_http_protocols_before_credential_resolution() {
    for provider in ["openai-compatible", "anthropic"] {
        let directory = tempfile::tempdir().unwrap();
        let resolutions = Arc::new(AtomicUsize::new(0));
        let resolver = UnavailableCountingResolver(resolutions.clone());
        let mut config = config(Some("secret-ref:provider-fixture"), provider);
        config.reported_token_limit = Some(0);
        let prepared = ForegroundProviderRun::prepare(&config, request()).unwrap();
        let events = EventLog::open(directory.path().join("events.jsonl")).unwrap();
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_anthropic_with_transport(
                HttpAnthropicTransport::new(resolver).unwrap(),
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_resolver(
                resolver,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        };
        assert_eq!(resolutions.load(Ordering::SeqCst), 0, "{provider}");
        assert!(
            matches!(
                result,
                Err(ProviderRunError::Controller(
                    LoopControllerError::ReportedTokenBudget(ReportedTokenBudgetError::Exhausted)
                ))
            ),
            "{result:?}"
        );
        assert!(calls.read_all().unwrap().is_empty());
        assert!(calls.read_failures().unwrap().is_empty());
        assert_failed_loop(&directory.path().join("events.jsonl"));
    }
}

fn assert_failed_loop(path: &std::path::Path) {
    let events = EventLog::open(path).unwrap().read_all().unwrap();
    assert_eq!(events.first().unwrap().event_type, "loop_started");
    let failures: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "loop_failed")
        .collect();
    assert_eq!(failures.len(), 1);
    let failed = failures[0];
    let started = &events[0];
    assert_eq!(started.project_id.as_deref(), Some("project-provider"));
    assert_eq!(started.goal_id.as_deref(), Some("goal-provider"));
    assert_eq!(started.task_run_id.as_deref(), Some("task-run-provider"));
    assert_eq!(started.loop_run_id.as_deref(), Some("loop-provider"));
    assert_eq!(failed.project_id, started.project_id);
    assert_eq!(failed.goal_id, started.goal_id);
    assert_eq!(failed.task_run_id, started.task_run_id);
    assert_eq!(failed.loop_run_id, started.loop_run_id);
    assert_eq!(failed.attempt_id, started.attempt_id);
    assert!(failed.attempt_id.is_some());
    assert_eq!(
        failed.payload,
        json!({"status": "failed", "summary": "foreground model step failed"})
    );
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
}

struct FailureFixtureTransport {
    calls: Cell<usize>,
    status: u16,
}

impl FailureFixtureTransport {
    fn response(&self, anthropic: bool) -> TransportResponse {
        self.calls.set(self.calls.get() + 1);
        if self.status != 200 {
            return TransportResponse::body(self.status, "RAW_UPSTREAM_BODY");
        }
        let invalid = "RAW_INVALID_DECISION";
        let body = if anthropic {
            json!({"id": "invalid-message", "type": "message", "role": "assistant", "model": "test-model",
                "content": [{"type": "text", "text": invalid}], "stop_reason": "end_turn",
                "usage": {"input_tokens": 3, "output_tokens": 2}})
        } else {
            json!({"id": "invalid-request", "model": "test-model",
                "choices": [{"message": {"content": invalid}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}})
        };
        TransportResponse::body(200, body.to_string())
    }
}

impl ProviderTransport for &FailureFixtureTransport {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        Ok(self.response(false))
    }
}

impl AnthropicTransport for &FailureFixtureTransport {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        Ok(self.response(true))
    }
}

fn assert_foreground_failure(provider: &str, status: u16) {
    let directory = tempfile::tempdir().unwrap();
    let events_path = directory.path().join("events.jsonl");
    let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
    let transport = FailureFixtureTransport {
        calls: Cell::new(0),
        status,
    };
    let prepared = ForegroundProviderRun::prepare(
        &config(Some("secret-ref:provider-fixture"), provider),
        request(),
    )
    .unwrap();
    let events = EventLog::open(&events_path).unwrap();
    let result = if provider == "anthropic" {
        prepared.run_anthropic_with_transport(
            &transport,
            events,
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
    } else {
        prepared.run_with_transport(
            &transport,
            events,
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
    };
    let error = match result.unwrap_err() {
        ProviderRunError::Controller(LoopControllerError::ModelAdapter(error)) => error,
        other => panic!("unexpected original error: {other:?}"),
    };
    let expected = match status {
        401 => ModelAdapterError::Provider {
            class: ProviderErrorClass::Authentication,
        },
        429 => ModelAdapterError::Provider {
            class: ProviderErrorClass::RateLimit,
        },
        200 => ModelAdapterError::DecisionOutputRejected {
            reason: DecisionOutputRejection::JsonSyntaxInvalid,
        },
        _ => unreachable!(),
    };
    assert_eq!(error, expected);
    assert_eq!(transport.calls.get(), 1);
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    assert!(calls.read_all().unwrap().is_empty());
    assert!(
        ToolCallRecordStore::open(directory.path().join("tool-calls.jsonl"))
            .unwrap()
            .read_page(0, 16)
            .unwrap()
            .is_empty()
    );
    assert_failed_loop(&events_path);
    let audit = format!(
        "{}{}",
        std::fs::read_to_string(events_path).unwrap(),
        serde_json::to_string(&calls.read_failures().unwrap()).unwrap()
    );
    for forbidden in [
        "RAW_UPSTREAM_BODY",
        "RAW_INVALID_DECISION",
        "secret-ref:provider-fixture",
    ] {
        assert!(!audit.contains(forbidden));
    }
}

#[test]
fn openai_authentication_failure_closes_foreground_loop() {
    assert_foreground_failure("openai-compatible", 401);
}
#[test]
fn anthropic_authentication_failure_closes_foreground_loop() {
    assert_foreground_failure("anthropic", 401);
}
#[test]
fn openai_retryable_failure_closes_foreground_loop() {
    assert_foreground_failure("openai-compatible", 429);
}
#[test]
fn anthropic_retryable_failure_closes_foreground_loop() {
    assert_foreground_failure("anthropic", 429);
}
#[test]
fn openai_invalid_decision_closes_foreground_loop() {
    assert_foreground_failure("openai-compatible", 200);
}
#[test]
fn anthropic_invalid_decision_closes_foreground_loop() {
    assert_foreground_failure("anthropic", 200);
}

struct FailingProgressSink {
    subject: ProgressSubject,
    status: ProgressStatus,
}

impl ProgressSink for FailingProgressSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if event.subject == self.subject && event.status == self.status {
            Err(std::io::Error::other("fixture progress failure"))
        } else {
            Ok(())
        }
    }
}

fn assert_progress_failure(
    subject: ProgressSubject,
    status: ProgressStatus,
    expected_sends: usize,
    completed: bool,
) {
    for provider in ["openai-compatible", "anthropic"] {
        let directory = tempfile::tempdir().unwrap();
        let events_path = directory.path().join("events.jsonl");
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let prepared = ForegroundProviderRun::prepare(
            &config(Some("secret-ref:provider-fixture"), provider),
            request(),
        )
        .unwrap();
        let openai = TestTransport {
            calls: Cell::new(0),
            status: 200,
        };
        let anthropic = AnthropicTestTransport {
            calls: Cell::new(0),
        };
        let sink = FailingProgressSink { subject, status };
        let events = EventLog::open(&events_path).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_anthropic_with_transport(&anthropic, events, calls.clone(), sink)
        } else {
            prepared.run_with_transport(&openai, events, calls.clone(), sink)
        };
        assert!(
            matches!(result, Err(ProviderRunError::Controller(LoopControllerError::ProgressReport(ref message))) if message == "fixture progress failure")
        );
        assert_eq!(openai.calls.get() + anthropic.calls.get(), expected_sends);
        assert_eq!(calls.read_all().unwrap().len(), expected_sends);
        assert!(calls.read_failures().unwrap().is_empty());
        if completed {
            let events = EventLog::open(events_path).unwrap().read_all().unwrap();
            assert_eq!(events.last().unwrap().event_type, "loop_completed");
            assert!(!events.iter().any(|event| event.event_type == "loop_failed"));
        } else {
            assert_failed_loop(&events_path);
        }
    }
}

#[test]
fn active_progress_failure_before_send_closes_foreground_loop() {
    assert_progress_failure(ProgressSubject::Model, ProgressStatus::Started, 0, false);
}
#[test]
fn active_progress_failure_after_send_closes_foreground_loop() {
    assert_progress_failure(ProgressSubject::Model, ProgressStatus::Completed, 1, false);
}
#[test]
fn completed_progress_failure_preserves_foreground_completion() {
    assert_progress_failure(ProgressSubject::Loop, ProgressStatus::Completed, 1, true);
}

struct UnwritableAuditSink {
    path: std::path::PathBuf,
}

impl ProgressSink for UnwritableAuditSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if event.subject == ProgressSubject::Model && event.status == ProgressStatus::Completed {
            std::fs::rename(&self.path, self.path.with_extension("saved"))?;
            std::fs::create_dir(&self.path)?;
            return Err(std::io::Error::other("original progress failure"));
        }
        Ok(())
    }
}

#[test]
fn foreground_closure_audit_error_takes_precedence_over_original_error() {
    for provider in ["openai-compatible", "anthropic"] {
        let directory = tempfile::tempdir().unwrap();
        let events_path = directory.path().join("events.jsonl");
        let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let prepared = ForegroundProviderRun::prepare(
            &config(Some("secret-ref:provider-fixture"), provider),
            request(),
        )
        .unwrap();
        let openai = TestTransport {
            calls: Cell::new(0),
            status: 200,
        };
        let anthropic = AnthropicTestTransport {
            calls: Cell::new(0),
        };
        let events = EventLog::open(&events_path).unwrap();
        let sink = UnwritableAuditSink {
            path: events_path.clone(),
        };
        let result = if provider == "anthropic" {
            prepared.run_anthropic_with_transport(&anthropic, events, calls.clone(), sink)
        } else {
            prepared.run_with_transport(&openai, events, calls.clone(), sink)
        };
        assert!(matches!(
            result,
            Err(ProviderRunError::Controller(LoopControllerError::EventLog(
                _
            )))
        ));
        assert_eq!(openai.calls.get() + anthropic.calls.get(), 1);
        assert_eq!(calls.read_all().unwrap().len(), 1);
        assert!(calls.read_failures().unwrap().is_empty());
        let saved_events = EventLog::open(events_path.with_extension("saved"))
            .unwrap()
            .read_all()
            .unwrap();
        assert!(!saved_events
            .iter()
            .any(|event| event.event_type == "loop_failed"));
    }
}

#[test]
fn foreground_outcomes_snapshot_frozen_task_local_usage_or_disabled_budget() {
    for provider in ["openai-compatible", "anthropic"] {
        for limit in [None, Some(1)] {
            for _ in 0..2 {
                let directory = tempfile::tempdir().unwrap();
                let calls = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
                let mut config = config(Some("secret-ref:provider-fixture"), provider);
                config.reported_token_limit = limit;
                let prepared = ForegroundProviderRun::prepare(&config, request()).unwrap();
                config.reported_token_limit = Some(0);
                let openai = TestTransport {
                    calls: Cell::new(0),
                    status: 200,
                };
                let anthropic = AnthropicTestTransport {
                    calls: Cell::new(0),
                };
                let events = EventLog::open(directory.path().join("events.jsonl")).unwrap();
                let outcome = if provider == "anthropic" {
                    prepared.run_anthropic_with_transport(
                        &anthropic,
                        events,
                        calls.clone(),
                        StreamReporter::new(Vec::new()),
                    )
                } else {
                    prepared.run_with_transport(
                        &openai,
                        events,
                        calls.clone(),
                        StreamReporter::new(Vec::new()),
                    )
                }
                .unwrap();
                assert_eq!(openai.calls.get() + anthropic.calls.get(), 1);
                assert_eq!(outcome.loop_status, LoopStatus::Completed);
                let records = calls.read_all().unwrap();
                assert_eq!(records.len(), 1);
                if let Some(limit) = limit {
                    let budget = outcome.reported_token_budget().unwrap();
                    let (input, output) = if provider == "anthropic" {
                        (17, 5)
                    } else {
                        (21, 8)
                    };
                    assert_eq!(budget.limit(), limit);
                    assert_eq!(budget.known_input_tokens(), input);
                    assert_eq!(budget.known_output_tokens(), output);
                    assert_eq!(budget.known_total_tokens(), input + output);
                    assert_eq!(budget.blocked(), None);
                    assert_eq!(budget.records().len(), 1);
                    assert_eq!(
                        budget.records()[0].model_call_id(),
                        records[0].model_call_id
                    );
                } else {
                    assert_eq!(outcome.reported_token_budget(), None);
                }
            }
        }
    }
}
