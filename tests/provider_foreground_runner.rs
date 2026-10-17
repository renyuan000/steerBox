use serde_json::json;
use std::cell::Cell;
use std::time::Duration;
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::LoopStatus;
use steerbox::event_log::EventLog;
use steerbox::model_adapter::ModelDecision;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunError, ProviderRunRequest};
use steerbox::runtime_config::{ConfigError, RuntimeConfig};
use steerbox::stream_reporter::StreamReporter;

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
