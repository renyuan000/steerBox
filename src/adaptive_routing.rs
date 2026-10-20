//! Secret-free contracts for bounded, task-local model adaptation.
//!
//! This module deliberately contains no provider URL, credential reference,
//! tool permission, or model prompt.  The runtime owns those boundaries; this
//! module only evaluates host-observable quality signals and returns a bounded
//! next action.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ADAPTIVE_ROUTING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EffectLevel {
    Fast,
    Balanced,
    Strong,
    XHigh,
}

impl EffectLevel {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Fast => Some(Self::Balanced),
            Self::Balanced => Some(Self::Strong),
            Self::Strong => Some(Self::XHigh),
            Self::XHigh => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualityStatus {
    Verified,
    Acceptable,
    NeedsUpgrade,
    Retryable,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualitySignal {
    VerificationPassed,
    VerificationFailed,
    LowQuality,
    ProviderTimeout,
    ProviderUnavailable,
    OutputTruncated,
    NoProgress,
    StructuredOutputInvalid,
    ToolResultFailed,
    LatencyBudgetExceeded,
    UsageBudgetExceeded,
    InsufficientEvidence,
    SecurityBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityAssessment {
    pub schema_version: u32,
    pub status: QualityStatus,
    pub signal: QualitySignal,
}

impl QualityAssessment {
    pub fn new(status: QualityStatus, signal: QualitySignal) -> Self {
        Self {
            schema_version: ADAPTIVE_ROUTING_SCHEMA_VERSION,
            status,
            signal,
        }
    }

    /// Derive one bounded assessment from host-observable evidence. The
    /// precedence is fail-closed: security and provider failures outrank
    /// quality hints, and missing acceptance evidence never becomes success.
    pub fn from_observation(observation: QualityObservation) -> Self {
        if observation.security_boundary {
            return Self::new(QualityStatus::Blocked, QualitySignal::SecurityBoundary);
        }
        if observation.provider_timeout {
            return Self::new(QualityStatus::Retryable, QualitySignal::ProviderTimeout);
        }
        if observation.provider_unavailable {
            return Self::new(QualityStatus::Retryable, QualitySignal::ProviderUnavailable);
        }
        if observation.usage_budget_exceeded {
            return Self::new(QualityStatus::Failed, QualitySignal::UsageBudgetExceeded);
        }
        if observation.structured_output_valid == Some(false) {
            return Self::new(
                QualityStatus::NeedsUpgrade,
                QualitySignal::StructuredOutputInvalid,
            );
        }
        if observation.verification_failed {
            return Self::new(
                QualityStatus::NeedsUpgrade,
                QualitySignal::VerificationFailed,
            );
        }
        if observation.tool_result_failed {
            return Self::new(QualityStatus::NeedsUpgrade, QualitySignal::ToolResultFailed);
        }
        if observation.output_truncated {
            return Self::new(QualityStatus::NeedsUpgrade, QualitySignal::OutputTruncated);
        }
        if observation.latency_budget_exceeded {
            return Self::new(
                QualityStatus::NeedsUpgrade,
                QualitySignal::LatencyBudgetExceeded,
            );
        }
        if observation.no_progress {
            return Self::new(QualityStatus::NeedsUpgrade, QualitySignal::NoProgress);
        }
        if observation.verification_passed || observation.accepted {
            return Self::new(QualityStatus::Verified, QualitySignal::VerificationPassed);
        }
        Self::new(QualityStatus::Failed, QualitySignal::InsufficientEvidence)
    }
}

/// Host-observable facts used to classify one model call. It carries no model
/// prompt, response body, endpoint, credential, or tool authorization.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityObservation {
    pub accepted: bool,
    pub verification_passed: bool,
    pub verification_failed: bool,
    pub structured_output_valid: Option<bool>,
    pub tool_result_failed: bool,
    pub provider_timeout: bool,
    pub provider_unavailable: bool,
    pub output_truncated: bool,
    pub latency_budget_exceeded: bool,
    pub usage_budget_exceeded: bool,
    pub no_progress: bool,
    pub security_boundary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdaptationContext {
    pub current_effect_level: EffectLevel,
    pub maximum_effect_level: EffectLevel,
    pub assessment: QualityAssessment,
    pub remaining_attempts: u32,
    pub cooldown_active: bool,
    pub budget_allows_upgrade: bool,
    pub fallback_available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdaptationAction {
    Continue,
    Upgrade,
    Fallback,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdaptationDecision {
    pub schema_version: u32,
    pub action: AdaptationAction,
    pub from_effect_level: EffectLevel,
    pub to_effect_level: Option<EffectLevel>,
    pub reason_code: String,
    pub creates_new_attempt: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdaptationError {
    #[error("maximum effect level is below current effect level")]
    MaximumBelowCurrent,
}

impl AdaptationDecision {
    pub fn from_context(context: AdaptationContext) -> Result<Self, AdaptationError> {
        if context.maximum_effect_level < context.current_effect_level {
            return Err(AdaptationError::MaximumBelowCurrent);
        }

        let current = context.current_effect_level;
        let stop = |reason_code: &'static str| Self {
            schema_version: ADAPTIVE_ROUTING_SCHEMA_VERSION,
            action: AdaptationAction::Stop,
            from_effect_level: current,
            to_effect_level: None,
            reason_code: reason_code.to_owned(),
            creates_new_attempt: false,
        };

        let decision = if context.assessment.signal == QualitySignal::SecurityBoundary {
            stop("security_or_policy_blocked")
        } else {
            match context.assessment.status {
                QualityStatus::Verified | QualityStatus::Acceptable => Self {
                    schema_version: ADAPTIVE_ROUTING_SCHEMA_VERSION,
                    action: AdaptationAction::Continue,
                    from_effect_level: current,
                    to_effect_level: None,
                    reason_code: "quality_accepted".to_owned(),
                    creates_new_attempt: false,
                },
                QualityStatus::Blocked => stop("security_or_policy_blocked"),
                QualityStatus::Failed => stop("quality_failed"),
                QualityStatus::Retryable => {
                    if context.remaining_attempts > 0 && context.fallback_available {
                        Self {
                            schema_version: ADAPTIVE_ROUTING_SCHEMA_VERSION,
                            action: AdaptationAction::Fallback,
                            from_effect_level: current,
                            to_effect_level: None,
                            reason_code: "retryable_provider_failure".to_owned(),
                            creates_new_attempt: true,
                        }
                    } else {
                        stop("retry_budget_or_fallback_exhausted")
                    }
                }
                QualityStatus::NeedsUpgrade => {
                    let Some(next) = current.next() else {
                        return Ok(stop("maximum_effect_level_reached"));
                    };
                    if next > context.maximum_effect_level {
                        stop("maximum_effect_level_reached")
                    } else if context.remaining_attempts == 0 {
                        stop("retry_budget_exhausted")
                    } else if context.cooldown_active {
                        stop("adaptation_cooldown_active")
                    } else if !context.budget_allows_upgrade {
                        stop("upgrade_budget_exhausted")
                    } else {
                        Self {
                            schema_version: ADAPTIVE_ROUTING_SCHEMA_VERSION,
                            action: AdaptationAction::Upgrade,
                            from_effect_level: current,
                            to_effect_level: Some(next),
                            reason_code: quality_reason(context.assessment.signal),
                            creates_new_attempt: false,
                        }
                    }
                }
            }
        };

        Ok(decision)
    }
}

fn quality_reason(signal: QualitySignal) -> String {
    match signal {
        QualitySignal::VerificationFailed => "verification_quality_below_target",
        QualitySignal::LowQuality => "quality_below_target",
        QualitySignal::OutputTruncated => "output_truncated",
        QualitySignal::NoProgress => "no_progress_detected",
        QualitySignal::VerificationPassed => "verification_passed",
        QualitySignal::ProviderTimeout => "provider_timeout",
        QualitySignal::ProviderUnavailable => "provider_unavailable",
        QualitySignal::StructuredOutputInvalid => "structured_output_invalid",
        QualitySignal::ToolResultFailed => "tool_result_failed",
        QualitySignal::LatencyBudgetExceeded => "latency_budget_exceeded",
        QualitySignal::UsageBudgetExceeded => "usage_budget_exceeded",
        QualitySignal::InsufficientEvidence => "insufficient_quality_evidence",
        QualitySignal::SecurityBoundary => "security_boundary_signal",
    }
    .to_owned()
}
