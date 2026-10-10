use crate::domain::RiskLevel;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlastRadius {
    Local,
    Module,
    System,
    Production,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfessionalDomain {
    SoftwareDevelopment,
    ContinuousOperations,
    SecurityAssessment,
    SecurityOperations,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TaskFeatures {
    pub estimated_steps: u32,
    pub uncertainty_level: f32,
    pub interdependency_count: u32,
    pub risk_level: RiskLevel,
    pub reversible: bool,
    pub blast_radius: BlastRadius,
    pub domain: ProfessionalDomain,
    pub requires_human_approval: bool,
    /// Authorized maximum multiplier, never permission to exceed the caller's cap.
    pub max_budget_multiplier: f32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TaskFeaturesError {
    #[error("invalid task feature {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
}

impl TaskFeatures {
    /// Revalidate public fields at the use boundary, before scoring or serializing.
    pub fn validate(&self) -> Result<(), TaskFeaturesError> {
        if !self.uncertainty_level.is_finite() || !(0.0..=1.0).contains(&self.uncertainty_level) {
            return Err(TaskFeaturesError::InvalidField {
                field: "uncertainty_level",
                reason: "must be finite and between zero and one",
            });
        }
        if !self.max_budget_multiplier.is_finite() || self.max_budget_multiplier < 1.0 {
            return Err(TaskFeaturesError::InvalidField {
                field: "max_budget_multiplier",
                reason: "must be finite and at least one",
            });
        }
        Ok(())
    }
}
