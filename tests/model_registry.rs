use serde_json::json;
use steerbox::model_registry::{
    EndpointProfile, ModelProfile, ModelRegistry, ModelRegistryError, ProviderProfile,
};
use steerbox::prompt_registry::{PromptProfile, PromptRegistry, PromptRegistryError};

fn registry_fixture() -> ModelRegistry {
    let mut registry = ModelRegistry::default();
    registry
        .register_provider(
            ProviderProfile::new("openai", "OpenAI", Some("env:OPENAI_API_KEY".into()))
                .expect("provider"),
        )
        .expect("provider registers");
    registry
        .register_endpoint(
            EndpointProfile::new(
                "openai-default",
                "openai",
                "https://api.openai.com/v1",
                "openai-compatible",
            )
            .expect("endpoint"),
        )
        .expect("endpoint registers");
    registry
        .register_model(
            ModelProfile::new(
                "gpt-5",
                "openai",
                "openai-default",
                "gpt-5",
                vec!["text".into(), "tool_calling".into()],
            )
            .expect("model"),
        )
        .expect("model registers");
    registry
}

#[test]
fn model_registry_registers_and_resolves_by_id() {
    let registry = registry_fixture();
    assert_eq!(
        registry.provider("openai").expect("provider").display_name,
        "OpenAI"
    );
    assert_eq!(
        registry
            .endpoint("openai-default")
            .expect("endpoint")
            .protocol,
        "openai-compatible"
    );
    assert_eq!(registry.model("gpt-5").expect("model").model_name, "gpt-5");
}

#[test]
fn model_registry_rejects_duplicate_ids_and_unbound_models() {
    let mut registry = registry_fixture();
    let duplicate_provider = registry
        .register_provider(
            ProviderProfile::new("openai", "OpenAI duplicate", None).expect("provider"),
        )
        .expect_err("duplicate provider must fail");
    assert_eq!(
        duplicate_provider,
        ModelRegistryError::DuplicateProvider("openai".into())
    );

    let duplicate_endpoint = registry
        .register_endpoint(
            EndpointProfile::new(
                "openai-default",
                "openai",
                "https://api.openai.com/v1",
                "openai-compatible",
            )
            .expect("endpoint"),
        )
        .expect_err("duplicate endpoint must fail");
    assert_eq!(
        duplicate_endpoint,
        ModelRegistryError::DuplicateEndpoint("openai-default".into())
    );

    let duplicate = registry
        .register_model(
            ModelProfile::new("gpt-5", "openai", "openai-default", "gpt-5", vec![]).expect("model"),
        )
        .expect_err("duplicate model must fail");
    assert_eq!(
        duplicate,
        ModelRegistryError::DuplicateModel("gpt-5".into())
    );

    let unbound = registry
        .register_model(
            ModelProfile::new("other", "missing", "openai-default", "other", vec![])
                .expect("model"),
        )
        .expect_err("unknown provider must fail");
    assert_eq!(
        unbound,
        ModelRegistryError::ProviderNotFound("missing".into())
    );
}

#[test]
fn model_registry_round_trips_without_secret_fields() {
    let registry = registry_fixture();
    let encoded = serde_json::to_value(&registry).expect("registry serializes");
    assert_eq!(
        encoded["providers"]["openai"]["auth_ref"],
        "env:OPENAI_API_KEY"
    );
    assert!(encoded["providers"]["openai"].get("api_key").is_none());
    let decoded: ModelRegistry = serde_json::from_value(encoded).expect("registry deserializes");
    assert_eq!(decoded, registry);
}

#[test]
fn prompt_profile_parses_version_and_role_task_binding() {
    let profile = PromptProfile::from_json_str(
        &json!({
            "prompt_id": "developer-loop",
            "version": 2,
            "binding": {"role": "developer", "task": "implement"},
            "template": "Inspect the repository and implement the requested change."
        })
        .to_string(),
    )
    .expect("prompt parses");
    let mut registry = PromptRegistry::default();
    registry
        .register(profile.clone())
        .expect("prompt registers");
    assert_eq!(registry.get("developer-loop", 2), Some(&profile));
    assert_eq!(
        registry.latest_for("developer-loop", "developer", "implement"),
        Some(&profile)
    );
}

