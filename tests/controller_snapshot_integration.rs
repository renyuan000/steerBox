//! Controller opt-in snapshot integration. Restore is explicit; verification
//! failure is recorded as restore-required and never auto-restored.
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use steerbox::approval_contract::{
    ApprovalDecision, ApprovalRequest, ApprovalScope, ApprovalStore,
};
use steerbox::audit_store::{CheckpointStore, RecoveryAction, SideEffectLedger};
use steerbox::domain::{GoalContract, HumanSteeringMode, LoopBudget};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::ReadOnlyLoopController;
use steerbox::model_contracts::{ActionPathScope, ModelActionEnvelope, VerificationContract};
use steerbox::policy_gate::PolicyGate;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject};
use steerbox::tool_executor::{ReadOnlyToolExecutor, WorkspaceToolExecutor};
use steerbox::tool_registry::ToolRegistry;
use steerbox::user_command_inbox::{CommandKind, UserCommand, UserCommandInbox};

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone)]
struct RecordingSink(Arc<Mutex<Vec<ProgressEvent>>>);

impl ProgressSink for RecordingSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    workspace: PathBuf,
    snapshot_root: PathBuf,
    events: PathBuf,
    checkpoints: PathBuf,
    traces: PathBuf,
    side_effects: PathBuf,
    goal: GoalContract,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), "before\n").unwrap();
        let snapshot_root = dir.path().join("snapshots");
        let mut goal = GoalContract::new(
            "goal-controller-snapshot",
            "project-controller-snapshot",
            "edit bounded markdown",
            vec![".".into()],
            Vec::new(),
            vec!["verified".into()],
        );
        goal.allowed_tools = vec![
            "edit_markdown".into(),
            "markdown_link_check".into(),
            "restore_workspace_snapshot".into(),
        ];
        goal.human_steering_mode = HumanSteeringMode::HumanInTheLoop;
        Self {
            events: workspace.join("events.jsonl"),
            checkpoints: workspace.join("checkpoints.jsonl"),
            traces: workspace.join("traces.jsonl"),
            side_effects: workspace.join("side-effects.jsonl"),
            _dir: dir,
            workspace,
            snapshot_root,
            goal,
        }
    }

    fn controller_with_snapshot_root(
        &self,
        snapshot_root: Option<&Path>,
    ) -> ReadOnlyLoopController {
        let controller = ReadOnlyLoopController::start(
            EventLog::open(&self.events).unwrap(),
            self.goal.clone(),
            "loop-controller-snapshot",
            "task-controller-snapshot",
            LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
        )
        .unwrap()
        .with_checkpoint_store(&self.checkpoints)
        .with_audit_stores(&self.traces, &self.side_effects);
        match snapshot_root {
            Some(root) => controller.with_snapshot_store(root),
            None => controller,
        }
    }

    fn controller(&self) -> ReadOnlyLoopController {
        self.controller_with_snapshot_root(Some(&self.snapshot_root))
    }

    fn controller_without_snapshot(&self) -> ReadOnlyLoopController {
        self.controller_with_snapshot_root(None)
    }

    fn controller_with_snapshot_root_path(&self, root: &Path) -> ReadOnlyLoopController {
        ReadOnlyLoopController::start(
            EventLog::open(&self.events).unwrap(),
            self.goal.clone(),
            "loop-controller-snapshot",
            "task-controller-snapshot",
            LoopBudget::new(10, 0, 0, 1, 0, 10, 1, 1),
        )
        .unwrap()
        .with_checkpoint_store(&self.checkpoints)
        .with_audit_stores(&self.traces, &self.side_effects)
        .with_snapshot_store(root)
    }

    fn action(&self, replacement: &str) -> ModelActionEnvelope {
        ModelActionEnvelope {
            schema_version: 1,
            action_id: "controller-snapshot-edit".into(),
            tool_id: "edit_markdown".into(),
            parameters: serde_json::json!({
                "path": "README.md",
                "expected": "before\n",
                "replacement": replacement
            }),
            path_scope: ActionPathScope {
                root: self.workspace.display().to_string(),
                allowed_paths: vec!["README.md".into()],
            },
            verification: VerificationContract {
                verifier_tool_id: "markdown_link_check".into(),
                parameters: serde_json::json!({"path": "README.md"}),
            },
        }
    }

    fn run_edit(
        &self,
        controller: &mut ReadOnlyLoopController,
        action: &ModelActionEnvelope,
        step_id: &str,
    ) -> Result<
        steerbox::tool_executor::ToolCallOutput,
        steerbox::loop_controller::LoopControllerError,
    > {
        controller.execute_workspace_edit_action(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            &WorkspaceToolExecutor,
            &ReadOnlyToolExecutor,
            &self.workspace,
            action,
            step_id,
            true,
        )
    }

    fn with_restore_approval(
        &self,
        controller: ReadOnlyLoopController,
        request_id: &str,
        snapshot_id: &str,
        expected_after_hash: &str,
    ) -> ReadOnlyLoopController {
        let controller = controller
            .with_approval_store(ApprovalStore::open(self.workspace.join("approvals.jsonl")));
        let request = ApprovalRequest::at(
            request_id,
            self.goal.project_id.clone(),
            "loop-controller-snapshot",
            "task-controller-snapshot",
            "restore_workspace_snapshot",
            restore_capability(snapshot_id, "README.md", expected_after_hash),
            "metadata_only",
            "restore one controller-bound workspace snapshot",
            Utc::now(),
            Utc::now() + Duration::minutes(15),
        )
        .unwrap();
        controller
            .approval_store()
            .unwrap()
            .submit(request)
            .unwrap();
        controller
            .approval_store()
            .unwrap()
            .resolve(request_id, ApprovalDecision::Approve(ApprovalScope::Once))
            .unwrap();
        controller
    }

    fn restore_authorized(
        &self,
        controller: &mut ReadOnlyLoopController,
        request_id: &str,
        step_id: &str,
        snapshot_id: &str,
        expected_after_hash: &str,
    ) -> Result<steerbox::workspace_edit::EditResult, steerbox::loop_controller::LoopControllerError>
    {
        controller.restore_workspace_snapshot_with_authorization(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            steerbox::loop_controller::RestoreApprovalContext {
                approval_request_id: request_id.into(),
                project_id: self.goal.project_id.clone(),
                goal_id: self.goal.goal_id.clone(),
                task_run_id: "task-controller-snapshot".into(),
                loop_run_id: "loop-controller-snapshot".into(),
                step_id: step_id.into(),
                snapshot_id: snapshot_id.into(),
                expected_after_hash: expected_after_hash.into(),
            },
        )
    }

    fn snapshot_ids(&self) -> Vec<String> {
        let mut ids = fs::read_dir(&self.snapshot_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn events(&self) -> Vec<steerbox::domain::EventEnvelope> {
        EventLog::open(&self.events).unwrap().read_all().unwrap()
    }
}

fn restore_capability(snapshot_id: &str, relative_path: &str, expected_after_hash: &str) -> String {
    let material = [snapshot_id, relative_path, expected_after_hash].join("\u{1f}");
    format!(
        "workspace-snapshot-restore:{:x}",
        Sha256::digest(material.as_bytes())
    )
}

#[test]
fn opt_in_controller_edit_captures_snapshot_before_mutation_and_explicit_restore_works() {
    let fixture = Fixture::new();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let mut controller = fixture
        .controller()
        .with_progress_sink(RecordingSink(progress.clone()));
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-snapshot-success",
        )
        .expect("approved edit succeeds");
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    let events = progress.lock().unwrap().clone();
    let workspace_events = events
        .iter()
        .filter(|event| {
            event.subject == ProgressSubject::Tool
                && event.subject_id == "step-snapshot-success:workspace_write"
        })
        .collect::<Vec<_>>();
    assert_eq!(workspace_events.len(), 2);
    assert_eq!(workspace_events[0].status, ProgressStatus::Started);
    assert_eq!(
        workspace_events[0].summary.as_deref(),
        Some("workspace mutation started")
    );
    assert_eq!(workspace_events[1].status, ProgressStatus::Completed);
    assert_eq!(
        workspace_events[1].summary.as_deref(),
        Some("workspace mutation completed")
    );
    assert_eq!(fixture.snapshot_ids().len(), 1);
    let events = fixture.events();
    let captured = events
        .iter()
        .find(|event| event.event_type == "workspace_snapshot_captured")
        .expect("controller records snapshot capture");
    assert_eq!(captured.payload["relative_path"], "README.md");
    assert_eq!(captured.payload["before_hash"], sha256(b"before\n"));
    let snapshot_id = captured.payload["snapshot_id"].as_str().unwrap();
    assert_eq!(snapshot_id, fixture.snapshot_ids()[0]);

    controller = fixture.with_restore_approval(
        controller,
        "restore-snapshot-success",
        snapshot_id,
        &sha256(b"after\n"),
    );
    fixture
        .restore_authorized(
            &mut controller,
            "restore-snapshot-success",
            "step-snapshot-success",
            snapshot_id,
            &sha256(b"after\n"),
        )
        .expect("authorized restore succeeds");
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn verification_failure_records_restore_required_without_automatic_restore() {
    let fixture = Fixture::new();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let mut controller = fixture
        .controller()
        .with_progress_sink(RecordingSink(progress.clone()));
    let result = fixture.run_edit(
        &mut controller,
        &fixture.action("after\n[broken](missing.md)"),
        "step-snapshot-verify-failed",
    );
    assert!(result.is_err(), "semantic verification must fail");
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n[broken](missing.md)"
    );
    assert_eq!(fixture.snapshot_ids().len(), 1);
    let recovery = CheckpointStore::open(&fixture.checkpoints)
        .latest_for_loop_step("loop-controller-snapshot", "step-snapshot-verify-failed")
        .unwrap()
        .expect("verification checkpoint exists");
    assert_eq!(recovery.action, RecoveryAction::RestoreRequired);
    let events = fixture.events();
    let snapshot_id = events
        .iter()
        .find(|event| event.event_type == "workspace_snapshot_captured")
        .and_then(|event| event.payload["snapshot_id"].as_str())
        .expect("snapshot capture event contains id")
        .to_owned();
    assert!(recovery
        .metadata
        .side_effect_refs
        .contains(&format!("snapshot:{snapshot_id}")));
    assert!(fixture
        .events()
        .iter()
        .all(|event| event.event_type != "workspace_snapshot_restored"));
    let events = fixture.events();
    let recovery_event = events
        .iter()
        .find(|event| event.event_type == "workspace_recovery_required")
        .expect("recovery required event exists");
    assert_eq!(
        recovery_event.payload["recovery_action"],
        "restore_required"
    );
    assert_eq!(recovery_event.payload["side_effect_status"], "uncertain");
    assert_eq!(recovery_event.payload["relative_path"], "README.md");
    assert_eq!(recovery_event.payload["snapshot_id"], snapshot_id);
    assert!(recovery_event.payload.get("before").is_none());
    assert!(recovery_event.payload.get("after").is_none());
    let progress_events = progress.lock().unwrap().clone();
    let workspace_events = progress_events
        .iter()
        .filter(|event| {
            event.subject == ProgressSubject::Tool
                && event.subject_id == "step-snapshot-verify-failed:workspace_write"
        })
        .collect::<Vec<_>>();
    assert_eq!(workspace_events.len(), 2);
    assert_eq!(workspace_events[0].status, ProgressStatus::Started);
    assert_eq!(workspace_events[1].status, ProgressStatus::Completed);
}

