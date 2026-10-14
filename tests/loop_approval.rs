use std::fs;
use steerbox::approval_contract::{ApprovalDecision, ApprovalScope, ApprovalStore};
use steerbox::domain::{GoalContract, HumanSteeringMode, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_contracts::{ActionPathScope, ModelActionEnvelope, VerificationContract};
use steerbox::policy_gate::PolicyGate;
use steerbox::tool_executor::{ReadOnlyToolExecutor, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;

fn controller(root: &std::path::Path) -> ReadOnlyLoopController {
    let mut goal = GoalContract::new(
        "goal",
        "project",
        "edit project",
        vec![".".into()],
        vec![],
        vec!["verified".into()],
    );
    goal.human_steering_mode = HumanSteeringMode::HumanInTheLoop;
    ReadOnlyLoopController::start(
        EventLog::open(root.join("events.jsonl")).unwrap(),
        goal,
        "loop",
        "task",
        LoopBudget::new(20, 0, 0, 1, 0, 20, 1, 1),
    )
    .unwrap()
    .with_checkpoint_store(root.join("checkpoints.jsonl"))
    .with_audit_stores(root.join("traces.jsonl"), root.join("side-effects.jsonl"))
    .with_approval_store(ApprovalStore::open(root.join("approvals.jsonl")))
}

fn action(root: &std::path::Path) -> ModelActionEnvelope {
    ModelActionEnvelope {
        schema_version: 1,
        action_id: "edit-1".into(),
        tool_id: "edit_markdown".into(),
        parameters: serde_json::json!({"path": "README.md", "expected": "before", "replacement": "after"}),
        path_scope: ActionPathScope {
            root: root.display().to_string(),
            allowed_paths: vec!["README.md".into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: serde_json::json!({"path":"README.md"}),
        },
    }
}

fn edit(
    controller: &mut ReadOnlyLoopController,
    root: &std::path::Path,
    action: &ModelActionEnvelope,
) -> Result<steerbox::tool_executor::ToolCallOutput, LoopControllerError> {
    controller.execute_workspace_edit_action(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        &WorkspaceToolExecutor,
        &ReadOnlyToolExecutor,
        root,
        action,
        "step-1",
        false,
    )
}

fn pending_id(
    result: Result<steerbox::tool_executor::ToolCallOutput, LoopControllerError>,
) -> String {
    match result {
        Err(LoopControllerError::ApprovalRequired { request_id }) => request_id,
        other => panic!("expected pending approval, got {other:?}"),
    }
}

#[test]
fn approval_retry_is_stable_and_does_not_repeat_pending_event() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "before").unwrap();
    let mut controller = controller(dir.path());
    let action = action(dir.path());
    let id = pending_id(edit(&mut controller, dir.path(), &action));
    assert_eq!(id, pending_id(edit(&mut controller, dir.path(), &action)));
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "before"
    );
    assert_eq!(controller.status(), LoopStatus::Running);
    let events = EventLog::open(dir.path().join("events.jsonl"))
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "human_intervention_required")
            .count(),
        1
    );
    let approvals = fs::read_to_string(dir.path().join("approvals.jsonl")).unwrap();
    assert!(!approvals.contains("replacement"));
    assert!(!approvals.contains("before"));
    assert!(!approvals.contains("after"));
}

#[test]
fn approved_edit_executes_but_changed_payload_does_not_reuse_approval() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "before").unwrap();
    let mut controller = controller(dir.path());
    let action = action(dir.path());
    let id = pending_id(edit(&mut controller, dir.path(), &action));
    controller
        .approval_store()
        .unwrap()
        .resolve(&id, ApprovalDecision::Approve(ApprovalScope::Once))
        .unwrap();
    let mut changed = action.clone();
    changed.parameters["replacement"] = serde_json::json!("different");
    assert!(edit(&mut controller, dir.path(), &changed).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "before"
    );
    edit(&mut controller, dir.path(), &action).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "after"
    );
}

#[test]
fn rejected_request_stays_denied_and_independent_observation_can_continue() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "before").unwrap();
    let mut controller = controller(dir.path());
    let action = action(dir.path());
    let id = pending_id(edit(&mut controller, dir.path(), &action));
    controller
        .approval_store()
        .unwrap()
        .resolve(&id, ApprovalDecision::Reject)
        .unwrap();
    assert!(matches!(
        edit(&mut controller, dir.path(), &action),
        Err(LoopControllerError::PolicyDenied { .. })
    ));
    controller
        .run_observation("unrelated", "metadata observation")
        .unwrap();
    assert_eq!(controller.status(), LoopStatus::Running);
}

#[test]
fn approval_survives_reopen_but_cannot_override_hard_block() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "before").unwrap();
    let mut first = controller(dir.path());
    let action = action(dir.path());
    let id = pending_id(edit(&mut first, dir.path(), &action));
    drop(first);
    ApprovalStore::open(dir.path().join("approvals.jsonl"))
        .resolve(&id, ApprovalDecision::Approve(ApprovalScope::Once))
        .unwrap();
    let mut goal = GoalContract::new(
        "goal",
        "project",
        "edit project",
        vec![".".into()],
        vec![],
        vec!["verified".into()],
    );
    goal.human_steering_mode = HumanSteeringMode::HumanInTheLoop;
    goal.forbidden_tools.push("edit_markdown".into());
    let mut blocked = ReadOnlyLoopController::start(
        EventLog::open(dir.path().join("events.jsonl")).unwrap(),
        goal,
        "loop",
        "task",
        LoopBudget::new(20, 0, 0, 1, 0, 20, 1, 1),
    )
    .unwrap()
    .with_approval_store(ApprovalStore::open(dir.path().join("approvals.jsonl")));
    assert!(matches!(
        edit(&mut blocked, dir.path(), &action),
        Err(LoopControllerError::PolicyDenied {
            decision: steerbox::policy_gate::PolicyDecision::Block,
            ..
        })
    ));
    assert_eq!(
        blocked
            .approval_store()
            .unwrap()
            .latest(&id)
            .unwrap()
            .status,
        steerbox::approval_contract::ApprovalStatus::Approved
    );
    drop(blocked);
    let mut resumed = controller(dir.path());
    edit(&mut resumed, dir.path(), &action).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "after"
    );
}
