use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steerbox::anthropic_adapter::{
    AnthropicAdapter, AnthropicAdapterError, AnthropicHeaders, AnthropicTransport,
    AnthropicTransportError,
};
use steerbox::model_adapter::{ModelAdapter, ModelDecision, ModelRequest};
use steerbox::provider_adapter::{ProviderMessage, ProviderRequest, TransportResponse};

#[derive(Clone)]
struct FakeTransport {
    response: TransportResponse,
    captured: Arc<Mutex<Option<CapturedRequest>>>,
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    endpoint: String,
    headers: AnthropicHeaders,
    body: String,
    timeout: Duration,
    streaming: bool,
}

impl FakeTransport {
    fn response(body: &str) -> Self {
        Self {
            response: TransportResponse::body(200, body),
            captured: Arc::new(Mutex::new(None)),
        }
    }

    fn status(status_code: u16, body: &str) -> Self {
        Self {
            response: TransportResponse::body(status_code, body),
            captured: Arc::new(Mutex::new(None)),
        }
    }
}

impl AnthropicTransport for FakeTransport {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        *self.captured.lock().expect("capture lock") = Some(CapturedRequest {
            endpoint: endpoint.to_owned(),
            headers: headers.clone(),
            body: request_body.to_owned(),
            timeout,
            streaming,
        });
        Ok(self.response.clone())
    }
}

fn response_body(text: &str) -> String {
    serde_json::json!({
        "id": "msg_fixture",
        "type": "message",
        "role": "assistant",
        "model": "claude-fixture",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 3, "output_tokens": 2}
    })
    .to_string()
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "claude-fixture".into(),
        messages: vec![
            ProviderMessage::system("system contract"),
            ProviderMessage::user("inspect repository"),
        ],
        auth_ref: Some("env:ANTHROPIC_API_KEY".into()),
        timeout_ms: 10_000,
        stream: false,
    }
}

#[test]
fn builds_anthropic_envelope_with_independent_auth_header_reference() {
    let transport = FakeTransport::response(&response_body(
        r#"{"decision":"complete_loop","summary":"done"}"#,
    ));
    let captured = transport.captured.clone();
    let adapter = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test/v1",
        transport,
        Some("env:CONFIGURED_KEY".into()),
    )
    .with_max_tokens(777)
    .with_timeout_ms(8_000);

    let response = adapter.complete(request()).expect("completion succeeds");
    assert_eq!(response.request_id, "msg_fixture");
    assert_eq!(response.model, "claude-fixture");
    assert_eq!(response.finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(response.usage.unwrap().total_tokens, Some(5));

    let captured = captured.lock().expect("capture lock").clone().unwrap();
    assert_eq!(
        captured.endpoint,
        "https://fixture.example.test/v1/messages"
    );
    assert_eq!(captured.headers.auth_header_name(), "x-api-key");
    assert_eq!(captured.headers.version_header_name(), "anthropic-version");
    assert_eq!(
        captured.headers.api_key_ref.as_deref(),
        Some("env:ANTHROPIC_API_KEY")
    );
    assert_eq!(captured.headers.anthropic_version, "2023-06-01");
    assert_eq!(captured.timeout, Duration::from_millis(8_000));
    assert!(!captured.streaming);
    assert!(!captured.body.contains("Authorization"));
    assert!(!captured.body.contains("ANTHROPIC_API_KEY"));

    let body: Value = serde_json::from_str(&captured.body).expect("valid request JSON");
    assert_eq!(body["model"], "claude-fixture");
    assert_eq!(body["max_tokens"], 777);
    assert_eq!(body["stream"], false);
    assert_eq!(body["system"], "system contract");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "inspect repository"
    );
}

#[test]
fn model_adapter_parses_only_the_decision_envelope() {
    let adapter = AnthropicAdapter::new(
        "anthropic",
        "http://localhost:4318",
        FakeTransport::response(&response_body(
            r#"{"decision":"request_human","summary":"manual review"}"#,
        )),
        Some("secret-ref:anthropic".into()),
    );
    let response = ModelAdapter::complete(
        &adapter,
        ModelRequest {
            model_id: "claude-fixture".into(),
            goal_id: "goal-1".into(),
            loop_run_id: "loop-1".into(),
            step_id: "step-1".into(),
            objective: "inspect repository".into(),
            observation: "read-only evidence".into(),
        },
    )
    .expect("model adapter succeeds");
    assert_eq!(response.decision, ModelDecision::RequestHuman);
    assert_eq!(response.summary, "manual review");
}

#[test]
fn rejects_streaming_and_malformed_or_empty_content() {
    let mut streaming = request();
    streaming.stream = true;
    let adapter = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        FakeTransport::response(&response_body("unused")),
        None,
    );
    assert_eq!(
        adapter.complete(streaming).unwrap_err(),
        AnthropicAdapterError::StreamingUnsupported
    );

    let malformed = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        FakeTransport::response("not-json"),
        None,
    );
    assert_eq!(
        malformed.complete(request()).unwrap_err(),
        AnthropicAdapterError::ResponseParse
    );

    let empty = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        FakeTransport::response(&response_body("   ")),
        None,
    );
    assert_eq!(
        empty.complete(request()).unwrap_err(),
        AnthropicAdapterError::ResponseParse
    );

    let streaming_body = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        FakeTransport::response("data: {\"type\":\"message_start\"}"),
        None,
    );
    assert_eq!(
        streaming_body.complete(request()).unwrap_err(),
        AnthropicAdapterError::StreamingUnsupported
    );
}

#[test]
fn classifies_statuses_without_exposing_upstream_body_or_credentials() {
    let secret_body = "provider-secret-body-and-api-key=do-not-return";
    for (status, expected) in [
        (
            400,
            AnthropicAdapterError::InvalidRequest {
                reason: "provider rejected the request",
            },
        ),
        (401, AnthropicAdapterError::Authentication),
        (403, AnthropicAdapterError::Permission),
        (408, AnthropicAdapterError::Timeout),
        (429, AnthropicAdapterError::RateLimit),
        (500, AnthropicAdapterError::Upstream),
    ] {
        let adapter = AnthropicAdapter::new(
            "anthropic",
            "https://fixture.example.test",
            FakeTransport::status(status, secret_body),
            Some("env:ANTHROPIC_API_KEY".into()),
        );
        let error = adapter.complete(request()).unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("provider-secret"));
        assert!(!error.to_string().contains("ANTHROPIC_API_KEY"));
    }
}

#[test]
fn rejects_auth_material_and_keeps_transport_errors_generic() {
    let invalid = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        FakeTransport::response(&response_body("unused")),
        Some("env:sk-real-secret".into()),
    );
    let error = invalid.complete(request()).unwrap_err();
    assert!(matches!(
        error,
        AnthropicAdapterError::InvalidRequest { .. }
    ));
    assert!(!error.to_string().contains("sk-real-secret"));

    struct ErrorTransport;
    impl AnthropicTransport for ErrorTransport {
        fn send(
            &self,
            _: &str,
            _: &AnthropicHeaders,
            _: &str,
            _: Duration,
            _: bool,
        ) -> Result<TransportResponse, AnthropicTransportError> {
            Err(AnthropicTransportError::Credential)
        }
    }

    let adapter = AnthropicAdapter::new(
        "anthropic",
        "https://fixture.example.test",
        ErrorTransport,
        Some("secret-ref:anthropic".into()),
    );
    assert_eq!(
        adapter.complete(request()).unwrap_err(),
        AnthropicAdapterError::Authentication
    );
}
