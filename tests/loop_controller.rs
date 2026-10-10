use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::audit_store::{SideEffectLedger, TraceStore};
use steerbox::domain::{BudgetKind, GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_adapter::{LocalFixtureModelAdapter, ModelDecision};
use steerbox::policy_gate::{PolicyDecision, PolicyGate};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput};
use steerbox::tool_registry::ToolRegistry;
use steerbox::user_command_inbox::{CommandKind, CommandStatus, UserCommand, UserCommandInbox};

fn test_data_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    PathBuf::from(format!("/temp/steerbox/test-loop-{nonce}"))
}

fn steering_command(
    command_id: &str,
    project_id: &str,
    goal_id: &str,
    task_run_id: &str,
    loop_run_id: &str,
    kind: CommandKind,
) -> UserCommand {
    UserCommand::new(
        command_id,
        "operator-1",
        project_id,
        goal_id,
        task_run_id,
        loop_run_id,
        kind,
        format!("corr-{command_id}"),
        serde_json::json!({"reason": "operator request"}),
    )
    .expect("steering command is valid")
}

#[test]
fn pause_command_is_applied_at_next_safe_boundary_and_resume_reopens_loop() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let events_path = directory.join("events.jsonl");
    let inbox_path = directory.join("commands.jsonl");
    let inbox = UserCommandInbox::open(&inbox_path);
    inbox
        .submit(steering_command(
            "pause-1",
            "project-steering",
            "goal-steering",
            "task-run-steering",
            "loop-steering",
            CommandKind::Pause,
        ))
        .expect("pause command submits");
    let goal = GoalContract::new(
        "goal-steering",
        "project-steering",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(2, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-steering",
        "task-run-steering",
        budget,
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    assert!(matches!(
        controller.run_observation("step-paused", "not executed"),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(controller.status(), LoopStatus::Paused);
    assert_eq!(
        inbox.latest("pause-1").expect("pause status exists").status,
        CommandStatus::Applied
    );

    inbox
        .submit(steering_command(
            "resume-1",
            "project-steering",
            "goal-steering",
            "task-run-steering",
            "loop-steering",
            CommandKind::Resume,
        ))
        .expect("resume command submits");
    controller
        .run_observation("step-resumed", "resumed safely")
        .expect("resume command applies before step");
    assert_eq!(controller.status(), LoopStatus::Running);

    let events = EventLog::open(events_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert!(events.iter().any(|event| event.event_type == "loop_paused"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "loop_resumed"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "user_command_applied"));
}

#[test]
fn cancel_command_stops_loop_at_safe_boundary_without_logging_payload() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let events_path = directory.join("events.jsonl");
    let inbox = UserCommandInbox::open(directory.join("commands.jsonl"));
    let mut command = steering_command(
        "cancel-1",
        "project-cancel",
        "goal-cancel",
        "task-run-cancel",
        "loop-cancel",
        CommandKind::Cancel,
    );
    command.payload = serde_json::json!({"reason": "stop", "note": "safe"});
    inbox.submit(command).expect("cancel command submits");
    let goal = GoalContract::new(
        "goal-cancel",
        "project-cancel",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events_path).expect("event log opens"),
        goal,
        "loop-cancel",
        "task-run-cancel",
        budget,
    )
    .expect("loop starts")
    .with_command_inbox(inbox.clone());

    assert!(matches!(
        controller.run_observation("step-cancelled", "not executed"),
        Err(LoopControllerError::Terminal)
    ));
    assert_eq!(controller.status(), LoopStatus::Cancelled);
    assert_eq!(
        inbox
            .latest("cancel-1")
            .expect("cancel status exists")
            .status,
        CommandStatus::Applied
    );
    let encoded = fs::read_to_string(events_path).expect("events readable");
    assert!(!encoded.contains("safe"));
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

#[test]
fn fixture_model_respects_model_call_budget_across_steps() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-model-budget",
        "project-model-budget",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(2, 0, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-model-budget",
        "task-run-model-budget",
        budget,
    )
    .expect("loop starts");
    let adapter =
        LocalFixtureModelAdapter::new(ModelDecision::ContinueObservation, "one model call allowed");
    controller
        .run_model_step(&adapter, "local-fixture", "step-1", "first observation")
        .expect("first model step completes");
    assert!(matches!(
        controller.run_model_step(&adapter, "local-fixture", "step-2", "second observation"),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ModelCalls))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(
        events.last().expect("budget event exists").event_type,
        "loop_budget_exhausted"
    );
    assert_eq!(
        events.last().expect("budget event exists").payload["budget_kind"],
        "ModelCalls"
    );
}

#[test]
fn fixture_model_stops_repeated_no_progress_actions() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let goal = GoalContract::new(
        "goal-no-progress",
        "project-no-progress",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(3, 0, 3, 1, 0, 3, 2, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-no-progress",
        "task-run-no-progress",
        budget,
    )
    .expect("loop starts");
    let adapter = LocalFixtureModelAdapter::new(
        ModelDecision::ContinueObservation,
        "same action with no new evidence",
    );
    controller
        .run_model_step(&adapter, "local-fixture", "step-1", "same observation")
        .expect("first step completes");
    assert!(matches!(
        controller.run_model_step(&adapter, "local-fixture", "step-2", "same observation"),
        Err(LoopControllerError::BudgetExhausted(
            BudgetKind::SameActionRepetitions
        ))
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(
        events.last().expect("no-progress event exists").event_type,
        "no_progress_detected"
    );
}

#[test]
fn controller_records_policy_and_tool_request_for_allowed_read_only_tool() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-request",
        "project-tool-request",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-request",
        "task-run-tool-request",
        budget,
    )
    .expect("loop starts");
    controller
        .request_tool_call(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            "git_status",
            "step-tool-request",
        )
        .expect("read-only tool request is allowed");
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[1].payload["decision"], "allow");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[2].payload["tool_id"], "git_status");
}

