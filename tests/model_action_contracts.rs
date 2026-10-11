use serde_json::json;
use steerbox::model_contracts::{
    ActionPathScope, ModelActionEnvelope, ModelContractError, VerificationContract,
    MODEL_ACTION_SCHEMA_VERSION,
};

fn valid_action() -> ModelActionEnvelope {
    ModelActionEnvelope {
        schema_version: MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-1".into(),
        tool_id: "markdown.write".into(),
        parameters: json!({
            "path": "src/lib.rs",
            "replacement": {"old": "before", "new": "after"}
        }),
        path_scope: ActionPathScope {
            root: "/workspace/project".into(),
            allowed_paths: vec!["src/lib.rs".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown.link_check".into(),
            parameters: json!({"path": "src/lib.rs"}),
        },
    }
}

#[test]
fn action_envelope_round_trips_as_a_versioned_structured_contract() {
    let action = valid_action();
    action.validate().expect("valid action contract");
    let encoded = action.to_json().expect("action serializes");
    let decoded = ModelActionEnvelope::parse_json(&encoded).expect("action parses");

    assert_eq!(decoded, action);
    assert!(encoded.contains("schema_version"));
    assert!(encoded.contains("tool_id"));
    assert!(encoded.contains("path_scope"));
    assert!(encoded.contains("verification"));
    assert!(!encoded.contains("decision"));
    assert!(!encoded.contains("summary"));
}

#[test]
fn action_envelope_rejects_unknown_fields_and_unsupported_versions() {
    let mut value = serde_json::to_value(valid_action()).expect("action value");
    value["unexpected"] = json!(true);
    assert!(ModelActionEnvelope::parse_json(&value.to_string()).is_err());

    let mut value = serde_json::to_value(valid_action()).expect("action value");
    value["schema_version"] = json!(MODEL_ACTION_SCHEMA_VERSION + 1);
    assert!(matches!(
        ModelActionEnvelope::parse_json(&value.to_string()),
        Err(ModelContractError::InvalidField {
            field: "schema_version",
            ..
        })
    ));
}

#[test]
fn action_envelope_rejects_free_shell_strings() {
    let mut action = valid_action();
    action.parameters = json!({"command": "cargo test --all"});
    assert!(matches!(
        action.validate(),
        Err(ModelContractError::InvalidField {
            field: "parameters",
            ..
        })
    ));
}

#[test]
fn action_envelope_rejects_missing_required_fields() {
    let mut value = serde_json::to_value(valid_action()).expect("action value");
    value
        .as_object_mut()
        .expect("object")
        .remove("verification");
    assert!(ModelActionEnvelope::parse_json(&value.to_string()).is_err());
}

#[test]
fn action_envelope_rejects_empty_or_out_of_bounds_tool_and_paths() {
    let mut action = valid_action();
    action.tool_id.clear();
    assert!(action.validate().is_err());

    let mut action = valid_action();
    action.parameters = json!({"path": "../outside.rs"});
    assert!(action.validate().is_err());

    let mut action = valid_action();
    action.path_scope.allowed_paths.clear();
    assert!(action.validate().is_err());
}

#[test]
fn action_envelope_rejects_incomplete_verification_contract() {
    let mut action = valid_action();
    action.verification.verifier_tool_id.clear();
    assert!(action.validate().is_err());

    let mut action = valid_action();
    action.verification.parameters = json!("cargo test");
    assert!(action.validate().is_err());
}
