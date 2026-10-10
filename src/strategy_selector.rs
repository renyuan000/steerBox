use crate::domain::RiskLevel;
use crate::execution_strategy::ExecutionStrategy;
use crate::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures, TaskFeaturesError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrategyDecision {
    pub strategy: ExecutionStrategy,
    pub complexity_score: f32,
    pub approval_required: bool,
    pub budget_multiplier: f32,
    pub verification_required: bool,
    pub verification_frequency: Option<u32>,
    pub selection_reason: String,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum StrategySelectionError {
    #[error(transparent)]
    InvalidFeatures(#[from] TaskFeaturesError),
    #[error("authorized budget is insufficient for the selected strategy")]
    BudgetInsufficient {
        strategy: ExecutionStrategy,
        required_multiplier: f32,
        max_multiplier: f32,
    },
    #[error("invalid strategy decision field {field}: does not match validated task features")]
    InvalidDecision { field: &'static str },
}

#[derive(Debug, Default, Clone)]
pub struct StrategySelector {
    _private: (),
}

impl StrategySelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pure rule-based selection. Does not run models, tools, or alter loop budgets.
    pub fn select(
        &self,
        features: &TaskFeatures,
    ) -> Result<StrategyDecision, StrategySelectionError> {
        features.validate()?;
        let complexity_score = complexity(features);
        let (strategy, selection_reason) = if features.risk_level == RiskLevel::Critical {
            (
                ExecutionStrategy::HighStakes,
                "critical_risk_requires_high_stakes",
            )
        } else if features.blast_radius == BlastRadius::Production {
            (
                ExecutionStrategy::HighStakes,
                "production_scope_requires_high_stakes",
            )
        } else if features.domain == ProfessionalDomain::SecurityAssessment
            && matches!(features.risk_level, RiskLevel::High | RiskLevel::Critical)
        {
            (
                ExecutionStrategy::SecurityAssessment,
                "security_assessment_domain_high_risk",
            )
        } else if features.domain == ProfessionalDomain::ContinuousOperations
            && features.requires_human_approval
        {
            (
                ExecutionStrategy::IncidentResponse,
                "continuous_operations_explicit_approval",
            )
        } else if complexity_score <= 0.3 {
            (ExecutionStrategy::Simple, "complexity_at_or_below_0_3")
        } else if complexity_score <= 0.6 {
            (ExecutionStrategy::Guided, "complexity_at_or_below_0_6")
        } else if complexity_score <= 0.8 {
            (
                ExecutionStrategy::Collaborative,
                "complexity_at_or_below_0_8",
            )
        } else if features.uncertainty_level > 0.7 {
            (
                ExecutionStrategy::Research,
                "high_complexity_and_uncertainty",
            )
        } else {
            (
                ExecutionStrategy::Collaborative,
                "high_complexity_with_bounded_uncertainty",
            )
        };

        let budget_multiplier = strategy.budget_multiplier();
        if budget_multiplier > features.max_budget_multiplier {
            return Err(StrategySelectionError::BudgetInsufficient {
                strategy,
                required_multiplier: budget_multiplier,
                max_multiplier: features.max_budget_multiplier,
            });
        }
        let approval_required = strategy.requires_approval()
            || matches!(features.risk_level, RiskLevel::High | RiskLevel::Critical)
            || features.blast_radius == BlastRadius::Production
            || !features.reversible
            || features.requires_human_approval;

        Ok(StrategyDecision {
            strategy,
            complexity_score,
            approval_required,
            budget_multiplier,
            verification_required: strategy.verification_required(),
            verification_frequency: strategy.verification_frequency(),
            selection_reason: selection_reason.into(),
        })
    }
}

impl StrategyDecision {
    /// Recompute from validated features so changed public fields cannot weaken policy.
    pub fn validate(&self, features: &TaskFeatures) -> Result<(), StrategySelectionError> {
        let expected = StrategySelector::default().select(features)?;
        let mismatched_field = if self.strategy != expected.strategy {
            Some("strategy")
        } else if self.complexity_score != expected.complexity_score {
            Some("complexity_score")
        } else if self.approval_required != expected.approval_required {
            Some("approval_required")
        } else if self.budget_multiplier != expected.budget_multiplier {
            Some("budget_multiplier")
        } else if self.verification_required != expected.verification_required {
            Some("verification_required")
        } else if self.verification_frequency != expected.verification_frequency {
            Some("verification_frequency")
        } else if self.selection_reason != expected.selection_reason {
            Some("selection_reason")
        } else {
            None
        };
        if let Some(field) = mismatched_field {
            return Err(StrategySelectionError::InvalidDecision { field });
        }
        Ok(())
    }
}

fn complexity(features: &TaskFeatures) -> f32 {
    let step_score = (features.estimated_steps as f32 / 20.0).min(1.0);
    let dependency_score = (features.interdependency_count as f32 / 5.0).min(1.0);
    (step_score * 0.4 + features.uncertainty_level * 0.4 + dependency_score * 0.2).min(1.0)
}