#[test]
fn prompt_registry_rejects_duplicate_version_and_invalid_profile() {
    let profile = PromptProfile::new("developer-loop", 1, "developer", "implement", "prompt")
        .expect("prompt");
    let mut registry = PromptRegistry::default();
    registry
        .register(profile.clone())
        .expect("prompt registers");
    let duplicate = registry
        .register(profile)
        .expect_err("duplicate prompt must fail");
    assert_eq!(
        duplicate,
        PromptRegistryError::DuplicatePrompt {
            prompt_id: "developer-loop".into(),
            version: 1
        }
    );
    let invalid = PromptProfile::new("developer-loop", 0, "developer", "implement", "prompt")
        .expect_err("zero version must fail");
    assert_eq!(invalid, PromptRegistryError::InvalidVersion);
}

#[test]
fn prompt_registry_round_trips_as_json() {
    let mut registry = PromptRegistry::default();
    registry
        .register(
            PromptProfile::new("planner", 1, "planner", "plan", "Make a safe plan.")
                .expect("prompt"),
        )
        .expect("prompt registers");
    let encoded = serde_json::to_string(&registry).expect("registry serializes");
    let decoded: PromptRegistry = serde_json::from_str(&encoded).expect("registry deserializes");
    assert_eq!(decoded, registry);
}

#[test]
fn provider_rejects_whitespace_auth_reference() {
    let error = ProviderProfile::new("local", "Local", Some(" \n\t".into()))
        .expect_err("whitespace auth reference must fail");
    assert_eq!(error, ModelRegistryError::EmptyField { field: "auth_ref" });
}

#[test]
fn model_registration_revalidates_public_profiles_before_insertion() {
    let mut registry = registry_fixture();
    let invalid_provider = ProviderProfile {
        provider_id: "local".into(),
        display_name: " ".into(),
        auth_ref: None,
    };
    assert!(registry.register_provider(invalid_provider).is_err());
    assert!(registry.provider("local").is_none());

    let invalid_auth = ProviderProfile {
        provider_id: "local".into(),
        display_name: "Local".into(),
        auth_ref: Some("\t".into()),
    };
    assert!(registry.register_provider(invalid_auth).is_err());
    assert!(registry.provider("local").is_none());

    let invalid_endpoint = EndpointProfile {
        endpoint_id: "invalid".into(),
        provider_id: "openai".into(),
        base_url: " ".into(),
        protocol: "openai-compatible".into(),
    };
    assert!(registry.register_endpoint(invalid_endpoint).is_err());
    assert!(registry.endpoint("invalid").is_none());

    let invalid_model = ModelProfile {
        model_id: "invalid".into(),
        provider_id: "openai".into(),
        endpoint_id: "openai-default".into(),
        model_name: "\n".into(),
        capabilities: vec![],
    };
    assert!(registry.register_model(invalid_model).is_err());
    assert!(registry.model("invalid").is_none());
}

#[test]
fn model_registration_rejects_cross_provider_endpoint() {
    let mut registry = registry_fixture();
    registry
        .register_provider(ProviderProfile::new("local", "Local", None).expect("provider"))
        .expect("second provider registers");
    let error = registry
        .register_model(
            ModelProfile::new("mismatch", "local", "openai-default", "fixture", vec![])
                .expect("model"),
        )
        .expect_err("cross-provider endpoint must fail");
    assert_eq!(error, ModelRegistryError::EndpointProviderMismatch);
    assert!(registry.model("mismatch").is_none());
}

#[test]
fn model_registry_json_rejects_mismatched_map_ids() {
    for (section, id, field) in [
        ("providers", "openai", "provider_id"),
        ("endpoints", "openai-default", "endpoint_id"),
        ("models", "gpt-5", "model_id"),
    ] {
        let mut encoded = serde_json::to_value(registry_fixture()).expect("fixture serializes");
        encoded[section][id][field] = json!("different-id");
        assert!(
            serde_json::from_value::<ModelRegistry>(encoded).is_err(),
            "{section} map key must match {field}"
        );
    }
}

#[test]
fn model_registry_json_rejects_invalid_fields_and_broken_associations() {
    for (section, id, field) in [
        ("providers", "openai", "display_name"),
        ("providers", "openai", "auth_ref"),
        ("endpoints", "openai-default", "base_url"),
        ("endpoints", "openai-default", "protocol"),
        ("models", "gpt-5", "model_name"),
    ] {
        let mut encoded = serde_json::to_value(registry_fixture()).expect("fixture serializes");
        encoded[section][id][field] = json!(" \t");
        assert!(
            serde_json::from_value::<ModelRegistry>(encoded).is_err(),
            "{section}.{field} must be validated after JSON decode"
        );
    }
    for (section, id, field) in [
        ("endpoints", "openai-default", "provider_id"),
        ("models", "gpt-5", "provider_id"),
        ("models", "gpt-5", "endpoint_id"),
    ] {
        let mut encoded = serde_json::to_value(registry_fixture()).expect("fixture serializes");
        encoded[section][id][field] = json!("missing");
        assert!(
            serde_json::from_value::<ModelRegistry>(encoded).is_err(),
            "{section}.{field} must refer to a registered profile"
        );
    }
}

