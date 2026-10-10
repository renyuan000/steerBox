use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

fn now() -> String {
    let timestamp: DateTime<Utc> = Utc::now();
    timestamp.to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub schema_version: u32,
    pub sequence_no: Option<u64>,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub portfolio_id: Option<String>,
    pub project_id: Option<String>,
    pub goal_id: Option<String>,
    pub task_id: Option<String>,
    pub task_run_id: Option<String>,
    pub attempt_id: Option<String>,
    pub loop_run_id: Option<String>,
    pub step_id: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
    pub producer: String,
    pub occurred_at: String,
    pub recorded_at: String,
    pub payload: Value,
    pub redaction_class: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    Tool,
    Model,
    Network,
    Context,
    Permission,
    Verification,
    Recovery,
    Timeout,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub tool_call_id: String,
    pub tool_id: String,
    pub project_id: String,
    pub goal_id: String,
    pub task_run_id: String,
    pub loop_run_id: String,
    pub step_id: String,
    pub side_effect_class: String,
    pub status: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LessonCandidate {
    pub candidate_id: String,
    pub project_id: String,
    pub source_event_ids: Vec<String>,
    pub title: String,
    pub summary: String,
    pub confidence: String,
    pub lifecycle_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolUseErrorEvent {
    pub error_event_id: String,
    pub tool_call_id: String,
    pub failure_category: FailureCategory,
    pub summary: String,
    pub lesson_candidate: Option<LessonCandidate>,
    pub redaction_class: String,
    pub occurred_at: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventEnvelopeError {
    #[error("event payload contains forbidden sensitive field: {field}")]
    SensitivePayload { field: String },
}

impl EventEnvelope {
    pub fn new(
        aggregate_type: impl Into<String>,
        event_id: impl Into<String>,
        event_type: impl Into<String>,
        aggregate_id: impl Into<String>,
        correlation_id: impl Into<String>,
        payload: Value,
    ) -> Result<Self, EventEnvelopeError> {
        if let Some(field) = forbidden_payload_field(&payload) {
            return Err(EventEnvelopeError::SensitivePayload { field });
        }
        let timestamp = now();
        Ok(Self {
            event_id: event_id.into(),
            event_type: event_type.into(),
            schema_version: 1,
            sequence_no: None,
            aggregate_type: aggregate_type.into(),
            aggregate_id: aggregate_id.into(),
            portfolio_id: None,
            project_id: None,
            goal_id: None,
            task_id: None,
            task_run_id: None,
            attempt_id: None,
            loop_run_id: None,
            step_id: None,
            correlation_id: correlation_id.into(),
            causation_id: None,
            producer: "steerbox-runtime".into(),
            occurred_at: timestamp.clone(),
            recorded_at: timestamp,
            payload,
            redaction_class: "summary".into(),
        })
    }
}

fn forbidden_payload_field(value: &Value) -> Option<String> {
    match value {
        Value::Object(fields) => fields.iter().find_map(|(key, value)| {
            if is_forbidden_payload_key(key) {
                Some(key.clone())
            } else {
                forbidden_payload_field(value)
            }
        }),
        Value::Array(values) => values.iter().find_map(forbidden_payload_field),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn is_forbidden_payload_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey"
            | "authorization"
            | "accesstoken"
            | "bearertoken"
            | "cookie"
            | "credentialresponse"
            | "clientsecret"
            | "password"
            | "privatekey"
            | "refreshtoken"
            | "secret"
            | "secretkey"
            | "sessiontoken"
            | "sshkey"
            | "token"
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanSteeringMode {
    AutonomousLoop,
    HumanSteerableLoop,
    HumanInTheLoop,
    ManualTakeover,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalContract {
    pub goal_id: String,
    pub project_id: String,
    pub schema_version: u32,
    pub object_version: u64,
    pub objective: String,
    pub scope: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub success_criteria: Vec<String>,
    pub quality_bar: String,
    pub constraints: Vec<String>,
    pub risk_level: RiskLevel,
    pub allowed_tools: Vec<String>,
    pub forbidden_tools: Vec<String>,
    pub allowed_models: Vec<String>,
    pub model_routing_policy: String,
    pub verification_plan: Vec<String>,
    pub stop_conditions: Vec<String>,
    pub human_steering_mode: HumanSteeringMode,
    pub checkpoint_policy: String,
    pub side_effect_boundary: String,
    pub rollback_or_compensation_plan: String,
    pub created_at: String,
    pub updated_at: String,
}

impl GoalContract {
    pub fn new(
        goal_id: impl Into<String>,
        project_id: impl Into<String>,
        objective: impl Into<String>,
        scope: Vec<String>,
        out_of_scope: Vec<String>,
        verification_plan: Vec<String>,
    ) -> Self {
        let timestamp = now();
        Self {
            goal_id: goal_id.into(),
            project_id: project_id.into(),
            schema_version: 1,
            object_version: 1,
            objective: objective.into(),
            scope,
            out_of_scope,
            success_criteria: vec!["verification_plan_passed".into()],
            quality_bar: "contract_default".into(),
            constraints: Vec::new(),
            risk_level: RiskLevel::Low,
            allowed_tools: Vec::new(),
            forbidden_tools: Vec::new(),
            allowed_models: Vec::new(),
            model_routing_policy: "rule_based".into(),
            verification_plan,
            stop_conditions: vec!["budget_exhausted".into(), "human_required".into()],
            human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
            checkpoint_policy: "before_file_write".into(),
            side_effect_boundary: "local_workspace_only".into(),
            rollback_or_compensation_plan: "restore_checkpoint_or_escalate".into(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    pub fn is_within_scope(&self, path: &str) -> bool {
        let Some(normalized) = scope_path(path) else {
            return false;
        };
        if normalized.is_empty() {
            return false;
        }
        let matches = |prefix: &String| {
            scope_path(prefix).is_some_and(|prefix| {
                prefix.is_empty()
                    || normalized == prefix
                    || normalized.starts_with(&format!("{prefix}/"))
            })
        };
        let excluded = self.out_of_scope.iter().any(matches);
        let included = self.scope.iter().any(matches);
        included && !excluded
    }
}

// Scope is a portable workspace-relative path, never an absolute or traversing path.
fn scope_path(path: &str) -> Option<String> {
    if path.is_empty() || path.contains('\\') || path.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_str()?),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopStatus {
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopRun {
    pub loop_run_id: String,
    pub goal_id: String,
    pub task_run_id: String,
    pub schema_version: u32,
    pub object_version: u64,
    pub status: LoopStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetKind {
    Steps,
    ToolCalls,
    ModelCalls,
    Attempts,
    GoalRevisions,
    NoProgressSteps,
    SameActionRepetitions,
    VerificationFailures,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopBudget {
    pub max_steps: u32,
    pub max_tool_calls: u32,
    pub max_model_calls: u32,
    pub max_attempts: u32,
    pub max_goal_revisions: u32,
    pub max_no_progress_steps: u32,
    pub max_same_action_repetitions: u32,
    pub max_consecutive_verification_failures: u32,
    pub steps_used: u32,
    pub tool_calls_used: u32,
    pub model_calls_used: u32,
    pub attempts_used: u32,
    pub goal_revisions_used: u32,
    pub no_progress_steps_used: u32,
    pub same_action_repetitions_used: u32,
    pub consecutive_verification_failures: u32,
}

impl LoopBudget {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_steps: u32,
        max_tool_calls: u32,
        max_model_calls: u32,
        max_attempts: u32,
        max_goal_revisions: u32,
        max_no_progress_steps: u32,
        max_same_action_repetitions: u32,
        max_consecutive_verification_failures: u32,
    ) -> Self {
        Self {
            max_steps,
            max_tool_calls,
            max_model_calls,
            max_attempts,
            max_goal_revisions,
            max_no_progress_steps,
            max_same_action_repetitions,
            max_consecutive_verification_failures,
            steps_used: 0,
            tool_calls_used: 0,
            model_calls_used: 0,
            attempts_used: 0,
            goal_revisions_used: 0,
            no_progress_steps_used: 0,
            same_action_repetitions_used: 0,
            consecutive_verification_failures: 0,
        }
    }

    pub fn consume_step(&mut self) -> Result<(), BudgetKind> {
        consume_budget(self.max_steps, &mut self.steps_used, BudgetKind::Steps)
    }

    pub fn consume_tool_call(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_tool_calls,
            &mut self.tool_calls_used,
            BudgetKind::ToolCalls,
        )
    }

    pub fn consume_model_call(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_model_calls,
            &mut self.model_calls_used,
            BudgetKind::ModelCalls,
        )
    }

    pub fn start_attempt(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_attempts,
            &mut self.attempts_used,
            BudgetKind::Attempts,
        )
    }

    pub fn consume_goal_revision(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_goal_revisions,
            &mut self.goal_revisions_used,
            BudgetKind::GoalRevisions,
        )
    }

    pub fn consume_no_progress_step(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_no_progress_steps,
            &mut self.no_progress_steps_used,
            BudgetKind::NoProgressSteps,
        )
    }

    pub fn consume_same_action_repetition(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_same_action_repetitions,
            &mut self.same_action_repetitions_used,
            BudgetKind::SameActionRepetitions,
        )
    }

    pub fn consume_verification_failure(&mut self) -> Result<(), BudgetKind> {
        consume_budget(
            self.max_consecutive_verification_failures,
            &mut self.consecutive_verification_failures,
            BudgetKind::VerificationFailures,
        )
    }

    pub fn record_verification_passed(&mut self) {
        self.consecutive_verification_failures = 0;
    }
}

fn consume_budget(limit: u32, used: &mut u32, kind: BudgetKind) -> Result<(), BudgetKind> {
    if *used >= limit {
        Err(kind)
    } else {
        *used += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoProgressTracker {
    pub max_no_progress_steps: u32,
    pub max_same_action_repetitions: u32,
    pub no_progress_steps: u32,
    pub same_action_repetitions: u32,
    pub last_action_fingerprint: Option<String>,
}

impl NoProgressTracker {
    pub fn new(max_no_progress_steps: u32, max_same_action_repetitions: u32) -> Self {
        Self {
            max_no_progress_steps: max_no_progress_steps.max(1),
            max_same_action_repetitions: max_same_action_repetitions.max(1),
            no_progress_steps: 0,
            same_action_repetitions: 0,
            last_action_fingerprint: None,
        }
    }

    pub fn observe(&mut self, action_fingerprint: impl Into<String>, made_progress: bool) -> bool {
        if made_progress {
            self.reset_after_progress();
            return false;
        }

        let action_fingerprint = action_fingerprint.into();
        self.no_progress_steps += 1;
        if self.last_action_fingerprint.as_deref() == Some(action_fingerprint.as_str()) {
            self.same_action_repetitions += 1;
        } else {
            self.last_action_fingerprint = Some(action_fingerprint);
            self.same_action_repetitions = 1;
        }
        self.stop_requested()
    }

    pub fn stop_requested(&self) -> bool {
        self.no_progress_steps >= self.max_no_progress_steps
            || self.same_action_repetitions >= self.max_same_action_repetitions
    }

    pub fn reset_after_progress(&mut self) {
        self.no_progress_steps = 0;
        self.same_action_repetitions = 0;
        self.last_action_fingerprint = None;
    }
}

impl LoopRun {
    pub fn new(
        loop_run_id: impl Into<String>,
        goal_id: impl Into<String>,
        task_run_id: impl Into<String>,
    ) -> Self {
        Self {
            loop_run_id: loop_run_id.into(),
            goal_id: goal_id.into(),
            task_run_id: task_run_id.into(),
            schema_version: 1,
            object_version: 1,
            status: LoopStatus::Running,
            started_at: now(),
            finished_at: None,
        }
    }

    pub fn complete(&mut self) {
        if self.status == LoopStatus::Running {
            self.status = LoopStatus::Completed;
            self.object_version += 1;
            self.finished_at = Some(now());
        }
    }

    pub fn pause(&mut self) {
        if self.status == LoopStatus::Running {
            self.status = LoopStatus::Paused;
            self.object_version += 1;
        }
    }

    pub fn resume(&mut self) {
        if self.status == LoopStatus::Paused {
            self.status = LoopStatus::Running;
            self.object_version += 1;
        }
    }

    pub fn cancel(&mut self) {
        if !self.is_terminal() {
            self.status = LoopStatus::Cancelled;
            self.object_version += 1;
            self.finished_at = Some(now());
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            LoopStatus::Completed | LoopStatus::Cancelled | LoopStatus::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopStepStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopStep {
    pub step_id: String,
    pub loop_run_id: String,
    pub step_no: u32,
    pub phase: String,
    pub schema_version: u32,
    pub status: LoopStepStatus,
    pub summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

impl LoopStep {
    pub fn new(
        step_id: impl Into<String>,
        loop_run_id: impl Into<String>,
        step_no: u32,
        phase: impl Into<String>,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            loop_run_id: loop_run_id.into(),
            step_no,
            phase: phase.into(),
            schema_version: 1,
            status: LoopStepStatus::Running,
            summary: None,
            started_at: now(),
            finished_at: None,
        }
    }

    pub fn complete(&mut self, summary: impl Into<String>) {
        if self.status == LoopStepStatus::Running {
            self.status = LoopStepStatus::Completed;
            self.summary = Some(summary.into());
            self.finished_at = Some(now());
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            LoopStepStatus::Completed | LoopStepStatus::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Backlog,
    Ready,
    Queued,
    Running,
    WaitingDependency,
    WaitingHuman,
    Verifying,
    Succeeded,
    Blocked,
    Failed,
    Canceled,
    Superseded,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum BoardColumn {
    Todo,
    Doing,
    Done,
    Attention,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskTransitionError {
    #[error("illegal task transition from {from:?} to {to:?}")]
    Illegal { from: TaskState, to: TaskState },
    #[error("task is terminal")]
    Terminal,
    #[error("task cannot succeed before required conditions: {missing:?}")]
    SuccessPreconditionFailed { missing: Vec<SuccessRequirement> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessRequirement {
    VerificationPassed,
    EvidencePresent,
    PolicyClear,
    AttemptFinished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub task_id: String,
    pub project_id: String,
    pub goal_id: String,
    pub title: String,
    pub schema_version: u32,
    pub object_version: u64,
    pub domain_state: TaskState,
    pub priority: u32,
    pub risk_level: RiskLevel,
    pub dependency_ids: Vec<String>,
    pub required_capabilities: Vec<String>,
    pub required_artifacts: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub success_criteria: Vec<String>,
    pub verification_plan: Vec<String>,
    pub verification_passed: bool,
    pub policy_blocked: bool,
    pub attempt_finished: bool,
    pub owner: Option<String>,
}

impl Task {
    pub fn new(
        task_id: impl Into<String>,
        project_id: impl Into<String>,
        goal_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            project_id: project_id.into(),
            goal_id: goal_id.into(),
            title: title.into(),
            schema_version: 1,
            object_version: 1,
            domain_state: TaskState::Backlog,
            priority: 0,
            risk_level: RiskLevel::Low,
            dependency_ids: Vec::new(),
            required_capabilities: Vec::new(),
            required_artifacts: Vec::new(),
            evidence_refs: Vec::new(),
            success_criteria: Vec::new(),
            verification_plan: Vec::new(),
            verification_passed: false,
            policy_blocked: false,
            attempt_finished: false,
            owner: None,
        }
    }

    pub fn record_verification_passed(&mut self) {
        self.verification_passed = true;
        self.object_version += 1;
    }

    pub fn add_evidence_ref(&mut self, evidence_ref: impl Into<String>) {
        let evidence_ref = evidence_ref.into();
        if !self.evidence_refs.contains(&evidence_ref) {
            self.evidence_refs.push(evidence_ref);
            self.object_version += 1;
        }
    }

    pub fn set_policy_blocked(&mut self, blocked: bool) {
        self.policy_blocked = blocked;
        self.object_version += 1;
    }

    pub fn finish_attempt(&mut self) {
        self.attempt_finished = true;
        self.object_version += 1;
    }

    pub fn transition_to(&mut self, next: TaskState) -> Result<(), TaskTransitionError> {
        if self.domain_state == TaskState::Archived {
            return Err(TaskTransitionError::Terminal);
        }
        if !is_allowed_transition(self.domain_state, next) {
            return Err(TaskTransitionError::Illegal {
                from: self.domain_state,
                to: next,
            });
        }
        if next == TaskState::Succeeded {
            let mut missing = Vec::new();
            if !self.verification_passed {
                missing.push(SuccessRequirement::VerificationPassed);
            }
            if self.evidence_refs.is_empty() {
                missing.push(SuccessRequirement::EvidencePresent);
            }
            if self.policy_blocked {
                missing.push(SuccessRequirement::PolicyClear);
            }
            if !self.attempt_finished {
                missing.push(SuccessRequirement::AttemptFinished);
            }
            if !missing.is_empty() {
                return Err(TaskTransitionError::SuccessPreconditionFailed { missing });
            }
        }
        self.domain_state = next;
        self.object_version += 1;
        Ok(())
    }

    pub fn board_column(&self) -> BoardColumn {
        match self.domain_state {
            TaskState::Backlog | TaskState::Ready | TaskState::Queued => BoardColumn::Todo,
            TaskState::Running
            | TaskState::WaitingDependency
            | TaskState::WaitingHuman
            | TaskState::Verifying => BoardColumn::Doing,
            TaskState::Succeeded
            | TaskState::Canceled
            | TaskState::Superseded
            | TaskState::Archived => BoardColumn::Done,
            TaskState::Blocked | TaskState::Failed => BoardColumn::Attention,
        }
    }
}

fn is_allowed_transition(from: TaskState, to: TaskState) -> bool {
    use TaskState::*;
    matches!(
        (from, to),
        (Backlog, Ready | Canceled)
            | (Ready, Queued | Blocked | Canceled)
            | (Queued, Running | Blocked | Canceled)
            | (
                Running,
                WaitingDependency | WaitingHuman | Verifying | Failed | Canceled
            )
            | (WaitingDependency, Ready | Blocked | Canceled)
            | (WaitingHuman, Running | Blocked | Canceled)
            | (Verifying, Succeeded | Failed | WaitingHuman | Canceled)
            | (Failed, Queued | Archived | Canceled)
            | (Blocked, Ready | Canceled | Archived)
            | (Succeeded, Archived | Superseded)
            | (Superseded, Archived)
            | (Canceled, Archived)
    )
}
