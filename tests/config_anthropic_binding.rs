use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::http_transport::{CredentialError, CredentialResolver, ResolvedCredential};
use steerbox::provider_adapter::TransportResponse;
use steerbox::runtime_config::{ConfigError, RuntimeConfig};

#[derive(Clone, Default)]
struct CountingTransport {
    sends: Arc<AtomicUsize>,
}

#[derive(Clone, Copy)]
struct FixtureResolver;

impl CredentialResolver for FixtureResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        ResolvedCredential::bearer("fixture-anthropic-key")
    }
}

impl AnthropicTransport for CountingTransport {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Err(AnthropicTransportError::Other)
    }
}

fn config(provider: &str, capabilities: &[&str]) -> RuntimeConfig {
    RuntimeConfig::from_json_str(
        &serde_json::json!({
            "schema_version": 1,
            "project_ref": "anthropic-binding-fixture",
            "data_root": "/temp/steerbox/anthropic-binding-fixture",
            "model_profiles": [{
                "model_id": "claude-fixture",
                "provider": provider,
                "endpoint": "https://fixture.example.test/v1",
                "auth_ref": "env:ANTHROPIC_API_KEY",
                "capabilities": capabilities
            }]
        })
        .to_string(),
    )
    .expect("fixture config parses")
}

#[test]
fn builds_anthropic_adapter_from_profile_metadata_without_transport_activity() {
    let transport = CountingTransport::default();
    let sends = Arc::clone(&transport.sends);
    let adapter = config("anthropic", &["text"])
        .build_anthropic_adapter("claude-fixture", transport)
        .expect("anthropic profile builds adapter");

    assert_eq!(adapter.provider_id(), "anthropic");
    assert_eq!(adapter.base_url(), "https://fixture.example.test/v1");
    assert_eq!(adapter.auth_ref(), Some("env:ANTHROPIC_API_KEY"));
    assert_eq!(sends.load(Ordering::SeqCst), 0);
}

#[test]
fn unknown_model_fails_closed_without_constructing_a_transport_call() {
    let transport = CountingTransport::default();
    let sends = Arc::clone(&transport.sends);
    let result = config("anthropic", &["text"]).build_anthropic_adapter("missing-model", transport);

    assert!(matches!(
        result,
        Err(ConfigError::ModelProfileNotFound { .. })
    ));
    assert_eq!(sends.load(Ordering::SeqCst), 0);
}

#[test]
fn non_anthropic_provider_fails_closed() {
    let result = config("openai", &["text"])
        .build_anthropic_adapter("claude-fixture", CountingTransport::default());

    assert!(matches!(
        result,
        Err(ConfigError::UnsupportedAnthropicProvider { .. })
    ));
}

#[test]
fn missing_text_capability_fails_closed() {
    let result = config("anthropic", &["vision"])
        .build_anthropic_adapter("claude-fixture", CountingTransport::default());

    assert!(matches!(
        result,
        Err(ConfigError::UnsupportedAnthropicCapability { .. })
    ));
}

#[test]
fn builds_http_anthropic_adapter_without_resolving_credentials_eagerly() {
    let adapter = config("anthropic", &["text"])
        .build_http_anthropic_adapter("claude-fixture", FixtureResolver)
        .expect("HTTP Anthropic adapter builds");
    assert_eq!(adapter.provider_id(), "anthropic");
    assert_eq!(adapter.base_url(), "https://fixture.example.test/v1");
}

#[test]
fn runtime_config_rejects_non_opaque_auth_ref_and_authenticated_endpoint() {
    for (auth_ref, endpoint) in [
        (
            "env:password=fixture-value",
            "https://fixture.example.test/v1",
        ),
        (
            "env:ANTHROPIC_API_KEY",
            "https://user:fixture@example.test/v1",
        ),
        (
            "env:ANTHROPIC_API_KEY",
            "https://fixture.example.test/v1?token=fixture-value",
        ),
        (
            "env:ANTHROPIC_API_KEY",
            "https://fixture.example.test/v1#secret=fixture-value",
        ),
    ] {
        let config = serde_json::json!({
            "schema_version": 1,
            "project_ref": "anthropic-boundary-fixture",
            "data_root": "/temp/steerbox/anthropic-boundary-fixture",
            "model_profiles": [{
                "model_id": "claude-fixture",
                "provider": "anthropic",
                "endpoint": endpoint,
                "auth_ref": auth_ref,
                "capabilities": ["text"]
            }]
        });
        let result = RuntimeConfig::from_json_str(&config.to_string());
        assert!(result.is_err(), "unsafe provider profile must be rejected");
        if let Err(error) = result {
            assert!(!error.to_string().contains("fixture-value"));
        }
    }
}

#[test]
fn anthropic_effect_is_rejected_in_json_instead_of_silently_ignored() {
    let mut encoded = serde_json::to_value(config("anthropic", &["text"])).unwrap();
    encoded["model_profiles"][0]["effect"] = serde_json::json!({
        "selected_level":"fast", "mapping":{"fast":"low"}
    });
    let error = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidModelEffect { .. }));
}

#[test]
fn anthropic_factories_reject_mutated_effect_before_transport_activity() {
    let mut config = config("anthropic", &["text"]);
    let mut profile = serde_json::to_value(&config.model_profiles[0]).unwrap();
    profile["effect"] = serde_json::json!({
        "selected_level":"fast", "mapping":{"fast":"low"}
    });
    config.model_profiles[0] = serde_json::from_value(profile).unwrap();
    let transport = CountingTransport::default();
    let sends = Arc::clone(&transport.sends);
    let error = config
        .build_anthropic_adapter("claude-fixture", transport)
        .err()
        .expect("mutated Anthropic effect is rejected");
    assert!(matches!(error, ConfigError::InvalidModelEffect { .. }));
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    let error = config
        .build_http_anthropic_adapter("claude-fixture", FixtureResolver)
        .expect_err("HTTP factory also rejects mutated Anthropic effect");
    assert!(matches!(error, ConfigError::InvalidModelEffect { .. }));
}
