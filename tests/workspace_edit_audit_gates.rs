//! Pre-mutation gates only: no rollback guarantee, Bash expansion, or second
//! EventLog append fault injection is claimed by these tests.
use std::fs;
use std::path::Path;
use steerbox::domain::{GoalContract, HumanSteeringMode, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_contracts::{ActionPathScope, ModelActionEnvelope, VerificationContract};
use steerbox::policy_gate::{PolicyDecision, PolicyGate, PolicyRequest};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallOutput, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;
use steerbox::user_command_inbox::{CommandKind, UserCommand, UserCommandInbox};

const ORIGINAL: &[u8] = b"# Original\r\nbefore\r\n\r\nExact bytes stay intact.\r\n";

fn controller(
    root: &Path,
    autonomous: bool,
    checkpoint: bool,
    ledger: bool,
) -> ReadOnlyLoopController {
    fs::write(root.join("README.md"), ORIGINAL).unwrap();
    let mut goal = GoalContract::new(
        "goal-gates",
        "project-gates",
        "edit bounded markdown",
        vec![".".into()],
        vec![],
        vec!["verified".into()],
    );
    goal.allowed_tools = vec!["edit_markdown".into(), "markdown_link_check".into()];
    goal.human_steering_mode = if autonomous {
        HumanSteeringMode::AutonomousLoop
    } else {
        HumanSteeringMode::HumanInTheLoop
    };
    let request = PolicyRequest {
        policy_event_id: "fixture-policy".into(),
        tool_id: "edit_markdown".into(),
        project_id: "project-gates".into(),
        goal_id: "goal-gates".into(),
        task_run_id: "task-gates".into(),
        loop_run_id: "loop-gates".into(),
        step_id: "step-gates".into(),
        correlation_id: "loop-gates".into(),
        human_steering_mode: goal.human_steering_mode,
    };
    assert_eq!(
        PolicyGate::new()
            .evaluate(&ToolRegistry::baseline().unwrap(), &goal, &request)
            .decision,
        if autonomous {
            PolicyDecision::Allow
        } else {
            PolicyDecision::RequireHumanApproval
        },
    );
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(root.join("events.jsonl")).unwrap(),
        goal,
        "loop-gates",
        "task-gates",
        LoopBudget::new(20, 0, 0, 1, 0, 20, 1, 1),
    )
    .unwrap();
    if checkpoint {
        controller = controller.with_checkpoint_store(root.join("checkpoints.jsonl"));
    }
    if ledger {
        controller = controller
            .with_audit_stores(root.join("traces.jsonl"), root.join("side-effects.jsonl"));
    }
    controller
}

fn edit(
    controller: &mut ReadOnlyLoopController,
    root: &Path,
    approved: bool,
) -> Result<ToolCallOutput, LoopControllerError> {
    let action = ModelActionEnvelope {
        schema_version: 1,
        action_id: "action-gates".into(),
        tool_id: "edit_markdown".into(),
        parameters: serde_json::json!({
            "path": "README.md",
            "expected": std::str::from_utf8(ORIGINAL).unwrap(),
            "replacement": "# Changed\nafter\n"
        }),
        path_scope: ActionPathScope {
            root: root.display().to_string(),
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path":"README.md"}),
        },
    };
    controller.execute_workspace_edit_action(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        &WorkspaceToolExecutor,
        &ReadOnlyToolExecutor,
        root,
        &action,
        "step-gates",
        approved,
    )
}

fn assert_original(root: &Path) {
    assert_eq!(
        fs::read(root.join("README.md")).unwrap(),
        ORIGINAL,
        "edit must preserve exact original bytes"
    );
}

fn assert_no_workspace_audit_files(root: &Path) {
    for file in ["checkpoints.jsonl", "traces.jsonl", "side-effects.jsonl"] {
        assert!(
            !root.join(file).exists(),
            "unexpected pre-write audit: {file}"
        );
    }
}

fn assert_no_mutation_events(path: &Path, started_is_forbidden: bool) {
    let events = EventLog::open(path).unwrap().read_all().unwrap();
    assert!(
        events.iter().all(|event| {
            !matches!(
                event.event_type.as_str(),
                "file_change_recorded" | "model_action_verified"
            ) && !(started_is_forbidden && event.event_type == "model_action_started")
        }),
        "no edit mutation event may be emitted"
    );
}

fn missing_stores(autonomous: bool, checkpoint: bool, ledger: bool, expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), autonomous, checkpoint, ledger);
    let result = edit(&mut controller, dir.path(), !autonomous);
    assert_original(dir.path());
    assert_no_workspace_audit_files(dir.path());
    assert_no_mutation_events(&dir.path().join("events.jsonl"), true);
    match result {
        Err(LoopControllerError::WorkspaceToolExecution(message)) => assert!(
            message.contains(expected),
            "unexpected gate error: {message}"
        ),
        other => panic!("expected missing {expected} audit gate, got {other:?}"),
    }
}

