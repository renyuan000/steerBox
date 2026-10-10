use std::fs;
use std::path::PathBuf;
use steerbox::domain::{EventEnvelope, LoopStatus, LoopStepStatus};
use steerbox::state_store::StateStore;

fn temp_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "steerbox-state-store-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn event(
    event_id: &str,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> EventEnvelope {
    EventEnvelope::new(
        aggregate_type,
        event_id,
        event_type,
        aggregate_id,
        "correlation-state-store",
        payload,
    )
    .expect("fixture event is safe")
}

#[test]
fn rebuilds_current_loop_and_step_state_from_events() {
    let events = vec![
        event(
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            serde_json::json!({"status": "running", "goal_id": "goal-1", "task_run_id": "run-1"}),
        ),
        event(
            "step-started",
            "step_started",
            "loop_step",
            "step-1",
            serde_json::json!({"loop_run_id": "loop-1", "step_no": 1, "phase": "observe"}),
        ),
        event(
            "step-completed",
            "step_completed",
            "loop_step",
            "step-1",
            serde_json::json!({"loop_run_id": "loop-1", "summary": "repository inspected"}),
        ),
        event(
            "loop-completed",
            "loop_completed",
            "loop_run",
            "loop-1",
            serde_json::json!({"status": "completed"}),
        ),
    ];

    let state = StateStore::rebuild(&events).expect("state rebuilds");
    let loop_state = state.loop_run("loop-1").expect("loop projection exists");
    assert_eq!(loop_state.goal_id, "goal-1");
    assert_eq!(loop_state.task_run_id, "run-1");
    assert_eq!(loop_state.status, LoopStatus::Completed);
    assert_eq!(loop_state.current_step_id.as_deref(), Some("step-1"));
    assert_eq!(loop_state.last_sequence_no, None);

    let step_state = state.step("step-1").expect("step projection exists");
    assert_eq!(step_state.loop_run_id, "loop-1");
    assert_eq!(step_state.status, LoopStepStatus::Completed);
    assert_eq!(step_state.summary.as_deref(), Some("repository inspected"));
}

#[test]
fn rebuild_rejects_conflicting_loop_identity() {
    let events = vec![
        event(
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            serde_json::json!({"status": "running", "goal_id": "goal-1", "task_run_id": "run-1"}),
        ),
        event(
            "loop-completed",
            "loop_completed",
            "loop_run",
            "loop-1",
            serde_json::json!({"status": "completed", "goal_id": "goal-2"}),
        ),
    ];

    let error = StateStore::rebuild(&events).expect_err("conflicting identity is rejected");
    assert!(error.to_string().contains("goal_id"));
}

#[test]
fn rebuilds_pause_resume_and_cancel_lifecycle() {
    let events = vec![
        event(
            "loop-started",
            "loop_started",
            "loop_run",
            "loop-1",
            serde_json::json!({"goal_id": "goal-1", "task_run_id": "run-1"}),
        ),
        event(
            "loop-paused",
            "loop_paused",
            "loop_run",
            "loop-1",
            serde_json::json!({}),
        ),
        event(
            "loop-resumed",
            "loop_resumed",
            "loop_run",
            "loop-1",
            serde_json::json!({}),
        ),
        event(
            "loop-cancelled",
            "loop_cancelled",
            "loop_run",
            "loop-1",
            serde_json::json!({}),
        ),
    ];

    let state = StateStore::rebuild(&events).expect("lifecycle rebuilds");
    assert_eq!(
        state.loop_run("loop-1").expect("loop exists").status,
        LoopStatus::Cancelled
    );
}

#[test]
fn state_store_persists_and_reopens_current_projection() {
    let events = vec![event(
        "loop-started",
        "loop_started",
        "loop_run",
        "loop-1",
        serde_json::json!({"goal_id": "goal-1", "task_run_id": "run-1"}),
    )];
    let state = StateStore::rebuild(&events).expect("state rebuilds");
    let path = temp_path("reopen");
    state.persist(&path).expect("state persists");
    let reopened = StateStore::load(&path).expect("state reopens");
    assert_eq!(reopened, state);
    fs::remove_file(path).expect("temporary state removes");
}

#[test]
fn concurrent_state_persists_do_not_share_a_temporary_file() {
    let path = temp_path("concurrent");
    let state = StateStore::default();
    let writers = (0..8)
        .map(|_| {
            let state = state.clone();
            let path = path.clone();
            std::thread::spawn(move || state.persist(path))
        })
        .collect::<Vec<_>>();

    for writer in writers {
        writer
            .join()
            .expect("writer thread completes")
            .expect("state persists");
    }
    assert_eq!(StateStore::load(&path).expect("final state loads"), state);
    fs::remove_file(path).expect("temporary state removes");
}
