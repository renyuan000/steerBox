use crate::domain::{
    BudgetKind, EventEnvelope, EventEnvelopeError, GoalContract, LoopBudget, LoopRun, LoopStatus,
    LoopStep,
};
use crate::event_log::{EventLog, EventLogError};
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
}

/// Minimal sequential loop that records observations only.
#[derive(Debug)]
pub struct ReadOnlyLoopController {
    goal: GoalContract,
    loop_run: LoopRun,
    budget: LoopBudget,
    event_log: EventLog,
    next_step_no: u32,
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
        })
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
