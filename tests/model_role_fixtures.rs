use serde::Deserialize;
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::model_router::{
    ModelRouteProfile, ModelRouter, ModelRouterError, PrivacyRequirement, RiskLevel, RouteRequest,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureModel {
    capabilities: Vec<String>,
    route: ModelRouteProfile,
}

fn configured_fixture() -> (ModelRegistry, Vec<ModelRouteProfile>) {
    // Fictional policy data, not measured prices/latencies or live credentials.
    let models: Vec<FixtureModel> =
        serde_json::from_str(include_str!("fixtures/model_roles.json")).expect("fixture parses");
    let mut registry = ModelRegistry::default();
    registry
        .register_provider(ProviderProfile::new("fixture", "Role fixture", None).unwrap())
        .unwrap();
    registry
        .register_endpoint(
            EndpointProfile::new("fixture-local", "fixture", "local://roles", "fixture").unwrap(),
        )
        .unwrap();
    let mut profiles = Vec::new();
    for model in models {
        registry
            .register_model(
                ModelProfile::new(
                    &model.route.model_id,
                    "fixture",
                    "fixture-local",
                    &model.route.model_id,
                    model.capabilities,
                )
                .unwrap(),
            )
            .unwrap();
        profiles.push(model.route);
    }
    (registry, profiles)
}

fn request() -> RouteRequest {
    RouteRequest {
        task_id: "role-fixture-task".into(),
        task_type: "code_change".into(),
        agent_role: "implementer".into(),
        risk_level: RiskLevel::Low,
        required_capabilities: vec!["text".into()],
        privacy_requirement: PrivacyRequirement::Internal,
        budget_limit: Some(0.05),
        latency_target_ms: Some(150),
        prompt_profile_id: "fixture-prompt".into(),
        prompt_profile_version: "1".into(),
    }
}

#[test]
fn configured_roles_keep_budget_latency_and_fallback_constraints() {
    let (registry, profiles) = configured_fixture();
    assert_eq!(profiles.len(), 5);
    let router = ModelRouter::new(&registry, profiles).unwrap();
    let decision = router.route(&request()).unwrap();
    assert_eq!(decision.selected_model, "cheap-fast");
    assert_eq!(decision.candidate_models, vec!["cheap-fast"]);
    assert!(decision.fallback_chain.is_empty());
    assert!(!decision.approval_required);

    let mut high_risk = request();
    high_risk.risk_level = RiskLevel::High;
    high_risk.budget_limit = Some(1.0);
    high_risk.latency_target_ms = Some(2_000);
    let decision = router.route(&high_risk).unwrap();
    assert_eq!(decision.selected_model, "strong-slow");
    assert!(decision.approval_required);

    high_risk.budget_limit = Some(0.001);
    assert!(matches!(
        router.route(&high_risk),
        Err(ModelRouterError::NoCompatibleModels { .. })
    ));
}

#[test]
fn configured_specialist_and_evaluator_require_matching_role_and_capability() {
    let (registry, profiles) = configured_fixture();
    let router = ModelRouter::new(&registry, profiles).unwrap();
    for (role, task, capability, expected) in [
        ("researcher", "code_search", "code_search", "specialized"),
        ("reviewer", "verification", "evaluation", "evaluator"),
    ] {
        let mut request = request();
        request.agent_role = role.into();
        request.task_type = task.into();
        request.required_capabilities = vec![capability.into()];
        request.budget_limit = Some(1.0);
        request.latency_target_ms = Some(2_000);
        assert_eq!(router.route(&request).unwrap().selected_model, expected);
        let mut wrong_role = request.clone();
        wrong_role.agent_role = "unconfigured-role".into();
        assert!(matches!(
            router.route(&wrong_role),
            Err(ModelRouterError::NoCompatibleModels { .. })
        ));
        request.required_capabilities.push("not-supported".into());
        assert!(matches!(
            router.route(&request),
            Err(ModelRouterError::NoCompatibleModels { .. })
        ));
    }
}

#[test]
fn configured_local_route_does_not_grant_restricted_data_authority() {
    let (registry, profiles) = configured_fixture();
    let router = ModelRouter::new(&registry, profiles).unwrap();
    let mut request = request();
    request.privacy_requirement = PrivacyRequirement::Restricted;
    request.budget_limit = Some(1.0);
    request.latency_target_ms = Some(2_000);
    let decision = router.route(&request).unwrap();
    assert_eq!(decision.selected_model, "local-private");
    assert_eq!(decision.candidate_models, vec!["local-private"]);
    assert!(decision.approval_required);
    assert!(decision.fallback_chain.is_empty());
}
