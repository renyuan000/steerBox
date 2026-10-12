use crate::model_contracts::RouteDecision;
use crate::model_registry::ModelRegistry;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// A bounded description of the task risk used by routing policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// The maximum privacy sensitivity a model route is allowed to handle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrivacyRequirement {
    Public,
    Internal,
    Sensitive,
    Restricted,
}

impl PrivacyRequirement {
    fn label(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
        }
    }
}

/// Inputs used by the deterministic router. Budget and latency are upper-bound
/// targets: a model outside either bound is not an eligible fallback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteRequest {
    pub task_id: String,
    pub task_type: String,
    pub agent_role: String,
    pub risk_level: RiskLevel,
    pub required_capabilities: Vec<String>,
    pub privacy_requirement: PrivacyRequirement,
    pub budget_limit: Option<f64>,
    pub latency_target_ms: Option<u64>,
    pub prompt_profile_id: String,
    pub prompt_profile_version: String,
}

/// Routing metadata kept beside ModelRegistry. The registry describes what a
/// model is; this profile describes when the model should be selected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRouteProfile {
    pub model_id: String,
    /// Empty means all task types.
    pub supported_task_types: Vec<String>,
    /// Empty means all agent roles.
    pub supported_agent_roles: Vec<String>,
    pub max_risk_level: RiskLevel,
    pub privacy_clearance: PrivacyRequirement,
    pub estimated_cost: f64,
    pub estimated_latency_ms: u64,
    /// Scores are policy metadata, not model output or credentials.
    pub quality_score: u8,
    pub reliability_score: u8,
    pub preferred_for_high_risk: bool,
    pub fallback_priority: i32,
}

/// Deterministic, secret-free explanation of one routing decision. The
/// timestamped route id is intentionally omitted so callers can diff or
/// replay explanations without nondeterministic noise.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteExplanation {
    pub selected_model: String,
    pub candidate_models: Vec<String>,
    pub rejected_candidates: Vec<String>,
    pub selection_reason: String,
    pub fallback_chain: Vec<String>,
    pub estimated_cost: Option<f64>,
    pub estimated_latency_ms: Option<u64>,
    pub approval_required: bool,
}

#[derive(Debug, Error, PartialEq)]
pub enum ModelRouterError {
    #[error("route request field must not be empty: {field}")]
    EmptyRequestField { field: &'static str },
    #[error("route request budget_limit must be finite and non-negative")]
    InvalidBudget,
    #[error("model route profile field must not be empty: {field}")]
    EmptyProfileField { field: &'static str },
    #[error("model route profile references unknown model: {0}")]
    UnknownModel(String),
    #[error("model route profile registered more than once: {0}")]
    DuplicateProfile(String),
    #[error("model route profile estimated_cost must be finite and non-negative")]
    InvalidCost,
    #[error("model route profile score must be between 0 and 100")]
    InvalidScore,
    #[error("no compatible model for task {task_id}: {reasons}")]
    NoCompatibleModels { task_id: String, reasons: String },
}

/// A pure, secret-free model selector. It does not make network calls and does
/// not resolve provider authentication; adapters do that after routing.
#[derive(Debug)]
pub struct ModelRouter<'a> {
    registry: &'a ModelRegistry,
    profiles: BTreeMap<String, ModelRouteProfile>,
}

impl<'a> ModelRouter<'a> {
    pub fn new(
        registry: &'a ModelRegistry,
        profiles: Vec<ModelRouteProfile>,
    ) -> Result<Self, ModelRouterError> {
        let mut indexed = BTreeMap::new();
        for profile in profiles {
            validate_profile(registry, &profile)?;
            if indexed.contains_key(&profile.model_id) {
                return Err(ModelRouterError::DuplicateProfile(profile.model_id));
            }
            indexed.insert(profile.model_id.clone(), profile);
        }
        Ok(Self {
            registry,
            profiles: indexed,
        })
    }

