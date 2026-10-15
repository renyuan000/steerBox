use crate::domain::{EventEnvelope, LoopStatus, LoopStepStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StateStoreError {
    #[error("state store I/O failed: {message}")]
    Io { message: String },
    #[error("state store JSON is invalid: {message}")]
    InvalidJson { message: String },
    #[error("event {event_id} has invalid {field}: {reason}")]
    InvalidField {
        event_id: String,
        field: String,
        reason: String,
    },
    #[error("event {event_id} conflicts with projected {field}: {expected} != {actual}")]
    IdentityConflict {
        event_id: String,
        field: String,
        expected: String,
        actual: String,
    },
    #[error("event {event_id} has invalid lifecycle transition: {reason}")]
    InvalidTransition { event_id: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopState {
    pub loop_run_id: String,
    pub goal_id: String,
    pub task_run_id: String,
    pub status: LoopStatus,
    pub current_step_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub last_sequence_no: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepState {
    pub step_id: String,
    pub loop_run_id: String,
    pub step_no: u32,
    pub phase: String,
    pub status: LoopStepStatus,
    pub summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub last_sequence_no: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateStore {
    loops: BTreeMap<String, LoopState>,
    steps: BTreeMap<String, StepState>,
}

impl StateStore {
    /// Rebuilds the in-memory current-state projection from events in log order.
    /// Unknown event types are ignored so the projection remains forward-compatible.
    pub fn rebuild(events: &[EventEnvelope]) -> Result<Self, StateStoreError> {
        let mut state = Self::default();
        for event in events {
            state.apply(event)?;
        }
        Ok(state)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, StateStoreError> {
        let contents = fs::read_to_string(path).map_err(|error| StateStoreError::Io {
            message: error.to_string(),
        })?;
        serde_json::from_str(&contents).map_err(|error| StateStoreError::InvalidJson {
            message: error.to_string(),
        })
    }

    pub fn persist(&self, path: impl AsRef<Path>) -> Result<(), StateStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| StateStoreError::Io {
                message: error.to_string(),
            })?;
        }
        let encoded =
            serde_json::to_vec_pretty(self).map_err(|error| StateStoreError::InvalidJson {
                message: error.to_string(),
            })?;
        let temp_id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("json.tmp.{}.{}", std::process::id(), temp_id));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| StateStoreError::Io {
                message: error.to_string(),
            })?;
        file.write_all(&encoded)
            .map_err(|error| StateStoreError::Io {
                message: error.to_string(),
            })?;
        file.sync_all().map_err(|error| StateStoreError::Io {
            message: error.to_string(),
        })?;
        fs::rename(&temporary, path).map_err(|error| StateStoreError::Io {
            message: error.to_string(),
        })?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| StateStoreError::Io {
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }

    pub fn loop_run(&self, loop_run_id: &str) -> Option<&LoopState> {
        self.loops.get(loop_run_id)
    }

    pub fn step(&self, step_id: &str) -> Option<&StepState> {
        self.steps.get(step_id)
    }

    pub fn loops(&self) -> impl Iterator<Item = &LoopState> {
        self.loops.values()
    }

    pub fn steps(&self) -> impl Iterator<Item = &StepState> {
        self.steps.values()
    }

    fn apply(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        match event.event_type.as_str() {
            "loop_started" => self.apply_loop_started(event),
            "loop_completed" | "loop_failed" => self.apply_loop_terminal(event),
            "loop_paused" | "loop_resumed" | "loop_cancelled" => self.apply_loop_control(event),
            "step_started" => self.apply_step_started(event),
            "step_completed" | "step_failed" => self.apply_step_terminal(event),
            _ => Ok(()),
        }
    }

    fn apply_loop_started(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        let loop_id = loop_id(event)?;
        let goal_id = required_string(event, "goal_id", event.goal_id.as_deref())?;
        let task_run_id = required_string(event, "task_run_id", event.task_run_id.as_deref())?;
        if let Some(existing) = self.loops.get(&loop_id) {
            check_identity(event, "goal_id", &existing.goal_id, &goal_id)?;
            check_identity(event, "task_run_id", &existing.task_run_id, &task_run_id)?;
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "loop_started was recorded more than once".into(),
            });
        }
        self.loops.insert(
            loop_id.clone(),
            LoopState {
                loop_run_id: loop_id,
                goal_id,
                task_run_id,
                status: LoopStatus::Running,
                current_step_id: None,
                started_at: event.occurred_at.clone(),
                finished_at: None,
                last_sequence_no: event.sequence_no,
            },
        );
        Ok(())
    }

    fn apply_loop_terminal(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        let loop_id = loop_id(event)?;
        let Some(loop_state) = self.loops.get_mut(&loop_id) else {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "terminal event precedes loop_started".into(),
            });
        };
        if let Some(goal_id) = optional_string(event, "goal_id", event.goal_id.as_deref())? {
            check_identity(event, "goal_id", &loop_state.goal_id, &goal_id)?;
        }
        if let Some(task_run_id) =
            optional_string(event, "task_run_id", event.task_run_id.as_deref())?
        {
            check_identity(event, "task_run_id", &loop_state.task_run_id, &task_run_id)?;
        }
        if loop_state.status != LoopStatus::Running {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "loop is already terminal".into(),
            });
        }
        loop_state.status = if event.event_type == "loop_completed" {
            LoopStatus::Completed
        } else {
            LoopStatus::Failed
        };
        loop_state.finished_at = Some(event.occurred_at.clone());
        loop_state.last_sequence_no = event.sequence_no.or(loop_state.last_sequence_no);
        Ok(())
    }

    fn apply_loop_control(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        let loop_id = loop_id(event)?;
        let Some(loop_state) = self.loops.get_mut(&loop_id) else {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "loop control event precedes loop_started".into(),
            });
        };
        if let Some(goal_id) = optional_string(event, "goal_id", event.goal_id.as_deref())? {
            check_identity(event, "goal_id", &loop_state.goal_id, &goal_id)?;
        }
        if let Some(task_run_id) =
            optional_string(event, "task_run_id", event.task_run_id.as_deref())?
        {
            check_identity(event, "task_run_id", &loop_state.task_run_id, &task_run_id)?;
        }
        match event.event_type.as_str() {
            "loop_paused" if loop_state.status == LoopStatus::Running => {
                loop_state.status = LoopStatus::Paused;
            }
            "loop_resumed" if loop_state.status == LoopStatus::Paused => {
                loop_state.status = LoopStatus::Running;
            }
            "loop_cancelled"
                if matches!(loop_state.status, LoopStatus::Running | LoopStatus::Paused) =>
            {
                loop_state.status = LoopStatus::Cancelled;
                loop_state.finished_at = Some(event.occurred_at.clone());
            }
            _ => {
                return Err(StateStoreError::InvalidTransition {
                    event_id: event.event_id.clone(),
                    reason: "invalid loop control transition".into(),
                });
            }
        }
        loop_state.last_sequence_no = event.sequence_no.or(loop_state.last_sequence_no);
        Ok(())
    }

    fn apply_step_started(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        let step_id = step_id(event)?;
        let loop_run_id = required_string(event, "loop_run_id", event.loop_run_id.as_deref())?;
        let step_no = required_u32(event, "step_no")?;
        let phase = required_string(event, "phase", None)?;
        let Some(loop_state) = self.loops.get_mut(&loop_run_id) else {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "step_started references an unknown loop".into(),
            });
        };
        if loop_state.status != LoopStatus::Running {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "step_started references a terminal loop".into(),
            });
        }
        if self.steps.contains_key(&step_id) {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "step_started was recorded more than once".into(),
            });
        }
        loop_state.current_step_id = Some(step_id.clone());
        loop_state.last_sequence_no = event.sequence_no.or(loop_state.last_sequence_no);
        self.steps.insert(
            step_id.clone(),
            StepState {
                step_id,
                loop_run_id,
                step_no,
                phase,
                status: LoopStepStatus::Running,
                summary: None,
                started_at: event.occurred_at.clone(),
                finished_at: None,
                last_sequence_no: event.sequence_no,
            },
        );
        Ok(())
    }

    fn apply_step_terminal(&mut self, event: &EventEnvelope) -> Result<(), StateStoreError> {
        let step_id = step_id(event)?;
        let Some(step_state) = self.steps.get_mut(&step_id) else {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "terminal event precedes step_started".into(),
            });
        };
        if let Some(loop_run_id) =
            optional_string(event, "loop_run_id", event.loop_run_id.as_deref())?
        {
            check_identity(event, "loop_run_id", &step_state.loop_run_id, &loop_run_id)?;
        }
        if step_state.status != LoopStepStatus::Running {
            return Err(StateStoreError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "step is already terminal".into(),
            });
        }
        step_state.status = if event.event_type == "step_completed" {
            LoopStepStatus::Completed
        } else {
            LoopStepStatus::Failed
        };
        step_state.summary = optional_string(event, "summary", None)?;
        step_state.finished_at = Some(event.occurred_at.clone());
        step_state.last_sequence_no = event.sequence_no.or(step_state.last_sequence_no);
        if let Some(loop_state) = self.loops.get_mut(&step_state.loop_run_id) {
            loop_state.last_sequence_no = event.sequence_no.or(loop_state.last_sequence_no);
        }
        Ok(())
    }
}

