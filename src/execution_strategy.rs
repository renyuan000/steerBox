use serde::{Deserialize, Serialize};

/// Strategy identifiers describe the intended execution policy, not executed agents.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    /// Low-complexity, bounded single-model work.
    Simple,
    /// Multi-step work with validation checkpoints.
    Guided,
    /// Complex work whose execution policy calls for cross-model verification.
    Collaborative,
    /// Critical or production work requiring human approval.
    HighStakes,
    /// High-complexity, high-uncertainty exploration.
    Research,
    /// Policy label requiring isolation and evidence; no assessment runtime is implied.
    SecurityAssessment,
    /// Policy label requiring a time window and rollback; the selector does not run response.
    IncidentResponse,
    /// Policy label requiring incremental and regression checks, not performed here.
    RefactorSafely,
    /// Policy label requiring complete evidence and repeated checks, not performed here.
    ComplianceAudit,
}

impl ExecutionStrategy {
    pub fn id(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Guided => "guided",
            Self::Collaborative => "collaborative",
            Self::HighStakes => "high_stakes",
            Self::Research => "research",
            Self::SecurityAssessment => "security_assessment",
            Self::IncidentResponse => "incident_response",
            Self::RefactorSafely => "refactor_safely",
            Self::ComplianceAudit => "compliance_audit",
        }
    }

    /// Fixed defaults from the design; specialized strategies inherit their base.
    /// This is metadata only and does not adjust any runtime loop protection.
    pub fn budget_multiplier(self) -> f32 {
        match self {
            Self::Simple => 1.0,
            Self::Guided | Self::RefactorSafely => 1.5,
            Self::Collaborative | Self::IncidentResponse => 2.5,
            Self::HighStakes | Self::SecurityAssessment | Self::ComplianceAudit => 3.0,
            Self::Research => 4.0,
        }
    }

    pub fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::HighStakes | Self::SecurityAssessment | Self::ComplianceAudit
        )
    }

    /// Intended verification cadence only; callers must execute and audit verification.
    pub fn verification_frequency(self) -> Option<u32> {
        match self {
            Self::Simple | Self::Research => None,
            Self::Guided | Self::RefactorSafely => Some(5),
            Self::Collaborative | Self::IncidentResponse => Some(3),
            Self::HighStakes | Self::SecurityAssessment | Self::ComplianceAudit => Some(1),
        }
    }

    pub fn verification_required(self) -> bool {
        self.verification_frequency().is_some()
    }
}
