use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use steerbox::adaptive_routing::EffectLevel;
use steerbox::model_adapter::{ModelAdapter, ModelRequest};
use steerbox::provider_adapter::{
    ModelEffectPolicy, OpenAiCompatibleAdapter, ProviderAdapterError, ProviderMessage,
    ProviderRequest, ProviderTransport, ReasoningEffort, TransportError, TransportResponse,
};
use steerbox::provider_model_adapter::ProviderModelAdapter;

#[derive(Clone, Debug)]
struct Send {
    body: String,
    endpoint: String,
    auth_ref: Option<String>,
    timeout: Duration,
    streaming: bool,
}

#[derive(Clone, Default, Debug)]
struct Transport(Arc<Mutex<Vec<Send>>>);

impl ProviderTransport for Transport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.0.lock().unwrap().push(Send {
            body: body.to_owned(),
            endpoint: endpoint.to_owned(),
            auth_ref: auth_ref.map(str::to_owned),
            timeout,
            streaming,
        });
        let decision = json!({"decision":"continue_observation","summary":"fixture observed"});
        let response = if streaming {
            format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"id":"fixture","model":"fixture-model","choices":[{
                    "delta":{"content":"fixture"},"finish_reason":"stop"
                }]})
            )
        } else {
            json!({"id":"fixture","model":"fixture-model","choices":[{
                "message":{"content":decision.to_string()},"finish_reason":"stop"
            }]})
            .to_string()
        };
        Ok(TransportResponse::body(200, response))
    }
}

fn adapter(transport: Transport) -> OpenAiCompatibleAdapter<Transport> {
    OpenAiCompatibleAdapter::new("fixture", "https://example.test/v1", transport)
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "fixture-model".into(),
        messages: vec![ProviderMessage::user("fixture task")],
        auth_ref: Some("secret-ref:fixture".into()),
        timeout_ms: 12_000,
        stream: false,
    }
}

fn policy() -> ModelEffectPolicy {
    ModelEffectPolicy::new(
        "fixture-model",
        BTreeMap::from([
            (EffectLevel::Fast, ReasoningEffort::Low),
            (EffectLevel::Balanced, ReasoningEffort::Medium),
            (EffectLevel::Strong, ReasoningEffort::High),
            (EffectLevel::XHigh, ReasoningEffort::XHigh),
        ]),
    )
    .unwrap()
}

#[test]
fn default_request_body_is_unchanged() {
    let transport = Transport::default();
    adapter(transport.clone()).complete(request()).unwrap();
    let sends = transport.0.lock().unwrap();
    let body: Value = serde_json::from_str(&sends[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "model":"fixture-model","messages":[{"role":"user","content":"fixture task"}],
            "stream":false
        })
    );
}

#[test]
fn declared_levels_bind_exact_wire_parameter_and_hash_without_auth_data() {
    for (level, expected) in [
        (EffectLevel::Fast, "low"),
        (EffectLevel::Balanced, "medium"),
        (EffectLevel::Strong, "high"),
        (EffectLevel::XHigh, "xhigh"),
    ] {
        let transport = Transport::default();
        let adapter = adapter(transport.clone())
            .with_effect_policy(policy(), level)
            .unwrap();
        let prepared = adapter.prepare_complete(request()).unwrap();
        adapter.complete_prepared(&prepared).unwrap();
        let sends = transport.0.lock().unwrap();
        let sent = &sends[0];
        let body: Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["reasoning_effort"], expected);
        assert_eq!(
            prepared.input_hash(),
            format!("sha256:{:x}", Sha256::digest(sent.body.as_bytes()))
        );
        assert_eq!(sent.endpoint, "https://example.test/v1/chat/completions");
        assert_eq!(sent.auth_ref.as_deref(), Some("secret-ref:fixture"));
        assert_eq!(sent.timeout, Duration::from_millis(12_000));
        for excluded in [
            "auth_ref",
            "secret-ref:",
            "timeout_ms",
            "effect_level",
            "endpoint",
        ] {
            assert!(!sent.body.contains(excluded));
        }
    }
}