#[test]
fn mutation_failure_reports_failed_without_leaking_error_details() {
    let fixture = Fixture::new();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let mut controller = fixture
        .controller()
        .with_progress_sink(RecordingSink(progress.clone()));
    let mut action = fixture.action("after\n");
    action.parameters["expected"] = serde_json::json!("not the current content");

    let result = fixture.run_edit(&mut controller, &action, "step-snapshot-mutation-failed");
    assert!(
        result.is_err(),
        "stale expected content must reject mutation"
    );
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );

    let workspace_events = progress
        .lock()
        .unwrap()
        .iter()
        .filter(|event| {
            event.subject == ProgressSubject::Tool
                && event.subject_id == "step-snapshot-mutation-failed:workspace_write"
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(workspace_events.len(), 2);
    assert_eq!(workspace_events[0].status, ProgressStatus::Started);
    assert_eq!(workspace_events[1].status, ProgressStatus::Failed);
    assert_eq!(
        workspace_events[1].summary.as_deref(),
        Some("workspace mutation failed")
    );
    assert!(workspace_events
        .iter()
        .all(|event| event.summary.as_deref().is_none_or(|summary| {
            !summary.contains("README") && !summary.contains("not the current content")
        })));
    assert!(fixture
        .events()
        .iter()
        .any(|event| event.event_type == "model_action_failed"));
}

#[test]
fn explicit_restore_keeps_approval_and_expected_after_hash_boundaries() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-snapshot-boundary",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    controller = fixture.with_restore_approval(
        controller,
        "restore-boundary",
        &snapshot_id,
        &sha256(b"after\n"),
    );
    assert!(controller
        .restore_workspace_snapshot_with_authorization(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            steerbox::loop_controller::RestoreApprovalContext {
                approval_request_id: "missing-restore-boundary".into(),
                project_id: fixture.goal.project_id.clone(),
                goal_id: fixture.goal.goal_id.clone(),
                task_run_id: "task-controller-snapshot".into(),
                loop_run_id: "loop-controller-snapshot".into(),
                step_id: "step-snapshot-boundary".into(),
                snapshot_id: snapshot_id.clone(),
                expected_after_hash: sha256(b"after\n"),
            },
        )
        .is_err());
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    assert!(fixture
        .restore_authorized(
            &mut controller,
            "restore-boundary",
            "step-snapshot-boundary",
            &snapshot_id,
            &sha256(b"wrong-current-version"),
        )
        .is_err());
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    fixture
        .restore_authorized(
            &mut controller,
            "restore-boundary",
            "step-snapshot-boundary",
            &snapshot_id,
            &sha256(b"after\n"),
        )
        .unwrap();
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn invalid_expected_after_hash_is_rejected_before_restore_event() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-invalid-restore-hash",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    controller = fixture.with_restore_approval(
        controller,
        "restore-invalid-hash",
        &snapshot_id,
        &sha256(b"after\n"),
    );
    let invalid_hashes = [
        "sha256:not-a-hash".to_owned(),
        format!("sha256:{}", "a".repeat(65)),
    ];

    for invalid_hash in invalid_hashes {
        let error = fixture
            .restore_authorized(
                &mut controller,
                "restore-invalid-hash",
                "step-invalid-restore-hash",
                &snapshot_id,
                &invalid_hash,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid expected current content hash"));
        assert!(!error.contains(&invalid_hash));
        assert!(!fixture
            .events()
            .iter()
            .any(|event| { event.event_type == "workspace_snapshot_restore_requested" }));
        let serialized = serde_json::to_string(&fixture.events()).unwrap();
        assert!(!serialized.contains(&invalid_hash));
    }
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
}

