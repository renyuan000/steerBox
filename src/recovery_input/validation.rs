use std::collections::HashSet;
use std::fmt;

use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::{RecoveryInputError, RecoveryState};
use crate::domain::{AttemptReason, AttemptStatus, TaskRunStatus};

pub(super) fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, RecoveryInputError> {
    let raw: UniqueJson =
        serde_json::from_slice(bytes).map_err(|_| RecoveryInputError::InvalidRecord)?;
    let decoded: T =
        serde_json::from_value(raw.0.clone()).map_err(|_| RecoveryInputError::InvalidRecord)?;
    let serialized =
        serde_json::to_value(&decoded).map_err(|_| RecoveryInputError::InvalidRecord)?;
    if !keys_match(&raw.0, &serialized) {
        return Err(RecoveryInputError::InvalidRecord);
    }
    Ok(decoded)
}

fn keys_match(raw: &Value, serialized: &Value) -> bool {
    match (raw, serialized) {
        (Value::Object(raw), Value::Object(serialized)) => raw.iter().all(|(key, value)| {
            if let Some(serialized_value) = serialized.get(key) {
                keys_match(value, serialized_value)
            } else {
                key == "reported_token_budget"
                    && value.is_null()
                    && serialized.contains_key("task_run_id")
                    && serialized.contains_key("attempts")
                    && serialized.contains_key("budget")
                    && serialized.contains_key("current_attempt_id")
            }
        }),
        (Value::Array(raw), Value::Array(serialized)) => {
            raw.len() == serialized.len()
                && raw
                    .iter()
                    .zip(serialized)
                    .all(|(raw, serialized)| keys_match(raw, serialized))
        }
        (Value::Object(_) | Value::Array(_), _) => false,
        _ => true,
    }
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Bool(value)))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|value| UniqueJson(Value::Number(value)))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value.to_owned())))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value)))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueJson>()? {
                    values.push(value.0);
                }
                Ok(UniqueJson(Value::Array(values)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                    values.insert(key, object.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(super) fn validate(state: &RecoveryState) -> Result<(), RecoveryInputError> {
    let goal = &state.goal;
    let loop_run = &state.loop_run;
    let task_run = &state.task_run;
    if ![
        goal.goal_id.as_str(),
        goal.project_id.as_str(),
        loop_run.loop_run_id.as_str(),
        loop_run.goal_id.as_str(),
        loop_run.task_run_id.as_str(),
        task_run.task_run_id.as_str(),
        task_run.task_id.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty())
        || goal.schema_version != 1
        || loop_run.schema_version != 1
        || task_run.schema_version != 1
        || goal.object_version == 0
        || loop_run.object_version == 0
        || task_run.object_version == 0
        || loop_run.goal_id != goal.goal_id
        || loop_run.task_run_id != task_run.task_run_id
        || state.next_step_no == 0
        || state.next_step_no == u32::MAX
    {
        return Err(RecoveryInputError::InvalidRecord);
    }
    for timestamp in [
        &goal.created_at,
        &goal.updated_at,
        &loop_run.started_at,
        &task_run.started_at,
    ] {
        check_timestamp(timestamp)?;
    }
    check_optional_timestamp(loop_run.finished_at.as_deref())?;
    check_optional_timestamp(task_run.finished_at.as_deref())?;
    check_optional_timestamp(task_run.heartbeat_at.as_deref())?;
    let task_time_valid = match task_run.status {
        TaskRunStatus::Running => task_run.finished_at.is_none(),
        TaskRunStatus::Succeeded | TaskRunStatus::Failed | TaskRunStatus::Cancelled => {
            task_run.finished_at.is_some()
        }
        TaskRunStatus::WaitingHuman | TaskRunStatus::Blocked => true,
    };
    if loop_run.is_terminal() != loop_run.finished_at.is_some() || !task_time_valid {
        return Err(RecoveryInputError::InvalidRecord);
    }
    check_optional_id(task_run.worker_id.as_deref())?;
    check_optional_id(task_run.workspace_ref.as_deref())?;
    let attempt_count =
        u32::try_from(task_run.attempts.len()).map_err(|_| RecoveryInputError::InvalidRecord)?;
    if attempt_count == 0 || task_run.budget.attempts_used < attempt_count {
        return Err(RecoveryInputError::InvalidRecord);
    }
    let mut seen = HashSet::new();
    for (index, attempt) in task_run.attempts.iter().enumerate() {
        let expected_number = u32::try_from(index)
            .ok()
            .and_then(|number| number.checked_add(1))
            .ok_or(RecoveryInputError::InvalidRecord)?;
        if attempt.attempt_id.trim().is_empty()
            || !seen.insert(attempt.attempt_id.as_str())
            || attempt.task_run_id != task_run.task_run_id
            || attempt.attempt_no != expected_number
            || attempt.status.is_terminal() != attempt.finished_at.is_some()
            || (attempt.status == AttemptStatus::Running && index + 1 != task_run.attempts.len())
        {
            return Err(RecoveryInputError::InvalidRecord);
        }
        match (attempt.reason, attempt.parent_attempt_id.as_deref()) {
            (AttemptReason::Initial, None) if index == 0 => {}
            (AttemptReason::Initial, _) | (_, None) => {
                return Err(RecoveryInputError::InvalidRecord);
            }
            (_, Some(parent_id)) => {
                if !task_run.attempts[..index]
                    .iter()
                    .any(|parent| parent.attempt_id == parent_id && parent.status.is_terminal())
                {
                    return Err(RecoveryInputError::InvalidRecord);
                }
            }
        }
        check_timestamp(&attempt.started_at)?;
        check_optional_timestamp(attempt.finished_at.as_deref())?;
        for value in [
            attempt.worker_id.as_deref(),
            attempt.workspace_ref.as_deref(),
            attempt.route_decision_id.as_deref(),
            attempt.resolved_prompt_pack_id.as_deref(),
        ] {
            check_optional_id(value)?;
        }
    }
    let current = task_run
        .attempts
        .last()
        .ok_or(RecoveryInputError::InvalidRecord)?;
    if task_run.current_attempt_id.as_deref() != Some(current.attempt_id.as_str()) {
        return Err(RecoveryInputError::InvalidRecord);
    }
    match &state.strategy {
        Some(strategy) => {
            if state.strategy_revision == 0
                || !strategy.budget_multiplier.is_finite()
                || strategy.budget_multiplier <= 0.0
                || !strategy.complexity_score.is_finite()
                || !(0.0..=1.0).contains(&strategy.complexity_score)
            {
                return Err(RecoveryInputError::InvalidRecord);
            }
        }
        None if state.strategy_revision != 0 => return Err(RecoveryInputError::InvalidRecord),
        None => {}
    }
    Ok(())
}

fn check_timestamp(value: &str) -> Result<(), RecoveryInputError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| RecoveryInputError::InvalidRecord)
}

fn check_optional_timestamp(value: Option<&str>) -> Result<(), RecoveryInputError> {
    value.map_or(Ok(()), check_timestamp)
}

fn check_optional_id(value: Option<&str>) -> Result<(), RecoveryInputError> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(RecoveryInputError::InvalidRecord)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, validate};
    use crate::domain::{
        AttemptReason, AttemptStatus, GoalContract, LoopBudget, LoopRun, LoopStatus,
        NoProgressTracker, TaskRun,
    };
    use crate::recovery_input::RecoveryState;

    fn state() -> RecoveryState {
        let budget = LoopBudget::new(2, 0, 1, 2, 0, 2, 2, 0);
        let mut task_run = TaskRun::new("task-run", "task", budget).unwrap();
        task_run
            .start_attempt("attempt-1", AttemptReason::Initial, None)
            .unwrap();
        RecoveryState {
            goal: GoalContract::new("goal", "project", "fixture", vec![], vec![], vec![]),
            loop_run: LoopRun::new("loop", "goal", "task-run"),
            task_run,
            next_step_no: 1,
            no_progress: NoProgressTracker::new(2, 2),
            strategy: None,
            strategy_revision: 0,
        }
    }

    #[test]
    fn decode_rejects_duplicate_keys_recursively() {
        for input in [
            r#"{"name":1,"name":2}"#,
            r#"{"items":[{"name":1,"name":2}]}"#,
            r#"{"name":1,"\u006eame":2}"#,
        ] {
            assert!(decode::<serde_json::Value>(input.as_bytes()).is_err());
        }
    }

    #[test]
    fn decode_rejects_unknown_nested_fields_even_when_null() {
        let mut serialized = serde_json::to_value(state()).unwrap();
        serialized["task_run"]["budget"]["unexpected"] = serde_json::Value::Null;
        assert!(decode::<RecoveryState>(&serde_json::to_vec(&serialized).unwrap()).is_err());
        let mut serialized = serde_json::to_value(state()).unwrap();
        serialized["task_run"]["attempts"][0]["unexpected"] = serde_json::json!(true);
        assert!(decode::<RecoveryState>(&serde_json::to_vec(&serialized).unwrap()).is_err());
    }

    #[test]
    fn decode_accepts_omitted_or_explicit_absent_token_budget() {
        let mut serialized = serde_json::to_value(state()).unwrap();
        assert!(decode::<RecoveryState>(&serde_json::to_vec(&serialized).unwrap()).is_ok());
        serialized["task_run"]["reported_token_budget"] = serde_json::Value::Null;
        assert!(decode::<RecoveryState>(&serde_json::to_vec(&serialized).unwrap()).is_ok());
        serialized["goal"]["reported_token_budget"] = serde_json::Value::Null;
        assert!(decode::<RecoveryState>(&serde_json::to_vec(&serialized).unwrap()).is_err());
    }

    #[test]
    fn exhausted_and_reduced_budgets_remain_exact() {
        let mut snapshot = state();
        snapshot.task_run.budget.max_steps = 0;
        snapshot.task_run.budget.steps_used = 4;
        snapshot.task_run.budget.max_attempts = 0;
        snapshot.task_run.budget.attempts_used = 3;
        snapshot.no_progress.observe("fixture", false);
        snapshot.no_progress.observe("fixture", false);
        snapshot.no_progress.max_no_progress_steps = 0;
        assert!(validate(&snapshot).is_ok());
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let restored: RecoveryState = decode(&bytes).unwrap();
        assert_eq!(restored.task_run.budget, snapshot.task_run.budget);
        assert_eq!(restored.no_progress, snapshot.no_progress);
    }

    #[test]
    fn paused_and_cancelled_loops_allow_running_task_and_attempt() {
        for status in [LoopStatus::Paused, LoopStatus::Cancelled] {
            let mut snapshot = state();
            snapshot.loop_run.status = status;
            if status == LoopStatus::Cancelled {
                snapshot.loop_run.finished_at = Some(snapshot.loop_run.started_at.clone());
            }
            assert!(validate(&snapshot).is_ok());
        }
    }

    #[test]
    fn blocked_and_over_limit_token_ledgers_survive_exactly() {
        use crate::reported_token_budget::ReportedTokenBudget;
        for cause in ["over_limit", "unknown", "conflict", "audit", "overflow"] {
            let mut snapshot = state();
            let mut ledger = ReportedTokenBudget::new(1);
            match cause {
                "over_limit" => ledger.settle("call", Some(2), Some(3)).unwrap(),
                "unknown" => {
                    assert!(ledger.settle("call", None, Some(3)).is_err());
                }
                "conflict" => {
                    ledger.settle("call", Some(2), Some(3)).unwrap();
                    assert!(ledger.settle("call", Some(3), Some(3)).is_err());
                }
                "audit" => ledger.block_audit_failure(),
                "overflow" => {
                    assert!(ledger.settle("call", Some(u64::MAX), Some(1)).is_err());
                }
                _ => unreachable!(),
            }
            snapshot.task_run.reported_token_budget = Some(ledger.clone());
            let restored: RecoveryState = decode(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
            validate(&restored).unwrap();
            let restored_ledger = restored.task_run.reported_token_budget().unwrap();
            assert_eq!(restored_ledger, &ledger, "{cause}");
            assert!(restored_ledger.check_send("new-call").is_err(), "{cause}");
        }
    }

    #[test]
    fn waiting_or_blocked_task_runs_preserve_optional_finish_times() {
        for status in [
            crate::domain::TaskRunStatus::WaitingHuman,
            crate::domain::TaskRunStatus::Blocked,
        ] {
            for finished in [false, true] {
                let mut snapshot = state();
                snapshot.task_run.status = status;
                if finished {
                    snapshot.task_run.finished_at = Some(snapshot.task_run.started_at.clone());
                }
                assert!(validate(&snapshot).is_ok());
            }
        }
    }

    #[test]
    fn rejects_attempt_count_underflow_and_earlier_running_attempts() {
        let mut snapshot = state();
        snapshot.task_run.budget.attempts_used = 0;
        assert!(validate(&snapshot).is_err());
        let mut snapshot = state();
        let mut child = snapshot.task_run.attempts[0].clone();
        child.attempt_id = "attempt-2".into();
        child.attempt_no = 2;
        child.reason = AttemptReason::Retry;
        child.parent_attempt_id = Some("attempt-1".into());
        snapshot.task_run.attempts.push(child);
        snapshot.task_run.budget.attempts_used = 2;
        snapshot.task_run.current_attempt_id = Some("attempt-2".into());
        assert!(validate(&snapshot).is_err());
    }

    #[test]
    fn rejects_invalid_parent_number_and_current_attempt() {
        let mut snapshot = state();
        snapshot.task_run.attempts[0].parent_attempt_id = Some("attempt-1".into());
        assert!(validate(&snapshot).is_err());
        let mut snapshot = state();
        snapshot.task_run.attempts[0].attempt_no = 2;
        assert!(validate(&snapshot).is_err());
        let mut snapshot = state();
        snapshot.task_run.current_attempt_id = Some("missing".into());
        assert!(validate(&snapshot).is_err());
    }

    #[test]
    fn terminal_parent_supports_running_child_but_current_must_be_latest() {
        let mut snapshot = state();
        snapshot
            .task_run
            .finish_attempt(
                "attempt-1",
                AttemptStatus::WaitingHuman,
                "waiting",
                vec![],
                vec![],
            )
            .unwrap();
        snapshot
            .task_run
            .start_attempt("attempt-2", AttemptReason::Resume, Some("attempt-1".into()))
            .unwrap();
        assert!(validate(&snapshot).is_ok());
        snapshot.task_run.current_attempt_id = Some("attempt-1".into());
        assert!(validate(&snapshot).is_err());
    }

    #[test]
    fn rejects_invalid_timestamps_and_version_or_next_step() {
        let mut snapshot = state();
        snapshot.task_run.heartbeat_at = Some("invalid".into());
        assert!(validate(&snapshot).is_err());
        let mut snapshot = state();
        snapshot.goal.schema_version = 2;
        assert!(validate(&snapshot).is_err());
        let mut snapshot = state();
        snapshot.loop_run.object_version = 0;
        assert!(validate(&snapshot).is_err());
        for next_step_no in [0, u32::MAX] {
            let mut snapshot = state();
            snapshot.next_step_no = next_step_no;
            assert!(validate(&snapshot).is_err());
        }
    }
}
