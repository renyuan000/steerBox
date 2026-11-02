use serde_json::{json, Value};
use steerbox::domain::{EventEnvelope, LoopStatus, LoopStepStatus};
use steerbox::execution_query::ExecutionQuery;
use steerbox::state_store::{StateStore, StateStoreError};

fn event(
    event_type: &str,
    loop_run_id: &str,
    step_id: Option<&str>,
    payload: Value,
) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        if step_id.is_some() {
            "loop_step"
        } else {
            "loop_run"
        },
        format!("{loop_run_id}:{event_type}"),
        event_type,
        step_id.unwrap_or(loop_run_id),
        "correlation-loop-step",
        payload,
    )
    .expect("fixture event is safe");
    event.loop_run_id = Some(loop_run_id.into());
    event.step_id = step_id.map(str::to_owned);
    event
}

fn shared_step_events() -> Vec<EventEnvelope> {
    let mut events = vec![
        event(
            "loop_started",
            "loop-1",
            None,
            json!({"goal_id": "goal-1", "task_run_id": "task-1"}),
        ),
        event(
            "loop_started",
            "loop-2",
            None,
            json!({"goal_id": "goal-2", "task_run_id": "task-2"}),
        ),
        event(
            "step_started",
            "loop-1",
            Some("step-1"),
            json!({"step_no": 1, "phase": "observe"}),
        ),
        event(
            "step_started",
            "loop-2",
            Some("step-1"),
            json!({"step_no": 1, "phase": "observe"}),
        ),
        event(
            "step_failed",
            "loop-2",
            Some("step-1"),
            json!({"summary": "second loop failure"}),
        ),
        event(
            "step_completed",
            "loop-1",
            Some("step-1"),
            json!({"summary": "first loop complete"}),
        ),
        event("loop_completed", "loop-1", None, json!({})),
        event("loop_failed", "loop-2", None, json!({})),
    ];
    for (index, event) in events.iter_mut().enumerate() {
        event.sequence_no = Some(index as u64 + 1);
    }
    events
}

#[test]
fn rebuilds_distinct_loop_steps_with_the_same_step_id() {
    let state = StateStore::rebuild(&shared_step_events()).expect("shared step IDs rebuild");
    assert_eq!(state.steps().count(), 2);
    for (loop_run_id, status, summary) in [
        ("loop-1", LoopStepStatus::Completed, "first loop complete"),
        ("loop-2", LoopStepStatus::Failed, "second loop failure"),
    ] {
        let step = state.step(loop_run_id, "step-1").unwrap();
        assert_eq!(step.step_id, "step-1");
        assert_eq!(step.status, status);
        assert_eq!(step.summary.as_deref(), Some(summary));
        assert_eq!(
            state
                .loop_run(loop_run_id)
                .unwrap()
                .current_step_id
                .as_deref(),
            Some("step-1")
        );
    }
    assert_eq!(
        state.step("loop-1", "step-1").unwrap().last_sequence_no,
        Some(6)
    );
    assert_eq!(
        state.step("loop-2", "step-1").unwrap().last_sequence_no,
        Some(5)
    );
    assert!(state.step("loop-3", "step-1").is_none());
    assert!(state.step("loop-1", "step-2").is_none());
}

#[test]
fn fresh_query_lists_shows_and_pages_loops_sharing_a_step_id() {
    let query =
        ExecutionQuery::from_events(&shared_step_events()).expect("query rebuilds shared steps");
    assert_eq!(query.list().len(), 2);
    for (loop_run_id, status, captured_sequence) in [
        ("loop-1", LoopStatus::Completed, 7),
        ("loop-2", LoopStatus::Failed, 8),
    ] {
        let record = query.record(loop_run_id).unwrap();
        assert_eq!(record.status, status);
        assert_eq!(record.event_count, 4);
        let snapshot = query.surface_snapshot(loop_run_id).unwrap();
        assert_eq!(snapshot.execution, record);
        assert_eq!(snapshot.captured_through_seq, Some(captured_sequence));
        assert_eq!(snapshot.events.len(), 4);
        assert!(snapshot
            .events
            .iter()
            .all(|event| event.loop_run_id == loop_run_id));
        let page = query.events_page(loop_run_id, 1, 2).unwrap();
        assert_eq!(page.total, 4);
        assert!(page.has_more);
        assert_eq!(page.events, snapshot.events[1..3]);
    }
}

#[test]
fn duplicate_step_start_within_the_same_loop_is_rejected() {
    let mut events = shared_step_events()[..3].to_vec();
    events.push(events[2].clone());
    let error = StateStore::rebuild(&events).unwrap_err();
    assert!(
        matches!(error, StateStoreError::InvalidTransition { ref reason, .. }
        if reason == "step_started was recorded more than once")
    );
}

#[test]
fn terminal_events_cannot_bind_to_a_step_in_another_loop() {
    for event_type in ["step_completed", "step_failed"] {
        let mut events = shared_step_events()[..3].to_vec();
        events.push(event(event_type, "loop-2", Some("step-1"), json!({})));
        let error = StateStore::rebuild(&events).unwrap_err();
        assert!(matches!(error, StateStoreError::InvalidTransition { .. }));
    }
}

#[test]
fn unbound_terminal_events_reject_ambiguous_steps_including_terminal_candidates() {
    for event_count in [4, 5] {
        for event_type in ["step_completed", "step_failed"] {
            let mut events = shared_step_events()[..event_count].to_vec();
            let mut terminal = event(event_type, "unbound", Some("step-1"), json!({}));
            terminal.loop_run_id = None;
            events.push(terminal);
            let error = StateStore::rebuild(&events).unwrap_err();
            assert!(
                matches!(error, StateStoreError::InvalidTransition { ref reason, .. }
                if reason == "terminal event has an ambiguous step_id without loop_run_id")
            );
        }
    }
}

