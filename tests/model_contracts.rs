use steerbox::model_contracts::{ModelCallRecord, RouteDecision};

fn route_decision() -> RouteDecision {
    RouteDecision {
        route_decision_id: "route-1".into(),
        task_id: "task-1".into(),
        task_type: "code_change".into(),
        agent_role: "implementer".into(),
        candidate_models: vec!["fixture-small".into(), "fixture-strong".into()],
        selected_provider: "fixture".into(),
        selected_model: "fixture-small".into(),
        selected_endpoint: "local://fixture".into(),
        selection_reason: "local secret-free fixture is sufficient".into(),
        rejected_candidates: vec!["fixture-strong:over_budget".into()],
        cost_estimate: Some(0.0),
        latency_estimate_ms: Some(10),
        risk_level: "low".into(),
        privacy_requirement: "internal".into(),
        fallback_chain: vec!["fixture-strong".into(), "fixture-local".into()],
        prompt_profile_id: "coding-default".into(),
        prompt_profile_version: "1".into(),
        approval_required: false,
        created_at: "2026-10-10T00:00:00Z".into(),
    }
}

fn model_call_record() -> ModelCallRecord {
    ModelCallRecord {
        model_call_id: "call-1".into(),
        task_id: "task-1".into(),
        task_run_id: "task-run-1".into(),
        agent_role: "implementer".into(),
        provider_id: "fixture".into(),
        endpoint_id: "local://fixture".into(),
        model_id: "fixture-small".into(),
        route_decision_id: "route-1".into(),
        resolved_prompt_pack_id: "prompt-pack-1".into(),
        input_context_hash: "sha256:input".into(),
        output_artifact_ref: "trace://call-1/output".into(),
        latency_ms: Some(10),
        input_tokens: Some(20),
        output_tokens: Some(30),
        estimated_cost: Some(0.0),
        streaming: true,
        finish_reason: "stop".into(),
        tool_call_count: 0,
        verification_status: "not_run".into(),
        error_class: None,
        started_at: "2026-10-10T00:00:00Z".into(),
        completed_at: Some("2026-10-10T00:00:00Z".into()),
        created_at: "2026-10-10T00:00:00Z".into(),
    }
}

#[test]
fn route_decision_round_trips_with_fallback_and_prompt_metadata() {
    let decision = route_decision();
    let encoded = serde_json::to_string(&decision).expect("route decision serializes");
    let decoded: RouteDecision = serde_json::from_str(&encoded).expect("route decision parses");

    assert_eq!(decoded, decision);
    assert_eq!(decoded.fallback_chain, ["fixture-strong", "fixture-local"]);
    assert_eq!(decoded.prompt_profile_version, "1");
}

#[test]
fn model_call_record_round_trips_with_audit_timestamps_and_verification() {
    let record = model_call_record();
    let encoded = serde_json::to_string(&record).expect("model call record serializes");
    let decoded: ModelCallRecord =
        serde_json::from_str(&encoded).expect("model call record parses");

    assert_eq!(decoded, record);
    assert_eq!(decoded.provider_id, "fixture");
    assert_eq!(decoded.endpoint_id, "local://fixture");
    assert_eq!(decoded.verification_status, "not_run");
    assert!(decoded.streaming);
    assert!(decoded.completed_at.is_some());
}

#[test]
fn contracts_do_not_serialize_authentication_material_or_accept_auth_fields() {
    let decision = route_decision();
    let record = model_call_record();
    decision
        .validate_secret_free()
        .expect("route decision is secret-free");
    record
        .validate_secret_free()
        .expect("model call record is secret-free");
    let decision_json = serde_json::to_value(&decision).expect("route decision becomes JSON");
    let record_json = serde_json::to_value(&record).expect("model call record becomes JSON");

    for encoded in [decision_json.to_string(), record_json.to_string()] {
        for forbidden in [
            "api_key",
            "access_token",
            "authorization",
            "client_secret",
            "password",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "forbidden field leaked: {forbidden}"
            );
        }
    }

    let mut malicious = serde_json::to_value(decision).expect("decision becomes JSON");
    malicious["api_key"] = serde_json::Value::String("fixture-secret-must-not-parse".into());
    assert!(serde_json::from_value::<RouteDecision>(malicious).is_err());
}
