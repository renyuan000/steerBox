use steerbox::domain::{
    BoardColumn, BudgetKind, EventEnvelope, EventEnvelopeError, GoalContract, HumanSteeringMode,
    LoopBudget, LoopRun, LoopStep, NoProgressTracker, RiskLevel, Task, TaskState,
};

#[test]
fn event_envelope_serializes_contract_fields_without_secret_payloads() {
    let event = EventEnvelope::new(
        "goal",
        "event-1",
        "goal_created",
        "goal-1",
        "correlation-1",
        serde_json::json!({"objective": "add a test"}),
    )
    .expect("event payload is safe");

    assert_eq!(event.schema_version, 1);
    assert_eq!(event.aggregate_type, "goal");
    assert_eq!(event.sequence_no, None);
    assert_eq!(event.redaction_class, "summary");
    let encoded = serde_json::to_string(&event).expect("event serializes");
    assert!(encoded.contains("goal_created"));
    assert!(!encoded.contains("api_key"));
}

#[test]
fn event_envelope_rejects_sensitive_payload_fields() {
    let result = EventEnvelope::new(
        "task",
        "event-2",
        "tool_call_completed",
        "task-1",
        "correlation-1",
        serde_json::json!({"result": {"api_key": "fixture-only"}}),
    );

    assert!(matches!(
        result,
        Err(EventEnvelopeError::SensitivePayload { field }) if field == "api_key"
    ));
}

#[test]
fn goal_contract_requires_explicit_scope_and_verification() {
    let goal = GoalContract::new(
        "goal-1",
        "project-1",
        "Add a test",
        vec!["src".into()],
        vec!["secrets".into()],
        vec!["cargo test".into()],
    );

    assert_eq!(goal.schema_version, 1);
    assert_eq!(goal.risk_level, RiskLevel::Low);
    assert_eq!(
        goal.human_steering_mode,
        HumanSteeringMode::HumanSteerableLoop
    );
    assert!(goal.is_within_scope("src/lib.rs"));
    assert!(!goal.is_within_scope("secrets/prod.env"));
}

#[test]
fn loop_run_and_step_have_terminal_state_rules() {
    let mut run = LoopRun::new("loop-1", "goal-1", "task-run-1");
    let mut step = LoopStep::new("step-1", "loop-1", 1, "observe");

    assert!(!run.is_terminal());
    assert!(!step.is_terminal());
    step.complete("observed");
    assert!(step.is_terminal());
    run.complete();
    assert!(run.is_terminal());
}

#[test]
fn task_state_machine_accepts_contract_transitions_and_rejects_invalid_ones() {
    let mut task = Task::new("task-1", "project-1", "goal-1", "Implement fixture");

    task.transition_to(TaskState::Ready)
        .expect("backlog -> ready");
    task.transition_to(TaskState::Queued)
        .expect("ready -> queued");
    task.transition_to(TaskState::Running)
        .expect("queued -> running");
    task.transition_to(TaskState::Verifying)
        .expect("running -> verifying");
    assert!(task.transition_to(TaskState::Succeeded).is_err());
    task.record_verification_passed();
    task.add_evidence_ref("evidence-1");
    task.finish_attempt();
    task.transition_to(TaskState::Succeeded)
        .expect("verification and evidence allow success");
    assert_eq!(task.board_column(), BoardColumn::Done);
    assert!(task.transition_to(TaskState::Running).is_err());
}

#[test]
fn board_column_maps_attention_states_separately_from_done() {
    let mut blocked = Task::new("task-2", "project-1", "goal-1", "Blocked fixture");
    blocked.transition_to(TaskState::Ready).unwrap();
    blocked.transition_to(TaskState::Blocked).unwrap();
    assert_eq!(blocked.board_column(), BoardColumn::Attention);

    let mut waiting = Task::new("task-3", "project-1", "goal-1", "Waiting fixture");
    waiting.transition_to(TaskState::Ready).unwrap();
    waiting.transition_to(TaskState::Queued).unwrap();
    waiting.transition_to(TaskState::Running).unwrap();
    waiting.transition_to(TaskState::WaitingHuman).unwrap();
    assert_eq!(waiting.board_column(), BoardColumn::Doing);
}

#[test]
fn loop_budget_and_no_progress_tracker_stop_repetition() {
    let mut budget = LoopBudget::new(2, 1, 1, 1, 1, 2, 2, 1);
    budget.consume_step().unwrap();
    budget.consume_step().unwrap();
    assert_eq!(budget.consume_step().unwrap_err(), BudgetKind::Steps);
    budget.consume_goal_revision().unwrap();
    assert_eq!(
        budget.consume_goal_revision().unwrap_err(),
        BudgetKind::GoalRevisions
    );
    budget.consume_no_progress_step().unwrap();
    budget.consume_no_progress_step().unwrap();
    assert_eq!(
        budget.consume_no_progress_step().unwrap_err(),
        BudgetKind::NoProgressSteps
    );
    budget.consume_same_action_repetition().unwrap();
    budget.consume_same_action_repetition().unwrap();
    assert_eq!(
        budget.consume_same_action_repetition().unwrap_err(),
        BudgetKind::SameActionRepetitions
    );
    budget.consume_verification_failure().unwrap();
    assert_eq!(
        budget.consume_verification_failure().unwrap_err(),
        BudgetKind::VerificationFailures
    );

    let mut verification_budget = LoopBudget::new(1, 1, 1, 1, 1, 1, 1, 2);
    verification_budget.consume_verification_failure().unwrap();
    verification_budget.record_verification_passed();
    verification_budget.consume_verification_failure().unwrap();
    verification_budget.consume_verification_failure().unwrap();
    assert_eq!(
        verification_budget
            .consume_verification_failure()
            .unwrap_err(),
        BudgetKind::VerificationFailures
    );

    let mut tracker = NoProgressTracker::new(2, 3);
    assert!(!tracker.observe("same-action", false));
    assert!(tracker.observe("same-action", false));
    assert!(tracker.stop_requested());

    tracker.reset_after_progress();
    assert!(!tracker.stop_requested());
    assert!(!tracker.observe("new-action", true));
}
