use steerbox::board_projection::BoardProjection;
use steerbox::domain::{BoardColumn, EventEnvelope, TaskState};

fn event(id: &str, event_type: &str, payload: serde_json::Value) -> EventEnvelope {
    let mut event = EventEnvelope::new("task", id, event_type, "task-1", "corr-board", payload)
        .expect("fixture event is safe");
    event.project_id = Some("project-1".into());
    event.goal_id = Some("goal-1".into());
    event.task_id = Some("task-1".into());
    event
}

#[test]
fn board_projection_rebuilds_task_state_and_board_column() {
    let events = vec![
        event(
            "created",
            "task_created",
            serde_json::json!({"title": "docs"}),
        ),
        event(
            "running",
            "task_state_changed",
            serde_json::json!({"state": "ready"}),
        ),
        event(
            "queued",
            "task_state_changed",
            serde_json::json!({"state": "queued"}),
        ),
        event(
            "started",
            "task_state_changed",
            serde_json::json!({"state": "running"}),
        ),
        event(
            "verifying",
            "task_state_changed",
            serde_json::json!({"state": "verifying"}),
        ),
        event(
            "evidence",
            "task_evidence_added",
            serde_json::json!({"evidence_ref": "trace:1"}),
        ),
        event(
            "verified",
            "task_verification_passed",
            serde_json::json!({}),
        ),
        event("attempt", "attempt_finished", serde_json::json!({})),
        event(
            "succeeded",
            "task_state_changed",
            serde_json::json!({"state": "succeeded"}),
        ),
    ];
    let projection = BoardProjection::rebuild(&events).expect("projection rebuilds");
    let task = projection.task("task-1").expect("task exists");
    assert_eq!(task.domain_state, TaskState::Succeeded);
    assert_eq!(task.board_column(), BoardColumn::Done);
    assert_eq!(task.evidence_refs, vec!["trace:1"]);
}

#[test]
fn board_projection_rejects_task_event_before_creation() {
    let error = BoardProjection::rebuild(&[event(
        "state",
        "task_state_changed",
        serde_json::json!({"state": "ready"}),
    )])
    .expect_err("missing task creation must fail");
    assert!(error.to_string().contains("precedes task_created"));
}

#[test]
fn board_projection_rejects_cross_project_task_update() {
    let created = event(
        "created",
        "task_created",
        serde_json::json!({"title":"docs"}),
    );
    let mut changed = event(
        "changed",
        "task_state_changed",
        serde_json::json!({"state":"ready"}),
    );
    changed.project_id = Some("other-project".into());
    assert!(BoardProjection::rebuild(&[created, changed]).is_err());
}

#[test]
fn board_projection_requires_task_identity_on_attempt_event() {
    let created = event(
        "created",
        "task_created",
        serde_json::json!({"title":"docs"}),
    );
    let mut finished = event("finished", "attempt_finished", serde_json::json!({}));
    finished.aggregate_type = "attempt".into();
    finished.task_id = None;
    assert!(BoardProjection::rebuild(&[created, finished]).is_err());
}