#[test]
fn snapshot_binding_is_not_registered_before_pre_mutation_audit_completes() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.checkpoints).unwrap();
    let mut controller = fixture.controller();
    let result = fixture.run_edit(
        &mut controller,
        &fixture.action("after\n"),
        "step-binding-audit-failure",
    );
    assert!(result.is_err());
    assert_eq!(fixture.snapshot_ids().len(), 1);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let restore = controller.restore_workspace_snapshot_with_authorization(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        steerbox::loop_controller::RestoreApprovalContext {
            approval_request_id: "unbound-restore".into(),
            project_id: fixture.goal.project_id.clone(),
            goal_id: fixture.goal.goal_id.clone(),
            task_run_id: "task-controller-snapshot".into(),
            loop_run_id: "loop-controller-snapshot".into(),
            step_id: "step-binding-audit-failure".into(),
            snapshot_id,
            expected_after_hash: sha256(b"before\n"),
        },
    );
    assert!(restore
        .unwrap_err()
        .to_string()
        .contains("snapshot is not bound to this controller run"));
}

#[test]
fn capture_failure_is_pre_mutation_and_does_not_start_tool_ledger() {
    let fixture = Fixture::new();
    let invalid_root = fixture.workspace.join("snapshots-inside-workspace");
    let mut controller = fixture.controller_with_snapshot_root_path(&invalid_root);
    let result = fixture.run_edit(
        &mut controller,
        &fixture.action("after\n"),
        "step-snapshot-capture-failed",
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    assert!(!fixture.events().iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "workspace_snapshot_captured" | "model_action_started"
        )
    }));
    assert!(!SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .iter()
        .any(|record| record.status == "started"));
}

