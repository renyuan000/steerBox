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
