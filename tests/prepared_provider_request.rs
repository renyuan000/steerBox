use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderErrorClass, ProviderMessage, ProviderRequest,
    ProviderTransport, TransportError, TransportResponse, MAX_TIMEOUT_MS,
};

#[derive(Clone)]
struct CapturedSend {
    endpoint: String,
    auth_ref: Option<String>,
    body: String,
    timeout: Duration,
    streaming: bool,
}

#[derive(Clone, Default)]
struct FakeTransport {
    sends: Arc<Mutex<Vec<CapturedSend>>>,
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
        self.sends.lock().expect("capture lock").push(CapturedSend {
            endpoint: endpoint.to_owned(),
            auth_ref: auth_ref.map(str::to_owned),
            body: request_body.to_owned(),
            timeout,
            streaming,
        });
        Ok(TransportResponse::body(
            200,
            r#"{"id":"fixture-completion","model":"fixture-model","choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}]}"#,
        ))
    }
}

fn fixture_request() -> ProviderRequest {
    ProviderRequest {
        model: "fixture-model".into(),
        messages: vec![
            ProviderMessage::system("fixture system"),
            ProviderMessage::user("quote: \"; newline:\n; unicode: 冻结请求"),
        ],
        auth_ref: Some("env:STEERBOX_FIXTURE_KEY".into()),
        timeout_ms: 12_000,
        stream: false,
    }
}

#[test]
fn prepared_hash_matches_exact_wire_bytes_sent() {
    let transport = FakeTransport::default();
    let sends = Arc::clone(&transport.sends);
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);
    let prepared = adapter
        .prepare_complete(fixture_request())
        .expect("prepare");
    let hash = prepared.input_hash().to_owned();

    adapter
        .complete_prepared(&prepared)
        .expect("send frozen request");

    let sends = sends.lock().expect("capture lock");
    assert_eq!(sends.len(), 1);
    let captured = &sends[0];
    let digest = format!("{:x}", Sha256::digest(captured.body.as_bytes()));
    // The contract specifies SHA256, without committing consumers to a prefix.
    assert_eq!(hash.strip_prefix("sha256:").unwrap_or(&hash), digest);
    assert_eq!(prepared.input_hash(), hash);
    assert_eq!(
        captured.endpoint,
        "https://example.test/v1/chat/completions"
    );
    assert_eq!(
        captured.auth_ref.as_deref(),
        Some("env:STEERBOX_FIXTURE_KEY")
    );
    assert_eq!(captured.timeout, Duration::from_millis(12_000));
    assert!(!captured.streaming);
    let body: serde_json::Value = serde_json::from_str(&captured.body).expect("wire JSON");
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(
        body["messages"][1]["content"],
        fixture_request().messages[1].content
    );
    assert_eq!(body["stream"], false);
    assert!(body.get("auth_ref").is_none());
    assert!(body.get("timeout_ms").is_none());
    assert!(!captured.body.contains("STEERBOX_FIXTURE_KEY"));
}

#[test]
fn mutating_original_request_cannot_change_frozen_snapshot() {
    let transport = FakeTransport::default();
    let sends = Arc::clone(&transport.sends);
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);
    let mut original = fixture_request();
    let prepared = adapter.prepare_complete(original.clone()).expect("prepare");
    let hash = prepared.input_hash().to_owned();

    original.model = "changed-model".into();
    original.messages[1].content = "changed message".into();
    original.auth_ref = Some("keychain:fixture/changed".into());
    original.timeout_ms = 1;
    original.stream = true;
    adapter
        .complete_prepared(&prepared)
        .expect("send original snapshot");
    adapter
        .complete_prepared(&prepared)
        .expect("re-send identical snapshot");

    let sends = sends.lock().expect("capture lock");
    assert_eq!(sends.len(), 2);
    assert_eq!(sends[0].body, sends[1].body);
    let body: serde_json::Value = serde_json::from_str(&sends[0].body).expect("wire JSON");
    assert_eq!(body["model"], "fixture-model");
    assert_ne!(body["model"], original.model);
    assert_eq!(
        body["messages"][1]["content"],
        fixture_request().messages[1].content
    );
    assert_eq!(
        sends[0].auth_ref.as_deref(),
        Some("env:STEERBOX_FIXTURE_KEY")
    );
    assert_eq!(sends[0].timeout, Duration::from_millis(12_000));
    assert!(!sends[0].streaming);
    assert_eq!(prepared.input_hash(), hash);
}

