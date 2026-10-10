use std::sync::{Arc, Mutex};
use std::time::Duration;

use steerbox::model_adapter::{ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest};
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderTransport, TransportError, TransportResponse,
};
use steerbox::provider_model_adapter::ProviderModelAdapter;

#[derive(Clone)]
struct FakeTransport {
    response: Result<TransportResponse, TransportError>,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl FakeTransport {
    fn response(body: &str) -> Self {
        Self {
            response: Ok(TransportResponse::body(200, body)),
            bodies: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn timeout() -> Self {
        Self {
            response: Err(TransportError::timeout("no credential body")),
            bodies: Arc::new(Mutex::new(Vec::new())),
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
        self.bodies
            .lock()
            .expect("bodies lock")
            .push(request_body.to_owned());
        self.response.clone()
    }
}

fn request() -> ModelRequest {
    ModelRequest {
        model_id: "provider-model".into(),
        goal_id: "goal-1".into(),
        loop_run_id: "loop-1".into(),
        step_id: "step-1".into(),
        objective: "finish the task".into(),
        observation: "the work is ready".into(),
    }
}

#[test]
fn strict_provider_decision_envelope_maps_to_existing_model_response() {
    let transport = FakeTransport::response(
        r#"{"id":"chatcmpl-1","model":"provider-model","choices":[{"message":{"role":"assistant","content":"{\"decision\":\"complete_loop\",\"summary\":\"done\"}"},"finish_reason":"stop"}]}"#,
    );
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport),
        Some("env:STEERBOX_PROVIDER_KEY".into()),
    );

    let response = adapter.complete(request()).expect("mapped model response");

    assert_eq!(response.decision, ModelDecision::CompleteLoop);
    assert_eq!(response.summary, "done");
}

#[test]
fn plain_provider_text_returns_explicit_unmappable_error() {
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new(
            "provider-a",
            "https://example.test/v1",
            FakeTransport::response(
                r#"{"id":"chatcmpl-2","model":"provider-model","choices":[{"message":{"role":"assistant","content":"The task is done."},"finish_reason":"stop"}]}"#,
            ),
        ),
        None,
    );

    let error = adapter
        .complete(request())
        .expect_err("natural language must not be guessed into a decision");

    assert_eq!(error, ModelAdapterError::ProviderTextUnmappable);
}

#[test]
fn provider_failures_keep_only_a_safe_classification() {
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new(
            "provider-a",
            "https://example.test/v1",
            FakeTransport::timeout(),
        ),
        Some("env:STEERBOX_PROVIDER_KEY".into()),
    );

    let error = adapter.complete(request()).expect_err("timeout expected");

    assert_eq!(
        error,
        ModelAdapterError::Provider {
            class: steerbox::provider_adapter::ProviderErrorClass::Timeout,
        }
    );
    assert!(!error.to_string().contains("credential"));
}

#[test]
fn provider_request_carries_only_auth_reference_and_validates_model_input() {
    let transport = FakeTransport::response(
        r#"{"id":"chatcmpl-3","model":"provider-model","choices":[{"message":{"role":"assistant","content":"{\"decision\":\"request_human\",\"summary\":\"approval\"}"},"finish_reason":"stop"}]}"#,
    );
    let bodies = Arc::clone(&transport.bodies);
    let adapter = ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("provider-a", "https://example.test/v1", transport),
        Some("keychain:steerbox/provider-a".into()),
    );

    let response = adapter.complete(request()).expect("mapped model response");
    assert_eq!(response.decision, ModelDecision::RequestHuman);
    let body = bodies.lock().expect("bodies lock");
    assert!(!body[0].contains("keychain:steerbox/provider-a"));

    let mut invalid = request();
    invalid.objective.clear();
    assert_eq!(
        adapter.complete(invalid).expect_err("empty objective"),
        ModelAdapterError::EmptyObjective
    );
}