fn loop_id(event: &EventEnvelope) -> Result<String, StateStoreError> {
    required_string(
        event,
        "loop_run_id",
        event.loop_run_id.as_deref().or(Some(&event.aggregate_id)),
    )
}

fn step_id(event: &EventEnvelope) -> Result<String, StateStoreError> {
    required_string(
        event,
        "step_id",
        event.step_id.as_deref().or(Some(&event.aggregate_id)),
    )
}

fn required_string(
    event: &EventEnvelope,
    field: &str,
    envelope_value: Option<&str>,
) -> Result<String, StateStoreError> {
    optional_string(event, field, envelope_value)?
        .ok_or_else(|| invalid_field(event, field, "is missing"))
}

fn optional_string(
    event: &EventEnvelope,
    field: &str,
    envelope_value: Option<&str>,
) -> Result<Option<String>, StateStoreError> {
    let payload_value = event.payload.get(field);
    let payload_value = match payload_value {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => return Err(invalid_field(event, field, "must be a string")),
    };
    if let (Some(envelope_value), Some(payload_value)) = (envelope_value, payload_value) {
        check_identity(event, field, envelope_value, payload_value)?;
    }
    Ok(envelope_value
        .or(payload_value)
        .filter(|value| !value.is_empty())
        .map(str::to_owned))
}

fn required_u32(event: &EventEnvelope, field: &str) -> Result<u32, StateStoreError> {
    event
        .payload
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid_field(event, field, "must be an unsigned 32-bit integer"))
}

fn check_identity(
    event: &EventEnvelope,
    field: &str,
    expected: &str,
    actual: &str,
) -> Result<(), StateStoreError> {
    if expected == actual {
        Ok(())
    } else {
        Err(StateStoreError::IdentityConflict {
            event_id: event.event_id.clone(),
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
        })
    }
}

fn invalid_field(event: &EventEnvelope, field: &str, reason: &str) -> StateStoreError {
    StateStoreError::InvalidField {
        event_id: event.event_id.clone(),
        field: field.into(),
        reason: reason.into(),
    }
}