    pub fn route(&self, request: &RouteRequest) -> Result<RouteDecision, ModelRouterError> {
        validate_request(request)?;

        let mut rejected = Vec::new();
        let mut candidates = Vec::new();
        for model in self.registry.models() {
            let Some(profile) = self.profiles.get(&model.model_id) else {
                rejected.push(format!("{}:missing_route_profile", model.model_id));
                continue;
            };
            if let Err(reason) = compatibility_reason(request, model, profile) {
                rejected.push(format!("{}:{reason}", model.model_id));
                continue;
            }

            let score = score_candidate(request, profile);
            let endpoint = self
                .registry
                .endpoint(&model.endpoint_id)
                .expect("ModelRegistry guarantees model endpoint binding");
            candidates.push(ScoredCandidate {
                model_id: model.model_id.clone(),
                provider_id: model.provider_id.clone(),
                endpoint_id: endpoint.endpoint_id.clone(),
                estimated_cost: profile.estimated_cost,
                estimated_latency_ms: profile.estimated_latency_ms,
                score,
            });
        }

        if candidates.is_empty() {
            return Err(ModelRouterError::NoCompatibleModels {
                task_id: request.task_id.clone(),
                reasons: if rejected.is_empty() {
                    "registry contains no routable models".into()
                } else {
                    rejected.join(", ")
                },
            });
        }

        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.model_id.cmp(&right.model_id))
        });
        let selected = &candidates[0];
        let candidate_models = candidates
            .iter()
            .map(|candidate| candidate.model_id.clone())
            .collect::<Vec<_>>();
        let fallback_chain = candidate_models.iter().skip(1).cloned().collect();
        let high_risk = request.risk_level >= RiskLevel::High;
        let selection_reason = if high_risk {
            "high-risk route prioritizes risk fit, quality, and reliability before cost/latency"
        } else {
            "low-risk route prioritizes cost and latency within the requested policy bounds"
        };

        let decision = RouteDecision {
            route_decision_id: format!(
                "route-{}-{}-{}",
                request.task_id,
                selected.model_id,
                Utc::now().timestamp_millis()
            ),
            task_id: request.task_id.clone(),
            task_type: request.task_type.clone(),
            agent_role: request.agent_role.clone(),
            candidate_models,
            selected_provider: selected.provider_id.clone(),
            selected_model: selected.model_id.clone(),
            selected_endpoint: selected.endpoint_id.clone(),
            selection_reason: selection_reason.into(),
            rejected_candidates: rejected,
            cost_estimate: Some(selected.estimated_cost),
            latency_estimate_ms: Some(selected.estimated_latency_ms),
            risk_level: request.risk_level.label().into(),
            privacy_requirement: request.privacy_requirement.label().into(),
            fallback_chain,
            prompt_profile_id: request.prompt_profile_id.clone(),
            prompt_profile_version: request.prompt_profile_version.clone(),
            approval_required: high_risk
                || request.privacy_requirement >= PrivacyRequirement::Restricted,
            created_at: Utc::now().to_rfc3339(),
        };
        // Keep the routing boundary honest if RouteDecision gains a sensitive field later.
        decision
            .validate_secret_free()
            .expect("router emits a secret-free RouteDecision");
        Ok(decision)
    }

    /// Return the same routing result as `route`, shaped for debug UIs and
    /// audit explainers without exposing route timestamps or credentials.
    pub fn explain(&self, request: &RouteRequest) -> Result<RouteExplanation, ModelRouterError> {
        let decision = self.route(request)?;
        Ok(RouteExplanation {
            selected_model: decision.selected_model,
            candidate_models: decision.candidate_models,
            rejected_candidates: decision.rejected_candidates,
            selection_reason: decision.selection_reason,
            fallback_chain: decision.fallback_chain,
            estimated_cost: decision.cost_estimate,
            estimated_latency_ms: decision.latency_estimate_ms,
            approval_required: decision.approval_required,
        })
    }
}

/// Select the next unattempted fallback in the route's declared order.
///
/// This is a pure planning helper: it does not invoke a model, mutate the
/// route, or retry a failed call. The primary model is always excluded, and
/// repeated attempted or fallback entries do not change the deterministic
/// first-eligible result.
pub fn next_fallback_model<'a>(
    route: &'a RouteDecision,
    attempted_models: &[String],
) -> Option<&'a str> {
    route.fallback_chain.iter().find_map(|candidate| {
        if candidate == &route.selected_model
            || attempted_models
                .iter()
                .any(|attempted| attempted == candidate)
        {
            None
        } else {
            Some(candidate.as_str())
        }
    })
}

