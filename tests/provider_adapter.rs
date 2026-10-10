use std::sync::{Arc, Mutex};
use std::time::Duration;

use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderErrorClass, ProviderMessage, ProviderRequest,
    ProviderStreamEvent, ProviderTransport, TransportError, TransportResponse,
};

type RecordedCall = (String, Option<String>, String, Duration, bool);

#[derive(Clone)]
struct FakeTransport {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    response: TransportResponse,
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            response: TransportResponse::body(200, "{}"),
        }
    }
}

impl FakeTransport {
    fn complete(body: &str) -> Self {
        Self {
            response: TransportResponse::body(200, body),
            ..Self::default()
        }
    }

    fn stream(body: &str) -> Self {
        Self {
            response: TransportResponse::body(200, body),
            ..Self::default()
        }
    }

    fn status(status_code: u16, body: &str) -> Self {
        Self {
            response: TransportResponse::body(status_code, body),
            ..Self::default()
        }
    }
}

impl ProviderTransport for FakeTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.calls.lock().expect("calls lock").push((
            endpoint.to_owned(),
            auth_ref.map(str::to_owned),
            request_body.to_owned(),
            timeout,
            streaming,
        ));
        Ok(self.response.clone())
    }
}

#[test]
fn complete_sends_openai_compatible_json_without_auth_body() {
    let transport = FakeTransport::complete(
        r#"{"id":"chatcmpl-1","model":"fixture-model","choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}"#,
    );
    let calls = Arc::clone(&transport.calls);
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);
    let response = adapter
        .complete(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("env:STEERBOX_PROVIDER_KEY".into()),
            timeout_ms: 12_000,
            stream: false,
        })
        .expect("complete response");

    assert_eq!(response.request_id, "chatcmpl-1");
    assert_eq!(response.model, "fixture-model");
    assert_eq!(response.text, "done");
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    let calls = calls.lock().expect("calls lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1.as_deref(), Some("env:STEERBOX_PROVIDER_KEY"));
    assert!(calls[0].2.contains(r#""model":"fixture-model"#));
    assert!(!calls[0].2.contains("STEERBOX_PROVIDER_KEY"));
    assert!(!calls[0].2.contains("api_key"));
    assert_eq!(calls[0].3, Duration::from_millis(12_000));
    assert!(!calls[0].4);
}

#[test]
fn streaming_body_becomes_secret_free_events() {
    let transport = FakeTransport::stream(
        "data: {\"id\":\"chatcmpl-2\",\"model\":\"fixture-model\",\"choices\":[{\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-2\",\"model\":\"fixture-model\",\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n",
    );
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);
    let events = adapter
        .stream(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("keychain:steerbox/provider-a".into()),
            timeout_ms: 30_000,
            stream: true,
        })
        .expect("stream events");

    assert_eq!(
        events,
        vec![
            ProviderStreamEvent::delta("chatcmpl-2", "fixture-model", "hel"),
            ProviderStreamEvent::delta("chatcmpl-2", "fixture-model", "lo"),
            ProviderStreamEvent::completed("chatcmpl-2", "fixture-model", "stop"),
        ]
    );
}

#[test]
fn auth_ref_must_be_a_reference_not_credential_body() {
    let adapter = OpenAiCompatibleAdapter::new(
        "provider-a",
        "https://example.test/v1",
        FakeTransport::default(),
    );
    let error = adapter
        .complete(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("sk-live-secret-value".into()),
            timeout_ms: 30_000,
            stream: false,
        })
        .expect_err("credential body must be rejected");
    assert_eq!(error.class(), ProviderErrorClass::InvalidRequest);
}

#[test]
fn transport_timeout_is_classified_without_retaining_transport_details() {
    #[derive(Clone)]
    struct TimeoutTransport;

    impl ProviderTransport for TimeoutTransport {
        fn send(
            &self,
            _endpoint: &str,
            _auth_ref: Option<&str>,
            _request_body: &str,
            _timeout: Duration,
            _streaming: bool,
        ) -> Result<TransportResponse, TransportError> {
            Err(TransportError::timeout(
                "upstream timeout with token=secret",
            ))
        }
    }

    let adapter =
        OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", TimeoutTransport);
    let error = adapter
        .complete(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("env:STEERBOX_PROVIDER_KEY".into()),
            timeout_ms: 30_000,
            stream: false,
        })
        .expect_err("timeout expected");
    assert_eq!(error.class(), ProviderErrorClass::Timeout);
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains("token="));
}

#[test]
fn timeout_zero_and_too_large_are_rejected_before_transport() {
    let adapter = OpenAiCompatibleAdapter::new(
        "provider-a",
        "https://example.test/v1",
        FakeTransport::default(),
    );
    for timeout_ms in [0, 300_001] {
        let error = adapter
            .complete(ProviderRequest {
                model: "fixture-model".into(),
                messages: vec![ProviderMessage::user("hello")],
                auth_ref: Some("env:STEERBOX_PROVIDER_KEY".into()),
                timeout_ms,
                stream: false,
            })
            .expect_err("invalid timeout expected");
        assert_eq!(error.class(), ProviderErrorClass::InvalidRequest);
    }
}

#[test]
fn non_success_status_is_classified_without_exposing_response_body() {
    let adapter = OpenAiCompatibleAdapter::new(
        "provider-a",
        "https://example.test/v1",
        FakeTransport::status(500, r#"{"error":"authorization=top-secret"}"#),
    );
    let error = adapter
        .complete(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("env:STEERBOX_PROVIDER_KEY".into()),
            timeout_ms: 30_000,
            stream: false,
        })
        .expect_err("upstream error expected");
    assert_eq!(error.class(), ProviderErrorClass::Upstream);
}

#[test]
fn malformed_stream_chunks_fail_closed_when_required_fields_are_missing() {
    let transport = FakeTransport::stream(
        "data: {\"id\":\"chatcmpl-3\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n",
    );
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);
    let error = adapter
        .stream(ProviderRequest {
            model: "fixture-model".into(),
            messages: vec![ProviderMessage::user("hello")],
            auth_ref: Some("env:STEERBOX_PROVIDER_KEY".into()),
            timeout_ms: 30_000,
            stream: true,
        })
        .expect_err("missing model must fail");
    assert_eq!(error.class(), ProviderErrorClass::ResponseParse);
}
