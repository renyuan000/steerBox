use steerbox::domain::RiskLevel;
use steerbox::execution_strategy::ExecutionStrategy;
use steerbox::strategy_selector::{StrategyDecision, StrategySelectionError, StrategySelector};
use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};

fn features() -> TaskFeatures {
    TaskFeatures {
        estimated_steps: 1,
        uncertainty_level: 0.0,
        interdependency_count: 0,
        risk_level: RiskLevel::Low,
        reversible: true,
        blast_radius: BlastRadius::Local,
        domain: ProfessionalDomain::SoftwareDevelopment,
        requires_human_approval: false,
        max_budget_multiplier: 4.0,
    }
}

#[test]
fn low_risk_single_step_selects_simple_and_preserves_inputs() {
    let input = features();
    let original = input.clone();
    let decision = StrategySelector::default().select(&input).unwrap();
    assert_eq!(decision.strategy, ExecutionStrategy::Simple);
    assert_eq!(decision.budget_multiplier, 1.0);
    assert!(!decision.approval_required);
    assert!(!decision.selection_reason.is_empty());
    decision.validate(&input).unwrap();
    assert_eq!(input, original);
}

#[test]
fn documented_complexity_thresholds_select_all_five_core_strategies() {
    for (steps, uncertainty, dependencies, expected) in [
        (15, 0.0, 0, ExecutionStrategy::Simple),
        (16, 0.0, 0, ExecutionStrategy::Guided),
        (20, 0.5, 0, ExecutionStrategy::Guided),
        (20, 0.51, 0, ExecutionStrategy::Collaborative),
        (20, 1.0, 0, ExecutionStrategy::Collaborative),
        (20, 0.9, 5, ExecutionStrategy::Research),
        (20, 0.7, 5, ExecutionStrategy::Collaborative),
    ] {
        let mut input = features();
        input.estimated_steps = steps;
        input.uncertainty_level = uncertainty;
        input.interdependency_count = dependencies;
        let decision = StrategySelector::default().select(&input).unwrap();
        assert_eq!(decision.strategy, expected);
        decision.validate(&input).unwrap();
    }

    let mut input = features();
    input.risk_level = RiskLevel::Critical;
    assert_eq!(
        StrategySelector::default().select(&input).unwrap().strategy,
        ExecutionStrategy::HighStakes
    );
}

#[test]
fn complexity_formula_caps_large_steps_and_dependencies() {
    let mut input = features();
    input.estimated_steps = u32::MAX;
    input.interdependency_count = u32::MAX;
    input.uncertainty_level = 1.0;
    let decision = StrategySelector::default().select(&input).unwrap();
    assert_eq!(decision.complexity_score, 1.0);
    assert_eq!(decision.strategy, ExecutionStrategy::Research);

    input.estimated_steps = 10;
    input.interdependency_count = 2;
    input.uncertainty_level = 0.25;
    let decision = StrategySelector::default().select(&input).unwrap();
    assert!((decision.complexity_score - 0.38).abs() < 0.000001);
}

#[test]
fn critical_and_production_override_domain_and_require_approval() {
    for (risk, blast_radius) in [
        (RiskLevel::Critical, BlastRadius::Local),
        (RiskLevel::Low, BlastRadius::Production),
    ] {
        let mut input = features();
        input.risk_level = risk;
        input.blast_radius = blast_radius;
        input.domain = ProfessionalDomain::SecurityAssessment;
        let decision = StrategySelector::default().select(&input).unwrap();
        assert_eq!(decision.strategy, ExecutionStrategy::HighStakes);
        assert!(decision.approval_required);
    }
}

#[test]
fn documented_domain_rules_take_priority_over_complexity() {
    let mut input = features();
    input.domain = ProfessionalDomain::SecurityAssessment;
    input.risk_level = RiskLevel::High;
    let decision = StrategySelector::default().select(&input).unwrap();
    assert_eq!(decision.strategy, ExecutionStrategy::SecurityAssessment);
    assert_eq!(decision.budget_multiplier, 3.0);
    assert!(decision.approval_required);

    input.domain = ProfessionalDomain::ContinuousOperations;
    input.risk_level = RiskLevel::Low;
    input.requires_human_approval = true;
    let decision = StrategySelector::default().select(&input).unwrap();
    assert_eq!(decision.strategy, ExecutionStrategy::IncidentResponse);
    assert_eq!(decision.budget_multiplier, 2.5);
    assert!(decision.approval_required);
}