#[cfg(unix)]
#[test]
fn public_snapshot_root_capture_failure_does_not_mutate_or_start_tool() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fs::create_dir(&fixture.snapshot_root).unwrap();
    fs::set_permissions(&fixture.snapshot_root, fs::Permissions::from_mode(0o755)).unwrap();
    let mut controller = fixture.controller();
    let result = fixture.run_edit(
        &mut controller,
        &fixture.action("after\n"),
        "step-snapshot-public-root",
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    assert!(!fixture
        .events()
        .iter()
        .any(|event| event.event_type == "model_action_started"));
    assert!(!SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .iter()
        .any(|record| record.status == "started"));
}

#[test]
fn controller_without_snapshot_store_keeps_existing_edit_behavior_without_snapshot_event() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller_without_snapshot();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-no-snapshot",
        )
        .expect("existing edit path remains usable without opt-in snapshots");
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    assert!(!fixture.snapshot_root.exists());
    assert!(fixture
        .events()
        .iter()
        .all(|event| event.event_type != "workspace_snapshot_captured"));
}

#[test]
fn restore_rejects_paused_and_terminal_controller_states() {
    let fixture = Fixture::new();
    let inbox = UserCommandInbox::open(fixture._dir.path().join("commands.jsonl"));
    let mut paused_controller = fixture.controller().with_command_inbox(inbox.clone());
    fixture
        .run_edit(
            &mut paused_controller,
            &fixture.action("after\n"),
            "step-restore-paused",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    inbox
        .submit(
            UserCommand::new(
                "pause-before-restore",
                "operator",
                &fixture.goal.project_id,
                &fixture.goal.goal_id,
                "task-controller-snapshot",
                "loop-controller-snapshot",
                CommandKind::Pause,
                "restore-boundary",
                serde_json::json!({"reason":"pause before explicit restore"}),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        fixture.restore_authorized(
            &mut paused_controller,
            "paused-restore",
            "step-restore-paused",
            &snapshot_id,
            &sha256(b"after\n"),
        ),
        Err(steerbox::loop_controller::LoopControllerError::Paused)
    ));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );

    let mut terminal_controller = fixture.controller_without_snapshot();
    terminal_controller.complete().unwrap();
    assert!(fixture
        .restore_authorized(
            &mut terminal_controller,
            "terminal-restore",
            "missing-step",
            "missing",
            &sha256(b"after\n"),
        )
        .is_err());
}