#[test]
fn model_registry_json_rejects_cross_provider_endpoint() {
    let mut encoded = serde_json::to_value(registry_fixture()).expect("fixture serializes");
    encoded["providers"]["local"] =
        json!({"provider_id": "local", "display_name": "Local", "auth_ref": null});
    encoded["models"]["gpt-5"]["provider_id"] = json!("local");
    assert!(serde_json::from_value::<ModelRegistry>(encoded).is_err());
}

#[test]
fn provider_rejects_credential_body_from_auth_ref() {
    for auth_ref in [
        "env:password=fixture-value",
        "env:-----BEGIN PRIVATE KEY-----",
        "env:https://user:fixture@example.test/key",
    ] {
        assert!(
            ProviderProfile::new("fixture", "Fixture", Some(auth_ref.into())).is_err(),
            "credential body must not be accepted as auth_ref: {auth_ref}"
        );
    }
}

#[test]
fn endpoint_rejects_authentication_material_in_url() {
    for base_url in [
        "https://user:fixture@example.test/v1",
        "https://example.test/v1?api_key=fixture-value",
        "https://example.test/v1#secret=fixture-value",
    ] {
        assert!(
            EndpointProfile::new("fixture-endpoint", "fixture", base_url, "https").is_err(),
            "authentication material must not be accepted in base_url: {base_url}"
        );
    }
}

fn prompt_registry_fixture() -> PromptRegistry {
    let mut registry = PromptRegistry::default();
    registry
        .register(
            PromptProfile::new("planner", 1, "planner", "plan", "Plan safely.").expect("prompt"),
        )
        .expect("prompt registers");
    registry
}

#[test]
fn prompt_registration_revalidates_public_profile() {
    let mut profile = PromptProfile::new("invalid", 1, "planner", "plan", "Plan.").expect("prompt");
    profile.binding.task = " ".into();
    let mut registry = PromptRegistry::default();
    assert!(registry.register(profile).is_err());
    assert!(registry.get("invalid", 1).is_none());
}

#[test]
fn prompt_registry_json_rejects_mismatched_map_id_and_version() {
    let mut encoded = serde_json::to_value(prompt_registry_fixture()).expect("fixture serializes");
    encoded["profiles"]["planner"]["1"]["prompt_id"] = json!("different");
    assert!(serde_json::from_value::<PromptRegistry>(encoded).is_err());

    let mut encoded = serde_json::to_value(prompt_registry_fixture()).expect("fixture serializes");
    encoded["profiles"]["planner"]["1"]["version"] = json!(2);
    assert!(serde_json::from_value::<PromptRegistry>(encoded).is_err());
}

#[test]
fn prompt_registry_json_rejects_invalid_profiles() {
    for field in ["template", "prompt_id"] {
        let mut encoded =
            serde_json::to_value(prompt_registry_fixture()).expect("fixture serializes");
        encoded["profiles"]["planner"]["1"][field] = json!(" ");
        assert!(serde_json::from_value::<PromptRegistry>(encoded).is_err());
    }
    for field in ["role", "task"] {
        let mut encoded =
            serde_json::to_value(prompt_registry_fixture()).expect("fixture serializes");
        encoded["profiles"]["planner"]["1"]["binding"][field] = json!(" ");
        assert!(serde_json::from_value::<PromptRegistry>(encoded).is_err());
    }
    let mut encoded = serde_json::to_value(prompt_registry_fixture()).expect("fixture serializes");
    encoded["profiles"]["planner"]["1"]["version"] = json!(0);
    assert!(serde_json::from_value::<PromptRegistry>(encoded).is_err());
}

#[test]
fn prompt_latest_selects_highest_version_with_matching_binding() {
    let mut registry = prompt_registry_fixture();
    let matched =
        PromptProfile::new("planner", 2, "planner", "plan", "Plan version two.").expect("prompt");
    registry
        .register(matched.clone())
        .expect("version registers");
    registry
        .register(
            PromptProfile::new("planner", 3, "reviewer", "review", "Review.").expect("prompt"),
        )
        .expect("unrelated binding registers");
    assert_eq!(
        registry.latest_for("planner", "planner", "plan"),
        Some(&matched)
    );
    assert!(registry
        .latest_for("planner", "planner", "implement")
        .is_none());
}