#[test]
fn domain_without_matching_rule_uses_complexity_not_an_invented_mapping() {
    for domain in [
        ProfessionalDomain::SoftwareDevelopment,
        ProfessionalDomain::ContinuousOperations,
        ProfessionalDomain::SecurityAssessment,
        ProfessionalDomain::SecurityOperations,
    ] {
        let mut input = features();
        input.domain = domain;
        assert_eq!(
            StrategySelector::default().select(&input).unwrap().strategy,
            ExecutionStrategy::Simple
        );
    }
}

#[test]
fn high_risk_irreversible_and_explicit_approval_cannot_be_downgraded() {
    let mut high_risk = features();
    high_risk.risk_level = RiskLevel::High;
    let mut irreversible = features();
    irreversible.reversible = false;
    let mut explicit = features();
    explicit.requires_human_approval = true;

    for input in [high_risk, irreversible, explicit] {
        let mut decision = StrategySelector::default().select(&input).unwrap();
        assert!(decision.approval_required);
        decision.approval_required = false;
        assert!(decision.validate(&input).is_err());
    }
}

#[test]
fn insufficient_authorized_budget_returns_error_without_safety_downgrade() {
    for (risk, steps, uncertainty, dependencies, selected, required) in [
        (
            RiskLevel::Critical,
            1,
            0.0,
            0,
            ExecutionStrategy::HighStakes,
            3.0,
        ),
        (RiskLevel::Low, 20, 0.9, 5, ExecutionStrategy::Research, 4.0),
    ] {
        let mut input = features();
        input.risk_level = risk;
        input.estimated_steps = steps;
        input.uncertainty_level = uncertainty;
        input.interdependency_count = dependencies;
        input.max_budget_multiplier = 1.0;
        assert_eq!(
            StrategySelector::default().select(&input).unwrap_err(),
            StrategySelectionError::BudgetInsufficient {
                strategy: selected,
                required_multiplier: required,
                max_multiplier: 1.0,
            }
        );
    }

    let mut input = features();
    input.risk_level = RiskLevel::Critical;
    input.max_budget_multiplier = 3.0;
    assert_eq!(
        StrategySelector::default()
            .select(&input)
            .unwrap()
            .budget_multiplier,
        3.0
    );
}

#[test]
fn invalid_uncertainty_and_budget_values_are_rejected_before_scoring() {
    for uncertainty in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut input = features();
        input.uncertainty_level = uncertainty;
        assert!(input.validate().is_err());
        assert!(StrategySelector::default().select(&input).is_err());
    }
    for budget in [0.0, 0.99, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut input = features();
        input.max_budget_multiplier = budget;
        assert!(input.validate().is_err());
        assert!(StrategySelector::default().select(&input).is_err());
    }
}

#[test]
fn decision_revalidation_rejects_modified_public_metadata() {
    let input = features();
    let valid = StrategySelector::default().select(&input).unwrap();
    let mut invalid_strategy = valid.clone();
    invalid_strategy.strategy = ExecutionStrategy::HighStakes;
    let mut invalid_multiplier = valid.clone();
    invalid_multiplier.budget_multiplier = 0.5;
    let mut invalid_score = valid.clone();
    invalid_score.complexity_score = f32::NAN;
    let mut invalid_reason = valid.clone();
    invalid_reason.selection_reason.clear();
    for decision in [
        invalid_strategy,
        invalid_multiplier,
        invalid_score,
        invalid_reason,
    ] {
        assert!(decision.validate(&input).is_err());
    }
}

