use crate::approval_contract::{ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore};
use crate::audit_store::{
    AuditStoreError, CheckpointMetadata, CheckpointStore, SideEffectLedger, SideEffectRecord,
    TraceRecord, TraceStore,
};
use crate::budget_calculator::BudgetCalculator;
use crate::domain::{
    AttemptLineageError, AttemptReason, AttemptStatus, BudgetKind, EventEnvelope,
    EventEnvelopeError, FailureCategory, GoalContract, LoopBudget, LoopRun, LoopStatus, LoopStep,
    NoProgressTracker, RiskLevel, TaskRun, TaskRunStatus,
};
use crate::event_log::{EventLog, EventLogError};
use crate::memory::{MemoryCandidate, MemoryQuery};
use crate::memory_store::{MemoryStore, MemoryStoreError};
use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelAdapterRegistry, ModelDecision, ModelRequest,
    ModelResponseWithAction,
};
use crate::model_call_store::{ModelCallStore, ModelCallStoreError};
use crate::model_contracts::{
    ModelActionEnvelope, ModelCallFailureRecord, ModelCallRecord, ModelContractError,
    RouteDecision, VerificationContract,
};
use crate::model_router::{
    next_fallback_model, ModelRouter, ModelRouterError, RiskLevel as RouteRiskLevel, RouteRequest,
};
use crate::policy_gate::{PolicyDecision, PolicyGate, PolicyGateError, PolicyRequest};
use crate::provider_adapter::{ProviderErrorClass, ProviderTransport};
use crate::provider_model_adapter::ProviderModelAdapter;
use crate::redaction::{Redactor, FAIL_CLOSED_MARKER};
use crate::resolved_prompt::ResolvedPromptPack;
use crate::strategy_selector::{StrategyDecision, StrategySelectionError, StrategySelector};
use crate::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use crate::task_features::TaskFeatures;
use crate::tool_executor::{
    ReadOnlyToolExecutor, ToolCallInput, ToolCallOutput, ToolExecutionError, WorkspaceToolExecutor,
};
use crate::tool_registry::ToolRegistry;
use crate::user_command_inbox::{
    CommandDrainResult, CommandDrainStopReason, CommandKind, CommandPollResult, CommandPoller,
    CommandStatus, NonBlockingCommandReader, RuntimeTick, RuntimeTickError, UserCommand,
    UserCommandInbox, UserCommandInboxError,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopControllerError {
    #[error("loop event log failed: {0}")]
    EventLog(#[from] EventLogError),
    #[error("loop event rejected: {0}")]
    EventEnvelope(#[from] EventEnvelopeError),
    #[error("memory store failed: {0}")]
    MemoryStore(#[from] MemoryStoreError),
    #[error("memory query scope does not match current loop")]
    MemoryQueryScopeMismatch,
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
    #[error("approval required: {request_id}")]
    ApprovalRequired { request_id: String },
    #[error("approval contract failed: {0}")]
    ApprovalContract(#[from] ApprovalError),
    #[error("tool execution failed: {0}")]
    ToolExecution(#[from] ToolExecutionError),
    #[error("workspace tool execution failed: {0}")]
    WorkspaceToolExecution(String),
    #[error("audit store failed: {0}")]
    AuditStore(#[from] AuditStoreError),
    #[error("user command inbox failed: {0}")]
    UserCommandInbox(#[from] UserCommandInboxError),
    #[error("user command inbox is not configured")]
    CommandInboxRequired,
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
    checkpoint_store: Option<CheckpointStore>,
    side_effect_ledger: Option<SideEffectLedger>,
    model_call_store: Option<ModelCallStore>,
    approval_store: Option<ApprovalStore>,
    progress_sink: Option<ProgressSinkHolder>,
    command_inbox: Option<UserCommandInbox>,
    strategy: Option<StrategyDecision>,
    strategy_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelActionExecution {
    pub action_id: String,
    pub action_output: ToolCallOutput,
    pub verification_output: ToolCallOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandApplyResult {
    pub commands: Vec<UserCommand>,
    pub stop_reason: CommandDrainStopReason,
    pub submitted_count: usize,
    pub applied_count: usize,
    pub rejected_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CommandApplyCounts {
    applied_count: usize,
    rejected_count: usize,
}

struct ModelActionFailure {
    error: LoopControllerError,
    attempted_tool_calls: u32,
    verification_started: bool,
}

pub struct ModelActionRuntime<'a> {
    pub registry: &'a ToolRegistry,
    pub gate: &'a PolicyGate,
    pub executor: &'a ReadOnlyToolExecutor,
    pub workspace: &'a std::path::Path,
}

impl<'a> ModelActionRuntime<'a> {
    pub fn new(
        registry: &'a ToolRegistry,
        gate: &'a PolicyGate,
        executor: &'a ReadOnlyToolExecutor,
        workspace: &'a std::path::Path,
    ) -> Self {
        Self {
            registry,
            gate,
            executor,
            workspace,
        }
    }
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
            checkpoint_store: None,
            side_effect_ledger: None,
            model_call_store: None,
            approval_store: None,
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

    /// Explicitly retrieves scoped memory candidate metadata for the current loop.
    ///
    /// This does not inject candidates into prompts or runtime context, validate source
    /// events, promote candidates, or grant authorization. The retrieval is recorded only
    /// as metadata in EventLog; if that audit write fails, no result is returned.
    pub fn retrieve_memory_candidates(
        &mut self,
        store: &MemoryStore,
        query: &MemoryQuery,
        retrieval_id: impl Into<String>,
    ) -> Result<Vec<MemoryCandidate>, LoopControllerError> {
        self.prepare_action()?;
        if query.project_id != self.goal.project_id
            || query
                .goal_id
                .as_deref()
                .is_some_and(|goal_id| goal_id != self.goal.goal_id)
            || query
                .task_id
                .as_deref()
                .is_some_and(|task_id| task_id != self.task_run.task_id)
        {
            return Err(LoopControllerError::MemoryQueryScopeMismatch);
        }

        let retrieval_id = retrieval_id.into();
        let attempt_id = self.task_run.current_attempt_id.clone();
        append_loop_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            "memory_retrieval_requested",
            &format!("{retrieval_id}:requested"),
            json!({
                "retrieval_id": retrieval_id,
                "project_id": query.project_id,
                "goal_id": query.goal_id,
                "task_id": query.task_id,
            }),
            attempt_id.clone(),
        )?;

        let candidates = store.read_catalog()?.retrieve(query);
        let candidate_ids = candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect::<Vec<_>>();
        append_loop_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            "memory_retrieval_filtered",
            &format!("{retrieval_id}:filtered"),
            json!({
                "retrieval_id": retrieval_id,
                "project_id": query.project_id,
                "goal_id": query.goal_id,
                "task_id": query.task_id,
                "candidate_ids": candidate_ids,
                "candidate_count": candidates.len(),
            }),
            attempt_id.clone(),
        )?;
        append_loop_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            "memory_retrieval_used",
            &format!("{retrieval_id}:used"),
            json!({
                "retrieval_id": retrieval_id,
                "candidate_ids": candidate_ids,
                "candidate_count": candidates.len(),
                "usage": "returned_metadata_only",
            }),
            attempt_id,
        )?;
        Ok(candidates)
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

    pub fn with_checkpoint_store(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.checkpoint_store = Some(CheckpointStore::open(path));
        self
    }

    pub fn with_model_call_store(mut self, store: ModelCallStore) -> Self {
        self.model_call_store = Some(store);
        self
    }

    pub fn with_approval_store(mut self, store: ApprovalStore) -> Self {
        self.approval_store = Some(store);
        self
    }

    pub fn approval_store(&self) -> Option<&ApprovalStore> {
        self.approval_store.as_ref()
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

    /// Polls one injected non-blocking command source and persists only a
    /// complete, validated command. Loop state changes remain deferred to the
    /// next action's existing safe-boundary processing.
    pub fn poll_and_submit_command<R>(
        &mut self,
        poller: &mut CommandPoller<R>,
    ) -> Result<CommandPollResult, LoopControllerError>
    where
        R: NonBlockingCommandReader,
    {
        let inbox = self
            .command_inbox
            .clone()
            .ok_or(LoopControllerError::CommandInboxRequired)?;
        let result = poller.poll()?;
        if let CommandPollResult::Command(command) = &result {
            inbox.submit(command.clone())?;
        }
        Ok(result)
    }

    /// Polls and persists at most `max_commands` complete commands for one
    /// runtime tick. Applying their effects remains deferred to the next
    /// action's existing safe boundary.
    pub fn poll_and_submit_commands<R>(
        &mut self,
        poller: &mut CommandPoller<R>,
        max_commands: usize,
    ) -> Result<CommandDrainResult, LoopControllerError>
    where
        R: NonBlockingCommandReader,
    {
        let inbox = self
            .command_inbox
            .clone()
            .ok_or(LoopControllerError::CommandInboxRequired)?;
        let result = poller.poll_many(max_commands)?;
        for command in &result.commands {
            inbox.submit(command.clone())?;
        }
        Ok(result)
    }

    /// Drains at most `max_commands`, stages those commands in the inbox, then
    /// applies all pending commands at the controller's existing safe boundary.
    pub fn poll_and_apply_commands<R>(
        &mut self,
        poller: &mut CommandPoller<R>,
        max_commands: usize,
    ) -> Result<CommandApplyResult, LoopControllerError>
    where
        R: NonBlockingCommandReader,
    {
        let inbox = self
            .command_inbox
            .clone()
            .ok_or(LoopControllerError::CommandInboxRequired)?;
        let tick = RuntimeTick::run(poller, max_commands, |command| {
            inbox.submit(command.clone()).map(|_| ())
        })
        .map_err(|error| match error {
            RuntimeTickError::Poll(error) => LoopControllerError::UserCommandInbox(error),
            RuntimeTickError::SafeBoundary { source, .. } => {
                LoopControllerError::UserCommandInbox(source)
            }
        })?;
        let counts = self.apply_pending_commands()?;
        Ok(CommandApplyResult {
            commands: tick.commands,
            stop_reason: tick.stop_reason,
            submitted_count: tick.applied_count,
            applied_count: counts.applied_count,
            rejected_count: counts.rejected_count,
        })
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
        let observation = redact_audit_text(&observation.into());
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
        let evidence_refs = self
            .task_run
            .attempt(&attempt_id)
            .map(|attempt| attempt.evidence_refs.clone())
            .unwrap_or_default();
        self.finish_task_run(
            &attempt_id,
            AttemptStatus::Succeeded,
            TaskRunStatus::Succeeded,
            "passed",
            Vec::new(),
            evidence_refs,
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

    pub fn run_model_step_with_action<A: ModelAdapter + ?Sized>(
        &mut self,
        adapter: &A,
        runtime: &ModelActionRuntime<'_>,
        model_id: impl Into<String>,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.run_model_step_with_route_and_action(
            model_id,
            step_id,
            observation,
            None,
            Some(runtime),
            true,
            |request| adapter.complete_with_action(request),
        )
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
        self.run_resolved_model_step_internal(adapter, request, pack, route, step_id, None)
    }

    pub fn run_resolved_model_step_with_action<T: ProviderTransport>(
        &mut self,
        adapter: &ProviderModelAdapter<T>,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
        route: &RouteDecision,
        runtime: &ModelActionRuntime<'_>,
        step_id: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.run_resolved_model_step_internal(adapter, request, pack, route, step_id, Some(runtime))
    }

    fn run_resolved_model_step_internal<T: ProviderTransport>(
        &mut self,
        adapter: &ProviderModelAdapter<T>,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
        route: &RouteDecision,
        step_id: impl Into<String>,
        action_runtime: Option<&ModelActionRuntime<'_>>,
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
        let step_id = step_id.into();
        let action_fingerprint = format!(
            "{}\n{}\n{}",
            request.model_id, self.goal.goal_id, request.observation
        );
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
        let model_call_id = format!("call-{}-{step_id}", self.loop_run.loop_run_id);
        let resolved = match adapter.complete_resolved_with_action(request, pack) {
            Ok(response) => response,
            Err(error) => {
                let (failure_stage, failure_class) = model_call_failure_metadata(&error);
                let failure_record = ModelCallFailureRecord {
                    schema_version: crate::model_contracts::MODEL_CALL_FAILURE_SCHEMA_VERSION,
                    model_call_id: model_call_id.clone(),
                    task_id: route.task_id.clone(),
                    task_run_id: self.loop_run.task_run_id.clone(),
                    agent_role: route.agent_role.clone(),
                    provider_id: pack.provider_id().into(),
                    endpoint_id: pack.endpoint_id().into(),
                    model_id: pack.model_id().into(),
                    route_decision_id: route.route_decision_id.clone(),
                    resolved_prompt_pack_id: pack.metadata().prompt_pack_id.clone(),
                    started_at: started_at.clone(),
                    ended_at: chrono::Utc::now().to_rfc3339(),
                    failure_stage: failure_stage.into(),
                    failure_class: failure_class.into(),
                    input_context_hash: None,
                    output_artifact_ref: None,
                    finish_reason: None,
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost: None,
                };
                if let Err(store_error) = store.append_failure(&failure_record) {
                    self.append_model_event(
                        "model_call_failed",
                        &format!("{step_id}:audit_failed"),
                        json!({
                            "error": redact_audit_text(&store_error.to_string()),
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
                        json!({
                            "summary": redact_audit_text(&store_error.to_string()),
                            "audit_failure": true,
                        }),
                        self.task_run.current_attempt_id.clone(),
                    )?;
                    self.report_progress(
                        ProgressSubject::Model,
                        model_progress_id.clone(),
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
                    return Err(LoopControllerError::ModelCallStore(store_error));
                }
                self.append_model_event(
                    "model_call_failed",
                    &format!("{step_id}:model_failed"),
                    json!({"error": redact_audit_text(&error.to_string()), "route_decision_id": route.route_decision_id}),
                )?;
                append_step_event(
                    &mut self.event_log,
                    &self.goal,
                    &self.loop_run,
                    &step,
                    "step_failed",
                    &format!("{step_id}:failed"),
                    json!({"summary": redact_audit_text(&error.to_string())}),
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
        let mut action_failure = None;
        let mut tool_call_count = 0;
        let mut verification_status = "not_run".to_owned();
        let mut error_class = None;
        if let Some(action) = resolved.response.action.as_ref() {
            let failure = match action_runtime {
                Some(runtime) => self
                    .execute_model_action_with_audit(
                        runtime.registry,
                        runtime.gate,
                        runtime.executor,
                        runtime.workspace,
                        action,
                        step_id.clone(),
                    )
                    .err(),
                None => Some(ModelActionFailure {
                    error: LoopControllerError::ModelAdapter(
                        ModelAdapterError::ProviderTextUnmappable,
                    ),
                    attempted_tool_calls: 0,
                    verification_started: false,
                }),
            };
            if let Some(failure) = failure {
                tool_call_count = failure.attempted_tool_calls;
                verification_status = if failure.verification_started {
                    "failed".into()
                } else {
                    "not_run".into()
                };
                error_class = Some(if failure.verification_started {
                    "verification_failed".into()
                } else {
                    model_action_error_class(&failure.error)
                });
                action_failure = Some(failure.error);
            } else {
                tool_call_count = 2;
                verification_status = "passed".into();
            }
        }
        let completed_at = chrono::Utc::now().to_rfc3339();
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
            tool_call_count,
            verification_status,
            error_class,
            started_at,
            completed_at: Some(completed_at),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(error) = store.append(&record) {
            self.append_model_event(
                "model_call_failed",
                &format!("{step_id}:audit_failed"),
                json!({
                    "error": redact_audit_text(&error.to_string()),
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
                json!({"summary": redact_audit_text(&error.to_string()), "audit_failure": true}),
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
        if let Some(error) = action_failure {
            self.append_model_event(
                "model_action_failed",
                &format!("{step_id}:action_failed"),
                json!({
                    "action_id": resolved.response.action.as_ref().map(|action| &action.action_id),
                    "error": redact_audit_text(&error.to_string()),
                    "verification_status": record.verification_status,
                }),
            )?;
            append_step_event(
                &mut self.event_log,
                &self.goal,
                &self.loop_run,
                &step,
                "step_failed",
                &format!("{step_id}:action_failed"),
                json!({"summary": redact_audit_text(&error.to_string())}),
                self.task_run.current_attempt_id.clone(),
            )?;
            self.report_progress(
                ProgressSubject::Model,
                model_progress_id,
                ProgressStatus::Failed,
                Some("model action failed"),
                None,
            )?;
            self.report_progress(
                ProgressSubject::Step,
                step.step_id.clone(),
                ProgressStatus::Failed,
                Some("action execution failed"),
                None,
            )?;
            return Err(error);
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
        self.run_model_step_with_route_and_action(
            model_id,
            step_id,
            observation,
            route_decision_id,
            None,
            true,
            |request| {
                adapter
                    .complete(request)
                    .map(|response| ModelResponseWithAction {
                        decision: response.decision,
                        summary: response.summary,
                        action: None,
                    })
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn run_model_step_with_route_and_action<F>(
        &mut self,
        model_id: impl Into<String>,
        step_id: impl Into<String>,
        observation: impl Into<String>,
        route_decision_id: Option<&str>,
        action_context: Option<&ModelActionRuntime<'_>>,
        check_no_progress: bool,
        complete: F,
    ) -> Result<ModelDecision, LoopControllerError>
    where
        F: FnOnce(ModelRequest) -> Result<ModelResponseWithAction, ModelAdapterError>,
    {
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
        let repeated_action = check_no_progress && self.next_step_no > 1;
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
        if check_no_progress && !repeated_action {
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
        let response = match complete(request) {
            Ok(response) => response,
            Err(error) => {
                self.append_model_event(
                    "model_call_failed",
                    &format!("{step_id}:model_failed"),
                    json!({"error": redact_audit_text(&error.to_string()), "route_decision_id": route_decision_id}),
                )?;
                append_step_event(
                    &mut self.event_log,
                    &self.goal,
                    &self.loop_run,
                    &step,
                    "step_failed",
                    &format!("{step_id}:failed"),
                    json!({"summary": redact_audit_text(&error.to_string())}),
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
        if let (Some(context), Some(action)) = (action_context, response.action.as_ref()) {
            if let Err(error) = self.execute_model_action(
                context.registry,
                context.gate,
                context.executor,
                context.workspace,
                action,
                step_id.clone(),
            ) {
                self.append_model_event(
                    "model_action_failed",
                    &format!("{step_id}:action_failed"),
                    json!({
                        "action_id": action.action_id,
                        "tool_id": action.tool_id,
                        "error": redact_audit_text(&error.to_string()),
                    }),
                )?;
                append_step_event(
                    &mut self.event_log,
                    &self.goal,
                    &self.loop_run,
                    &step,
                    "step_failed",
                    &format!("{step_id}:action_failed"),
                    json!({"summary": redact_audit_text(&error.to_string())}),
                    self.task_run.current_attempt_id.clone(),
                )?;
                self.report_progress(
                    ProgressSubject::Step,
                    step.step_id.clone(),
                    ProgressStatus::Failed,
                    Some("action execution failed"),
                    None,
                )?;
                return Err(error);
            }
        }
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
        let result = self.run_routed_model_step_inner(
            router,
            adapters,
            route_request,
            step_id.into(),
            observation.into(),
        );
        if matches!(
            &result,
            Err(LoopControllerError::EventLog(_) | LoopControllerError::ProgressReport(_))
        ) && self.loop_run.status == LoopStatus::Running
        {
            self.close_failed_routed_state()?;
        }
        result
    }

    fn run_routed_model_step_inner(
        &mut self,
        router: &ModelRouter<'_>,
        adapters: &ModelAdapterRegistry,
        mut route_request: RouteRequest,
        step_id: String,
        observation: String,
    ) -> Result<ModelDecision, LoopControllerError> {
        self.prepare_step_action()?;
        route_request.risk_level = max_route_risk(route_request.risk_level, self.goal.risk_level);
        let route_decision = router.route(&route_request)?;
        route_decision.validate()?;
        if !self.goal.allowed_models.is_empty()
            && !self
                .goal
                .allowed_models
                .iter()
                .any(|model_id| model_id == &route_decision.selected_model)
        {
            return Err(ModelRouterError::NoCompatibleModels {
                task_id: route_request.task_id,
                reasons: "selected model is outside goal allowed_models".into(),
            }
            .into());
        }
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
        let primary_model = route_decision.selected_model.clone();
        let primary_result = self.run_model_step_with_route(
            adapter,
            primary_model.clone(),
            step_id.clone(),
            observation.clone(),
            Some(&route_decision.route_decision_id),
        );
        let mut attempted_models = vec![primary_model];
        let mut result = primary_result;
        loop {
            let Err(error) = result else {
                return result;
            };
            if !is_retryable_model_error(&error) {
                if matches!(
                    error,
                    LoopControllerError::ModelAdapter(_) | LoopControllerError::BudgetExhausted(_)
                ) {
                    self.close_failed_routed_run()?;
                }
                return Err(error);
            }
            let Some(fallback_model) = next_fallback_model(&route_decision, &attempted_models)
            else {
                self.close_failed_routed_run()?;
                return Err(error);
            };
            if !self.goal.allowed_models.is_empty()
                && !self
                    .goal
                    .allowed_models
                    .iter()
                    .any(|allowed| allowed == fallback_model)
            {
                self.close_failed_routed_run()?;
                return Err(error);
            }
            let fallback_model = fallback_model.to_owned();
            let Some(fallback_adapter) = adapters.get(&fallback_model) else {
                self.close_failed_routed_run()?;
                return Err(LoopControllerError::ModelAdapterNotFound(fallback_model));
            };
            let fallback_index = attempted_models.len();
            let fallback_route_id = format!(
                "{}:fallback:{}",
                route_decision.route_decision_id, fallback_index
            );
            let fallback_step_id = format!("{step_id}:fallback:{fallback_index}");
            let parent_attempt_id =
                self.task_run.current_attempt_id.clone().ok_or_else(|| {
                    AttemptLineageError::AttemptNotFound("current_attempt_id".into())
                })?;
            let parent_evidence_refs = self
                .task_run
                .attempt(&parent_attempt_id)
                .map(|attempt| attempt.evidence_refs.clone())
                .unwrap_or_default();
            self.task_run.finish_attempt(
                &parent_attempt_id,
                AttemptStatus::Failed,
                "model_retryable_failure",
                Vec::new(),
                parent_evidence_refs,
            )?;
            let child_attempt_id = format!(
                "{}:attempt:fallback:{}",
                self.loop_run.loop_run_id,
                self.task_run.attempts.len()
            );
            let child_attempt = match self.task_run.start_attempt(
                child_attempt_id,
                AttemptReason::Fallback,
                Some(parent_attempt_id),
            ) {
                Ok(attempt) => attempt,
                Err(error) => {
                    self.close_failed_routed_run()?;
                    return Err(error.into());
                }
            };
            self.budget = self.task_run.budget.clone();
            if let Some(attempt) = self
                .task_run
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_id == child_attempt.attempt_id)
            {
                attempt.route_decision_id = Some(fallback_route_id.clone());
                self.task_run.object_version += 1;
            }
            self.append_model_event(
                "model_route_fallback",
                &format!("{fallback_route_id}:selected"),
                json!({
                    "route_decision_id": route_decision.route_decision_id,
                    "fallback_route_decision_id": fallback_route_id,
                    "from_model": attempted_models.last(),
                    "to_model": fallback_model,
                    "attempted_models": attempted_models,
                    "reason": "retryable_model_failure",
                    "error": redact_audit_text(&error.to_string()),
                }),
            )?;
            attempted_models.push(fallback_model.clone());
            result = self.run_model_step_with_route_and_action(
                fallback_model,
                fallback_step_id,
                observation.clone(),
                Some(&fallback_route_id),
                None,
                false,
                |request| {
                    fallback_adapter
                        .complete(request)
                        .map(|response| ModelResponseWithAction {
                            decision: response.decision,
                            summary: response.summary,
                            action: None,
                        })
                },
            );
        }
    }

    fn close_failed_routed_run(&mut self) -> Result<(), LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Ok(());
        }
        self.close_failed_routed_state()?;
        self.append_model_event(
            "loop_failed",
            &format!("{}:failed", self.loop_run.loop_run_id),
            json!({"status": "failed", "summary": "routed model step failed"}),
        )?;
        Ok(())
    }

    fn close_failed_routed_state(&mut self) -> Result<(), LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Ok(());
        }
        let attempt_id = self
            .task_run
            .current_attempt_id
            .clone()
            .ok_or_else(|| AttemptLineageError::AttemptNotFound("current_attempt_id".into()))?;
        let attempt = self
            .task_run
            .attempt(&attempt_id)
            .ok_or_else(|| AttemptLineageError::AttemptNotFound(attempt_id.clone()))?;
        if !attempt.status.is_terminal() {
            self.task_run.finish_attempt(
                &attempt_id,
                AttemptStatus::Failed,
                "model_step_failed",
                Vec::new(),
                attempt.evidence_refs.clone(),
            )?;
        }
        let finished_at = chrono::Utc::now().to_rfc3339();
        self.task_run.status = TaskRunStatus::Failed;
        self.task_run.finished_at = Some(finished_at.clone());
        self.sync_task_run_budget();
        self.loop_run.status = LoopStatus::Failed;
        self.loop_run.finished_at = Some(finished_at);
        self.loop_run.object_version += 1;
        Ok(())
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
            if self.policy_approval_allows(&request, &result, false, "tool-call-registration")? {
                // A previously approved scoped request authorized this call.
            } else {
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
            redacted_tool_success_payload(&input.tool_id, &output),
        )?;
        self.record_tool_audit(
            registry,
            &input.tool_id,
            &step_id,
            "completed",
            "tool call completed; output redacted",
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

    /// Execute one validated model action through policy, a fixed read-only
    /// executor, and its declared verification tool. No free-form shell input
    /// is accepted by this path.
    pub fn execute_model_action(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        executor: &ReadOnlyToolExecutor,
        workspace: &std::path::Path,
        action: &ModelActionEnvelope,
        step_id: impl Into<String>,
    ) -> Result<ModelActionExecution, LoopControllerError> {
        self.execute_model_action_with_audit(registry, gate, executor, workspace, action, step_id)
            .map_err(|failure| failure.error)
    }

    /// Execute the registered Markdown workspace-edit recipe after policy allows
    /// it. Autonomous bounded edits may proceed without a human marker; higher
    /// risk edits still require explicit approval. Verification remains read-only
    /// and all persisted payloads are metadata-only. Checkpoint and side-effect
    /// stores must be configured; pre-mutation audit errors prevent the edit.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_workspace_edit_action(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        workspace_executor: &WorkspaceToolExecutor,
        read_only_executor: &ReadOnlyToolExecutor,
        workspace: &std::path::Path,
        action: &ModelActionEnvelope,
        step_id: impl Into<String>,
        approved: bool,
    ) -> Result<ToolCallOutput, LoopControllerError> {
        self.prepare_action()?;
        action.validate()?;
        workspace_executor
            .map_action(registry, workspace, action)
            .map_err(|error| LoopControllerError::WorkspaceToolExecution(error.to_string()))?;
        let step_id = step_id.into();
        let request = PolicyRequest {
            policy_event_id: format!("{step_id}:policy"),
            tool_id: action.tool_id.clone(),
            project_id: self.goal.project_id.clone(),
            goal_id: self.goal.goal_id.clone(),
            task_run_id: self.loop_run.task_run_id.clone(),
            loop_run_id: self.loop_run.loop_run_id.clone(),
            step_id: step_id.clone(),
            correlation_id: self.loop_run.loop_run_id.clone(),
            human_steering_mode: self.goal.human_steering_mode,
        };
        let decision = gate.evaluate(registry, &self.goal, &request);
        gate.record_decision(&mut self.event_log, &request, &decision)?;
        // Bind authority to the complete validated action, not model summary text.
        // Only the digest is persisted; the parameters never enter the approval log.
        let action_bytes = serde_json::to_vec(action).map_err(|_| {
            LoopControllerError::WorkspaceToolExecution("action binding failed".into())
        })?;
        let capability = format!("workspace-edit:{:x}", Sha256::digest(action_bytes));
        let permitted = match decision.decision {
            PolicyDecision::Allow => true,
            PolicyDecision::RequireHumanApproval => {
                self.policy_approval_allows(&request, &decision, approved, &capability)?
            }
            PolicyDecision::RequireCheckpoint | PolicyDecision::Warn | PolicyDecision::Block => {
                false
            }
        };
        if !permitted {
            self.append_model_event(
                "model_action_failed",
                &format!("{step_id}:action_failed"),
                json!({"action_id": action.action_id, "tool_id": action.tool_id, "phase": "approval"}),
            )?;
            return Err(LoopControllerError::PolicyDenied {
                decision: decision.decision,
                reason: if decision.decision == PolicyDecision::RequireHumanApproval {
                    "workspace edit approval was not provided".into()
                } else {
                    "workspace edit was denied by policy".into()
                },
            });
        }
        // Authorization alone cannot make an unaudited mutation safe. Keep
        // these requirements local to workspace edits, not read-only tools.
        let checkpoint_store = self.checkpoint_store.as_ref().ok_or_else(|| {
            LoopControllerError::WorkspaceToolExecution(
                "workspace edit requires a checkpoint store".into(),
            )
        })?;
        if self.side_effect_ledger.is_none() {
            return Err(LoopControllerError::WorkspaceToolExecution(
                "workspace edit requires a side-effect ledger".into(),
            ));
        }
        let execution_approved = permitted;
        let relative_path = action
            .parameters
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                LoopControllerError::WorkspaceToolExecution("missing edit path".into())
            })?;
        let before_hash = workspace_file_hash(workspace, relative_path)?;
        checkpoint_store.append(&CheckpointMetadata {
            checkpoint_id: format!("{step_id}:checkpoint:before-write"),
            goal_id: self.goal.goal_id.clone(),
            loop_run_id: self.loop_run.loop_run_id.clone(),
            step_id: Some(step_id.clone()),
            files: vec![relative_path.to_owned()],
            diff_boundary: format!("before={before_hash};after=pending"),
            recoverability: "local_workspace_restore_or_escalate".into(),
            side_effect_refs: vec![format!("{step_id}:side-effect")],
            created_at: chrono::Utc::now().to_rfc3339(),
        })?;
        self.append_model_event(
            "model_action_started",
            &format!("{step_id}:action_started"),
            json!({"action_id": action.action_id, "tool_id": action.tool_id}),
        )?;
        self.record_tool_audit(
            registry,
            &action.tool_id,
            &step_id,
            "started",
            "workspace edit approved; mutation started",
        )?;
        let _action_output = match workspace_executor.execute(
            registry,
            workspace,
            &self.goal,
            ToolCallInput {
                tool_id: action.tool_id.clone(),
                arguments: action.parameters.clone(),
            },
            execution_approved,
        ) {
            Ok(output) => output,
            Err(error) => {
                self.record_tool_audit(
                    registry,
                    &action.tool_id,
                    &step_id,
                    "failed",
                    "workspace edit failed before verification",
                )?;
                self.append_model_event(
                    "model_action_failed",
                    &format!("{step_id}:action_failed"),
                    json!({"action_id": action.action_id, "tool_id": action.tool_id, "phase": "execute"}),
                )?;
                if let Some(store) = &self.checkpoint_store {
                    let after_boundary = workspace_file_hash(workspace, relative_path)
                        .unwrap_or_else(|_| "unavailable".into());
                    store.append(&CheckpointMetadata {
                        checkpoint_id: format!("{step_id}:checkpoint:write-failed"),
                        goal_id: self.goal.goal_id.clone(),
                        loop_run_id: self.loop_run.loop_run_id.clone(),
                        step_id: Some(step_id.clone()),
                        files: vec![relative_path.to_owned()],
                        diff_boundary: format!("before={before_hash};after={after_boundary}"),
                        recoverability: "write_failed_local_workspace_restore_or_escalate".into(),
                        side_effect_refs: vec![format!("{step_id}:side-effect")],
                        created_at: chrono::Utc::now().to_rfc3339(),
                    })?;
                }
                return Err(LoopControllerError::WorkspaceToolExecution(
                    error.to_string(),
                ));
            }
        };
        let after_hash = workspace_file_hash(workspace, relative_path)?;
        self.append_model_event(
            "file_change_recorded",
            &format!("{step_id}:file_change_recorded"),
            json!({"action_id": action.action_id, "tool_id": action.tool_id, "path": action.parameters.get("path").and_then(|value| value.as_str())}),
        )?;
        self.record_tool_audit(
            registry,
            &action.tool_id,
            &step_id,
            "completed",
            "workspace edit completed; output redacted",
        )?;
        let verification_action = ModelActionEnvelope {
            schema_version: action.schema_version,
            action_id: action.action_id.clone(),
            tool_id: action.verification.verifier_tool_id.clone(),
            parameters: action.verification.parameters.clone(),
            path_scope: action.path_scope.clone(),
            verification: VerificationContract {
                verifier_tool_id: action.verification.verifier_tool_id.clone(),
                parameters: action.verification.parameters.clone(),
            },
        };
        let verification_output = match self.execute_read_only_tool(
            registry,
            gate,
            read_only_executor,
            workspace,
            ToolCallInput {
                tool_id: verification_action.tool_id,
                arguments: verification_action.parameters,
            },
            format!("{step_id}:verification"),
        ) {
            Ok(output) => output,
            Err(error) => {
                self.record_tool_audit(
                    registry,
                    &action.tool_id,
                    &step_id,
                    "verification_failed",
                    "workspace edit verification failed",
                )?;
                self.append_model_event(
                    "model_action_failed",
                    &format!("{step_id}:verification_failed"),
                    json!({"action_id": action.action_id, "tool_id": action.tool_id, "phase": "verification"}),
                )?;
                if let Some(store) = &self.checkpoint_store {
                    store.append(&CheckpointMetadata {
                        checkpoint_id: format!("{step_id}:checkpoint:verification-failed"),
                        goal_id: self.goal.goal_id.clone(),
                        loop_run_id: self.loop_run.loop_run_id.clone(),
                        step_id: Some(step_id.clone()),
                        files: vec![relative_path.to_owned()],
                        diff_boundary: format!("before={before_hash};after={after_hash}"),
                        recoverability: "verification_failed_local_workspace_restore_or_escalate"
                            .into(),
                        side_effect_refs: vec![format!("{step_id}:side-effect")],
                        created_at: chrono::Utc::now().to_rfc3339(),
                    })?;
                }
                return Err(error);
            }
        };
        if verification_output
            .result
            .get("missing_local_links")
            .and_then(|value| value.as_array())
            .is_some_and(|missing| !missing.is_empty())
        {
            self.record_tool_audit(
                registry,
                &action.tool_id,
                &step_id,
                "verification_failed",
                "workspace edit verification reported missing local links",
            )?;
            self.append_model_event(
                "model_action_failed",
                &format!("{step_id}:verification_failed"),
                json!({"action_id": action.action_id, "tool_id": action.tool_id, "phase": "verification"}),
            )?;
            if let Some(store) = &self.checkpoint_store {
                store.append(&CheckpointMetadata {
                    checkpoint_id: format!("{step_id}:checkpoint:verification-failed"),
                    goal_id: self.goal.goal_id.clone(),
                    loop_run_id: self.loop_run.loop_run_id.clone(),
                    step_id: Some(step_id.clone()),
                    files: vec![relative_path.to_owned()],
                    diff_boundary: format!("before={before_hash};after={after_hash}"),
                    recoverability: "verification_failed_local_workspace_restore_or_escalate"
                        .into(),
                    side_effect_refs: vec![format!("{step_id}:side-effect")],
                    created_at: chrono::Utc::now().to_rfc3339(),
                })?;
            }
            return Err(LoopControllerError::WorkspaceToolExecution(
                "workspace edit verification reported missing local links".into(),
            ));
        }
        if let Some(store) = &self.checkpoint_store {
            store.append(&CheckpointMetadata {
                checkpoint_id: format!("{step_id}:checkpoint:after-write"),
                goal_id: self.goal.goal_id.clone(),
                loop_run_id: self.loop_run.loop_run_id.clone(),
                step_id: Some(step_id.clone()),
                files: vec![relative_path.to_owned()],
                diff_boundary: format!("before={before_hash};after={after_hash}"),
                recoverability: "verified_local_workspace_restore_or_escalate".into(),
                side_effect_refs: vec![format!("{step_id}:side-effect")],
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
        }
        self.append_model_event(
            "model_action_verified",
            &format!("{step_id}:action_verified"),
            json!({"action_id": action.action_id, "tool_id": action.tool_id, "verifier_tool_id": action.verification.verifier_tool_id}),
        )?;
        Ok(ToolCallOutput {
            summary: format!(
                "workspace edit completed; verification: {}",
                verification_output.summary
            ),
            result: serde_json::json!({"action_id": action.action_id, "verified": true}),
        })
    }

    fn execute_model_action_with_audit(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        executor: &ReadOnlyToolExecutor,
        workspace: &std::path::Path,
        action: &ModelActionEnvelope,
        step_id: impl Into<String>,
    ) -> Result<ModelActionExecution, ModelActionFailure> {
        if let Err(error) = action.validate() {
            return Err(ModelActionFailure {
                error: error.into(),
                attempted_tool_calls: 0,
                verification_started: false,
            });
        }
        if let Err(error) = executor.map_action(registry, workspace, action) {
            return Err(ModelActionFailure {
                error: error.into(),
                attempted_tool_calls: 0,
                verification_started: false,
            });
        }
        let step_id = step_id.into();
        if let Err(error) = self.append_model_event(
            "model_action_started",
            &format!("{step_id}:action_started"),
            json!({"action_id": action.action_id, "tool_id": action.tool_id}),
        ) {
            return Err(ModelActionFailure {
                error,
                attempted_tool_calls: 0,
                verification_started: false,
            });
        }
        let action_output = match self.execute_read_only_tool(
            registry,
            gate,
            executor,
            workspace,
            ToolCallInput {
                tool_id: action.tool_id.clone(),
                arguments: action.parameters.clone(),
            },
            step_id.clone(),
        ) {
            Ok(output) => output,
            Err(error) => {
                return Err(ModelActionFailure {
                    error,
                    attempted_tool_calls: 1,
                    verification_started: false,
                })
            }
        };

        let verification_action = ModelActionEnvelope {
            schema_version: action.schema_version,
            action_id: action.action_id.clone(),
            tool_id: action.verification.verifier_tool_id.clone(),
            parameters: action.verification.parameters.clone(),
            path_scope: action.path_scope.clone(),
            verification: VerificationContract {
                verifier_tool_id: action.verification.verifier_tool_id.clone(),
                parameters: action.verification.parameters.clone(),
            },
        };
        if let Err(error) = executor.map_action(registry, workspace, &verification_action) {
            return Err(ModelActionFailure {
                error: error.into(),
                attempted_tool_calls: 1,
                verification_started: true,
            });
        }
        let verification_output = match self.execute_read_only_tool(
            registry,
            gate,
            executor,
            workspace,
            ToolCallInput {
                tool_id: verification_action.tool_id.clone(),
                arguments: verification_action.parameters.clone(),
            },
            format!("{step_id}:verification"),
        ) {
            Ok(output) => output,
            Err(error) => {
                return Err(ModelActionFailure {
                    error,
                    attempted_tool_calls: 2,
                    verification_started: true,
                })
            }
        };
        if let Err(error) = self.append_model_event(
            "model_action_verified",
            &format!("{step_id}:action_verified"),
            json!({
                "action_id": action.action_id,
                "tool_id": action.tool_id,
                "verifier_tool_id": action.verification.verifier_tool_id,
            }),
        ) {
            return Err(ModelActionFailure {
                error,
                attempted_tool_calls: 2,
                verification_started: true,
            });
        }
        Ok(ModelActionExecution {
            action_id: action.action_id.clone(),
            action_output,
            verification_output,
        })
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
            redact_audit_payload(payload),
        )?;
        event.project_id = Some(self.goal.project_id.clone());
        event.goal_id = Some(self.goal.goal_id.clone());
        event.task_run_id = Some(self.loop_run.task_run_id.clone());
        event.attempt_id = self.task_run.current_attempt_id.clone();
        event.loop_run_id = Some(self.loop_run.loop_run_id.clone());
        let route_decision_id = (event_type == "model_route_decided")
            .then(|| {
                event.payload["route_decision_id"]
                    .as_str()
                    .map(str::to_owned)
            })
            .flatten();
        let sequence = self.event_log.append(event)?;
        if matches!(
            event_type,
            "model_route_decided"
                | "model_route_fallback"
                | "model_call_started"
                | "model_call_failed"
                | "model_call_completed"
                | "loop_budget_exhausted"
                | "no_progress_detected"
                | "loop_failed"
        ) {
            if let Some(attempt) = self.task_run.attempts.iter_mut().find(|attempt| {
                Some(attempt.attempt_id.as_str()) == self.task_run.current_attempt_id.as_deref()
            }) {
                if let Some(route_decision_id) = route_decision_id {
                    attempt.route_decision_id = Some(route_decision_id);
                }
                if !attempt
                    .evidence_refs
                    .iter()
                    .any(|reference| reference == event_id)
                {
                    attempt.evidence_refs.push(event_id.to_owned());
                }
                self.task_run.object_version += 1;
            }
        }
        Ok(sequence)
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

    fn policy_approval_allows(
        &mut self,
        request: &PolicyRequest,
        result: &crate::policy_gate::PolicyResult,
        explicitly_approved: bool,
        capability: &str,
    ) -> Result<bool, LoopControllerError> {
        if result.decision != PolicyDecision::RequireHumanApproval {
            return Ok(false);
        }
        let Some(store) = &self.approval_store else {
            return Ok(explicitly_approved);
        };
        let request_id = format!(
            "{}:{}:{}:approval",
            self.loop_run.loop_run_id, request.step_id, request.tool_id
        );
        let now = chrono::Utc::now();
        let approval = ApprovalRequest::at(
            request_id.clone(),
            request.project_id.clone(),
            self.loop_run.loop_run_id.clone(),
            request.task_run_id.clone(),
            request.tool_id.clone(),
            capability,
            "metadata_only",
            "policy requires approval for a registered operation",
            now,
            now + chrono::Duration::minutes(15),
        )?;
        let is_new = store.latest(&request_id).is_none();
        let record = store.submit(approval.clone())?;
        if matches!(
            record.status,
            ApprovalStatus::Rejected | ApprovalStatus::Expired | ApprovalStatus::Consumed
        ) {
            return Err(LoopControllerError::PolicyDenied {
                decision: PolicyDecision::RequireHumanApproval,
                reason: "approval is rejected, expired, or already consumed".into(),
            });
        }
        if store.consume(&approval)? {
            return Ok(true);
        }
        if is_new {
            self.append_model_event(
                "human_intervention_required",
                &format!("{request_id}:pending"),
                json!({
                    "approval_request_id": request_id,
                    "status": "pending_human",
                    "tool_id": request.tool_id,
                    "reason": "policy requires approval",
                }),
            )?;
        }
        Err(LoopControllerError::ApprovalRequired { request_id })
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

    fn apply_pending_commands(&mut self) -> Result<CommandApplyCounts, LoopControllerError> {
        let Some(inbox) = self.command_inbox.clone() else {
            return Ok(CommandApplyCounts::default());
        };
        let commands = inbox.pending_for_loop(&self.goal.project_id, &self.loop_run.loop_run_id)?;
        let mut counts = CommandApplyCounts::default();
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
                counts.rejected_count += 1;
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
                counts.applied_count += 1;
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
                counts.rejected_count += 1;
            }
        }
        Ok(counts)
    }
}

fn max_route_risk(route_risk: RouteRiskLevel, goal_risk: RiskLevel) -> RouteRiskLevel {
    let goal_risk = match goal_risk {
        RiskLevel::Low => RouteRiskLevel::Low,
        RiskLevel::Medium => RouteRiskLevel::Medium,
        RiskLevel::High => RouteRiskLevel::High,
        RiskLevel::Critical => RouteRiskLevel::Critical,
    };
    route_risk.max(goal_risk)
}

fn is_retryable_model_error(error: &LoopControllerError) -> bool {
    matches!(
        error,
        LoopControllerError::ModelAdapter(ModelAdapterError::Provider {
            class: ProviderErrorClass::RateLimit
                | ProviderErrorClass::Timeout
                | ProviderErrorClass::Transport
                | ProviderErrorClass::Upstream,
        })
    )
}

fn model_call_failure_metadata(error: &ModelAdapterError) -> (&'static str, &'static str) {
    match error {
        ModelAdapterError::Provider { class } => (
            "provider_invocation",
            match class {
                ProviderErrorClass::InvalidRequest => "invalid_request",
                ProviderErrorClass::Authentication => "authentication",
                ProviderErrorClass::Permission => "permission",
                ProviderErrorClass::RateLimit => "rate_limit",
                ProviderErrorClass::Timeout => "timeout",
                ProviderErrorClass::Transport => "transport",
                ProviderErrorClass::Upstream => "upstream",
                ProviderErrorClass::ResponseParse => "response_parse",
            },
        ),
        ModelAdapterError::ProviderTextUnmappable => ("provider_response", "response_unmappable"),
        ModelAdapterError::EmptyModelId | ModelAdapterError::EmptyObjective => {
            ("provider_request", "invalid_request")
        }
        ModelAdapterError::ResolvedBindingMismatch { .. } => {
            ("provider_request", "binding_mismatch")
        }
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
        ToolExecutionError::UnsafeToolProfile(_) => FailureCategory::Permission,
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
        "summary": redact_audit_text(&error.to_string()),
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

fn redacted_tool_success_payload(tool_id: &str, output: &ToolCallOutput) -> serde_json::Value {
    let result = &output.result;
    json!({
        "tool_id": tool_id,
        "executed": result.get("executed").and_then(serde_json::Value::as_bool),
        "exit_code": result.get("exit_code"),
        "stdout_truncated": result.get("stdout_truncated"),
        "stderr_truncated": result.get("stderr_truncated"),
        "output_redacted": true,
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
        redact_audit_payload(payload),
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

fn model_action_error_class(error: &LoopControllerError) -> String {
    match error {
        LoopControllerError::PolicyDenied { .. } => "policy_denied".into(),
        LoopControllerError::ToolExecution(_) => "tool_execution".into(),
        LoopControllerError::ModelContract(_) => "model_contract".into(),
        LoopControllerError::PolicyGate(_) => "policy_gate".into(),
        LoopControllerError::ModelAdapter(_) => "model_adapter".into(),
        _ => "action_execution".into(),
    }
}

fn workspace_file_hash(
    workspace: &std::path::Path,
    relative_path: &str,
) -> Result<String, LoopControllerError> {
    let contents = std::fs::read(workspace.join(relative_path)).map_err(|error| {
        LoopControllerError::WorkspaceToolExecution(format!(
            "workspace file hash failed for {relative_path}: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(relative_path.as_bytes());
    hasher.update([0]);
    hasher.update(contents);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn redact_audit_text(value: &str) -> String {
    Redactor::default().redact_text_fail_closed(value)
}

fn redact_audit_payload(payload: serde_json::Value) -> serde_json::Value {
    Redactor::default()
        .redact_json_value(&payload)
        .unwrap_or_else(|_| json!({"redaction": FAIL_CLOSED_MARKER}))
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
        redact_audit_payload(payload),
    )?;
    event.project_id = Some(goal.project_id.clone());
    event.goal_id = Some(goal.goal_id.clone());
    event.task_run_id = Some(loop_run.task_run_id.clone());
    event.attempt_id = attempt_id;
    event.loop_run_id = Some(loop_run.loop_run_id.clone());
    event.step_id = Some(step.step_id.clone());
    Ok(event_log.append(event)?)
}
