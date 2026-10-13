#[path = "../src/model_contracts.rs"]
#[allow(dead_code)]
mod model_contracts;
#[path = "../src/model_registry.rs"]
#[allow(dead_code)]
mod model_registry;
#[path = "../src/model_router.rs"]
mod model_router;

use model_contracts::RouteDecision;
use model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use model_router::{
    next_fallback_model, ModelRouteProfile, ModelRouter, PrivacyRequirement, RiskLevel,
    RouteRequest,
};

fn registry_fixture() -> ModelRegistry {
    let mut registry = ModelRegistry::default();
    registry
        .register_provider(ProviderProfile::new("fixture", "Fixture", None).unwrap())
        .unwrap();
    registry
        .register_endpoint(
            EndpointProfile::new("fixture-endpoint", "fixture", "local://fixture", "fixture")
                .unwrap(),
        )
        .unwrap();
    for (id, name) in [
        ("cheap-fast", "cheap-fast"),
        ("strong-safe", "strong-safe"),
        ("backup", "backup"),
    ] {
        registry
            .register_model(
                ModelProfile::new(
                    id,
                    "fixture",
                    "fixture-endpoint",
                    name,
                    vec!["text".into(), "tool_calling".into()],
                )
                .unwrap(),
            )
            .unwrap();
    }
    registry
}

fn request(risk_level: RiskLevel) -> RouteRequest {
    RouteRequest {
        task_id: "task-1".into(),
        task_type: "code_change".into(),
        agent_role: "implementer".into(),
        risk_level,
        required_capabilities: vec!["text".into()],
        privacy_requirement: PrivacyRequirement::Internal,
        budget_limit: None,
        latency_target_ms: None,
        prompt_profile_id: "coding-default".into(),
        prompt_profile_version: "1".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn profile(
    model_id: &str,
    max_risk_level: RiskLevel,
    privacy_clearance: PrivacyRequirement,
    estimated_cost: f64,
    estimated_latency_ms: u64,
    quality_score: u8,
    reliability_score: u8,
    preferred_for_high_risk: bool,
) -> ModelRouteProfile {
    ModelRouteProfile {
        model_id: model_id.into(),
        supported_task_types: vec!["code_change".into()],
        supported_agent_roles: vec!["implementer".into()],
        max_risk_level,
        privacy_clearance,
        estimated_cost,
        estimated_latency_ms,
        quality_score,
        reliability_score,
        preferred_for_high_risk,
        fallback_priority: 0,
    }
}

fn router_fixture() -> ModelRouter<'static> {
    let registry = Box::leak(Box::new(registry_fixture()));
    ModelRouter::new(
        registry,
        vec![
            profile(
                "cheap-fast",
                RiskLevel::Critical,
                PrivacyRequirement::Restricted,
                0.01,
                80,
                55,
                70,
                false,
            ),
            profile(
                "strong-safe",
                RiskLevel::Critical,
                PrivacyRequirement::Restricted,
                0.50,
                400,
                95,
                95,
                true,
            ),
            profile(
                "backup",
                RiskLevel::Critical,
                PrivacyRequirement::Restricted,
                0.10,
                180,
                70,
                80,
                false,
            ),
        ],
    )
    .unwrap()
}

#[test]
fn low_risk_prefers_cheap_and_fast_model() {
    let router = router_fixture();
    let decision = router.route(&request(RiskLevel::Low)).unwrap();

    assert_eq!(decision.selected_model, "cheap-fast");
    assert_eq!(decision.selected_provider, "fixture");
    assert_eq!(decision.selected_endpoint, "fixture-endpoint");
    assert!(decision.fallback_chain.contains(&"strong-safe".into()));
    decision.validate_secret_free().unwrap();
}

#[test]
fn high_risk_prefers_strong_model_even_when_it_costs_more() {
    let router = router_fixture();
    let decision = router.route(&request(RiskLevel::High)).unwrap();

    assert_eq!(decision.selected_model, "strong-safe");
    assert!(decision.approval_required);
    assert!(decision.selection_reason.contains("high-risk"));
    assert_eq!(decision.cost_estimate, Some(0.50));
}

#[test]
fn capability_mismatch_returns_explicit_no_compatible_error() {
    let router = router_fixture();
    let mut request = request(RiskLevel::Low);
    request.required_capabilities = vec!["vision".into()];

    let error = router
        .route(&request)
        .expect_err("vision has no compatible model");
    assert!(error.to_string().contains("no compatible model"));
    assert!(error.to_string().contains("vision"));
}

#[test]
fn route_decision_contains_ordered_fallback_chain() {
    let router = router_fixture();
    let decision: RouteDecision = router.route(&request(RiskLevel::High)).unwrap();

    assert_eq!(decision.selected_model, "strong-safe");
    assert_eq!(decision.fallback_chain, ["backup", "cheap-fast"]);
    assert_eq!(
        decision.candidate_models,
        ["strong-safe", "backup", "cheap-fast"]
    );
    assert!(decision
        .rejected_candidates
        .iter()
        .all(|candidate| !candidate.starts_with("strong-safe:")));
}

#[test]
fn next_fallback_model_preserves_route_order_and_skips_primary() {
    let router = router_fixture();
    let mut decision = router.route(&request(RiskLevel::High)).unwrap();
    decision.fallback_chain = vec![
        decision.selected_model.clone(),
        "backup".into(),
        "backup".into(),
        "cheap-fast".into(),
    ];

    assert_eq!(next_fallback_model(&decision, &[]), Some("backup"));
    assert_eq!(
        next_fallback_model(
            &decision,
            &["strong-safe".into(), "backup".into(), "backup".into()],
        ),
        Some("cheap-fast")
    );
}

#[test]
fn next_fallback_model_returns_none_when_empty_or_exhausted() {
    let router = router_fixture();
    let mut decision = router.route(&request(RiskLevel::High)).unwrap();
    decision.fallback_chain.clear();
    assert_eq!(next_fallback_model(&decision, &[]), None);

    let decision = router.route(&request(RiskLevel::High)).unwrap();
    assert_eq!(
        next_fallback_model(
            &decision,
            &["strong-safe".into(), "backup".into(), "cheap-fast".into()],
        ),
        None
    );
}

#[test]
fn next_fallback_model_skips_primary_even_if_it_was_not_attempted() {
    let router = router_fixture();
    let mut decision = router.route(&request(RiskLevel::High)).unwrap();
    decision.fallback_chain = vec![decision.selected_model.clone(), "backup".into()];

    assert_eq!(next_fallback_model(&decision, &[]), Some("backup"));
}

#[test]
fn next_fallback_model_skips_entries_outside_the_candidate_set() {
    let router = router_fixture();
    let mut decision = router.route(&request(RiskLevel::High)).unwrap();
    decision.fallback_chain = vec!["unregistered-model".into(), "backup".into()];

    assert_eq!(next_fallback_model(&decision, &[]), Some("backup"));
}

#[test]
fn risk_and_privacy_bounds_remove_incompatible_candidates() {
    let registry = Box::leak(Box::new(registry_fixture()));
    let router = ModelRouter::new(
        registry,
        vec![profile(
            "cheap-fast",
            RiskLevel::Low,
            PrivacyRequirement::Internal,
            0.01,
            80,
            55,
            70,
            false,
        )],
    )
    .unwrap();
    let mut request = request(RiskLevel::High);
    request.privacy_requirement = PrivacyRequirement::Restricted;

    let error = router
        .route(&request)
        .expect_err("risk and privacy exceed profile");
    assert!(error.to_string().contains("max_risk_level") || error.to_string().contains("privacy"));
}

#[test]
fn rejects_non_finite_cost_and_blank_profile_fields() {
    let registry = Box::leak(Box::new(registry_fixture()));
    let mut invalid_cost = profile(
        "cheap-fast",
        RiskLevel::Low,
        PrivacyRequirement::Internal,
        f64::NAN,
        80,
        55,
        70,
        false,
    );
    let error = ModelRouter::new(registry, vec![invalid_cost.clone()])
        .expect_err("NaN must not enter routing policy");
    assert_eq!(error, model_router::ModelRouterError::InvalidCost);

    invalid_cost.estimated_cost = f64::INFINITY;
    let error = ModelRouter::new(registry, vec![invalid_cost])
        .expect_err("infinite cost must not enter routing policy");
    assert_eq!(error, model_router::ModelRouterError::InvalidCost);

    let mut blank_id = profile(
        "cheap-fast",
        RiskLevel::Low,
        PrivacyRequirement::Internal,
        0.01,
        80,
        55,
        70,
        false,
    );
    blank_id.model_id = " ".into();
    let error =
        ModelRouter::new(registry, vec![blank_id]).expect_err("blank model id must be rejected");
    assert_eq!(
        error,
        model_router::ModelRouterError::EmptyProfileField { field: "model_id" }
    );
}

#[test]
fn empty_capability_and_wildcard_profile_are_valid_and_secret_free() {
    let registry = Box::leak(Box::new(registry_fixture()));
    let mut wildcard = profile(
        "cheap-fast",
        RiskLevel::Low,
        PrivacyRequirement::Internal,
        0.01,
        80,
        55,
        70,
        false,
    );
    wildcard.supported_task_types.clear();
    wildcard.supported_agent_roles.clear();
    let router = ModelRouter::new(registry, vec![wildcard]).unwrap();
    let mut request = request(RiskLevel::Low);
    request.required_capabilities.clear();
    request.task_type = "unlisted-task".into();
    request.agent_role = "unlisted-role".into();

    let decision = router.route(&request).unwrap();
    let encoded = serde_json::to_string(&decision).unwrap();
    for forbidden in [
        "api_key",
        "access_token",
        "authorization",
        "client_secret",
        "password",
        "private_key",
        "refresh_token",
        "secret",
        "session_token",
        "ssh_key",
        "token",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "secret field leaked into RouteDecision JSON: {forbidden}"
        );
    }
    decision.validate_secret_free().unwrap();
}