#[test]
fn unbound_terminal_events_retain_existing_unique_identity_semantics() {
    for (event_type, status) in [
        ("step_completed", LoopStepStatus::Completed),
        ("step_failed", LoopStepStatus::Failed),
    ] {
        let mut events = shared_step_events()[..3].to_vec();
        let mut terminal = event(event_type, "unbound", Some("step-1"), json!({}));
        terminal.loop_run_id = None;
        terminal.sequence_no = Some(4);
        events.push(terminal);
        let state = StateStore::rebuild(&events).unwrap();
        let step = state.step("loop-1", "step-1").unwrap();
        assert_eq!(step.status, status);
        assert_eq!(step.last_sequence_no, Some(4));
        assert!(state.step("loop-2", "step-1").is_none());
    }
}

#[test]
fn step_terminal_after_loop_terminal_keeps_existing_lifecycle_behavior() {
    let shared = shared_step_events();
    let events = vec![
        shared[0].clone(),
        shared[2].clone(),
        shared[6].clone(),
        shared[5].clone(),
    ];
    let state = StateStore::rebuild(&events).unwrap();
    assert_eq!(
        state.loop_run("loop-1").unwrap().status,
        LoopStatus::Completed
    );
    assert_eq!(
        state.step("loop-1", "step-1").unwrap().status,
        LoopStepStatus::Completed
    );
}

#[test]
fn loop_scoped_steps_persist_and_reopen_without_losing_either_step() {
    let state = StateStore::rebuild(&shared_step_events()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    state.persist(&path).unwrap();
    let encoded: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        encoded["steps"]["loop-1"]["step-1"]["loop_run_id"],
        "loop-1"
    );
    assert_eq!(
        encoded["steps"]["loop-2"]["step-1"]["loop_run_id"],
        "loop-2"
    );
    assert!(encoded["steps"].get("step-1").is_none());
    let reopened = StateStore::load(&path).unwrap();
    assert_eq!(reopened, state);
    assert_eq!(reopened.steps().count(), 2);
    assert_eq!(
        reopened.step("loop-1", "step-1").unwrap().status,
        LoopStepStatus::Completed
    );
    assert_eq!(
        reopened.step("loop-2", "step-1").unwrap().status,
        LoopStepStatus::Failed
    );
}

#[test]
fn load_rejects_nonempty_legacy_flat_steps_instead_of_migrating_them() {
    let legacy = json!({
        "loops": {},
        "steps": {"step-1": {
            "step_id": "step-1", "loop_run_id": "loop-1", "step_no": 1,
            "phase": "observe", "status": "completed", "summary": null,
            "started_at": "2026-11-02T00:00:00Z", "finished_at": null,
            "last_sequence_no": 3
        }}
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-state.json");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(matches!(
        StateStore::load(&path),
        Err(StateStoreError::InvalidJson { .. })
    ));
}

#[test]
fn load_accepts_an_empty_legacy_steps_map() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-empty-state.json");
    std::fs::write(&path, br#"{"loops":{},"steps":{}}"#).unwrap();
    let state = StateStore::load(&path).unwrap();
    assert_eq!(state.steps().count(), 0);
    assert_eq!(state.loops().count(), 0);
}

#[test]
fn terminal_without_any_matching_step_is_rejected() {
    for loop_run_id in [None, Some("loop-1"), Some("unknown-loop")] {
        for event_type in ["step_completed", "step_failed"] {
            let mut events = shared_step_events()[..2].to_vec();
            let mut terminal = event(event_type, "unbound", Some("step-1"), json!({}));
            terminal.loop_run_id = loop_run_id.map(str::to_owned);
            events.push(terminal);
            let error = StateStore::rebuild(&events).unwrap_err();
            assert!(
                matches!(error, StateStoreError::InvalidTransition { ref reason, .. }
                if reason == "terminal event precedes step_started")
            );
        }
    }
}

#[test]
fn repeated_terminal_in_the_same_loop_is_rejected() {
    for event_type in ["step_completed", "step_failed"] {
        let mut events = shared_step_events();
        events.push(event(event_type, "loop-1", Some("step-1"), json!({})));
        let error = StateStore::rebuild(&events).unwrap_err();
        assert!(
            matches!(error, StateStoreError::InvalidTransition { ref reason, .. }
            if reason == "step is already terminal")
        );
    }
}

#[test]
fn terminal_loop_fields_retain_type_and_identity_validation() {
    for event_type in ["step_completed", "step_failed"] {
        let mut invalid_events = shared_step_events()[..3].to_vec();
        invalid_events.push(event(
            event_type,
            "loop-1",
            Some("step-1"),
            json!({"loop_run_id": 7}),
        ));
        let error = StateStore::rebuild(&invalid_events).unwrap_err();
        assert!(
            matches!(error, StateStoreError::InvalidField { ref field, .. }
            if field == "loop_run_id")
        );
        let mut conflicting_events = shared_step_events()[..4].to_vec();
        conflicting_events.push(event(
            event_type,
            "loop-1",
            Some("step-1"),
            json!({"loop_run_id": "loop-2"}),
        ));
        let error = StateStore::rebuild(&conflicting_events).unwrap_err();
        assert!(
            matches!(error, StateStoreError::IdentityConflict { ref field, .. }
            if field == "loop_run_id")
        );
    }
}

#[test]
fn empty_terminal_loop_id_retains_missing_identity_semantics() {
    let mut events = shared_step_events()[..3].to_vec();
    events.push(event("step_completed", "", Some("step-1"), json!({})));
    let state = StateStore::rebuild(&events).unwrap();
    assert_eq!(
        state.step("loop-1", "step-1").unwrap().status,
        LoopStepStatus::Completed
    );
}
