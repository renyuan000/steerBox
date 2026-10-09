use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::domain::{
    BoardColumn, BudgetKind, EventEnvelope, EventEnvelopeError, GoalContract, HumanSteeringMode,
    LoopBudget, LoopRun, LoopStep, NoProgressTracker, RiskLevel, Task, TaskState,
};
use steerbox::event_log::EventLog;
use steerbox::runtime_config::{ConfigError, RuntimeConfig};

fn test_data_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    PathBuf::from(format!("/temp/steerbox/test-{label}-{nonce}"))
}

#[test]
fn goal_scope_rejects_traversal_and_absolute_paths() {
    let goal = GoalContract::new("g", "p", "docs", vec!["docs".into()], vec![], vec![]);
    for path in [
        "../docs/a.md",
        "docs/../../a.md",
        "/docs/a.md",
        "docs\\a.md",
    ] {
        assert!(!goal.is_within_scope(path), "unsafe path allowed: {path}");
    }
    assert!(goal.is_within_scope("./docs/a.md"));
    assert!(!goal.is_within_scope("docs-other/a.md"));
}

#[test]
fn goal_root_scope_excludes_named_subtrees() {
    let goal = GoalContract::new(
        "g",
        "p",
        "docs",
        vec![".".into()],
        vec!["private".into()],
        vec![],
    );
    assert!(goal.is_within_scope("docs/a.md"));
    assert!(!goal.is_within_scope("private/a.md"));
    assert!(!goal.is_within_scope("../a.md"));
}

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
fn event_envelope_rejects_common_credential_field_names_recursively() {
    for key in [
        "access_token",
        "client_secret",
        "session_token",
        "ssh_key",
        "private_key",
        "secret_key",
    ] {
        let payload = serde_json::json!({"nested": [{key: "fixture-only"}]});
        let result = EventEnvelope::new(
            "task",
            "event-sensitive-field",
            "tool_call_completed",
            "task-1",
            "correlation-1",
            payload,
        );

        assert!(
            matches!(result, Err(EventEnvelopeError::SensitivePayload { .. })),
            "sensitive field {key} must be rejected"
        );
    }
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

#[test]
fn runtime_config_requires_temp_root_and_auth_refs_only() {
    let config = RuntimeConfig::from_json_str(
        r#"{
          "schema_version": 1,
          "project_ref": "demo-project",
          "data_root": "/temp/steerbox/demo-project",
          "model_profiles": [{
            "model_id": "local-fixture",
            "provider": "fixture",
            "endpoint": "local://fixture",
            "auth_ref": "fixture-auth",
            "capabilities": ["text"]
          }]
        }"#,
    )
    .expect("valid runtime config");

    assert_eq!(config.project_ref, "demo-project");
    assert_eq!(config.model_profiles.len(), 1);
    assert!(RuntimeConfig::from_json_str(
        r#"{"schema_version":1,"project_ref":"demo","data_root":"/tmp/volatile"}"#
    )
    .is_err());
    assert!(RuntimeConfig::from_json_str(
        r#"{"schema_version":1,"project_ref":"demo","data_root":"/temp/steerbox/other"}"#
    )
    .is_err());
    assert!(matches!(
        RuntimeConfig::from_json_str(
            r#"{"schema_version":1,"project_ref":"demo","data_root":"/temp/steerbox/demo","api_key":"fixture-only"}"#
        ),
        Err(ConfigError::UnknownField { .. })
    ));
}

#[test]
fn event_log_appends_sequences_and_recovers_after_reopen() {
    let directory = test_data_dir("event-log");
    fs::create_dir_all(&directory).expect("test data directory creates");
    let path = directory.join("events.jsonl");
    let first = EventEnvelope::new(
        "goal",
        "event-1",
        "goal_created",
        "goal-1",
        "correlation-1",
        serde_json::json!({"objective": "fixture"}),
    )
    .expect("event payload is safe");
    let second = EventEnvelope::new(
        "task",
        "event-2",
        "task_created",
        "task-1",
        "correlation-1",
        serde_json::json!({"title": "fixture"}),
    )
    .expect("event payload is safe");

    let mut log = EventLog::open(&path).expect("event log opens");
    assert!(path.exists());
    assert_eq!(log.append(first).expect("first event appends"), 1);
    assert_eq!(log.append(second).expect("second event appends"), 2);
    drop(log);

    let mut reopened = EventLog::open(&path).expect("event log reopens");
    let third = EventEnvelope::new(
        "task",
        "event-3",
        "task_state_changed",
        "task-1",
        "correlation-1",
        serde_json::json!({"state": "running"}),
    )
    .expect("event payload is safe");
    assert_eq!(reopened.append(third).expect("third event appends"), 3);
    let events = reopened.read_all().expect("events read");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].sequence_no, Some(1));
    assert_eq!(events[2].sequence_no, Some(3));
    assert_eq!(
        fs::read_to_string(path)
            .expect("event log reads")
            .lines()
            .count(),
        3
    );
}

#[test]
fn event_log_reads_events_in_bounded_pages() {
    let directory = test_data_dir("event-pages");
    fs::create_dir_all(&directory).expect("test directory creates");
    let path = directory.join("events.jsonl");
    let mut log = EventLog::open(&path).expect("event log opens");

    for index in 1..=5 {
        let event = EventEnvelope::new(
            "task",
            format!("event-{index}"),
            "task_state_changed",
            "task-1",
            "correlation-1",
            serde_json::json!({"index": index}),
        )
        .expect("event payload is safe");
        log.append(event).expect("event appends");
    }

    let first_page = log.read_page(0, 2).expect("first page reads");
    let second_page = log.read_page(2, 2).expect("second page reads");
    let final_page = log.read_page(4, 2).expect("final page reads");

    assert_eq!(
        first_page
            .iter()
            .map(|event| event.sequence_no)
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2)]
    );
    assert_eq!(
        second_page
            .iter()
            .map(|event| event.sequence_no)
            .collect::<Vec<_>>(),
        vec![Some(3), Some(4)]
    );
    assert_eq!(
        final_page
            .iter()
            .map(|event| event.sequence_no)
            .collect::<Vec<_>>(),
        vec![Some(5)]
    );
    assert!(log
        .read_page(0, 0)
        .expect("zero-sized page reads")
        .is_empty());
}
