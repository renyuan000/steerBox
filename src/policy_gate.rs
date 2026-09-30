use crate::domain::{EventEnvelope, GoalContract, HumanSteeringMode, RiskLevel};
use crate::event_log::{EventLog, EventLogError};
use crate::tool_registry::{SideEffectClass, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Warn,
    RequireCheckpoint,
    RequireHumanApproval,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRequest {
    pub policy_event_id: String,
    pub tool_id: String,
    pub project_id: String,
    pub goal_id: String,
    pub task_run_id: String,
    pub loop_run_id: String,
    pub step_id: String,
    pub correlation_id: String,
    pub human_steering_mode: HumanSteeringMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyResult {
    pub decision: PolicyDecision,
    pub reason: String,
    pub tool_id: String,
    pub side_effect_class: Option<SideEffectClass>,
    pub risk_level: Option<RiskLevel>,
}

#[derive(Debug, Error)]
pub enum PolicyGateError {
    #[error("policy event construction failed: {0}")]
    EventEnvelope(#[from] crate::domain::EventEnvelopeError),
    #[error("policy event log failed: {0}")]
    EventLog(#[from] EventLogError),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PolicyGate;

impl PolicyGate {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(
        &self,
        registry: &ToolRegistry,
        goal: &GoalContract,
        request: &PolicyRequest,
    ) -> PolicyResult {
        if request.project_id != goal.project_id {
            return result(
                request,
                PolicyDecision::Block,
                "request project does not match goal project",
                None,
                None,
            );
        }
        if goal
            .forbidden_tools
            .iter()
            .any(|tool| tool == &request.tool_id)
        {
            return result(
                request,
                PolicyDecision::Block,
                "tool is forbidden by the goal",
                None,
                None,
            );
        }
        let Some(profile) = registry.get(&request.tool_id) else {
            return result(
                request,
                PolicyDecision::Block,
                "tool is not registered",
                None,
                None,
            );
        };
        if !goal.allowed_tools.is_empty()
            && !goal
                .allowed_tools
                .iter()
                .any(|tool| tool == &request.tool_id)
        {
            return result(
                request,
                PolicyDecision::Block,
                "tool is not allowed by the goal",
                Some(profile.side_effect_class),
                Some(profile.risk_level),
            );
        }
        if !profile
            .allowed_in_modes
            .contains(&request.human_steering_mode)
        {
            return result(
                request,
                PolicyDecision::Block,
                "tool is not allowed in the current steering mode",
                Some(profile.side_effect_class),
                Some(profile.risk_level),
            );
        }
        if profile.side_effect_class != SideEffectClass::None
            || matches!(profile.risk_level, RiskLevel::High | RiskLevel::Critical)
        {
            return result(
                request,
                PolicyDecision::RequireHumanApproval,
                "tool requires human approval before side effects",
                Some(profile.side_effect_class),
                Some(profile.risk_level),
            );
        }
        result(
            request,
            PolicyDecision::Allow,
            "registered low-risk read-only tool is allowed",
            Some(profile.side_effect_class),
            Some(profile.risk_level),
        )
    }

    pub fn record_decision(
        &self,
        event_log: &mut EventLog,
        request: &PolicyRequest,
        result: &PolicyResult,
    ) -> Result<u64, PolicyGateError> {
        let mut event = EventEnvelope::new(
            "policy",
            request.policy_event_id.clone(),
            "policy_decision",
            &request.tool_id,
            &request.correlation_id,
            json!({
                "decision": result.decision,
                "reason": result.reason,
                "tool_id": request.tool_id,
                "side_effect_class": result.side_effect_class,
                "risk_level": result.risk_level,
            }),
        )?;
        event.project_id = Some(request.project_id.clone());
        event.goal_id = Some(request.goal_id.clone());
        event.task_run_id = Some(request.task_run_id.clone());
        event.loop_run_id = Some(request.loop_run_id.clone());
        event.step_id = Some(request.step_id.clone());
        event_log.append(event).map_err(PolicyGateError::from)
    }
}

fn result(
    request: &PolicyRequest,
    decision: PolicyDecision,
    reason: &str,
    side_effect_class: Option<SideEffectClass>,
    risk_level: Option<RiskLevel>,
) -> PolicyResult {
    PolicyResult {
        decision,
        reason: reason.into(),
        tool_id: request.tool_id.clone(),
        side_effect_class,
        risk_level,
    }
}
