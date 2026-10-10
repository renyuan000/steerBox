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

#[test]
fn valid_contracts_pass_semantic_validation_without_changing_serialization() {
    let decision = route_decision();
    let record = model_call_record();
    let route_json = serde_json::to_value(&decision).expect("route serializes");
    let record_json = serde_json::to_value(&record).expect("record serializes");

    decision.validate().expect("valid route is accepted");
    record.validate().expect("valid record is accepted");

    assert_eq!(serde_json::to_value(decision).unwrap(), route_json);
    assert_eq!(serde_json::to_value(record).unwrap(), record_json);
}

#[test]
fn route_required_identity_and_binding_fields_reject_whitespace() {
    for field in [
        "route_decision_id",
        "task_id",
        "task_type",
        "agent_role",
        "selected_provider",
        "selected_model",
        "selected_endpoint",
        "prompt_profile_id",
        "prompt_profile_version",
    ] {
        let mut value = serde_json::to_value(route_decision()).unwrap();
        value[field] = serde_json::json!(" \t\n ");
        let decision: RouteDecision = serde_json::from_value(value).unwrap();
        let error = decision.validate().expect_err("blank identity must fail");
        assert!(
            error.to_string().contains(field),
            "field must be identified"
        );
    }
}

#[test]
fn model_call_required_identity_and_reference_fields_reject_whitespace() {
    for field in [
        "model_call_id",
        "task_id",
        "task_run_id",
        "agent_role",
        "provider_id",
        "endpoint_id",
        "model_id",
        "route_decision_id",
        "resolved_prompt_pack_id",
        "input_context_hash",
        "output_artifact_ref",
    ] {
        let mut value = serde_json::to_value(model_call_record()).unwrap();
        value[field] = serde_json::json!(" \t\n ");
        let record: ModelCallRecord = serde_json::from_value(value).unwrap();
        let error = record.validate().expect_err("blank reference must fail");
        assert!(
            error.to_string().contains(field),
            "field must be identified"
        );
    }
}

#[test]
fn route_selected_model_must_be_a_candidate_without_echoing_external_values() {
    let mut decision = route_decision();
    decision.selected_model = "fake-external-model-do-not-echo".into();
    let error = decision
        .validate()
        .expect_err("unknown selection must fail");

    assert!(error.to_string().contains("selected_model"));
    assert!(!error
        .to_string()
        .contains("fake-external-model-do-not-echo"));
}

#[test]
fn fallback_chain_rejects_empty_selected_and_duplicate_entries() {
    for chain in [
        vec![" \t ".into()],
        vec!["fixture-small".into()],
        vec!["fixture-local".into(), "fixture-local".into()],
    ] {
        let mut decision = route_decision();
        decision.fallback_chain = chain;
        let error = decision.validate().expect_err("invalid fallback must fail");
        assert!(error.to_string().contains("fallback_chain"));
    }
}

#[test]
fn fallback_chain_allows_ordered_models_outside_primary_candidates() {
    let mut decision = route_decision();
    decision.fallback_chain = vec!["fixture-extra-first".into(), "fixture-extra-second".into()];
    decision
        .validate()
        .expect("fallback models need not be primary candidates");
    assert_eq!(
        decision.fallback_chain,
        ["fixture-extra-first", "fixture-extra-second"]
    );
}

#[test]
fn route_and_model_call_reject_negative_or_nonfinite_costs_before_serialization() {
    for cost in [-0.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut decision = route_decision();
        decision.cost_estimate = Some(cost);
        let error = decision
            .validate()
            .expect_err("invalid route cost must fail");
        assert!(error.to_string().contains("cost_estimate"));

        let mut record = model_call_record();
        record.estimated_cost = Some(cost);
        let error = record.validate().expect_err("invalid call cost must fail");
        assert!(error.to_string().contains("estimated_cost"));
    }
}

#[test]
fn route_and_model_call_accept_absent_zero_and_positive_costs() {
    for cost in [None, Some(0.0), Some(0.001)] {
        let mut decision = route_decision();
        decision.cost_estimate = cost;
        decision.validate().expect("valid route cost is accepted");

        let mut record = model_call_record();
        record.estimated_cost = cost;
        record.validate().expect("valid call cost is accepted");
    }
}
