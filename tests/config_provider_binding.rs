use std::sync::{Arc, Mutex};
use steerbox::http_transport::{CredentialError, CredentialResolver, ResolvedCredential};
use steerbox::runtime_config::{ConfigError, RuntimeConfig};

#[derive(Clone, Default)]
struct NeverCalledResolver {
    refs: Arc<Mutex<Vec<String>>>,
}

impl CredentialResolver for NeverCalledResolver {
    fn resolve(&self, auth_ref: &str) -> Result<ResolvedCredential, CredentialError> {
        self.refs
            .lock()
            .expect("resolver refs lock")
            .push(auth_ref.into());
        Err(CredentialError::Unavailable)
    }
}

fn config() -> RuntimeConfig {
    RuntimeConfig::from_json_str(
        r#"{
          "schema_version": 1,
          "project_ref": "binding-fixture",
          "data_root": "/temp/steerbox/binding-fixture",
          "model_profiles": [{
            "model_id": "logical-model",
            "provider": "fixture-provider",
            "endpoint": "https://fixture.example.test/v1",
            "auth_ref": "env:FIXTURE_PROVIDER_KEY",
            "capabilities": ["text"]
          }]
        }"#,
    )
    .expect("fixture config parses")
}

#[test]
fn profile_builds_provider_adapter_with_metadata_only_and_does_not_resolve_credentials() {
    let config = config();
    let resolver = NeverCalledResolver::default();
    let refs = Arc::clone(&resolver.refs);
    let adapter = config
        .build_http_provider_adapter("logical-model", resolver)
        .expect("profile builds HTTP provider adapter");

    assert_eq!(adapter.provider_id(), "fixture-provider");
    assert_eq!(
        adapter.endpoint_base_url(),
        "https://fixture.example.test/v1"
    );
    assert_eq!(adapter.model_id(), "logical-model");
    assert_eq!(adapter.auth_ref(), Some("env:FIXTURE_PROVIDER_KEY"));
    assert!(refs.lock().expect("resolver refs lock").is_empty());
}

#[test]
fn profile_lookup_rejects_unknown_model_without_constructing_a_transport() {
    let result =
        config().build_http_provider_adapter("missing-model", NeverCalledResolver::default());
    assert!(matches!(
        result,
        Err(ConfigError::ModelProfileNotFound { .. })
    ));
}

#[test]
fn profile_config_contains_only_opaque_auth_reference_not_credential_material() {
    let encoded = serde_json::to_string(&config()).expect("config serializes");
    assert!(encoded.contains("env:FIXTURE_PROVIDER_KEY"));
    for forbidden in ["fixture-token", "api_key", "password", "authorization"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn legacy_profile_without_protocol_normalizes_to_openai_compatible_chat() {
    let config = config();
    assert_eq!(
        config.model_profiles[0].endpoint_protocol,
        "openai-compatible-chat"
    );
}

#[test]
fn unsupported_protocol_is_rejected_before_http_adapter_construction() {
    let encoded = serde_json::json!({
        "schema_version": 1,
        "project_ref": "binding-fixture",
        "data_root": "/temp/steerbox/binding-fixture",
        "model_profiles": [{
            "model_id": "logical-model",
            "provider": "openai",
            "endpoint": "https://fixture.example.test/v1",
            "endpoint_protocol": "openai-responses",
            "auth_ref": "env:FIXTURE_PROVIDER_KEY",
            "capabilities": ["text"]
        }]
    });
    assert!(matches!(
        RuntimeConfig::from_json_str(&encoded.to_string()),
        Err(ConfigError::UnsupportedEndpointProtocol { protocol })
            if protocol == "openai-responses"
    ));
}
