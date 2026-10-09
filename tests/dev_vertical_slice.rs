use std::fs;
use std::path::PathBuf;
use steerbox::audit_store::{
    CheckpointMetadata, CheckpointStore, SideEffectLedger, SideEffectRecord,
};
use steerbox::domain::{EventEnvelope, GoalContract, HumanSteeringMode, RiskLevel};
use steerbox::event_log::EventLog;
use steerbox::policy_gate::{PolicyGate, PolicyRequest};
use steerbox::tool_registry::{SideEffectClass, ToolProfile, ToolRegistry};
use steerbox::workspace_edit::{edit_markdown, verify_markdown_links};

fn workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!("steerbox-dev-slice-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("workspace creates");
    path
}

#[test]
fn markdown_dev_slice_is_auditable_and_reversible_at_metadata_boundary() {
    let root = workspace();
    fs::write(root.join("README.md"), "See [guide](guide.md).\n").expect("readme writes");
    fs::write(root.join("guide.md"), "guide\n").expect("guide writes");
    let audit = root.join(".steerbox");
    let event_path = audit.join("events.jsonl");
    let checkpoint_path = audit.join("checkpoints.jsonl");
    let side_effect_path = audit.join("side-effects.jsonl");
    let mut log = EventLog::open(&event_path).expect("event log opens");
    let goal = GoalContract::new(
        "goal-dev",
        "project-dev",
        "update markdown",
        vec![".".into()],
        vec![".steerbox".into()],
        vec!["markdown_link_check".into()],
    );
    let mut registry = ToolRegistry::new();
    registry
        .register(ToolProfile {
            tool_id: "write_file".into(),
            name: "write markdown".into(),
            description: "approved local markdown write".into(),
            input_schema: serde_json::json!({"path":"string"}),
            output_schema: serde_json::json!({"changed":"boolean"}),
            side_effect_class: SideEffectClass::WorkspaceWrite,
            risk_level: RiskLevel::High,
            allowed_in_modes: vec![HumanSteeringMode::HumanSteerableLoop],
            recipe_ref: Some("file-write-verification".into()),
            version: 1,
        })
        .expect("write profile registers");
    let request = PolicyRequest {
        policy_event_id: "policy-dev".into(),
        tool_id: "write_file".into(),
        project_id: "project-dev".into(),
        goal_id: "goal-dev".into(),
        task_run_id: "task-run-dev".into(),
        loop_run_id: "loop-dev".into(),
        step_id: "step-dev".into(),
        correlation_id: "loop-dev".into(),
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let gate = PolicyGate::new();
    let decision = gate.evaluate(&registry, &goal, &request);
    assert_eq!(
        decision.decision,
        steerbox::policy_gate::PolicyDecision::RequireHumanApproval
    );
    gate.record_decision(&mut log, &request, &decision)
        .expect("policy event records");

    CheckpointStore::open(&checkpoint_path)
        .append(&CheckpointMetadata {
            checkpoint_id: "checkpoint-before-readme".into(),
            goal_id: "goal-dev".into(),
            loop_run_id: "loop-dev".into(),
            step_id: Some("step-dev".into()),
            files: vec!["README.md".into()],
            diff_boundary: "README.md:+0/-0 before write".into(),
            recoverability: "original-content-captured-by-caller".into(),
            side_effect_refs: vec!["side-effect-readme".into()],
            created_at: "2026-10-08T00:00:00Z".into(),
        })
        .expect("checkpoint records");
    let edit = edit_markdown(
        &root,
        &goal,
        "README.md",
        "See [guide](guide.md).\n",
        "See [guide](guide.md) and [notes](notes.md).\n",
        true,
    )
    .expect("approved edit succeeds");
    assert!(edit.changed);
    let verification = verify_markdown_links(&root, "README.md").expect("links verify");
    assert!(!verification.passed);
    assert_eq!(verification.missing_links, vec!["notes.md"]);
    SideEffectLedger::open(&side_effect_path)
        .append(&SideEffectRecord {
            side_effect_id: "side-effect-readme".into(),
            goal_id: "goal-dev".into(),
            loop_run_id: "loop-dev".into(),
            tool_id: "write_file".into(),
            class: "reversible_local_change".into(),
            status: "completed".into(),
            summary: "README.md changed; original content is known".into(),
            created_at: "2026-10-08T00:00:00Z".into(),
        })
        .expect("side effect records");
    let mut changed = EventEnvelope::new(
        "workspace",
        "file-change-dev",
        "file_change_recorded",
        "README.md",
        "loop-dev",
        serde_json::json!({"path":"README.md","changed":true,"verification":"failed_missing_link"}),
    )
    .expect("file event builds");
    changed.project_id = Some("project-dev".into());
    changed.goal_id = Some("goal-dev".into());
    changed.loop_run_id = Some("loop-dev".into());
    changed.step_id = Some("step-dev".into());
    log.append(changed).expect("file event records");

    assert_eq!(
        CheckpointStore::open(checkpoint_path)
            .read_all()
            .expect("checkpoint reads")
            .len(),
        1
    );
    assert_eq!(
        SideEffectLedger::open(side_effect_path)
            .read_all()
            .expect("side effects read")
            .len(),
        1
    );
    assert!(EventLog::open(event_path)
        .expect("event log reopens")
        .read_all()
        .expect("events read")
        .iter()
        .any(|event| event.event_type == "policy_decision"));
    fs::remove_dir_all(root).expect("workspace removes");
}
