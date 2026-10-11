use crate::audit_store::{
    AuditStoreError, SideEffectLedger, SideEffectRecord, TraceRecord, TraceStore,
};
use crate::budget_calculator::BudgetCalculator;
use crate::domain::{
    AttemptLineageError, AttemptReason, AttemptStatus, BudgetKind, EventEnvelope,
    EventEnvelopeError, FailureCategory, GoalContract, LoopBudget, LoopRun, LoopStatus, LoopStep,
    NoProgressTracker, RiskLevel, TaskRun, TaskRunStatus,
};
use crate::event_log::{EventLog, EventLogError};
use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelAdapterRegistry, ModelDecision, ModelRequest,
};
use crate::model_call_store::{ModelCallStore, ModelCallStoreError};
use crate::model_contracts::{ModelCallRecord, ModelContractError, RouteDecision};
use crate::model_router::{ModelRouter, ModelRouterError, RouteRequest};
use crate::policy_gate::{PolicyDecision, PolicyGate, PolicyGateError, PolicyRequest};
use crate::provider_adapter::ProviderTransport;
use crate::provider_model_adapter::ProviderModelAdapter;
use crate::resolved_prompt::ResolvedPromptPack;
use crate::strategy_selector::{StrategyDecision, StrategySelectionError, StrategySelector};
use crate::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use crate::task_features::TaskFeatures;
use crate::tool_executor::{
    ReadOnlyToolExecutor, ToolCallInput, ToolCallOutput, ToolExecutionError,
};
use crate::tool_registry::ToolRegistry;
use crate::user_command_inbox::{
    CommandKind, CommandStatus, UserCommandInbox, UserCommandInboxError,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopControllerError {
    #[error("loop event log failed: {0}")]
    EventLog(#[from] EventLogError),
    #[error("loop event rejected: {0}")]
    EventEnvelope(#[from] EventEnvelopeError),
    #[error("loop budget exhausted: {0:?}")]
    BudgetExhausted(BudgetKind),
    #[error("task run lineage failed: {0}")]
    TaskRun(#[from] AttemptLineageError),
    #[error("loop is already terminal")]
    Terminal,
    #[error("loop is paused")]
    Paused,
    #[error("model adapter failed: {0}")]
    ModelAdapter(#[from] ModelAdapterError),
    #[error("model routing failed: {0}")]
    ModelRouter(#[from] ModelRouterError),
    #[error("no adapter registered for routed model: {0}")]
    ModelAdapterNotFound(String),
    #[error("model route requires human approval: {route_decision_id}")]
    ModelApprovalRequired { route_decision_id: String },
    #[error("route decision serialization failed: {0}")]
    RouteDecisionSerialization(String),
    #[error("model audit contract rejected: {0}")]
    ModelContract(#[from] ModelContractError),
    #[error("model call store failed: {0}")]
    ModelCallStore(#[from] ModelCallStoreError),
    #[error("resolved model call requires a model call store")]
    ModelCallStoreRequired,
    #[error("strategy selection failed: {0}")]
    StrategySelection(#[from] StrategySelectionError),
    #[error("execution strategy requires human approval: {strategy_event_id}")]
    StrategyApprovalRequired { strategy_event_id: String },
    #[error("strategy revision number overflow")]
    StrategyRevisionOverflow,
    #[error("tool call denied by policy: {decision:?}: {reason}")]
    PolicyDenied {
        decision: PolicyDecision,
        reason: String,
    },
    #[error("policy gate failed: {0}")]
    PolicyGate(#[from] PolicyGateError),
    #[error("tool execution failed: {0}")]
    ToolExecution(#[from] ToolExecutionError),
    #[error("audit store failed: {0}")]
    AuditStore(#[from] AuditStoreError),
    #[error("user command inbox failed: {0}")]
    UserCommandInbox(#[from] UserCommandInboxError),
    #[error("progress reporting failed: {0}")]
    ProgressReport(String),
}

struct ProgressSinkHolder {
    sink: Box<dyn ProgressSink>,
}

impl std::fmt::Debug for ProgressSinkHolder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProgressSinkHolder(..)")
    }
}

/// Minimal sequential loop that records observations only.
#[derive(Debug)]
pub struct ReadOnlyLoopController {
    goal: GoalContract,
    loop_run: LoopRun,
    task_run: TaskRun,
    budget: LoopBudget,
    event_log: EventLog,
    next_step_no: u32,
    no_progress: NoProgressTracker,
    trace_store: Option<TraceStore>,
    side_effect_ledger: Option<SideEffectLedger>,
    model_call_store: Option<ModelCallStore>,
    progress_sink: Option<ProgressSinkHolder>,
    command_inbox: Option<UserCommandInbox>,
    strategy: Option<StrategyDecision>,
    strategy_revision: u64,
}

impl ReadOnlyLoopController {
    pub fn start(
        event_log: EventLog,
        goal: GoalContract,
        loop_run_id: impl Into<String>,
        task_run_id: impl Into<String>,
        budget: LoopBudget,
    ) -> Result<Self, LoopControllerError> {
        let loop_run_id = loop_run_id.into();
        let task_run_id = task_run_id.into();
        // Preserve the legacy constructor while making the task identity
        // explicit in the new TaskRun model. Callers that have a real task
        // identity should use start_with_task_run.
        let task_run = TaskRun::new(task_run_id.clone(), task_run_id.clone(), budget)?;
        Self::start_with_task_run(event_log, goal, loop_run_id, task_run)
    }

    pub fn start_with_task_run(
        mut event_log: EventLog,
        goal: GoalContract,
        loop_run_id: impl Into<String>,
        mut task_run: TaskRun,
    ) -> Result<Self, LoopControllerError> {
        let loop_run_id = loop_run_id.into();
        let loop_run = LoopRun::new(
            loop_run_id.clone(),
            goal.goal_id.clone(),
            task_run.task_run_id.clone(),
        );
        if task_run.attempts.is_empty() {
            task_run.start_attempt(
                format!("{loop_run_id}:attempt:1"),
                AttemptReason::Initial,
                None,
            )?;
        } else if task_run.current_attempt_id.is_none()
            || task_run
                .current_attempt_id
                .as_deref()
                .and_then(|attempt_id| task_run.attempt(attempt_id))
                .is_none()
        {
            return Err(AttemptLineageError::AttemptNotFound("current_attempt_id".into()).into());
        }
        let budget = task_run.budget.clone();
        let max_no_progress_steps = budget.max_no_progress_steps;
        let max_same_action_repetitions = budget.max_same_action_repetitions;
        append_loop_event(
            &mut event_log,
            &goal,
            &loop_run,
            "loop_started",
            &format!("{}:started", loop_run.loop_run_id),
            json!({"status": "running"}),
            task_run.current_attempt_id.clone(),
        )?;
        Ok(Self {
            goal,
            loop_run,
            task_run,
            budget,
            event_log,
            next_step_no: 1,
            no_progress: NoProgressTracker::new(max_no_progress_steps, max_same_action_repetitions),
            trace_store: None,
            side_effect_ledger: None,
            model_call_store: None,
            progress_sink: None,
            command_inbox: None,
            strategy: None,
            strategy_revision: 0,
        })
    }

    pub fn start_with_progress_sink<S: ProgressSink + 'static>(
        event_log: EventLog,
        goal: GoalContract,
        loop_run_id: impl Into<String>,
        task_run_id: impl Into<String>,
        budget: LoopBudget,
        sink: S,
    ) -> Result<Self, LoopControllerError> {
        let mut controller = Self::start(event_log, goal, loop_run_id, task_run_id, budget)?;
        controller.progress_sink = Some(ProgressSinkHolder {
            sink: Box::new(sink),
        });
        controller.report_progress(
            ProgressSubject::Loop,
            controller.loop_run.loop_run_id.clone(),
            ProgressStatus::Started,
            Some("loop started"),
            None,
        )?;
        Ok(controller)
    }

    /// Select an execution policy from explicit features before starting a loop.
    /// The authorized budget is a hard ceiling, not a strategy-generated grant.
    pub fn start_with_strategy(
        event_log: EventLog,
        goal: GoalContract,
        loop_run_id: impl Into<String>,
        task_run_id: impl Into<String>,
        base_budget: LoopBudget,
        authorized_budget: LoopBudget,
        features: TaskFeatures,
    ) -> Result<Self, LoopControllerError> {
        let features = with_goal_risk_floor(features, goal.risk_level);
        let decision = StrategySelector::new().select(&features)?;
        decision.validate(&features)?;
        let budget =
            BudgetCalculator::calculate(&base_budget, &authorized_budget, decision.strategy);
        let mut controller =
            Self::start(event_log, goal, loop_run_id, task_run_id, authorized_budget)?;
        controller.record_strategy(
            features,
            decision,
            &base_budget,
            budget,
            "strategy_selected",
        )?;
        Ok(controller)
    }

    /// Changes policy only at a safe boundary. Limits can tighten but never expand,
    /// and neither consumed budget nor repeat-detection history is reset.
    pub fn select_strategy(
        &mut self,
        features: TaskFeatures,
        base_budget: &LoopBudget,
    ) -> Result<StrategyDecision, LoopControllerError> {
        self.prepare_action()?;
        let features = with_goal_risk_floor(features, self.goal.risk_level);
        let decision = StrategySelector::new().select(&features)?;
        decision.validate(&features)?;
        let budget = BudgetCalculator::calculate(base_budget, &self.budget, decision.strategy);
        let event_type = if self.strategy.is_some() {
            "strategy_adjusted"
        } else {
            "strategy_selected"
        };
        self.record_strategy(features, decision.clone(), base_budget, budget, event_type)?;
        Ok(decision)
    }

    fn record_strategy(
        &mut self,
        features: TaskFeatures,
        decision: StrategyDecision,
        base_budget: &LoopBudget,
        budget: LoopBudget,
        event_type: &str,
    ) -> Result<(), LoopControllerError> {
        let revision = self
            .strategy_revision
            .checked_add(1)
            .ok_or(LoopControllerError::StrategyRevisionOverflow)?;
        let event_id = format!("{}:strategy:{revision}", self.loop_run.loop_run_id);
        self.append_model_event(
            event_type,
            &event_id,
            json!({
                "schema_version": 1,
                "strategy_revision": revision,
                "previous_strategy": self.strategy.as_ref().map(|value| value.strategy),
                "decision": decision,
                "features": features,
                "base_budget": base_budget,
                "previous_budget": self.budget,
                "effective_budget": budget,
                "goal_verification_required": !self.goal.verification_plan.is_empty(),
                "verification_requirement_only": true,
                "execution_mode": "sequential_only",
                "no_progress_steps": self.no_progress.no_progress_steps,
                "same_action_repetitions": self.no_progress.same_action_repetitions,
            }),
        )?;
        // Commit the fail-closed state before the supplementary approval event.
        // An I/O error in that event must not leave the old permissive policy live.
        self.strategy_revision = revision;
        self.no_progress.max_no_progress_steps = budget.max_no_progress_steps;
        self.no_progress.max_same_action_repetitions = budget.max_same_action_repetitions;
        self.no_progress.no_progress_steps = self
            .no_progress
            .no_progress_steps
            .max(budget.no_progress_steps_used);
        self.no_progress.same_action_repetitions = self
            .no_progress
            .same_action_repetitions
            .max(budget.same_action_repetitions_used);
        self.budget = budget;
        self.sync_task_run_budget();
        self.strategy = Some(decision.clone());
        if decision.approval_required {
            self.append_model_event(
                "human_intervention_required",
                &format!("{event_id}:approval_required"),
                json!({
                    "strategy_event_id": event_id,
                    "reason": "execution strategy requires human approval",
                }),
            )?;
        }
        Ok(())
    }

    pub fn strategy(&self) -> Option<&StrategyDecision> {
        self.strategy.as_ref()
    }

    pub fn budget(&self) -> &LoopBudget {
        &self.budget
    }

    pub fn task_run(&self) -> &TaskRun {
        &self.task_run
    }

    pub fn current_attempt_id(&self) -> Option<&str> {
        self.task_run.current_attempt_id.as_deref()
    }

    pub fn with_audit_stores(
        mut self,
        trace_path: impl Into<std::path::PathBuf>,
        side_effect_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.trace_store = Some(TraceStore::open(trace_path));
        self.side_effect_ledger = Some(SideEffectLedger::open(side_effect_path));
        self
    }

    pub fn with_model_call_store(mut self, store: ModelCallStore) -> Self {
        self.model_call_store = Some(store);
        self
    }

    pub fn with_progress_sink<S: ProgressSink + 'static>(mut self, sink: S) -> Self {
        self.progress_sink = Some(ProgressSinkHolder {
            sink: Box::new(sink),
        });
        self
    }

    pub fn with_command_inbox(mut self, inbox: UserCommandInbox) -> Self {
        self.command_inbox = Some(inbox);
        self
    }

    pub fn run_observation(
        &mut self,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<LoopStep, LoopControllerError> {
        self.prepare_step_action()?;
        if let Err(kind) = self.budget.consume_step() {
            append_loop_event(
                &mut self.event_log,
                &self.goal,
                &self.loop_run,
                "loop_budget_exhausted",
                &format!("{}:budget:step", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
                self.task_run.current_attempt_id.clone(),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        self.sync_task_run_budget();
        let mut step = LoopStep::new(
            step_id,
            self.loop_run.loop_run_id.clone(),
            self.next_step_no,
            "observe",
        );
        self.next_step_no += 1;
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_started",
            &format!("{}:started", step.step_id),
            json!({"phase": step.phase, "step_no": step.step_no}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Started,
            Some("step started"),
            None,
        )?;
        let observation = observation.into();
        step.complete(observation.clone());
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_completed",
            &format!("{}:completed", step.step_id),
            json!({"summary": observation}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Completed,
            Some("step completed"),
            None,
        )?;
        Ok(step)
    }

    pub fn complete(&mut self) -> Result<LoopRun, LoopControllerError> {
        self.prepare_action()?;
        let attempt_id = self
            .current_attempt_id()
            .ok_or_else(|| AttemptLineageError::AttemptNotFound("current_attempt_id".into()))?
            .to_owned();
        self.finish_task_run(
            &attempt_id,
            AttemptStatus::Succeeded,
            TaskRunStatus::Succeeded,
            "passed",
            Vec::new(),
            Vec::new(),
        )?;
        self.loop_run.complete();
        append_loop_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            "loop_completed",
            &format!("{}:completed", self.loop_run.loop_run_id),
            json!({"status": "completed"}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Loop,
            self.loop_run.loop_run_id.clone(),
            ProgressStatus::Completed,
            Some("loop completed"),
            None,
        )?;
        Ok(self.loop_run.clone())
    }

    pub fn fail(&mut self, summary: impl Into<String>) -> Result<LoopRun, LoopControllerError> {
        self.prepare_action()?;
        let summary = summary.into();
        let attempt_id = self
            .current_attempt_id()
            .ok_or_else(|| AttemptLineageError::AttemptNotFound("current_attempt_id".into()))?
            .to_owned();
        let event_id = format!("{}:failed", self.loop_run.loop_run_id);
        self.finish_task_run(
            &attempt_id,
            AttemptStatus::Failed,
            TaskRunStatus::Failed,
            "failed",
            Vec::new(),
            vec![event_id.clone()],
        )?;
        self.loop_run.status = LoopStatus::Failed;
        self.loop_run.object_version += 1;
        self.loop_run.finished_at = Some(chrono::Utc::now().to_rfc3339());
        self.append_model_event(
            "loop_failed",
            &event_id,
            json!({"status": "failed", "summary": summary}),
        )?;
        self.report_progress(
            ProgressSubject::Loop,
            self.loop_run.loop_run_id.clone(),
            ProgressStatus::Failed,
            Some("loop failed"),
            None,
        )?;
        Ok(self.loop_run.clone())
    }

    pub fn run_model_step<A: ModelAdapter + ?Sized>(
        &mut self,
        adapter: &A,
        model_id: impl Into<String>,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.run_model_step_with_route(adapter, model_id, step_id, observation, None)
    }

    /// Execute a resolved provider call and persist its audit record before
    /// publishing completion events. This path is intentionally separate from
    /// the legacy fixture adapter path, which has no resolved prompt or wire hash.
    pub fn run_resolved_model_step<T: ProviderTransport>(
        &mut self,
        adapter: &ProviderModelAdapter<T>,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
        route: &RouteDecision,
        step_id: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        let store = self
            .model_call_store
            .clone()
            .ok_or(LoopControllerError::ModelCallStoreRequired)?;
        route.validate()?;
        for (actual, expected, field) in [
            (
                route.route_decision_id.as_str(),
                pack.metadata().route_decision_id.as_str(),
                "route_decision_id",
            ),
            (
                route.task_id.as_str(),
                pack.metadata().task_id.as_str(),
                "task_id",
            ),
            (
                route.selected_provider.as_str(),
                pack.provider_id(),
                "provider_id",
            ),
            (
                route.selected_endpoint.as_str(),
                pack.endpoint_id(),
                "endpoint_id",
            ),
            (route.selected_model.as_str(), pack.model_id(), "model_id"),
        ] {
            if actual != expected {
                return Err(LoopControllerError::ModelAdapter(
                    ModelAdapterError::ResolvedBindingMismatch { field },
                ));
            }
        }
        if route.approval_required {
            return Err(LoopControllerError::ModelApprovalRequired {
                route_decision_id: route.route_decision_id.clone(),
            });
        }
        self.prepare_step_action()?;
        if let Err(kind) = self.budget.consume_step() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:step", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        self.sync_task_run_budget();
        if let Err(kind) = self.budget.consume_model_call() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:model_call", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        self.sync_task_run_budget();
        let step_id = step_id.into();
        let mut step = LoopStep::new(
            step_id.clone(),
            self.loop_run.loop_run_id.clone(),
            self.next_step_no,
            "model_decision",
        );
        self.next_step_no += 1;
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_started",
            &format!("{step_id}:started"),
            json!({"phase": step.phase, "step_no": step.step_no}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Started,
            Some("step started"),
            None,
        )?;
        self.append_model_event(
            "model_call_started",
            &format!("{step_id}:model_started"),
            json!({
                "model_id": request.model_id,
                "route_decision_id": route.route_decision_id,
                "resolved_prompt_pack_id": pack.metadata().prompt_pack_id,
            }),
        )?;
        let model_progress_id = request.model_id.clone();
        self.report_progress(
            ProgressSubject::Model,
            model_progress_id.clone(),
            ProgressStatus::Started,
            Some("model call started"),
            None,
        )?;

        let started_at = chrono::Utc::now().to_rfc3339();
        let resolved = match adapter.complete_resolved(request, pack) {
            Ok(response) => response,
            Err(error) => {
                self.append_model_event(
                    "model_call_failed",
                    &format!("{step_id}:model_failed"),
                    json!({"error": error.to_string(), "route_decision_id": route.route_decision_id}),
                )?;
                append_step_event(
                    &mut self.event_log,
                    &self.goal,
                    &self.loop_run,
                    &step,
                    "step_failed",
                    &format!("{step_id}:failed"),
                    json!({"summary": error.to_string()}),
                    self.task_run.current_attempt_id.clone(),
                )?;
                self.report_progress(
                    ProgressSubject::Model,
                    model_progress_id.clone(),
                    ProgressStatus::Failed,
                    Some("model call failed"),
                    None,
                )?;
                self.report_progress(
                    ProgressSubject::Step,
                    step.step_id.clone(),
                    ProgressStatus::Failed,
                    Some("step failed"),
                    None,
                )?;
                return Err(LoopControllerError::ModelAdapter(error));
            }
        };
        let completed_at = chrono::Utc::now().to_rfc3339();
        let model_call_id = format!("call-{}-{step_id}", self.loop_run.loop_run_id);
        let output_artifact_ref = format!("trace://{model_call_id}/output");
        let record = ModelCallRecord {
            model_call_id: model_call_id.clone(),
            task_id: route.task_id.clone(),
            task_run_id: self.loop_run.task_run_id.clone(),
            agent_role: route.agent_role.clone(),
            provider_id: pack.provider_id().into(),
            endpoint_id: pack.endpoint_id().into(),
            model_id: pack.model_id().into(),
            route_decision_id: route.route_decision_id.clone(),
            resolved_prompt_pack_id: pack.metadata().prompt_pack_id.clone(),
            input_context_hash: resolved.input_context_hash.clone(),
            output_artifact_ref: output_artifact_ref.clone(),
            latency_ms: None,
            input_tokens: resolved
                .provider_response
                .usage
                .as_ref()
                .and_then(|usage| usage.prompt_tokens),
            output_tokens: resolved
                .provider_response
                .usage
                .as_ref()
                .and_then(|usage| usage.completion_tokens),
            estimated_cost: None,
            streaming: false,
            finish_reason: resolved
                .provider_response
                .finish_reason
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            tool_call_count: 0,
            verification_status: "not_run".into(),
            error_class: None,
            started_at,
            completed_at: Some(completed_at),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(error) = store.append(&record) {
            self.append_model_event(
                "model_call_failed",
                &format!("{step_id}:audit_failed"),
                json!({
                    "error": error.to_string(),
                    "route_decision_id": route.route_decision_id,
                    "model_call_id": model_call_id,
                    "audit_failure": true,
                }),
            )?;
            append_step_event(
                &mut self.event_log,
                &self.goal,
                &self.loop_run,
                &step,
                "step_failed",
                &format!("{step_id}:audit_failed"),
                json!({"summary": error.to_string(), "audit_failure": true}),
                self.task_run.current_attempt_id.clone(),
            )?;
            self.report_progress(
                ProgressSubject::Model,
                step.step_id.clone(),
                ProgressStatus::Failed,
                Some("model audit failed"),
                None,
            )?;
            self.report_progress(
                ProgressSubject::Step,
                step.step_id.clone(),
                ProgressStatus::Failed,
                Some("step failed"),
                None,
            )?;
            return Err(LoopControllerError::ModelCallStore(error));
        }
        let decision = resolved.response.decision;
        self.append_model_event(
            "model_call_completed",
            &format!("{step_id}:model_completed"),
            json!({
                "model_call_id": model_call_id,
                "provider_id": record.provider_id,
                "endpoint_id": record.endpoint_id,
                "model_id": record.model_id,
                "route_decision_id": record.route_decision_id,
                "resolved_prompt_pack_id": record.resolved_prompt_pack_id,
                "input_context_hash": record.input_context_hash,
                "output_artifact_ref": output_artifact_ref,
                "decision": decision,
                "summary": resolved.response.summary,
            }),
        )?;
        self.report_progress(
            ProgressSubject::Model,
            model_progress_id,
            ProgressStatus::Completed,
            Some(model_decision_progress_summary(decision)),
            None,
        )?;
        step.complete(resolved.response.summary);
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_completed",
            &format!("{step_id}:completed"),
            json!({"decision": decision, "model_call_id": record.model_call_id}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Completed,
            Some("step completed"),
            None,
        )?;
        if decision == ModelDecision::CompleteLoop {
            self.complete()?;
        }
        Ok(decision)
    }

    fn run_model_step_with_route<A: ModelAdapter + ?Sized>(
        &mut self,
        adapter: &A,
        model_id: impl Into<String>,
        step_id: impl Into<String>,
        observation: impl Into<String>,
        route_decision_id: Option<&str>,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.prepare_step_action()?;
        if let Err(kind) = self.budget.consume_step() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:step", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        self.sync_task_run_budget();
        if let Err(kind) = self.budget.consume_model_call() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:model_call", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        self.sync_task_run_budget();
        let step_id = step_id.into();
        let observation = observation.into();
        let model_id = model_id.into();
        let action_fingerprint = format!("{model_id}\n{}\n{observation}", self.goal.goal_id);
        let repeated_action = self.next_step_no > 1;
        if repeated_action && self.no_progress.observe(&action_fingerprint, false) {
            self.append_model_event(
                "no_progress_detected",
                &format!("{step_id}:no_progress"),
                json!({
                    "no_progress_steps": self.no_progress.no_progress_steps,
                    "same_action_repetitions": self.no_progress.same_action_repetitions,
                }),
            )?;
            return Err(LoopControllerError::BudgetExhausted(
                if self.no_progress.same_action_repetitions
                    >= self.no_progress.max_same_action_repetitions
                {
                    BudgetKind::SameActionRepetitions
                } else {
                    BudgetKind::NoProgressSteps
                },
            ));
        }
        if !repeated_action {
            self.no_progress.observe(&action_fingerprint, false);
        }
        let mut step = LoopStep::new(
            step_id.clone(),
            self.loop_run.loop_run_id.clone(),
            self.next_step_no,
            "model_decision",
        );
        self.next_step_no += 1;
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_started",
            &format!("{step_id}:started"),
            json!({"phase": step.phase, "step_no": step.step_no}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Started,
            Some("step started"),
            None,
        )?;
        let request = ModelRequest {
            model_id,
            goal_id: self.goal.goal_id.clone(),
            loop_run_id: self.loop_run.loop_run_id.clone(),
            step_id: step_id.clone(),
            objective: self.goal.objective.clone(),
            observation,
        };
        let model_progress_id = request.model_id.clone();
        self.append_model_event(
            "model_call_started",
            &format!("{step_id}:model_started"),
            json!({
                "model_id": request.model_id,
                "step_id": step_id,
                "route_decision_id": route_decision_id,
            }),
        )?;
        self.report_progress(
            ProgressSubject::Model,
            model_progress_id.clone(),
            ProgressStatus::Started,
            Some("model call started"),
            None,
        )?;
        let response = match adapter.complete(request) {
            Ok(response) => response,
            Err(error) => {
                self.append_model_event(
                    "model_call_failed",
                    &format!("{step_id}:model_failed"),
                    json!({"error": error.to_string(), "route_decision_id": route_decision_id}),
                )?;
                append_step_event(
                    &mut self.event_log,
                    &self.goal,
                    &self.loop_run,
                    &step,
                    "step_failed",
                    &format!("{step_id}:failed"),
                    json!({"summary": error.to_string()}),
                    self.task_run.current_attempt_id.clone(),
                )?;
                self.report_progress(
                    ProgressSubject::Model,
                    model_progress_id.clone(),
                    ProgressStatus::Failed,
                    Some("model call failed"),
                    None,
                )?;
                self.report_progress(
                    ProgressSubject::Step,
                    step.step_id.clone(),
                    ProgressStatus::Failed,
                    Some("step failed"),
                    None,
                )?;
                return Err(LoopControllerError::ModelAdapter(error));
            }
        };
        let decision = response.decision;
        let summary = response.summary.clone();
        self.append_model_event(
            "model_call_completed",
            &format!("{step_id}:model_completed"),
            json!({
                "decision": decision,
                "summary": summary,
                "route_decision_id": route_decision_id,
            }),
        )?;
        self.report_progress(
            ProgressSubject::Model,
            model_progress_id,
            ProgressStatus::Completed,
            Some(model_decision_progress_summary(decision)),
            None,
        )?;
        if decision == ModelDecision::RequestHuman {
            self.append_model_event(
                "human_intervention_required",
                &format!("{step_id}:human_required"),
                json!({"reason": response.summary}),
            )?;
        }
        step.complete(response.summary);
        append_step_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            &step,
            "step_completed",
            &format!("{step_id}:completed"),
            json!({"decision": decision}),
            self.task_run.current_attempt_id.clone(),
        )?;
        self.report_progress(
            ProgressSubject::Step,
            step.step_id.clone(),
            ProgressStatus::Completed,
            Some("step completed"),
            None,
        )?;
        if decision == ModelDecision::CompleteLoop {
            self.complete()?;
        }
        Ok(decision)
    }

    pub fn run_routed_model_step(
        &mut self,
        router: &ModelRouter<'_>,
        adapters: &ModelAdapterRegistry,
        route_request: RouteRequest,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.prepare_step_action()?;
        let route_decision = router.route(&route_request)?;
        route_decision.validate()?;
        let route_payload = serde_json::to_value(&route_decision)
            .map_err(|error| LoopControllerError::RouteDecisionSerialization(error.to_string()))?;
        self.append_model_event(
            "model_route_decided",
            &route_decision.route_decision_id,
            route_payload,
        )?;
        // No approval resolver is wired yet. Record the request and fail closed
        // before allocating a step, consuming call budget, or invoking an adapter.
        if route_decision.approval_required {
            self.append_model_event(
                "human_intervention_required",
                &format!("{}:approval_required", route_decision.route_decision_id),
                json!({
                    "route_decision_id": route_decision.route_decision_id,
                    "reason": "model route requires human approval",
                }),
            )?;
            return Err(LoopControllerError::ModelApprovalRequired {
                route_decision_id: route_decision.route_decision_id,
            });
        }
        let adapter = adapters
            .get(&route_decision.selected_model)
            .ok_or_else(|| {
                LoopControllerError::ModelAdapterNotFound(route_decision.selected_model.clone())
            })?;
        self.run_model_step_with_route(
            adapter,
            route_decision.selected_model,
            step_id,
            observation,
            Some(&route_decision.route_decision_id),
        )
    }

    pub fn request_tool_call(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        tool_id: impl Into<String>,
        step_id: impl Into<String>,
    ) -> Result<(), LoopControllerError> {
        self.prepare_step_action()?;
        let tool_id = tool_id.into();
        let step_id = step_id.into();
        let request = PolicyRequest {
            policy_event_id: format!("{step_id}:policy"),
            tool_id: tool_id.clone(),
            project_id: self.goal.project_id.clone(),
            goal_id: self.goal.goal_id.clone(),
            task_run_id: self.loop_run.task_run_id.clone(),
            loop_run_id: self.loop_run.loop_run_id.clone(),
            step_id: step_id.clone(),
            correlation_id: self.loop_run.loop_run_id.clone(),
            human_steering_mode: self.goal.human_steering_mode,
        };
        let result = gate.evaluate(registry, &self.goal, &request);
        gate.record_decision(&mut self.event_log, &request, &result)?;
        if result.decision != PolicyDecision::Allow {
            self.report_progress(
                ProgressSubject::Tool,
                format!("{step_id}:{tool_id}"),
                ProgressStatus::Failed,
                Some("tool call denied by policy"),
                None,
            )?;
            return Err(LoopControllerError::PolicyDenied {
                decision: result.decision,
                reason: result.reason,
            });
        }
        if self.strategy.is_some() {
            if let Err(kind) = self.budget.consume_tool_call() {
                self.append_model_event(
                    "loop_budget_exhausted",
                    &format!("{step_id}:budget:tool_call"),
                    json!({"budget_kind": format!("{kind:?}")}),
                )?;
                return Err(LoopControllerError::BudgetExhausted(kind));
            }
            self.sync_task_run_budget();
        }
        self.append_model_event(
            "tool_call_requested",
            &format!("{step_id}:tool_requested"),
            json!({
                "tool_id": tool_id,
                "side_effect_class": result.side_effect_class,
                "risk_level": result.risk_level,
            }),
        )?;
        self.report_progress(
            ProgressSubject::Tool,
            format!("{step_id}:{tool_id}"),
            ProgressStatus::Started,
            Some("tool call started"),
            None,
        )?;
        Ok(())
    }

    pub fn execute_read_only_tool(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        executor: &ReadOnlyToolExecutor,
        workspace: &std::path::Path,
        input: ToolCallInput,
        step_id: impl Into<String>,
    ) -> Result<ToolCallOutput, LoopControllerError> {
        let step_id = step_id.into();
        self.request_tool_call(registry, gate, input.tool_id.clone(), step_id.clone())?;
        let output = match executor.execute(registry, workspace, input.clone()) {
            Ok(output) => output,
            Err(error) => {
                let tool_call_id =
                    format!("{}:{step_id}:{}", self.loop_run.loop_run_id, input.tool_id);
                let failure_event_id = format!("{step_id}:tool_failed");
                self.append_model_event(
                    "tool_call_failed",
                    &failure_event_id,
                    tool_failure_payload(&failure_event_id, &tool_call_id, &input.tool_id, &error),
                )?;
                self.record_tool_audit(
                    registry,
                    &input.tool_id,
                    &step_id,
                    "failed",
                    &error.to_string(),
                )?;
                self.report_progress(
                    ProgressSubject::Tool,
                    format!("{step_id}:{}", input.tool_id),
                    ProgressStatus::Failed,
                    Some("tool call failed"),
                    None,
                )?;
                return Err(error.into());
            }
        };
        self.append_model_event(
            "tool_call_completed",
            &format!("{step_id}:tool_completed"),
            json!({"tool_id": input.tool_id, "summary": output.summary, "result": output.result}),
        )?;
        self.record_tool_audit(
            registry,
            &input.tool_id,
            &step_id,
            "completed",
            &output.summary,
        )?;
        self.report_progress(
            ProgressSubject::Tool,
            format!("{step_id}:{}", input.tool_id),
            ProgressStatus::Completed,
            Some("tool call completed"),
            None,
        )?;
        Ok(output)
    }

    fn record_tool_audit(
        &self,
        registry: &ToolRegistry,
        tool_id: &str,
        step_id: &str,
        status: &str,
        summary: &str,
    ) -> Result<(), LoopControllerError> {
        let occurred_at = chrono::Utc::now().to_rfc3339();
        if let Some(store) = &self.trace_store {
            store.append(&TraceRecord {
                trace_id: format!("{step_id}:tool:{status}"),
                goal_id: Some(self.goal.goal_id.clone()),
                loop_run_id: Some(self.loop_run.loop_run_id.clone()),
                step_id: Some(step_id.to_owned()),
                kind: format!("tool_{status}"),
                summary: summary.to_owned(),
                occurred_at: occurred_at.clone(),
            })?;
        }
        if let Some(ledger) = &self.side_effect_ledger {
            let class = registry
                .get(tool_id)
                .map(|profile| format!("{:?}", profile.side_effect_class).to_ascii_lowercase())
                .unwrap_or_else(|| "unknown".into());
            ledger.append(&SideEffectRecord {
                side_effect_id: format!("{step_id}:side-effect"),
                goal_id: self.goal.goal_id.clone(),
                loop_run_id: self.loop_run.loop_run_id.clone(),
                tool_id: tool_id.to_owned(),
                class,
                status: status.to_owned(),
                summary: summary.to_owned(),
                created_at: occurred_at,
            })?;
        }
        Ok(())
    }

    fn append_model_event(
        &mut self,
        event_type: &str,
        event_id: &str,
        payload: serde_json::Value,
    ) -> Result<u64, LoopControllerError> {
        let mut event = EventEnvelope::new(
            "loop_run",
            event_id,
            event_type,
            &self.loop_run.loop_run_id,
            &self.loop_run.loop_run_id,
            payload,
        )?;
        event.project_id = Some(self.goal.project_id.clone());
        event.goal_id = Some(self.goal.goal_id.clone());
        event.task_run_id = Some(self.loop_run.task_run_id.clone());
        event.attempt_id = self.task_run.current_attempt_id.clone();
        event.loop_run_id = Some(self.loop_run.loop_run_id.clone());
        Ok(self.event_log.append(event)?)
    }

    pub fn status(&self) -> LoopStatus {
        self.loop_run.status
    }

    fn report_progress(
        &mut self,
        subject: ProgressSubject,
        subject_id: String,
        status: ProgressStatus,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> Result<(), LoopControllerError> {
        let summary = summary.map(str::to_owned);
        if let Some(holder) = &mut self.progress_sink {
            holder
                .sink
                .report(ProgressEvent {
                    subject,
                    subject_id,
                    status,
                    summary,
                    progress,
                })
                .map_err(|error| LoopControllerError::ProgressReport(error.to_string()))?;
        }
        Ok(())
    }

    fn sync_task_run_budget(&mut self) {
        self.task_run.budget = self.budget.clone();
        self.task_run.heartbeat_at = Some(chrono::Utc::now().to_rfc3339());
        self.task_run.object_version += 1;
    }

    fn finish_task_run(
        &mut self,
        attempt_id: &str,
        attempt_status: AttemptStatus,
        task_status: TaskRunStatus,
        verification_status: &str,
        artifact_refs: Vec<String>,
        evidence_refs: Vec<String>,
    ) -> Result<(), LoopControllerError> {
        self.task_run.finish_attempt(
            attempt_id,
            attempt_status,
            verification_status,
            artifact_refs,
            evidence_refs,
        )?;
        self.task_run.status = task_status;
        self.task_run.finished_at = Some(chrono::Utc::now().to_rfc3339());
        self.task_run.heartbeat_at = self.task_run.finished_at.clone();
        self.task_run.object_version += 1;
        self.sync_task_run_budget();
        Ok(())
    }

    fn prepare_action(&mut self) -> Result<(), LoopControllerError> {
        self.apply_pending_commands()?;
        match self.loop_run.status {
            LoopStatus::Running => {
                if self
                    .strategy
                    .as_ref()
                    .is_some_and(|decision| decision.approval_required)
                {
                    return Err(LoopControllerError::StrategyApprovalRequired {
                        strategy_event_id: format!(
                            "{}:strategy:{}",
                            self.loop_run.loop_run_id, self.strategy_revision
                        ),
                    });
                }
                Ok(())
            }
            LoopStatus::Paused => Err(LoopControllerError::Paused),
            LoopStatus::Completed | LoopStatus::Cancelled | LoopStatus::Failed => {
                Err(LoopControllerError::Terminal)
            }
        }
    }

    fn prepare_step_action(&mut self) -> Result<(), LoopControllerError> {
        self.prepare_action()?;
        if self.strategy.is_some() {
            let exhausted =
                if self.no_progress.no_progress_steps >= self.no_progress.max_no_progress_steps {
                    Some(BudgetKind::NoProgressSteps)
                } else if self.no_progress.same_action_repetitions
                    >= self.no_progress.max_same_action_repetitions
                {
                    Some(BudgetKind::SameActionRepetitions)
                } else if self.budget.consecutive_verification_failures
                    >= self.budget.max_consecutive_verification_failures
                {
                    Some(BudgetKind::VerificationFailures)
                } else {
                    None
                };
            if let Some(kind) = exhausted {
                self.append_model_event(
                    "loop_budget_exhausted",
                    &format!("{}:strategy:guard", self.loop_run.loop_run_id),
                    json!({
                        "budget_kind": format!("{kind:?}"),
                        "strategy_revision": self.strategy_revision,
                    }),
                )?;
                return Err(LoopControllerError::BudgetExhausted(kind));
            }
        }
        Ok(())
    }

    fn apply_pending_commands(&mut self) -> Result<(), LoopControllerError> {
        let Some(inbox) = self.command_inbox.clone() else {
            return Ok(());
        };
        let commands = inbox.pending_for_loop(&self.goal.project_id, &self.loop_run.loop_run_id)?;
        for entry in commands {
            let command = entry.command;
            if command.goal_id != self.goal.goal_id
                || command.task_run_id != self.loop_run.task_run_id
            {
                inbox.update_status(&command.command_id, CommandStatus::Rejected)?;
                self.append_model_event(
                    "user_command_rejected",
                    &format!("{}:rejected", command.command_id),
                    json!({
                        "command_id": command.command_id,
                        "reason": "runtime target mismatch",
                    }),
                )?;
                continue;
            }

            let applied = match command.command_kind {
                CommandKind::Pause => {
                    self.loop_run.pause();
                    self.append_model_event(
                        "loop_paused",
                        &format!("{}:paused", command.command_id),
                        json!({"command_id": command.command_id}),
                    )?;
                    self.report_progress(
                        ProgressSubject::Loop,
                        self.loop_run.loop_run_id.clone(),
                        ProgressStatus::Paused,
                        Some("loop paused"),
                        None,
                    )?;
                    true
                }
                CommandKind::Resume => {
                    self.loop_run.resume();
                    self.append_model_event(
                        "loop_resumed",
                        &format!("{}:resumed", command.command_id),
                        json!({"command_id": command.command_id}),
                    )?;
                    self.report_progress(
                        ProgressSubject::Loop,
                        self.loop_run.loop_run_id.clone(),
                        ProgressStatus::Resumed,
                        Some("loop resumed"),
                        None,
                    )?;
                    true
                }
                CommandKind::Cancel => {
                    self.loop_run.cancel();
                    self.append_model_event(
                        "loop_cancelled",
                        &format!("{}:cancelled", command.command_id),
                        json!({"command_id": command.command_id}),
                    )?;
                    self.report_progress(
                        ProgressSubject::Loop,
                        self.loop_run.loop_run_id.clone(),
                        ProgressStatus::Cancelled,
                        Some("loop cancelled"),
                        None,
                    )?;
                    true
                }
                _ => false,
            };

            if applied {
                inbox.update_status(&command.command_id, CommandStatus::Applied)?;
                self.append_model_event(
                    "user_command_applied",
                    &format!("{}:applied", command.command_id),
                    json!({
                        "command_id": command.command_id,
                        "command_kind": command.command_kind,
                        "actor_ref": command.actor_ref,
                    }),
                )?;
            } else {
                inbox.update_status(&command.command_id, CommandStatus::Rejected)?;
                self.append_model_event(
                    "user_command_rejected",
                    &format!("{}:rejected", command.command_id),
                    json!({
                        "command_id": command.command_id,
                        "reason": "command kind is not implemented at this safe boundary",
                    }),
                )?;
            }
        }
        Ok(())
    }
}

fn with_goal_risk_floor(mut features: TaskFeatures, goal_risk: RiskLevel) -> TaskFeatures {
    features.risk_level = match (features.risk_level, goal_risk) {
        (RiskLevel::Critical, _) | (_, RiskLevel::Critical) => RiskLevel::Critical,
        (RiskLevel::High, _) | (_, RiskLevel::High) => RiskLevel::High,
        (RiskLevel::Medium, _) | (_, RiskLevel::Medium) => RiskLevel::Medium,
        _ => RiskLevel::Low,
    };
    features
}

fn tool_failure_payload(
    failure_event_id: &str,
    tool_call_id: &str,
    tool_id: &str,
    error: &ToolExecutionError,
) -> serde_json::Value {
    let category = match error {
        ToolExecutionError::TimedOut => FailureCategory::Timeout,
        ToolExecutionError::InvalidWorkspace(_) | ToolExecutionError::OutsideWorkspace(_) => {
            FailureCategory::Permission
        }
        ToolExecutionError::NotRegistered(_)
        | ToolExecutionError::NotImplemented(_)
        | ToolExecutionError::InvalidArguments
        | ToolExecutionError::StartFailed
        | ToolExecutionError::WaitFailed
        | ToolExecutionError::OutputCollectionFailed => FailureCategory::Tool,
    };
    json!({
        "tool_id": tool_id,
        "tool_call_id": tool_call_id,
        "failure_category": category,
        "summary": error.to_string(),
        "lesson_candidate": {
            "candidate_id": format!("{tool_call_id}:lesson"),
            "source_event_ids": [failure_event_id],
            "title": "Review failed tool call before retry",
            "summary": "Classify the failure and preserve the original evidence before retrying.",
            "confidence": "low",
            "lifecycle_state": "candidate"
        },
        "redaction_class": "summary"
    })
}

fn append_loop_event(
    event_log: &mut EventLog,
    goal: &GoalContract,
    loop_run: &LoopRun,
    event_type: &str,
    event_id: &str,
    payload: serde_json::Value,
    attempt_id: Option<String>,
) -> Result<u64, LoopControllerError> {
    let mut event = EventEnvelope::new(
        "loop_run",
        event_id,
        event_type,
        &loop_run.loop_run_id,
        &loop_run.loop_run_id,
        payload,
    )?;
    event.project_id = Some(goal.project_id.clone());
    event.goal_id = Some(goal.goal_id.clone());
    event.task_run_id = Some(loop_run.task_run_id.clone());
    event.attempt_id = attempt_id;
    event.loop_run_id = Some(loop_run.loop_run_id.clone());
    Ok(event_log.append(event)?)
}

fn model_decision_progress_summary(decision: ModelDecision) -> &'static str {
    match decision {
        ModelDecision::CompleteLoop => "fixture decision: complete loop",
        ModelDecision::ContinueObservation => "continue observation; loop remains running",
        ModelDecision::RequestHuman => "human intervention required; loop remains running",
    }
}

#[allow(clippy::too_many_arguments)]
fn append_step_event(
    event_log: &mut EventLog,
    goal: &GoalContract,
    loop_run: &LoopRun,
    step: &LoopStep,
    event_type: &str,
    event_id: &str,
    payload: serde_json::Value,
    attempt_id: Option<String>,
) -> Result<u64, LoopControllerError> {
    let mut event = EventEnvelope::new(
        "loop_step",
        event_id,
        event_type,
        &step.step_id,
        &loop_run.loop_run_id,
        payload,
    )?;
    event.project_id = Some(goal.project_id.clone());
    event.goal_id = Some(goal.goal_id.clone());
    event.task_run_id = Some(loop_run.task_run_id.clone());
    event.attempt_id = attempt_id;
    event.loop_run_id = Some(loop_run.loop_run_id.clone());
    event.step_id = Some(step.step_id.clone());
    Ok(event_log.append(event)?)
}
