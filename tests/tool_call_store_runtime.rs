use std::fs;

use steerbox::audit_store::SideEffectLedger;
use steerbox::domain::{GoalContract, LoopBudget};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::policy_gate::PolicyGate;
use steerbox::tool_call_store::ToolCallRecordStore;
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput};
use steerbox::tool_registry::ToolRegistry;

fn controller(
    root: &std::path::Path,
    loop_id: &str,
    store: ToolCallRecordStore,
) -> ReadOnlyLoopController {
    let mut goal = GoalContract::new(
        format!("goal-{loop_id}"),
        "project-tool-record",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["git status".into()],
    );
    goal.allowed_tools = vec!["git_status".into()];
    ReadOnlyLoopController::start(
        EventLog::open(root.join(format!("{loop_id}-events.jsonl"))).expect("event log opens"),
        goal,
        loop_id,
        format!("task-{loop_id}"),
        LoopBudget::new(2, 2, 2, 2, 0, 2, 2, 2),
    )
    .expect("controller starts")
    .with_tool_call_store(store)
}

#[test]
fn read_only_runtime_persists_started_completed_and_failed_metadata() {
    let root = tempfile::tempdir().expect("audit root creates");
    let tool_calls = root.path().join("tool-calls.jsonl");
    let mut controller = controller(
        root.path(),
        "loop-tool-record",
        ToolCallRecordStore::open(&tool_calls).expect("tool call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();
    let workspace = tempfile::tempdir().expect("workspace creates");

    controller
        .execute_read_only_tool(
            &registry,
            &gate,
            &ReadOnlyToolExecutor,
            workspace.path(),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-complete",
        )
        .expect("read-only tool completes");
    assert!(controller
        .execute_read_only_tool(
            &registry,
            &gate,
            &ReadOnlyToolExecutor,
            std::path::Path::new("/tmp/steerbox-tool-record-missing-workspace"),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-fail",
        )
        .is_err());

    let records = ToolCallRecordStore::open(&tool_calls)
        .expect("tool call store reopens")
        .read_page(0, 10)
        .expect("tool call records read");
    assert_eq!(
        records
            .iter()
            .map(|record| record.status.as_str())
            .collect::<Vec<_>>(),
        ["started", "completed", "started", "failed"]
    );
    assert_eq!(records[3].summary, "tool call failed; error redacted");
    assert!(records[0]
        .tool_call_id
        .starts_with("loop-tool-record:tool:"));
    assert!(records[2]
        .tool_call_id
        .starts_with("loop-tool-record:tool:"));
    assert_ne!(records[0].tool_call_id, records[2].tool_call_id);
    let persisted = fs::read_to_string(tool_calls).expect("tool call log reads");
    assert!(!persisted.contains("arguments"));
    assert!(!persisted.contains("raw_output"));
    assert!(!persisted.contains("error_body"));
    assert!(!persisted.contains("steerbox-tool-record-missing-workspace"));
}

#[test]
fn repeated_same_step_tool_calls_get_distinct_call_ids() {
    let root = tempfile::tempdir().expect("audit root creates");
    let tool_calls = root.path().join("tool-calls.jsonl");
    let mut controller = controller(
        root.path(),
        "loop-tool-record-repeat",
        ToolCallRecordStore::open(&tool_calls).expect("tool call store opens"),
    );
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let workspace = tempfile::tempdir().expect("workspace creates");

    for _ in 0..2 {
        controller
            .execute_read_only_tool(
                &registry,
                &PolicyGate::new(),
                &ReadOnlyToolExecutor,
                workspace.path(),
                ToolCallInput {
                    tool_id: "git_status".into(),
                    arguments: serde_json::json!({}),
                },
                "same-step",
            )
            .expect("repeated read-only tool completes");
    }

    let records = ToolCallRecordStore::open(&tool_calls)
        .expect("tool call store reopens")
        .read_page(0, 10)
        .expect("tool call records read");
    assert_eq!(records.len(), 4);
    assert_eq!(records[0].status, "started");
    assert_eq!(records[1].status, "completed");
    assert_eq!(records[2].status, "started");
    assert_eq!(records[3].status, "completed");
    assert_eq!(records[0].tool_call_id, records[1].tool_call_id);
    assert_eq!(records[2].tool_call_id, records[3].tool_call_id);
    assert_ne!(records[0].tool_call_id, records[2].tool_call_id);
}

#[test]
fn tool_call_store_failure_stops_before_execution_and_completion() {
    let root = tempfile::tempdir().expect("audit root creates");
    let failing_path = root.path().join("tool-call-directory");
    fs::write(&failing_path, "").expect("store file creates");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&failing_path, fs::Permissions::from_mode(0o600))
            .expect("store permissions apply");
    }
    let store = ToolCallRecordStore::open(&failing_path).expect("tool call store opens");
    fs::remove_file(&failing_path).expect("store file removes");
    fs::create_dir(&failing_path).expect("failing store path creates");
    let mut controller = controller(root.path(), "loop-tool-record-fail-closed", store);
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let workspace = tempfile::tempdir().expect("workspace creates");

    let result = controller.execute_read_only_tool(
        &registry,
        &PolicyGate::new(),
        &ReadOnlyToolExecutor,
        workspace.path(),
        ToolCallInput {
            tool_id: "git_status".into(),
            arguments: serde_json::json!({}),
        },
        "step-store-failure",
    );
    assert!(matches!(result, Err(LoopControllerError::ToolCallStore(_))));
    assert!(!workspace.path().join(".git").exists());
    let events = EventLog::open(
        root.path()
            .join("loop-tool-record-fail-closed-events.jsonl"),
    )
    .expect("event log reopens")
    .read_all()
    .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "tool_call_requested"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "tool_call_completed"));
}

#[test]
fn tool_call_runtime_does_not_change_side_effect_audit_semantics() {
    let root = tempfile::tempdir().expect("audit root creates");
    let tool_calls = root.path().join("tool-calls.jsonl");
    let side_effects = root.path().join("side-effects.jsonl");
    let mut controller = controller(
        root.path(),
        "loop-tool-record-side-effect",
        ToolCallRecordStore::open(&tool_calls).expect("tool call store opens"),
    )
    .with_audit_stores(root.path().join("traces.jsonl"), &side_effects);
    let workspace = tempfile::tempdir().expect("workspace creates");
    controller
        .execute_read_only_tool(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            workspace.path(),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: serde_json::json!({}),
            },
            "step-side-effect",
        )
        .expect("read-only tool completes");
    let records = SideEffectLedger::open(side_effects)
        .read_all()
        .expect("side effect records read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "completed");
    assert_eq!(records[0].class, "none");
}