#[test]
fn restore_preflight_failure_keeps_metadata_and_file_body_out_of_events() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-failed",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    controller = fixture.with_restore_approval(
        controller,
        "restore-preflight-failure",
        &snapshot_id,
        &sha256(b"after\n"),
    );
    fs::write(
        fixture.workspace.join("README.md"),
        "human changed the file\n",
    )
    .unwrap();
    assert!(fixture
        .restore_authorized(
            &mut controller,
            "restore-preflight-failure",
            "step-restore-failed",
            &snapshot_id,
            &sha256(b"after\n"),
        )
        .is_err());
    let events = fixture.events();
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("human changed the file"));
    assert!(!serialized.contains("after\n"));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"human changed the file\n"
    );
}

#[test]
fn authorized_snapshot_restore_uses_policy_approval_and_side_effect_ledger() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-authorized-restore",
        )
        .unwrap();
    controller = controller.with_approval_store(ApprovalStore::open(
        fixture.workspace.join("approvals.jsonl"),
    ));
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let expected_after_hash = sha256(b"after\n");
    let capability = restore_capability(&snapshot_id, "README.md", &expected_after_hash);
    let request = ApprovalRequest::at(
        "restore-approval",
        fixture.goal.project_id.clone(),
        "loop-controller-snapshot",
        "task-controller-snapshot",
        "restore_workspace_snapshot",
        capability,
        "metadata_only",
        "restore one controller-bound workspace snapshot",
        Utc::now(),
        Utc::now() + Duration::minutes(15),
    )
    .unwrap();
    controller
        .approval_store()
        .unwrap()
        .submit(request)
        .unwrap();
    controller
        .approval_store()
        .unwrap()
        .resolve(
            "restore-approval",
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .unwrap();

    let edit = controller
        .restore_workspace_snapshot_with_authorization(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            steerbox::loop_controller::RestoreApprovalContext {
                approval_request_id: "restore-approval".into(),
                project_id: fixture.goal.project_id.clone(),
                goal_id: fixture.goal.goal_id.clone(),
                task_run_id: "task-controller-snapshot".into(),
                loop_run_id: "loop-controller-snapshot".into(),
                step_id: "step-authorized-restore".into(),
                snapshot_id: snapshot_id.clone(),
                expected_after_hash,
            },
        )
        .unwrap();
    assert!(edit.changed);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    let ledger = SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .into_iter()
        .filter(|record| record.side_effect_id == format!("{snapshot_id}:restore"))
        .collect::<Vec<_>>();
    assert_eq!(
        ledger
            .iter()
            .map(|record| record.status.as_str())
            .collect::<Vec<_>>(),
        vec!["planned", "started", "completed"]
    );
    let events = fixture.events();
    assert!(events
        .iter()
        .any(|event| event.event_type == "policy_decision"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "workspace_snapshot_restore_requested"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "workspace_snapshot_restored"));
}

