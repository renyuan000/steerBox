use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::domain::{GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};

fn test_data_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    PathBuf::from(format!("/temp/steerbox/test-loop-{nonce}"))
}

#[test]
fn readonly_loop_records_start_step_and_completion_events() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-1",
        "project-1",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(log, goal, "loop-1", "task-run-1", budget)
        .expect("loop starts");
    let step = controller
        .run_observation("step-1", "repository inspected")
        .expect("observation completes");
    let run = controller.complete().expect("loop completes");
    assert_eq!(step.step_no, 1);
    assert!(step.is_terminal());
    assert_eq!(run.status, LoopStatus::Completed);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].event_type, "loop_started");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "step_completed");
    assert_eq!(events[3].event_type, "loop_completed");
    assert_eq!(events[1].loop_run_id.as_deref(), Some("loop-1"));
    assert_eq!(events[1].step_id.as_deref(), Some("step-1"));
    assert_eq!(events[2].sequence_no, Some(3));
}

#[test]
fn readonly_loop_stops_when_step_budget_is_exhausted() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-2",
        "project-2",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(0, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(log, goal, "loop-2", "task-run-2", budget)
        .expect("loop starts");
    assert!(matches!(
        controller.run_observation("step-1", "not run"),
        Err(LoopControllerError::BudgetExhausted(_))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].event_type, "loop_budget_exhausted");
}
