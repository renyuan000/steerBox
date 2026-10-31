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

fn encoded_with_effect(effect: serde_json::Value) -> serde_json::Value {
    let mut encoded = serde_json::to_value(config()).unwrap();
    encoded["model_profiles"][0]["effect"] = effect;
    encoded
}

#[test]
fn optional_effect_defaults_to_absent_without_changing_schema_version() {
    let encoded = serde_json::to_value(config()).unwrap();
    assert_eq!(encoded["schema_version"], 1);
    assert!(encoded["model_profiles"][0].get("effect").is_none());
    assert_eq!(
        RuntimeConfig::from_json_str(&encoded.to_string()).unwrap(),
        config()
    );
}

#[test]
fn declared_sparse_effect_round_trips_and_builds_without_resolving_credentials() {
    let effect = serde_json::json!({
        "selected_level": "x_high",
        "mapping": {"fast": "low", "x_high": "xhigh"}
    });
    let config = RuntimeConfig::from_json_str(&encoded_with_effect(effect.clone()).to_string())
        .expect("optional typed effect parses");
    let resolver = NeverCalledResolver::default();
    let refs = Arc::clone(&resolver.refs);
    let adapter = config
        .build_http_provider_adapter("logical-model", resolver)
        .expect("declared effect builds");
    assert_eq!(adapter.model_id(), "logical-model");
    assert!(refs.lock().unwrap().is_empty());
    assert_eq!(
        serde_json::to_value(&config).unwrap()["model_profiles"][0]["effect"],
        effect
    );
}

#[test]
fn invalid_effect_declarations_fail_closed_with_effect_errors() {
    for effect in [
        serde_json::json!({"selected_level":"fast", "mapping":{}}),
        serde_json::json!({"selected_level":"strong", "mapping":{"fast":"low"}}),
        serde_json::json!({"selected_level":"fast", "mapping":{"fast":"high","strong":"low"}}),
        serde_json::json!({"selected_level":"fast", "mapping":{"fast":"low","strong":"low"}}),
    ] {
        let error = RuntimeConfig::from_json_str(&encoded_with_effect(effect).to_string())
            .expect_err("invalid effect is rejected");
        assert!(
            matches!(error, ConfigError::InvalidModelEffect { .. }),
            "{error:?}"
        );
    }
}

#[test]
fn effect_keeps_nested_unknown_fields_and_enum_values_fail_closed() {
    let encoded = encoded_with_effect(serde_json::json!({
        "selected_level":"fast", "mapping":{"fast":"low"}, "automatic":true
    }));
    assert!(matches!(
        RuntimeConfig::from_json_str(&encoded.to_string()),
        Err(ConfigError::UnknownField { field }) if field == "automatic"
    ));
    for effect in [
        serde_json::json!({"selected_level":"xhigh", "mapping":{"fast":"low"}}),
        serde_json::json!({"selected_level":"fast", "mapping":{"turbo":"low"}}),
        serde_json::json!({"selected_level":"fast", "mapping":{"fast":"turbo"}}),
        serde_json::json!({"mapping":{"fast":"low"}}),
        serde_json::json!({"selected_level":"fast"}),
    ] {
        assert!(matches!(
            RuntimeConfig::from_json_str(&encoded_with_effect(effect).to_string()),
            Err(ConfigError::InvalidJson { .. })
        ));
    }
}

#[test]
fn factory_revalidates_mutated_profile_effect_without_resolving_credentials() {
    let encoded = encoded_with_effect(serde_json::json!({
        "selected_level":"strong", "mapping":{"fast":"low"}
    }));
    let mut config = config();
    // Profile deserialization alone does not run RuntimeConfig's validation.
    config.model_profiles[0] =
        serde_json::from_value(encoded["model_profiles"][0].clone()).unwrap();
    let resolver = NeverCalledResolver::default();
    let refs = Arc::clone(&resolver.refs);
    let error = config
        .build_http_provider_adapter("logical-model", resolver)
        .err()
        .expect("mutated effect is rejected");
    assert!(matches!(error, ConfigError::InvalidModelEffect { .. }));
    assert!(refs.lock().unwrap().is_empty());
}

#[test]
fn duplicate_effect_mapping_keys_are_rejected_in_raw_json() {
    for encoded in [
        r#"{"schema_version":1,"project_ref":"binding-fixture","data_root":"/temp/steerbox/binding-fixture",
            "model_profiles":[{"model_id":"logical-model","provider":"fixture-provider",
            "endpoint":"https://fixture.example.test/v1",
            "effect":{"selected_level":"fast","mapping":{"fast":"low","fast":"medium"}}}]}"#,
        r#"{"schema_version":1,"project_ref":"binding-fixture","data_root":"/temp/steerbox/binding-fixture",
            "model_profiles":[{"model_id":"logical-model","provider":"fixture-provider",
            "endpoint":"https://fixture.example.test/v1",
            "effect":{"selected_level":"fast","mapping":{"fast":"low","fast":"low"}}}]}"#,
    ] {
        assert!(matches!(
            RuntimeConfig::from_json_str(encoded),
            Err(ConfigError::InvalidJson { message }) if message.contains("duplicate effect mapping level")
        ));
    }
}