#[test]
fn snapshot_restore_without_matching_approval_does_not_modify_workspace() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-needs-approval",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let result = controller.restore_workspace_snapshot_with_authorization(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        steerbox::loop_controller::RestoreApprovalContext {
            approval_request_id: "missing-approval".into(),
            project_id: fixture.goal.project_id.clone(),
            goal_id: fixture.goal.goal_id.clone(),
            task_run_id: "task-controller-snapshot".into(),
            loop_run_id: "loop-controller-snapshot".into(),
            step_id: "step-restore-needs-approval".into(),
            snapshot_id,
            expected_after_hash: sha256(b"after\n"),
        },
    );
    assert!(matches!(
        result,
        Err(steerbox::loop_controller::LoopControllerError::ApprovalRequired { .. })
    ));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    assert!(!SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .iter()
        .any(|record| record.tool_id == "restore_workspace_snapshot"));
}

#[test]
fn restore_ledger_preflight_failure_does_not_consume_approval_or_modify_workspace() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-ledger-failure",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let expected_after_hash = sha256(b"after\n");
    let approval_store = ApprovalStore::open(fixture.workspace.join("approvals.jsonl"));
    controller = controller.with_approval_store(approval_store);
    let request = ApprovalRequest::at(
        "restore-ledger-failure",
        fixture.goal.project_id.clone(),
        "loop-controller-snapshot",
        "task-controller-snapshot",
        "restore_workspace_snapshot",
        restore_capability(&snapshot_id, "README.md", &expected_after_hash),
        "metadata_only",
        "restore one controller-bound workspace snapshot",
        Utc::now(),
        Utc::now() + Duration::minutes(15),
    )
    .unwrap();
    controller
        .approval_store()
        .unwrap()
        .submit(request)
        .unwrap();
    controller
        .approval_store()
        .unwrap()
        .resolve(
            "restore-ledger-failure",
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .unwrap();

    fs::remove_file(&fixture.side_effects).unwrap();
    fs::create_dir(&fixture.side_effects).unwrap();
    let result = controller.restore_workspace_snapshot_with_authorization(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        steerbox::loop_controller::RestoreApprovalContext {
            approval_request_id: "restore-ledger-failure".into(),
            project_id: fixture.goal.project_id.clone(),
            goal_id: fixture.goal.goal_id.clone(),
            task_run_id: "task-controller-snapshot".into(),
            loop_run_id: "loop-controller-snapshot".into(),
            step_id: "step-restore-ledger-failure".into(),
            snapshot_id,
            expected_after_hash,
        },
    );
    assert!(matches!(
        result,
        Err(steerbox::loop_controller::LoopControllerError::AuditStore(
            _
        )) | Err(steerbox::loop_controller::LoopControllerError::WorkspaceToolExecution(_))
    ));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    assert_eq!(
        controller
            .approval_store()
            .unwrap()
            .latest("restore-ledger-failure")
            .unwrap()
            .status,
        steerbox::approval_contract::ApprovalStatus::Approved
    );
}

