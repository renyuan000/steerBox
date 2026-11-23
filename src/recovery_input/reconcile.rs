use super::{
    io, validation, RecoveryInputError, RecoveryInputPaths, RecoverySource, RecoveryState,
    RecoveryVerdict, SourceBoundary, SourceContent, AUDIT_CAP, EVENT_CAP, SOURCES,
};
use crate::audit_store::{
    CheckpointMetadata, SideEffectAuditStatus, SideEffectRecord, TraceRecord,
};
use crate::domain::{EventEnvelope, ToolCallRecord};
use crate::model_contracts::{ModelCallFailureRecord, ModelCallRecord};
use crate::state_store::StateStore;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) const RECORD_LIMIT: u32 = 10_000;
const LINE_CAP: usize = 1024 * 1024;

pub(super) struct Scan {
    pub(super) sources: Vec<SourceBoundary>,
    pub(super) event_sequence: u64,
    pub(super) verdict: RecoveryVerdict,
}

pub(super) fn scan(
    paths: &RecoveryInputPaths,
    state: &RecoveryState,
) -> Result<Scan, RecoveryInputError> {
    paths.validate()?;
    let mut result = Scan {
        sources: Vec::with_capacity(7),
        event_sequence: 0,
        verdict: RecoveryVerdict::InputsMatchOnly,
    };
    let mut pending = false;
    let mut audit_records = false;
    let mut missing = false;
    let mut consistent = true;
    for (index, kind) in SOURCES.into_iter().enumerate() {
        let content = if let Some(path) = &paths.paths[index] {
            let cap = if kind == RecoverySource::Events {
                EVENT_CAP
            } else {
                AUDIT_CAP
            };
            if let Some(bytes) = io::read_source(path, cap)? {
                let lines = lines(&bytes)?;
                match kind {
                    RecoverySource::Events => {
                        let mut events = Vec::with_capacity(lines.len());
                        let mut ids = HashSet::new();
                        for (offset, line) in lines.iter().enumerate() {
                            let event: EventEnvelope = validation::decode(line)?;
                            if event.schema_version != 1
                                || event.sequence_no != Some(offset as u64 + 1)
                                || event.event_id.trim().is_empty()
                                || !ids.insert(event.event_id.clone())
                            {
                                return Err(RecoveryInputError::InvalidRecord);
                            }
                            // Deserialize does not itself enforce the sensitive-payload-key gate.
                            EventEnvelope::new(
                                "check",
                                "check",
                                "check",
                                "check",
                                "check",
                                event.payload.clone(),
                            )
                            .map_err(|_| RecoveryInputError::InvalidRecord)?;
                            if belongs_to_loop(&event, &state.loop_run.loop_run_id)
                                && (!loop_identity_matches(&event, &state.loop_run.loop_run_id)
                                    || !identity_matches(
                                        &event,
                                        "goal_id",
                                        event.goal_id.as_deref(),
                                        &state.goal.goal_id,
                                    )
                                    || !identity_matches(
                                        &event,
                                        "task_run_id",
                                        event.task_run_id.as_deref(),
                                        &state.task_run.task_run_id,
                                    )
                                    || !identity_matches(
                                        &event,
                                        "project_id",
                                        event.project_id.as_deref(),
                                        &state.goal.project_id,
                                    )
                                    || !attempt_matches(&event, state))
                            {
                                consistent = false;
                            }
                            events.push(event);
                        }
                        result.event_sequence = events.len() as u64;
                        consistent &= projected_state_matches(&events, state);
                    }
                    RecoverySource::ModelCalls => {
                        for line in &lines {
                            let record: ModelCallRecord = validation::decode(line)?;
                            record
                                .validate()
                                .map_err(|_| RecoveryInputError::InvalidRecord)?;
                        }
                        audit_records |= !lines.is_empty();
                    }
                    RecoverySource::ModelFailures => {
                        for line in &lines {
                            let record: ModelCallFailureRecord = validation::decode(line)?;
                            record
                                .validate()
                                .map_err(|_| RecoveryInputError::InvalidRecord)?;
                        }
                        audit_records |= !lines.is_empty();
                    }
                    RecoverySource::ToolCalls => {
                        for line in &lines {
                            let record: ToolCallRecord = validation::decode(line)?;
                            pending |= !matches!(
                                record.status.as_str(),
                                "completed" | "failed" | "blocked"
                            );
                        }
                        audit_records |= !lines.is_empty();
                    }
                    RecoverySource::SideEffects => {
                        for line in &lines {
                            let record: SideEffectRecord = validation::decode(line)?;
                            pending |= !matches!(
                                record.audit_status(),
                                SideEffectAuditStatus::Completed
                                    | SideEffectAuditStatus::Failed
                                    | SideEffectAuditStatus::Blocked
                            );
                        }
                        audit_records |= !lines.is_empty();
                    }
                    RecoverySource::Traces => {
                        for line in &lines {
                            let _: TraceRecord = validation::decode(line)?;
                        }
                        audit_records |= !lines.is_empty();
                    }
                    RecoverySource::Checkpoints => {
                        for line in &lines {
                            let _: CheckpointMetadata = validation::decode(line)?;
                        }
                        audit_records |= !lines.is_empty();
                    }
                }
                SourceContent::Present {
                    bytes: bytes.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    records: lines.len() as u32,
                }
            } else {
                if kind == RecoverySource::Events {
                    return Err(RecoveryInputError::UnsafeOrUnavailable);
                }
                missing = true;
                SourceContent::Missing
            }
        } else {
            SourceContent::Unconfigured
        };
        result.sources.push(SourceBoundary { kind, content });
    }
    result.verdict = if !consistent {
        RecoveryVerdict::InconsistentState
    } else if missing {
        RecoveryVerdict::SourcesUnavailable
    } else if pending {
        RecoveryVerdict::PendingAudit
    } else if audit_records {
        RecoveryVerdict::AuditReconciliationRequired
    } else {
        RecoveryVerdict::InputsMatchOnly
    };
    Ok(result)
}