#[test]
fn controller_records_policy_and_denies_forbidden_tool_request() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-denied",
        "project-tool-denied",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["manual_review".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    goal.forbidden_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-denied",
        "task-run-tool-denied",
        budget,
    )
    .expect("loop starts");
    assert!(matches!(
        controller.request_tool_call(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            "git_status",
            "step-tool-denied",
        ),
        Err(LoopControllerError::PolicyDenied {
            decision: PolicyDecision::Block,
            ..
        })
    ));
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[1].payload["decision"], "block");
}

#[test]
fn controller_executes_allowed_git_status_and_records_completion() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-execution",
        "project-tool-execution",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-execution",
        "task-run-tool-execution",
        budget,
    )
    .expect("loop starts");
    let workspace = std::env::current_dir().expect("current directory exists");
    let output = controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-execution",
        )
        .expect("allowed read-only tool executes");
    assert!(output.result["executed"].as_bool().unwrap());
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[3].event_type, "tool_call_completed");
    assert_eq!(events[3].payload["tool_id"], "git_status");
}

#[test]
fn controller_persists_tool_trace_and_explicit_no_side_effect_record() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-audit",
        "project-tool-audit",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-audit",
        "task-run-tool-audit",
        LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1),
    )
    .expect("loop starts")
    .with_audit_stores(
        directory.join("traces.jsonl"),
        directory.join("side-effects.jsonl"),
    );
    let workspace = std::env::current_dir().expect("current directory exists");
    controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-audit",
        )
        .expect("read-only tool executes");
    let traces = TraceStore::open(directory.join("traces.jsonl"))
        .read_all()
        .expect("trace reads");
    let side_effects = SideEffectLedger::open(directory.join("side-effects.jsonl"))
        .read_all()
        .expect("side effect ledger reads");
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].step_id.as_deref(), Some("step-tool-audit"));
    assert_eq!(side_effects.len(), 1);
    assert_eq!(side_effects[0].class, "none");
    assert_eq!(side_effects[0].status, "completed");
}

#[test]
fn controller_records_tool_failure_after_policy_allows_request() {
    let directory = test_data_dir();
    fs::create_dir_all(&directory).expect("test directory creates");
    let log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    let mut goal = GoalContract::new(
        "goal-tool-failure",
        "project-tool-failure",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    let budget = LoopBudget::new(1, 1, 1, 1, 0, 1, 1, 1);
    let mut controller = ReadOnlyLoopController::start(
        log,
        goal,
        "loop-tool-failure",
        "task-run-tool-failure",
        budget,
    )
    .expect("loop starts");
    assert!(controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            std::path::Path::new("/tmp/steerbox-no-such-workspace"),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-tool-failure",
        )
        .is_err());
    let events = EventLog::open(directory.join("events.jsonl"))
        .expect("event log reopens")
        .read_all()
        .expect("events read");
    assert_eq!(events[1].event_type, "policy_decision");
    assert_eq!(events[2].event_type, "tool_call_requested");
    assert_eq!(events[3].event_type, "tool_call_failed");
    assert_eq!(events[3].payload["failure_category"], "permission");
    assert_eq!(
        events[3].payload["tool_call_id"],
        "loop-tool-failure:step-tool-failure:git_status"
    );
    assert_eq!(
        events[3].payload["lesson_candidate"]["lifecycle_state"],
        "candidate"
    );
    assert!(!events[3].payload.to_string().contains("api_key"));
}
