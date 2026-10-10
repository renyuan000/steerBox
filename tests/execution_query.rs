use serde_json::json;
use steerbox::domain::{EventEnvelope, LoopStatus};
use steerbox::execution_query::{ExecutionEventSurface, ExecutionQuery, ExecutionSurfaceSnapshot};

fn event(
    sequence_no: u64,
    event_id: &str,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        aggregate_type,
        event_id,
        event_type,
        aggregate_id,
        "correlation-query",
        payload,
    )
    .expect("fixture event is safe");
    event.sequence_no = Some(sequence_no);
    event.project_id = Some("project-1".into());
    event
}

#[test]
fn projects_detached_execution_surface_without_event_payloads() {
    let events = vec![
        event(
            1,
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            json!({
                "status": "running",
                "goal_id": "goal-1",
                "task_run_id": "task-run-1"
            }),
        ),
        event(
            2,
            "step-started",
            "step_started",
            "loop_step",
            "step-1",
            json!({"loop_run_id": "loop-1", "step_no": 1, "phase": "observe"}),
        ),
        event(
            3,
            "model-started",
            "model_call_started",
            "model_call",
            "model-call-1",
            json!({"loop_run_id": "loop-1", "step_id": "step-1"}),
        ),
        event(
            4,
            "model-completed",
            "model_call_completed",
            "model_call",
            "model-call-1",
            json!({"loop_run_id": "loop-1", "step_id": "step-1"}),
        ),
        event(
            5,
            "step-completed",
            "step_completed",
            "loop_step",
            "step-1",
            json!({"loop_run_id": "loop-1", "summary": "observation complete"}),
        ),
        event(
            6,
            "loop-completed",
            "loop_completed",
            "loop_run",
            "loop-1",
            json!({"status": "completed"}),
        ),
    ];

    let query = ExecutionQuery::from_events(&events).expect("query projection builds");
    let record = query.record("loop-1").expect("execution exists");
    assert_eq!(record.loop_run_id, "loop-1");
    assert_eq!(record.project_id.as_deref(), Some("project-1"));
    assert_eq!(record.goal_id, "goal-1");
    assert_eq!(record.task_run_id, "task-run-1");
    assert_eq!(record.status, LoopStatus::Completed);
    assert_eq!(record.event_count, 6);
    assert_eq!(record.captured_through_sequence, Some(6));

    let snapshot = query.surface_snapshot("loop-1").expect("snapshot exists");
    assert_eq!(snapshot.events.len(), 6);
    assert_eq!(snapshot.captured_through_seq, Some(6));
    assert!(snapshot
        .events
        .iter()
        .all(|event| event.surface == ExecutionEventSurface::Current));
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("observation complete"));
}

#[test]
fn list_returns_one_record_per_started_execution_and_ignores_unbound_events() {
    let events = vec![
        event(
            1,
            "loop-started-1",
            "loop_started",
            "loop_run",
            "loop-1",
            json!({"goal_id": "goal-1", "task_run_id": "task-run-1"}),
        ),
        event(
            2,
            "unbound",
            "diagnostic",
            "diagnostic",
            "diagnostic-1",
            json!({"message": "not a runtime execution"}),
        ),
        event(
            3,
            "loop-started-2",
            "loop_started",
            "loop_run",
            "loop-2",
            json!({"goal_id": "goal-2", "task_run_id": "task-run-2"}),
        ),
    ];

    let query = ExecutionQuery::from_events(&events).expect("query projection builds");
    let records = query.list();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].loop_run_id, "loop-1");
    assert_eq!(records[1].loop_run_id, "loop-2");
    assert!(query.surface_snapshot("missing").is_none());
}

#[test]
fn query_preserves_log_only_event_surface_for_events_without_loop_binding() {
    let events = vec![
        event(
            1,
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            json!({"goal_id": "goal-1", "task_run_id": "task-run-1"}),
        ),
        event(
            2,
            "log-only",
            "diagnostic",
            "loop_run",
            "loop-1",
            json!({"message": "runtime note"}),
        ),
    ];

    let query = ExecutionQuery::from_events(&events).expect("query projection builds");
    let snapshot: ExecutionSurfaceSnapshot =
        query.surface_snapshot("loop-1").expect("snapshot exists");
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(snapshot.events[1].surface, ExecutionEventSurface::LogOnly);
}

#[test]
fn does_not_attach_an_unbound_other_aggregate_with_a_colliding_id() {
    let events = vec![
        event(
            1,
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            json!({"goal_id": "goal-1", "task_run_id": "task-run-1"}),
        ),
        event(
            2,
            "task-collision",
            "task_updated",
            "task",
            "loop-1",
            json!({"message": "same identifier, different aggregate"}),
        ),
    ];

    let query = ExecutionQuery::from_events(&events).expect("query projection builds");
    let snapshot = query.surface_snapshot("loop-1").expect("snapshot exists");
    assert_eq!(snapshot.events.len(), 1);
}

#[test]
fn pages_metadata_only_events_from_the_detached_snapshot() {
    let events = vec![
        event(
            1,
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            json!({"goal_id": "goal-1", "task_run_id": "task-run-1"}),
        ),
        event(
            2,
            "step-started",
            "step_started",
            "loop_step",
            "step-1",
            json!({"loop_run_id": "loop-1", "step_no": 1, "phase": "observe"}),
        ),
        event(
            3,
            "model-started",
            "model_call_started",
            "model_call",
            "model-call-1",
            json!({"loop_run_id": "loop-1", "step_id": "step-1"}),
        ),
        event(
            4,
            "model-completed",
            "model_call_completed",
            "model_call",
            "model-call-1",
            json!({"loop_run_id": "loop-1", "step_id": "step-1"}),
        ),
        event(
            5,
            "step-completed",
            "step_completed",
            "loop_step",
            "step-1",
            json!({"loop_run_id": "loop-1", "summary": "safe summary"}),
        ),
        event(
            6,
            "loop-completed",
            "loop_completed",
            "loop_run",
            "loop-1",
            json!({"status": "completed"}),
        ),
    ];

    let query = ExecutionQuery::from_events(&events).expect("query projection builds");
    let page = query
        .events_page("loop-1", 1, 2)
        .expect("execution page exists");
    assert_eq!(page.loop_run_id, "loop-1");
    assert_eq!(page.offset, 1);
    assert_eq!(page.limit, 2);
    assert_eq!(page.total, 6);
    assert!(page.has_more);
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].event_id, "step-started");
    assert_eq!(page.events[1].event_id, "model-started");
    assert!(!serde_json::to_string(&page)
        .unwrap()
        .contains("safe summary"));

    let final_page = query
        .events_page("loop-1", 5, 2)
        .expect("execution page exists");
    assert_eq!(final_page.events.len(), 1);
    assert!(!final_page.has_more);
    assert!(query.events_page("missing", 0, 10).is_none());
}