#[test]
fn nine_strategy_ids_and_inherited_multipliers_match_design_and_round_trip() {
    for (strategy, id, multiplier) in [
        (ExecutionStrategy::Simple, "simple", 1.0),
        (ExecutionStrategy::Guided, "guided", 1.5),
        (ExecutionStrategy::Collaborative, "collaborative", 2.5),
        (ExecutionStrategy::HighStakes, "high_stakes", 3.0),
        (ExecutionStrategy::Research, "research", 4.0),
        (
            ExecutionStrategy::SecurityAssessment,
            "security_assessment",
            3.0,
        ),
        (
            ExecutionStrategy::IncidentResponse,
            "incident_response",
            2.5,
        ),
        (ExecutionStrategy::RefactorSafely, "refactor_safely", 1.5),
        (ExecutionStrategy::ComplianceAudit, "compliance_audit", 3.0),
    ] {
        assert_eq!(strategy.id(), id);
        assert_eq!(strategy.budget_multiplier(), multiplier);
        let encoded = serde_json::to_string(&strategy).unwrap();
        assert_eq!(encoded, format!("\"{id}\""));
        assert_eq!(
            serde_json::from_str::<ExecutionStrategy>(&encoded).unwrap(),
            strategy
        );
    }
}

#[test]
fn validated_feature_and_decision_contracts_round_trip() {
    let mut input = features();
    input.blast_radius = BlastRadius::Module;
    let encoded = serde_json::to_string(&input).unwrap();
    let decoded: TaskFeatures = serde_json::from_str(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, input);
    input.blast_radius = BlastRadius::System;
    let decision = StrategySelector::default().select(&input).unwrap();
    let encoded = serde_json::to_string(&decision).unwrap();
    let decoded: StrategyDecision = serde_json::from_str(&encoded).unwrap();
    decoded.validate(&input).unwrap();
    assert_eq!(decoded, decision);
}

#[test]
fn verification_metadata_matches_design_and_cannot_be_modified_after_selection() {
    for (strategy, frequency) in [
        (ExecutionStrategy::Simple, None),
        (ExecutionStrategy::Guided, Some(5)),
        (ExecutionStrategy::Collaborative, Some(3)),
        (ExecutionStrategy::HighStakes, Some(1)),
        (ExecutionStrategy::Research, None),
        (ExecutionStrategy::SecurityAssessment, Some(1)),
        (ExecutionStrategy::IncidentResponse, Some(3)),
        (ExecutionStrategy::RefactorSafely, Some(5)),
        (ExecutionStrategy::ComplianceAudit, Some(1)),
    ] {
        assert_eq!(strategy.verification_frequency(), frequency);
        assert_eq!(strategy.verification_required(), frequency.is_some());
    }

    let mut input = features();
    input.risk_level = RiskLevel::Critical;
    let valid = StrategySelector::new().select(&input).unwrap();
    assert!(valid.verification_required);
    assert_eq!(valid.verification_frequency, Some(1));
    let mut disabled = valid.clone();
    disabled.verification_required = false;
    assert!(disabled.validate(&input).is_err());
    let mut delayed = valid;
    delayed.verification_frequency = Some(10);
    assert!(delayed.validate(&input).is_err());
}

#[test]
fn json_null_for_required_numeric_features_is_rejected_by_deserialization() {
    for field in ["uncertainty_level", "max_budget_multiplier"] {
        let mut json = serde_json::to_value(features()).unwrap();
        json[field] = serde_json::Value::Null;
        assert!(serde_json::from_value::<TaskFeatures>(json).is_err());
    }
}

#[test]
fn json_out_of_range_numeric_features_are_rejected_at_validation_boundary() {
    for (field, value) in [
        ("uncertainty_level", -0.1),
        ("uncertainty_level", 1.1),
        ("max_budget_multiplier", 0.99),
    ] {
        let mut json = serde_json::to_value(features()).unwrap();
        json[field] = serde_json::json!(value);
        let decoded: TaskFeatures = serde_json::from_value(json).unwrap();
        assert!(decoded.validate().is_err());
        assert!(StrategySelector::new().select(&decoded).is_err());
    }
}

#[test]
fn json_unknown_feature_fields_are_rejected_instead_of_silently_discarded() {
    let mut json = serde_json::to_value(features()).unwrap();
    json["fake_unknown_feature"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TaskFeatures>(json).is_err());
}
