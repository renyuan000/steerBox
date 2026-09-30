use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::domain::{GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_adapter::{LocalFixtureModelAdapter, ModelDecision};

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

#[test]
fn fixture_model_complete_decision_records_model_events_and_completes_loop() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-model",
        "project-model",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-model", "task-run-model", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::CompleteLoop, "fixture complete");
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-model",
            "repository inspected",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::CompleteLoop);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "model_call_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "step_completed");
    assert_eq!(events[5].event_type, "loop_completed");
}

#[test]
fn fixture_model_human_decision_records_intervention_event_without_side_effects() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-human",
        "project-human",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-human", "task-run-human", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::RequestHuman, "approval required");
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-human",
            "reviewed high-risk request",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::RequestHuman);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "human_intervention_required");
    assert_eq!(events[5].event_type, "step_completed");
}

#[test]
fn fixture_model_rejects_empty_model_id_and_records_failure_event() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-error",
        "project-error",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-error", "task-run-error", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::ContinueObservation, "not returned");
    assert!(matches!(
        controller.run_model_step(&adapter, "", "step-error", "observation"),
        Err(LoopControllerError::ModelAdapter(_))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[3].event_type, "model_call_failed");
}

#[test]
fn fixture_model_continue_decision_leaves_loop_running_and_records_step_events() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-continue",
        "project-continue",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
    let mut controller =
        ReadOnlyLoopController::start(log, goal, "loop-continue", "task-run-continue", budget)
            .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(
        ModelDecision::ContinueObservation,
        "more observation needed",
    );
    let decision = controller
        .run_model_step(
            &adapter,
            "local-fixture",
            "step-continue",
            "initial observation",
        )
        .expect("model step completes");
    assert_eq!(decision, ModelDecision::ContinueObservation);
    assert_eq!(controller.status(), LoopStatus::Running);
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "step_started");
    assert_eq!(events[2].event_type, "model_call_started");
    assert_eq!(events[3].event_type, "model_call_completed");
    assert_eq!(events[4].event_type, "step_completed");
}