fn lines(bytes: &[u8]) -> Result<Vec<&[u8]>, RecoveryInputError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(b"\n") || std::str::from_utf8(bytes).is_err() {
        return Err(RecoveryInputError::InvalidRecord);
    }
    let mut lines = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() || line.len() >= LINE_CAP || lines.len() >= RECORD_LIMIT as usize {
            return Err(RecoveryInputError::LimitReached);
        }
        lines.push(line);
    }
    Ok(lines)
}

fn belongs_to_loop(event: &EventEnvelope, loop_id: &str) -> bool {
    event.loop_run_id.as_deref() == Some(loop_id)
        || event
            .payload
            .get("loop_run_id")
            .and_then(serde_json::Value::as_str)
            == Some(loop_id)
        || (lifecycle_event(event) && event.aggregate_id == loop_id)
}

fn lifecycle_event(event: &EventEnvelope) -> bool {
    matches!(
        event.event_type.as_str(),
        "loop_started"
            | "loop_completed"
            | "loop_failed"
            | "loop_paused"
            | "loop_resumed"
            | "loop_cancelled"
    )
}

fn loop_identity_matches(event: &EventEnvelope, expected: &str) -> bool {
    event.loop_run_id.as_deref().is_none_or(|id| id == expected)
        && event
            .payload
            .get("loop_run_id")
            .filter(|value| !value.is_null())
            .is_none_or(|value| value.as_str() == Some(expected))
        && (!lifecycle_event(event) || event.aggregate_id == expected)
}

fn identity_matches(
    event: &EventEnvelope,
    key: &str,
    envelope: Option<&str>,
    expected: &str,
) -> bool {
    let payload = event.payload.get(key).filter(|value| !value.is_null());
    (envelope.is_some() || payload.is_some())
        && envelope.is_none_or(|value| value == expected)
        && payload.is_none_or(|value| value.as_str() == Some(expected))
}

fn attempt_matches(event: &EventEnvelope, state: &RecoveryState) -> bool {
    let payload = event
        .payload
        .get("attempt_id")
        .filter(|value| !value.is_null());
    let known = |id: &str| {
        state
            .task_run
            .attempts
            .iter()
            .any(|attempt| attempt.attempt_id == id)
    };
    event.attempt_id.as_deref().is_none_or(known)
        && payload.is_none_or(|value| value.as_str().is_some_and(known))
        && match (event.attempt_id.as_deref(), payload) {
            (Some(envelope), Some(value)) => value.as_str() == Some(envelope),
            _ => true,
        }
}

fn projected_state_matches(events: &[EventEnvelope], state: &RecoveryState) -> bool {
    let Ok(projected) = StateStore::rebuild(events) else {
        return false;
    };
    let Some(loop_state) = projected.loop_run(&state.loop_run.loop_run_id) else {
        return false;
    };
    // Projection timestamps are event times, not the independently created domain times.
    if loop_state.goal_id != state.goal.goal_id
        || loop_state.task_run_id != state.task_run.task_run_id
        || loop_state.status != state.loop_run.status
    {
        return false;
    }
    let mut last = 0;
    for event in events.iter().filter(|event| {
        event.event_type == "step_started" && belongs_to_loop(event, &state.loop_run.loop_run_id)
    }) {
        let Some(number) = event
            .payload
            .get("step_no")
            .and_then(serde_json::Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
        else {
            return false;
        };
        if number <= last {
            return false;
        }
        last = number;
    }
    last.checked_add(1) == Some(state.next_step_no)
}