#[test]
fn restore_expired_approval_records_blocked_without_modifying_workspace() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-expired-approval",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let expected_after_hash = sha256(b"after\n");
    controller = controller.with_approval_store(ApprovalStore::open(
        fixture.workspace.join("approvals.jsonl"),
    ));
    let now = Utc::now();
    let request = ApprovalRequest::at(
        "restore-expired-approval",
        fixture.goal.project_id.clone(),
        "loop-controller-snapshot",
        "task-controller-snapshot",
        "restore_workspace_snapshot",
        restore_capability(&snapshot_id, "README.md", &expected_after_hash),
        "metadata_only",
        "restore one controller-bound workspace snapshot",
        now,
        now + Duration::milliseconds(250),
    )
    .unwrap();
    controller
        .approval_store()
        .unwrap()
        .submit(request)
        .unwrap();
    controller
        .approval_store()
        .unwrap()
        .resolve(
            "restore-expired-approval",
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(350));

    let result = controller.restore_workspace_snapshot_with_authorization(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        steerbox::loop_controller::RestoreApprovalContext {
            approval_request_id: "restore-expired-approval".into(),
            project_id: fixture.goal.project_id.clone(),
            goal_id: fixture.goal.goal_id.clone(),
            task_run_id: "task-controller-snapshot".into(),
            loop_run_id: "loop-controller-snapshot".into(),
            step_id: "step-restore-expired-approval".into(),
            snapshot_id: snapshot_id.clone(),
            expected_after_hash,
        },
    );
    assert!(matches!(
        result,
        Err(steerbox::loop_controller::LoopControllerError::ApprovalRequired { .. })
    ));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    let events = fixture.events();
    let blocked = events
        .iter()
        .find(|event| event.event_type == "workspace_snapshot_restore_blocked")
        .expect("blocked restore event is recorded");
    assert_eq!(blocked.payload["reason"], "approval_not_consumed");
    assert_eq!(blocked.payload["snapshot_id"], snapshot_id);
    assert!(!serde_json::to_string(&blocked.payload)
        .unwrap()
        .contains("restore one controller-bound workspace snapshot"));
    assert!(SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .iter()
        .any(|record| {
            record.tool_id == "restore_workspace_snapshot" && record.status == "blocked"
        }));
}