#[test]
fn prepared_request_cannot_cross_provider_or_endpoint() {
    let origin_transport = FakeTransport::default();
    let origin_sends = Arc::clone(&origin_transport.sends);
    let origin =
        OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", origin_transport);
    let prepared = origin.prepare_complete(fixture_request()).expect("prepare");

    for (provider_id, base_url) in [
        ("provider-b", "https://example.test/v1"),
        ("provider-a", "https://other.example.test/v1"),
        ("provider-a", "https://example.test/v2"),
    ] {
        let transport = FakeTransport::default();
        let sends = Arc::clone(&transport.sends);
        let foreign = OpenAiCompatibleAdapter::new(provider_id, base_url, transport);
        let error = foreign
            .complete_prepared(&prepared)
            .expect_err("binding mismatch");
        assert_eq!(error.class(), ProviderErrorClass::InvalidRequest);
        assert!(sends.lock().expect("capture lock").is_empty());
    }
    assert!(origin_sends.lock().expect("capture lock").is_empty());
}

#[test]
fn trailing_slash_normalization_matches_actual_endpoint_binding() {
    let origin = OpenAiCompatibleAdapter::new(
        "provider-a",
        "https://example.test/v1///",
        FakeTransport::default(),
    );
    let prepared = origin.prepare_complete(fixture_request()).expect("prepare");
    let transport = FakeTransport::default();
    let sends = Arc::clone(&transport.sends);
    let normalized =
        OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);

    normalized
        .complete_prepared(&prepared)
        .expect("same effective endpoint");

    let sends = sends.lock().expect("capture lock");
    assert_eq!(sends.len(), 1);
    assert_eq!(
        sends[0].endpoint,
        "https://example.test/v1/chat/completions"
    );
}

#[test]
fn invalid_preparation_never_sends() {
    let transport = FakeTransport::default();
    let sends = Arc::clone(&transport.sends);
    let adapter = OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport);

    let mut invalid_requests = Vec::new();
    let mut streaming = fixture_request();
    streaming.stream = true;
    invalid_requests.push(streaming);
    for timeout_ms in [0, MAX_TIMEOUT_MS + 1] {
        let mut request = fixture_request();
        request.timeout_ms = timeout_ms;
        invalid_requests.push(request);
    }
    for auth_ref in ["not-a-reference", "env:", "env:fixture value"] {
        let mut request = fixture_request();
        request.auth_ref = Some(auth_ref.into());
        invalid_requests.push(request);
    }
    let mut empty_model = fixture_request();
    empty_model.model.clear();
    invalid_requests.push(empty_model);
    let mut empty_messages = fixture_request();
    empty_messages.messages.clear();
    invalid_requests.push(empty_messages);

    for request in invalid_requests {
        // PreparedProviderRequest deliberately does not expose body-bearing Debug.
        let error = adapter
            .prepare_complete(request)
            .expect_err("invalid input");
        assert_eq!(error.class(), ProviderErrorClass::InvalidRequest);
        assert!(sends.lock().expect("capture lock").is_empty());
    }
}

#[test]
fn identical_wire_inputs_share_hash_but_transport_metadata_is_excluded() {
    let adapter = OpenAiCompatibleAdapter::new(
        "provider-a",
        "https://example.test/v1",
        FakeTransport::default(),
    );
    let first = adapter
        .prepare_complete(fixture_request())
        .expect("prepare first");
    let identical = adapter
        .prepare_complete(fixture_request())
        .expect("prepare identical");
    assert_eq!(first.input_hash(), identical.input_hash());

    let mut changed_auth = fixture_request();
    changed_auth.auth_ref = Some("keychain:fixture/provider-a".into());
    let changed_auth = adapter
        .prepare_complete(changed_auth)
        .expect("prepare changed auth");
    assert_eq!(first.input_hash(), changed_auth.input_hash());

    let mut changed_timeout = fixture_request();
    changed_timeout.timeout_ms = MAX_TIMEOUT_MS;
    let changed_timeout = adapter
        .prepare_complete(changed_timeout)
        .expect("prepare changed timeout");
    assert_eq!(first.input_hash(), changed_timeout.input_hash());

    let mut changed_model = fixture_request();
    changed_model.model = "another-fixture-model".into();
    let changed_model = adapter
        .prepare_complete(changed_model)
        .expect("prepare changed model");
    assert_ne!(first.input_hash(), changed_model.input_hash());

    let mut changed_message = fixture_request();
    changed_message.messages[1].content.push('!');
    let changed_message = adapter
        .prepare_complete(changed_message)
        .expect("prepare changed message");
    assert_ne!(first.input_hash(), changed_message.input_hash());
}