#[test]
fn invalid_policy_is_rejected_without_sending() {
    for (model, mapping) in [
        (
            "",
            BTreeMap::from([(EffectLevel::Fast, ReasoningEffort::Low)]),
        ),
        ("fixture-model", BTreeMap::new()),
        (
            "fixture-model",
            BTreeMap::from([
                (EffectLevel::Fast, ReasoningEffort::High),
                (EffectLevel::Strong, ReasoningEffort::Medium),
            ]),
        ),
        (
            "fixture-model",
            BTreeMap::from([
                (EffectLevel::Balanced, ReasoningEffort::High),
                (EffectLevel::Strong, ReasoningEffort::High),
            ]),
        ),
    ] {
        assert!(matches!(
            ModelEffectPolicy::new(model, mapping),
            Err(ProviderAdapterError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn undeclared_level_or_wrong_model_fails_closed() {
    let transport = Transport::default();
    let sparse = ModelEffectPolicy::new(
        "fixture-model",
        BTreeMap::from([(EffectLevel::Balanced, ReasoningEffort::Medium)]),
    )
    .unwrap();
    assert!(adapter(transport.clone())
        .with_effect_policy(sparse, EffectLevel::XHigh)
        .is_err());
    let adapter = adapter(transport.clone())
        .with_effect_policy(policy(), EffectLevel::Strong)
        .unwrap();
    let mut wrong = request();
    wrong.model = "different-model".into();
    assert!(adapter.complete(wrong.clone()).is_err());
    wrong.stream = true;
    assert!(adapter.stream(wrong).is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}

#[test]
fn stream_uses_the_same_binding_and_keeps_auth_out_of_body() {
    let transport = Transport::default();
    let adapter = adapter(transport.clone())
        .with_effect_policy(policy(), EffectLevel::Fast)
        .unwrap();
    let mut request = request();
    request.stream = true;
    adapter.stream(request).unwrap();
    let sends = transport.0.lock().unwrap();
    let sent = &sends[0];
    let body: Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(body["stream"], true);
    assert!(sent.streaming);
    assert!(!sent.body.contains("secret-ref:"));
}

#[test]
fn prepared_requests_cannot_bypass_a_different_effect_policy() {
    let source = adapter(Transport::default())
        .with_effect_policy(policy(), EffectLevel::Strong)
        .unwrap();
    let prepared = source.prepare_complete(request()).unwrap();
    for selected in [None, Some(EffectLevel::Balanced)] {
        let transport = Transport::default();
        let target = adapter(transport.clone());
        let target = match selected {
            None => target,
            Some(level) => target.with_effect_policy(policy(), level).unwrap(),
        };
        assert!(target.complete_prepared(&prepared).is_err());
        assert!(transport.0.lock().unwrap().is_empty());
    }
    let plain = adapter(Transport::default())
        .prepare_complete(request())
        .unwrap();
    assert!(source.complete_prepared(&plain).is_err());
    let transport = Transport::default();
    let wrong_model_policy = ModelEffectPolicy::new(
        "other-model",
        BTreeMap::from([(EffectLevel::Strong, ReasoningEffort::High)]),
    )
    .unwrap();
    let wrong_model = adapter(transport.clone())
        .with_effect_policy(wrong_model_policy, EffectLevel::Strong)
        .unwrap();
    assert!(wrong_model.complete_prepared(&prepared).is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}

#[test]
fn changed_wire_effort_changes_hash_but_identical_wire_does_not() {
    let balanced = adapter(Transport::default())
        .with_effect_policy(policy(), EffectLevel::Balanced)
        .unwrap();
    let strong = adapter(Transport::default())
        .with_effect_policy(policy(), EffectLevel::Strong)
        .unwrap();
    let a = balanced.prepare_complete(request()).unwrap();
    let b = strong.prepare_complete(request()).unwrap();
    assert_ne!(a.input_hash(), b.input_hash());
    let sparse = ModelEffectPolicy::new(
        "fixture-model",
        BTreeMap::from([(EffectLevel::Strong, ReasoningEffort::Medium)]),
    )
    .unwrap();
    let equivalent = adapter(Transport::default())
        .with_effect_policy(sparse, EffectLevel::Strong)
        .unwrap();
    assert_eq!(
        a.input_hash(),
        equivalent.prepare_complete(request()).unwrap().input_hash()
    );
    equivalent.complete_prepared(&a).unwrap();
}

#[test]
fn model_adapter_bridge_preserves_host_selected_effect() {
    let transport = Transport::default();
    let provider = adapter(transport.clone())
        .with_effect_policy(policy(), EffectLevel::Strong)
        .unwrap();
    let model = ProviderModelAdapter::new(provider, Some("secret-ref:fixture".into()));
    model
        .complete(ModelRequest {
            model_id: "fixture-model".into(),
            goal_id: "fixture-goal".into(),
            loop_run_id: "fixture-loop".into(),
            step_id: "fixture-step".into(),
            objective: "inspect fixture".into(),
            observation: "fixture".into(),
        })
        .unwrap();
    let sends = transport.0.lock().unwrap();
    let body: Value = serde_json::from_str(&sends[0].body).unwrap();
    assert_eq!(body["reasoning_effort"], "high");
}
