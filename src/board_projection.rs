use crate::domain::{EventEnvelope, Task, TaskState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BoardProjectionError {
    #[error("event {event_id} is missing task field {field}")]
    MissingField { event_id: String, field: String },
    #[error("event {event_id} has invalid task field {field}: {reason}")]
    InvalidField {
        event_id: String,
        field: String,
        reason: String,
    },
    #[error("event {event_id} cannot apply task transition: {reason}")]
    InvalidTransition { event_id: String, reason: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardProjection {
    tasks: BTreeMap<String, Task>,
}

impl BoardProjection {
    pub fn rebuild(events: &[EventEnvelope]) -> Result<Self, BoardProjectionError> {
        let mut projection = Self::default();
        for event in events {
            projection.apply(event)?;
        }
        Ok(projection)
    }

    pub fn task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    fn apply(&mut self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        match event.event_type.as_str() {
            "task_created" => self.apply_task_created(event),
            "task_state_changed" => self.apply_state_changed(event),
            "task_verification_passed" => self.apply_verification_passed(event),
            "task_evidence_added" => self.apply_evidence_added(event),
            "task_policy_blocked" => self.apply_policy_blocked(event),
            "attempt_finished" => self.apply_attempt_finished(event),
            _ => Ok(()),
        }
    }

    fn apply_task_created(&mut self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        let task_id = task_id(event);
        let project_id = required_string(event, "project_id", event.project_id.as_deref())?;
        let goal_id = required_string(event, "goal_id", event.goal_id.as_deref())?;
        let title = required_string(event, "title", value_string(&event.payload, "title"))?;
        if self.tasks.contains_key(&task_id) {
            return Err(BoardProjectionError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "task_created was recorded more than once".into(),
            });
        }
        self.tasks.insert(
            task_id.clone(),
            Task::new(task_id, project_id, goal_id, title),
        );
        Ok(())
    }

    fn apply_state_changed(&mut self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        self.validate_task_identity(event)?;
        let task = self.task_mut(event)?;
        let value = value_string(&event.payload, "state").ok_or_else(|| {
            BoardProjectionError::MissingField {
                event_id: event.event_id.clone(),
                field: "state".into(),
            }
        })?;
        let state = serde_json::from_value::<TaskState>(Value::String(value.to_owned())).map_err(
            |error| BoardProjectionError::InvalidField {
                event_id: event.event_id.clone(),
                field: "state".into(),
                reason: error.to_string(),
            },
        )?;
        task.transition_to(state)
            .map_err(|error| BoardProjectionError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: error.to_string(),
            })
    }

    fn apply_verification_passed(
        &mut self,
        event: &EventEnvelope,
    ) -> Result<(), BoardProjectionError> {
        self.validate_task_identity(event)?;
        self.task_mut(event)?.record_verification_passed();
        Ok(())
    }

    fn apply_evidence_added(&mut self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        self.validate_task_identity(event)?;
        let evidence = required_string(
            event,
            "evidence_ref",
            value_string(&event.payload, "evidence_ref"),
        )?;
        self.task_mut(event)?.add_evidence_ref(evidence);
        Ok(())
    }

    fn apply_policy_blocked(&mut self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        self.validate_task_identity(event)?;
        let blocked = event
            .payload
            .get("blocked")
            .and_then(Value::as_bool)
            .ok_or_else(|| BoardProjectionError::MissingField {
                event_id: event.event_id.clone(),
                field: "blocked".into(),
            })?;
        self.task_mut(event)?.set_policy_blocked(blocked);
        Ok(())
    }

    fn apply_attempt_finished(
        &mut self,
        event: &EventEnvelope,
    ) -> Result<(), BoardProjectionError> {
        if event.task_id.is_none() {
            return Err(BoardProjectionError::MissingField {
                event_id: event.event_id.clone(),
                field: "task_id".into(),
            });
        }
        self.validate_task_identity(event)?;
        self.task_mut(event)?.finish_attempt();
        Ok(())
    }

    fn validate_task_identity(&self, event: &EventEnvelope) -> Result<(), BoardProjectionError> {
        let task_id = task_id(event);
        let task = self
            .task(&task_id)
            .ok_or_else(|| BoardProjectionError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "task event precedes task_created".into(),
            })?;
        if let Some(project_id) = event.project_id.as_deref() {
            if project_id != task.project_id {
                return Err(BoardProjectionError::InvalidField {
                    event_id: event.event_id.clone(),
                    field: "project_id".into(),
                    reason: "does not match projected task".into(),
                });
            }
        }
        if let Some(goal_id) = event.goal_id.as_deref() {
            if goal_id != task.goal_id {
                return Err(BoardProjectionError::InvalidField {
                    event_id: event.event_id.clone(),
                    field: "goal_id".into(),
                    reason: "does not match projected task".into(),
                });
            }
        }
        Ok(())
    }

    fn task_mut(&mut self, event: &EventEnvelope) -> Result<&mut Task, BoardProjectionError> {
        let task_id = task_id(event);
        self.tasks
            .get_mut(&task_id)
            .ok_or_else(|| BoardProjectionError::InvalidTransition {
                event_id: event.event_id.clone(),
                reason: "task event precedes task_created".into(),
            })
    }
}

fn task_id(event: &EventEnvelope) -> String {
    event
        .task_id
        .clone()
        .unwrap_or_else(|| event.aggregate_id.clone())
}

fn value_string<'a>(payload: &'a Value, field: &str) -> Option<&'a str> {
    payload.get(field).and_then(Value::as_str)
}

fn required_string(
    event: &EventEnvelope,
    field: &str,
    value: Option<&str>,
) -> Result<String, BoardProjectionError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| BoardProjectionError::MissingField {
            event_id: event.event_id.clone(),
            field: field.into(),
        })
}