struct ScoredCandidate {
    model_id: String,
    provider_id: String,
    endpoint_id: String,
    estimated_cost: f64,
    estimated_latency_ms: u64,
    score: i64,
}

fn validate_request(request: &RouteRequest) -> Result<(), ModelRouterError> {
    for (field, value) in [
        ("task_id", request.task_id.as_str()),
        ("task_type", request.task_type.as_str()),
        ("agent_role", request.agent_role.as_str()),
        ("prompt_profile_id", request.prompt_profile_id.as_str()),
        (
            "prompt_profile_version",
            request.prompt_profile_version.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(ModelRouterError::EmptyRequestField { field });
        }
    }
    if request
        .required_capabilities
        .iter()
        .any(|capability| capability.trim().is_empty())
    {
        return Err(ModelRouterError::EmptyRequestField {
            field: "required_capabilities",
        });
    }
    if request
        .budget_limit
        .is_some_and(|budget| !budget.is_finite() || budget < 0.0)
    {
        return Err(ModelRouterError::InvalidBudget);
    }
    Ok(())
}

fn validate_profile(
    registry: &ModelRegistry,
    profile: &ModelRouteProfile,
) -> Result<(), ModelRouterError> {
    if profile.model_id.trim().is_empty() {
        return Err(ModelRouterError::EmptyProfileField { field: "model_id" });
    }
    if registry.model(&profile.model_id).is_none() {
        return Err(ModelRouterError::UnknownModel(profile.model_id.clone()));
    }
    if !profile.estimated_cost.is_finite() || profile.estimated_cost < 0.0 {
        return Err(ModelRouterError::InvalidCost);
    }
    if profile.quality_score > 100 || profile.reliability_score > 100 {
        return Err(ModelRouterError::InvalidScore);
    }
    Ok(())
}

fn compatibility_reason(
    request: &RouteRequest,
    model: &crate::model_registry::ModelProfile,
    profile: &ModelRouteProfile,
) -> Result<(), String> {
    if !profile.supported_task_types.is_empty()
        && !profile
            .supported_task_types
            .iter()
            .any(|task_type| task_type == &request.task_type)
    {
        return Err("task_type_mismatch".into());
    }
    if !profile.supported_agent_roles.is_empty()
        && !profile
            .supported_agent_roles
            .iter()
            .any(|role| role == &request.agent_role)
    {
        return Err("agent_role_mismatch".into());
    }
    if request.risk_level > profile.max_risk_level {
        return Err("max_risk_level_exceeded".into());
    }
    if request.privacy_requirement > profile.privacy_clearance {
        return Err("privacy_clearance_exceeded".into());
    }
    if let Some(limit) = request.budget_limit {
        if profile.estimated_cost > limit {
            return Err("budget_limit_exceeded".into());
        }
    }
    if let Some(target) = request.latency_target_ms {
        if profile.estimated_latency_ms > target {
            return Err("latency_target_exceeded".into());
        }
    }
    if request.required_capabilities.iter().any(|required| {
        !model
            .capabilities
            .iter()
            .any(|capability| capability == required)
    }) {
        let missing = request
            .required_capabilities
            .iter()
            .find(|required| {
                !model
                    .capabilities
                    .iter()
                    .any(|capability| capability == *required)
            })
            .expect("any missing capability was checked above");
        return Err(format!("required_capability_missing:{missing}"));
    }
    Ok(())
}

fn score_candidate(request: &RouteRequest, profile: &ModelRouteProfile) -> i64 {
    let cost_penalty = (profile.estimated_cost * 1_000.0) as i64;
    let latency_penalty = profile.estimated_latency_ms as i64;
    let policy_bonus = profile.fallback_priority as i64 * 10;
    if request.risk_level >= RiskLevel::High {
        let safety_bonus = if profile.preferred_for_high_risk {
            1_000_000
        } else {
            0
        };
        safety_bonus
            + i64::from(profile.quality_score) * 10_000
            + i64::from(profile.reliability_score) * 1_000
            + policy_bonus
            - cost_penalty
            - latency_penalty
    } else {
        i64::from(profile.quality_score) * 100
            + i64::from(profile.reliability_score) * 10
            + policy_bonus
            - cost_penalty * 10
            - latency_penalty * 10
    }
}