#[test]
fn restore_snapshot_preflight_failure_does_not_consume_approval_or_start_ledger() {
    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-snapshot-failure",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    let expected_after_hash = sha256(b"after\n");
    let approval_store = ApprovalStore::open(fixture.workspace.join("approvals.jsonl"));
    controller = controller.with_approval_store(approval_store);
    let request = ApprovalRequest::at(
        "restore-snapshot-failure",
        fixture.goal.project_id.clone(),
        "loop-controller-snapshot",
        "task-controller-snapshot",
        "restore_workspace_snapshot",
        restore_capability(&snapshot_id, "README.md", &expected_after_hash),
        "metadata_only",
        "restore one controller-bound workspace snapshot",
        Utc::now(),
        Utc::now() + Duration::minutes(15),
    )
    .unwrap();
    controller
        .approval_store()
        .unwrap()
        .submit(request)
        .unwrap();
    controller
        .approval_store()
        .unwrap()
        .resolve(
            "restore-snapshot-failure",
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .unwrap();
    fs::write(
        fixture.snapshot_root.join(format!("{snapshot_id}.json")),
        b"not-json",
    )
    .unwrap();

    let result = controller.restore_workspace_snapshot_with_authorization(
        &ToolRegistry::baseline().unwrap(),
        &PolicyGate::new(),
        steerbox::loop_controller::RestoreApprovalContext {
            approval_request_id: "restore-snapshot-failure".into(),
            project_id: fixture.goal.project_id.clone(),
            goal_id: fixture.goal.goal_id.clone(),
            task_run_id: "task-controller-snapshot".into(),
            loop_run_id: "loop-controller-snapshot".into(),
            step_id: "step-restore-snapshot-failure".into(),
            snapshot_id,
            expected_after_hash,
        },
    );
    assert!(matches!(
        result,
        Err(steerbox::loop_controller::LoopControllerError::WorkspaceSnapshot(_))
    ));
    assert_eq!(
        controller
            .approval_store()
            .unwrap()
            .latest("restore-snapshot-failure")
            .unwrap()
            .status,
        steerbox::approval_contract::ApprovalStatus::Approved
    );
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
    assert!(!SideEffectLedger::open(&fixture.side_effects)
        .read_all()
        .unwrap()
        .iter()
        .any(|record| record.tool_id == "restore_workspace_snapshot"));
}

#[test]
fn physical_restore_with_restored_event_failure_returns_side_effect_uncertain() {
    use std::thread;
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let mut controller = fixture.controller();
    fixture
        .run_edit(
            &mut controller,
            &fixture.action("after\n"),
            "step-restore-audit-uncertain",
        )
        .unwrap();
    let snapshot_id = fixture.snapshot_ids().pop().unwrap();
    controller = fixture.with_restore_approval(
        controller,
        "restore-audit-uncertain",
        &snapshot_id,
        &sha256(b"after\n"),
    );
    let event_path = fixture.events.clone();
    let backup_path = fixture.events.with_extension("jsonl.before-uncertain");
    let watcher_backup_path = backup_path.clone();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(contents) = fs::read(&event_path) {
                if contents
                    .windows(b"workspace_snapshot_restore_requested".len())
                    .any(|window| window == b"workspace_snapshot_restore_requested")
                {
                    fs::rename(&event_path, &watcher_backup_path).unwrap();
                    fs::create_dir(&event_path).unwrap();
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::yield_now();
        }
    });

    let result = fixture.restore_authorized(
        &mut controller,
        "restore-audit-uncertain",
        "step-restore-audit-uncertain",
        &snapshot_id,
        &sha256(b"after\n"),
    );
    assert!(
        watcher.join().unwrap(),
        "restore request event was not observed"
    );
    assert!(matches!(
        result,
        Err(
            steerbox::loop_controller::LoopControllerError::WorkspaceSnapshotAuditUncertain {
                status: steerbox::audit_store::SideEffectAuditStatus::Uncertain
            }
        )
    ));
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    let request_log = fs::read_to_string(backup_path).unwrap();
    assert!(request_log.contains("workspace_snapshot_restore_requested"));
}