#[test]
fn policy_allowed_edit_requires_checkpoint() {
    missing_stores(true, false, true, "checkpoint");
}
#[test]
fn explicitly_approved_edit_requires_checkpoint() {
    missing_stores(false, false, true, "checkpoint");
}
#[test]
fn policy_allowed_edit_requires_side_effect_ledger() {
    missing_stores(true, true, false, "side-effect");
}
#[test]
fn explicitly_approved_edit_requires_side_effect_ledger() {
    missing_stores(false, true, false, "side-effect");
}
#[test]
fn policy_allowed_edit_requires_audit_stores_when_both_missing() {
    missing_stores(true, false, false, "checkpoint");
}
#[test]
fn explicitly_approved_edit_requires_audit_stores_when_both_missing() {
    missing_stores(false, false, false, "checkpoint");
}

fn pending_command_blocks_edit(kind: CommandKind, expected_status: LoopStatus) {
    let dir = tempfile::tempdir().unwrap();
    let inbox = UserCommandInbox::open(dir.path().join("commands.jsonl"));
    let mut controller = controller(dir.path(), true, true, true).with_command_inbox(inbox.clone());
    inbox
        .submit(
            UserCommand::new(
                "command-gates",
                "operator",
                "project-gates",
                "goal-gates",
                "task-gates",
                "loop-gates",
                kind,
                "corr-gates",
                serde_json::json!({"reason":"stop before mutation"}),
            )
            .unwrap(),
        )
        .unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_workspace_audit_files(dir.path());
    assert_no_mutation_events(&dir.path().join("events.jsonl"), true);
    assert_eq!(controller.status(), expected_status);
    match expected_status {
        LoopStatus::Paused => assert!(matches!(result, Err(LoopControllerError::Paused))),
        LoopStatus::Cancelled => assert!(matches!(result, Err(LoopControllerError::Terminal))),
        other => panic!("unexpected expected status: {other:?}"),
    }
}

#[test]
fn pending_pause_blocks_edit_before_mutation() {
    pending_command_blocks_edit(CommandKind::Pause, LoopStatus::Paused);
}
#[test]
fn pending_cancel_blocks_edit_before_mutation() {
    pending_command_blocks_edit(CommandKind::Cancel, LoopStatus::Cancelled);
}

#[test]
fn completed_loop_blocks_edit_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    controller.complete().unwrap();
    let events_before = fs::read(dir.path().join("events.jsonl")).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_workspace_audit_files(dir.path());
    assert_eq!(
        fs::read(dir.path().join("events.jsonl")).unwrap(),
        events_before
    );
    assert_no_mutation_events(&dir.path().join("events.jsonl"), true);
    assert!(matches!(result, Err(LoopControllerError::Terminal)));
    assert_eq!(controller.status(), LoopStatus::Completed);
}

#[test]
fn failed_loop_blocks_edit_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    controller.fail("fixture terminal failure").unwrap();
    let events_before = fs::read(dir.path().join("events.jsonl")).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_workspace_audit_files(dir.path());
    assert_eq!(
        fs::read(dir.path().join("events.jsonl")).unwrap(),
        events_before
    );
    assert_no_mutation_events(&dir.path().join("events.jsonl"), true);
    assert!(matches!(result, Err(LoopControllerError::Terminal)));
    assert_eq!(controller.status(), LoopStatus::Failed);
}

#[test]
fn first_policy_event_append_failure_prevents_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    let event_path = dir.path().join("events.jsonl");
    let saved_path = dir.path().join("events.saved.jsonl");
    fs::rename(&event_path, &saved_path).unwrap();
    fs::create_dir(&event_path).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_mutation_events(&saved_path, true);
    assert!(matches!(result, Err(LoopControllerError::PolicyGate(_))));
}

#[test]
fn pre_write_checkpoint_append_failure_prevents_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    fs::create_dir(dir.path().join("checkpoints.jsonl")).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_mutation_events(&dir.path().join("events.jsonl"), true);
    assert!(matches!(result, Err(LoopControllerError::AuditStore(_))));
}

#[test]
fn pre_mutation_trace_append_failure_prevents_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    fs::create_dir(dir.path().join("traces.jsonl")).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_mutation_events(&dir.path().join("events.jsonl"), false);
    assert!(matches!(result, Err(LoopControllerError::AuditStore(_))));
}

#[test]
fn pre_mutation_side_effect_append_failure_prevents_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut controller = controller(dir.path(), true, true, true);
    fs::create_dir(dir.path().join("side-effects.jsonl")).unwrap();
    let result = edit(&mut controller, dir.path(), false);
    assert_original(dir.path());
    assert_no_mutation_events(&dir.path().join("events.jsonl"), false);
    assert!(matches!(result, Err(LoopControllerError::AuditStore(_))));
}