#[test]
fn explain_returns_deterministic_secret_free_route_debug_data() {
    let registry = Box::leak(Box::new(registry_fixture()));
    let router = ModelRouter::new(
        registry,
        vec![
            profile(
                "cheap-fast",
                RiskLevel::Critical,
                PrivacyRequirement::Restricted,
                0.01,
                80,
                55,
                70,
                false,
            ),
            profile(
                "strong-safe",
                RiskLevel::Critical,
                PrivacyRequirement::Restricted,
                0.50,
                400,
                95,
                95,
                true,
            ),
        ],
    )
    .unwrap();
    let explanation = router.explain(&request(RiskLevel::Low)).unwrap();
    assert_eq!(explanation.selected_model, "cheap-fast");
    assert_eq!(
        explanation.candidate_models,
        vec!["cheap-fast", "strong-safe"]
    );
    assert!(explanation
        .rejected_candidates
        .iter()
        .any(|candidate| candidate == "backup:missing_route_profile"));
    assert_eq!(explanation.fallback_chain, vec!["strong-safe"]);
    assert_eq!(explanation.estimated_cost, Some(0.01));
    assert_eq!(explanation.estimated_latency_ms, Some(80));
    assert!(!explanation.approval_required);
    assert!(explanation.selection_reason.contains("low-risk"));
    let encoded = serde_json::to_string(&explanation).unwrap();
    assert!(!encoded.contains("auth"));
    assert!(!encoded.contains("token"));
}
