use crate::audit_store::{
    AuditStoreError, SideEffectLedger, SideEffectRecord, TraceRecord, TraceStore,
};
use crate::domain::{
    BudgetKind, EventEnvelope, EventEnvelopeError, GoalContract, LoopBudget, LoopRun, LoopStatus,
    LoopStep, NoProgressTracker,
};
use crate::event_log::{EventLog, EventLogError};
use crate::model_adapter::{ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest};
use crate::policy_gate::{PolicyDecision, PolicyGate, PolicyGateError, PolicyRequest};
use crate::tool_executor::{
    ReadOnlyToolExecutor, ToolCallInput, ToolCallOutput, ToolExecutionError,
};
use crate::tool_registry::ToolRegistry;
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
    #[error("loop is already terminal")]
    Terminal,
    #[error("model adapter failed: {0}")]
    ModelAdapter(#[from] ModelAdapterError),
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
}

/// Minimal sequential loop that records observations only.
#[derive(Debug)]
pub struct ReadOnlyLoopController {
    goal: GoalContract,
    loop_run: LoopRun,
    budget: LoopBudget,
    event_log: EventLog,
    next_step_no: u32,
    no_progress: NoProgressTracker,
    trace_store: Option<TraceStore>,
    side_effect_ledger: Option<SideEffectLedger>,
}

impl ReadOnlyLoopController {
    pub fn start(
        mut event_log: EventLog,
        goal: GoalContract,
        loop_run_id: impl Into<String>,
        task_run_id: impl Into<String>,
        budget: LoopBudget,
    ) -> Result<Self, LoopControllerError> {
        let loop_run = LoopRun::new(loop_run_id, goal.goal_id.clone(), task_run_id);
        let max_no_progress_steps = budget.max_no_progress_steps;
        let max_same_action_repetitions = budget.max_same_action_repetitions;
        append_loop_event(
            &mut event_log,
            &goal,
            &loop_run,
            "loop_started",
            &format!("{}:started", loop_run.loop_run_id),
            json!({"status": "running"}),
        )?;
        Ok(Self {
            goal,
            loop_run,
            budget,
            event_log,
            next_step_no: 1,
            no_progress: NoProgressTracker::new(max_no_progress_steps, max_same_action_repetitions),
            trace_store: None,
            side_effect_ledger: None,
        })
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

    pub fn run_observation(
        &mut self,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<LoopStep, LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Err(LoopControllerError::Terminal);
        }
        if let Err(kind) = self.budget.consume_step() {
            append_loop_event(
                &mut self.event_log,
                &self.goal,
                &self.loop_run,
                "loop_budget_exhausted",
                &format!("{}:budget:step", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
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
        )?;
        Ok(step)
    }

    pub fn complete(&mut self) -> Result<LoopRun, LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Err(LoopControllerError::Terminal);
        }
        self.loop_run.complete();
        append_loop_event(
            &mut self.event_log,
            &self.goal,
            &self.loop_run,
            "loop_completed",
            &format!("{}:completed", self.loop_run.loop_run_id),
            json!({"status": "completed"}),
        )?;
        Ok(self.loop_run.clone())
    }

    pub fn run_model_step<A: ModelAdapter>(
        &mut self,
        adapter: &A,
        model_id: impl Into<String>,
        step_id: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<ModelDecision, LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Err(LoopControllerError::Terminal);
        }
        if let Err(kind) = self.budget.consume_step() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:step", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
        if let Err(kind) = self.budget.consume_model_call() {
            self.append_model_event(
                "loop_budget_exhausted",
                &format!("{}:budget:model_call", self.loop_run.loop_run_id),
                json!({"budget_kind": format!("{kind:?}")}),
            )?;
            return Err(LoopControllerError::BudgetExhausted(kind));
        }
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
        )?;
        let request = ModelRequest {
            model_id,
            goal_id: self.goal.goal_id.clone(),
            loop_run_id: self.loop_run.loop_run_id.clone(),
            step_id: step_id.clone(),
            objective: self.goal.objective.clone(),
            observation,
        };
        self.append_model_event(
            "model_call_started",
            &format!("{step_id}:model_started"),
            json!({"model_id": request.model_id, "step_id": step_id}),
        )?;
        let response = match adapter.complete(request) {
            Ok(response) => response,
            Err(error) => {
                self.append_model_event(
                    "model_call_failed",
                    &format!("{step_id}:model_failed"),
                    json!({"error": error.to_string()}),
                )?;
                return Err(LoopControllerError::ModelAdapter(error));
            }
        };
        let decision = response.decision;
        let summary = response.summary.clone();
        self.append_model_event(
            "model_call_completed",
            &format!("{step_id}:model_completed"),
            json!({"decision": decision, "summary": summary}),
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
        )?;
        if decision == ModelDecision::CompleteLoop {
            self.complete()?;
        }
        Ok(decision)
    }

    pub fn request_tool_call(
        &mut self,
        registry: &ToolRegistry,
        gate: &PolicyGate,
        tool_id: impl Into<String>,
        step_id: impl Into<String>,
    ) -> Result<(), LoopControllerError> {
        if self.loop_run.status != LoopStatus::Running {
            return Err(LoopControllerError::Terminal);
        }
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
            return Err(LoopControllerError::PolicyDenied {
                decision: result.decision,
                reason: result.reason,
            });
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
                self.append_model_event(
                    "tool_call_failed",
                    &format!("{step_id}:tool_failed"),
                    json!({"tool_id": input.tool_id, "error": error.to_string()}),
                )?;
                self.record_tool_audit(
                    registry,
                    &input.tool_id,
                    &step_id,
                    "failed",
                    &error.to_string(),
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
        event.loop_run_id = Some(self.loop_run.loop_run_id.clone());
        Ok(self.event_log.append(event)?)
    }

    pub fn status(&self) -> LoopStatus {
        self.loop_run.status
    }
}

fn append_loop_event(
    event_log: &mut EventLog,
    goal: &GoalContract,
    loop_run: &LoopRun,
    event_type: &str,
    event_id: &str,
    payload: serde_json::Value,
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
    event.loop_run_id = Some(loop_run.loop_run_id.clone());
    Ok(event_log.append(event)?)
}

fn append_step_event(
    event_log: &mut EventLog,
    goal: &GoalContract,
    loop_run: &LoopRun,
    step: &LoopStep,
    event_type: &str,
    event_id: &str,
    payload: serde_json::Value,
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
    event.loop_run_id = Some(loop_run.loop_run_id.clone());
    event.step_id = Some(step.step_id.clone());
    Ok(event_log.append(event)?)
}
