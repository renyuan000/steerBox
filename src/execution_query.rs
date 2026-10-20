use crate::domain::{EventEnvelope, LoopStatus};
use crate::event_log::{EventLog, EventLogError};
use crate::state_store::{LoopState, StateStore, StateStoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Placement of an event in the detached execution surface.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEventSurface {
    Current,
    LogOnly,
}

/// Safe event metadata for history views. Payloads are intentionally omitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionEventRecord {
    pub loop_run_id: String,
    pub event_id: String,
    pub sequence_no: Option<u64>,
    pub event_type: String,
    pub occurred_at: String,
    pub surface: ExecutionEventSurface,
}

/// Detached identity and current state for one logical loop execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub loop_run_id: String,
    pub project_id: Option<String>,
    pub goal_id: String,
    pub task_run_id: String,
    pub status: LoopStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub event_count: usize,
    pub captured_through_sequence: Option<u64>,
}

/// One detached observation of an execution's current model-facing history surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionSurfaceSnapshot {
    pub execution: ExecutionRecord,
    pub captured_through_seq: Option<u64>,
    pub events: Vec<ExecutionEventRecord>,
}

/// A bounded page over one detached execution snapshot.
/// `offset` and `limit` are snapshot-local positions, not a live cursor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionEventPage {
    pub loop_run_id: String,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
    pub events: Vec<ExecutionEventRecord>,
}

#[derive(Debug, Error)]
pub enum ExecutionQueryError {
    #[error("execution state projection failed: {0}")]
    State(#[from] StateStoreError),
    #[error("execution event log read failed: {0}")]
    EventLog(#[from] EventLogError),
}

/// Read-only, detached query boundary over an immutable event observation.
/// It never returns event payloads and does not subscribe to future writes.
#[derive(Debug, Clone)]
pub struct ExecutionQuery {
    events: Vec<EventEnvelope>,
    state: StateStore,
}

impl ExecutionQuery {
    pub fn from_events(events: &[EventEnvelope]) -> Result<Self, ExecutionQueryError> {
        Ok(Self {
            events: events.to_vec(),
            state: StateStore::rebuild(events)?,
        })
    }

    pub fn from_event_log(log: &EventLog) -> Result<Self, ExecutionQueryError> {
        Self::from_events(&log.read_all()?)
    }

    pub fn list(&self) -> Vec<ExecutionRecord> {
        self.state
            .loops()
            .map(|loop_state| self.record_from_state(loop_state))
            .collect()
    }

    pub fn record(&self, loop_run_id: &str) -> Option<ExecutionRecord> {
        self.state
            .loop_run(loop_run_id)
            .map(|loop_state| self.record_from_state(loop_state))
    }

    pub fn surface_snapshot(&self, loop_run_id: &str) -> Option<ExecutionSurfaceSnapshot> {
        let execution = self.record(loop_run_id)?;
        let events = self.event_records(loop_run_id);
        let captured_through_seq = events.iter().filter_map(|event| event.sequence_no).max();

        Some(ExecutionSurfaceSnapshot {
            execution,
            captured_through_seq,
            events,
        })
    }

    pub fn events_page(
        &self,
        loop_run_id: &str,
        offset: usize,
        limit: usize,
    ) -> Option<ExecutionEventPage> {
        self.record(loop_run_id)?;
        let all_events = self.event_records(loop_run_id);
        let total = all_events.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        let events = all_events[start..end].to_vec();
        let has_more = limit != 0 && end < total;

        Some(ExecutionEventPage {
            loop_run_id: loop_run_id.to_owned(),
            offset,
            limit,
            total,
            has_more,
            events,
        })
    }

    fn event_records(&self, loop_run_id: &str) -> Vec<ExecutionEventRecord> {
        self.events
            .iter()
            .filter(|event| event_belongs_to_loop(event, loop_run_id))
            .map(|event| ExecutionEventRecord {
                loop_run_id: loop_run_id.to_owned(),
                event_id: event.event_id.clone(),
                sequence_no: event.sequence_no,
                event_type: event.event_type.clone(),
                occurred_at: event.occurred_at.clone(),
                surface: if is_current_surface_event(&event.event_type) {
                    ExecutionEventSurface::Current
                } else {
                    ExecutionEventSurface::LogOnly
                },
            })
            .collect()
    }

    fn record_from_state(&self, loop_state: &LoopState) -> ExecutionRecord {
        let loop_events = self
            .events
            .iter()
            .filter(|event| event_belongs_to_loop(event, &loop_state.loop_run_id))
            .collect::<Vec<_>>();
        let project_id = loop_events
            .iter()
            .find_map(|event| event.project_id.clone());
        let captured_through_sequence = loop_events
            .iter()
            .filter_map(|event| event.sequence_no)
            .max();

        ExecutionRecord {
            loop_run_id: loop_state.loop_run_id.clone(),
            project_id,
            goal_id: loop_state.goal_id.clone(),
            task_run_id: loop_state.task_run_id.clone(),
            status: loop_state.status,
            started_at: loop_state.started_at.clone(),
            finished_at: loop_state.finished_at.clone(),
            event_count: loop_events.len(),
            captured_through_sequence,
        }
    }
}

fn event_loop_run_id(event: &EventEnvelope) -> Option<String> {
    event
        .loop_run_id
        .clone()
        .or_else(|| {
            event
                .payload
                .get("loop_run_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| (event.aggregate_type == "loop_run").then(|| event.aggregate_id.clone()))
}

fn event_belongs_to_loop(event: &EventEnvelope, loop_run_id: &str) -> bool {
    event_loop_run_id(event).as_deref() == Some(loop_run_id)
}

fn is_current_surface_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "loop_started"
            | "loop_completed"
            | "loop_failed"
            | "loop_paused"
            | "loop_resumed"
            | "loop_cancelled"
            | "step_started"
            | "step_completed"
            | "step_failed"
            | "model_call_started"
            | "model_call_completed"
            | "model_call_failed"
            | "model_quality_assessed"
            | "tool_call_requested"
            | "tool_call_completed"
            | "tool_call_failed"
            | "policy_decision"
            | "human_intervention_required"
            | "user_command_received"
            | "user_command_applied"
            | "checkpoint_created"
            | "verification_completed"
    )
}
