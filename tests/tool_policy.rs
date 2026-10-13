use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use steerbox::domain::{GoalContract, HumanSteeringMode, RiskLevel};
use steerbox::event_log::EventLog;
use steerbox::policy_gate::{PolicyDecision, PolicyGate, PolicyRequest};
use steerbox::tool_registry::{SideEffectClass, SideEffectDisposition, ToolProfile, ToolRegistry};

fn test_data_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    PathBuf::from(format!("/temp/steerbox/test-tool-policy-{label}-{nonce}"))
}

fn goal(allowed_tools: Vec<&str>, forbidden_tools: Vec<&str>) -> GoalContract {
    let mut goal = GoalContract::new(
        "goal-policy",
        "project-policy",
        "inspect repository",
        vec![".".into()],
        Vec::new(),
        vec!["cargo test".into()],
    );
    goal.allowed_tools = allowed_tools.into_iter().map(str::to_owned).collect();
    goal.forbidden_tools = forbidden_tools.into_iter().map(str::to_owned).collect();
    goal
}

fn request(tool_id: &str) -> PolicyRequest {
    request_in_mode(tool_id, HumanSteeringMode::HumanSteerableLoop)
}

fn request_in_mode(tool_id: &str, human_steering_mode: HumanSteeringMode) -> PolicyRequest {
    PolicyRequest {
        policy_event_id: format!("policy-event-{tool_id}"),
        tool_id: tool_id.into(),
        project_id: "project-policy".into(),
        goal_id: "goal-policy".into(),
        task_run_id: "task-run-policy".into(),
        loop_run_id: "loop-policy".into(),
        step_id: "step-policy".into(),
        correlation_id: "correlation-policy".into(),
        human_steering_mode,
    }
}

#[test]
fn baseline_registry_exposes_read_only_recipes() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    for (tool_id, recipe_ref) in [
        ("git_status", "git-status-readonly"),
        ("rg_fixed", "rg-fixed-search"),
        ("markdown_link_check", "markdown-link-check"),
    ] {
        let profile = registry.get(tool_id).expect("baseline tool exists");
        assert_eq!(profile.side_effect_class, SideEffectClass::None);
        assert_eq!(profile.risk_level, RiskLevel::Low);
        assert_eq!(profile.recipe_ref.as_deref(), Some(recipe_ref));
    }
}

#[test]
fn side_effect_classes_have_conservative_dispositions() {
    assert_eq!(
        SideEffectClass::None.disposition(),
        SideEffectDisposition::ReadOnly
    );
    assert_eq!(
        SideEffectClass::WorkspaceRead.disposition(),
        SideEffectDisposition::ReadOnly
    );
    assert_eq!(
        SideEffectClass::WorkspaceWrite.disposition(),
        SideEffectDisposition::Reversible
    );
    assert!(SideEffectClass::None.automatic_execution_allowed());
    assert!(SideEffectClass::WorkspaceRead.automatic_execution_allowed());
    assert!(SideEffectClass::WorkspaceWrite.automatic_execution_allowed());
}

#[test]
fn external_and_unknown_side_effects_fail_closed() {
    for class in [
        SideEffectClass::ExternalNetwork,
        SideEffectClass::ExternalSystem,
        SideEffectClass::Unknown,
    ] {
        assert_eq!(class.disposition(), SideEffectDisposition::ManualRequired);
        assert!(!class.automatic_execution_allowed());
    }
}

#[test]
fn side_effect_labels_accept_existing_debug_and_snake_case_values() {
    for label in ["none", "read_only", "readonly"] {
        assert_eq!(
            SideEffectClass::from_audit_label(label),
            SideEffectClass::None,
            "label {label} should be read-only"
        );
    }
    for label in ["workspace_read", "workspaceread"] {
        assert_eq!(
            SideEffectClass::from_audit_label(label),
            SideEffectClass::WorkspaceRead,
            "label {label} should be a workspace read"
        );
    }
    for label in [
        "workspace_write",
        "workspacewrite",
        "reversible_local_change",
    ] {
        assert_eq!(
            SideEffectClass::from_audit_label(label),
            SideEffectClass::WorkspaceWrite,
            "label {label} should be a workspace write"
        );
    }
    assert_eq!(
        SideEffectClass::from_audit_label("future-external-class"),
        SideEffectClass::Unknown
    );
}

#[test]
fn baseline_registry_exposes_workspace_edit_as_approval_gated() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let profile = registry
        .get("edit_markdown")
        .expect("workspace edit profile exists");
    assert_eq!(profile.side_effect_class, SideEffectClass::WorkspaceWrite);
    assert_eq!(profile.risk_level, RiskLevel::Medium);
    assert_eq!(
        profile.recipe_ref.as_deref(),
        Some("markdown-edit-workspace")
    );

    let decision = PolicyGate::new().evaluate(
        &registry,
        &goal(vec!["edit_markdown"], Vec::new()),
        &request("edit_markdown"),
    );
    assert_eq!(decision.decision, PolicyDecision::RequireHumanApproval);
}

#[test]
fn registry_rejects_duplicate_without_replacing_original_profile() {
    let mut registry = ToolRegistry::baseline().expect("baseline tools register");
    let original = registry
        .get("git_status")
        .expect("baseline tool exists")
        .clone();
    let mut replacement = original.clone();
    replacement.description = "replacement must not be installed".into();

    let error = registry
        .register(replacement)
        .expect_err("duplicate is rejected");
    assert_eq!(error.to_string(), "tool is already registered: git_status");
    assert_eq!(registry.get("git_status"), Some(&original));
}

#[test]
fn policy_gate_allows_an_explicitly_allowed_read_only_tool_and_records_event() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let goal = goal(vec!["git_status"], Vec::new());
    let request = request("git_status");
    let gate = PolicyGate::new();
    let decision = gate.evaluate(&registry, &goal, &request);

    assert_eq!(decision.decision, PolicyDecision::Allow);
    assert_eq!(decision.side_effect_class, Some(SideEffectClass::None));

    let directory = test_data_dir("allow");
    fs::create_dir_all(&directory).expect("test directory creates");
    let mut log = EventLog::open(directory.join("events.jsonl")).expect("event log opens");
    gate.record_decision(&mut log, &request, &decision)
        .expect("policy event records");
    let events = log.read_all().expect("events read");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "policy_decision");
    assert_eq!(events[0].project_id.as_deref(), Some("project-policy"));
    assert_eq!(events[0].goal_id.as_deref(), Some("goal-policy"));
    assert_eq!(events[0].payload["decision"], "allow");
    assert_eq!(events[0].payload["tool_id"], "git_status");
}

#[test]
fn policy_gate_blocks_unregistered_and_forbidden_tools() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let gate = PolicyGate::new();

    let unregistered = gate.evaluate(
        &registry,
        &goal(vec!["unknown_tool"], Vec::new()),
        &request("unknown_tool"),
    );
    assert_eq!(unregistered.decision, PolicyDecision::Block);
    assert!(unregistered.reason.contains("not registered"));

    let forbidden = gate.evaluate(
        &registry,
        &goal(vec!["git_status"], vec!["git_status"]),
        &request("git_status"),
    );
    assert_eq!(forbidden.decision, PolicyDecision::Block);
    assert!(forbidden.reason.contains("forbidden"));
}

#[test]
fn policy_gate_requires_human_approval_for_workspace_write() {
    let mut registry = ToolRegistry::new();
    registry
        .register(ToolProfile {
            tool_id: "write_file".into(),
            name: "write_file".into(),
            description: "write a file in the workspace".into(),
            input_schema: serde_json::json!({"path": "string", "content": "string"}),
            output_schema: serde_json::json!({"changed": "boolean"}),
            side_effect_class: SideEffectClass::WorkspaceWrite,
            risk_level: RiskLevel::High,
            allowed_in_modes: vec![HumanSteeringMode::HumanSteerableLoop],
            recipe_ref: Some("file-write-verification".into()),
            version: 1,
        })
        .expect("write tool registers");

    let decision = PolicyGate::new().evaluate(
        &registry,
        &goal(vec!["write_file"], Vec::new()),
        &request("write_file"),
    );
    assert_eq!(decision.decision, PolicyDecision::RequireHumanApproval);
    assert!(decision.reason.contains("approval"));
}

#[test]
fn autonomous_mode_allows_scoped_medium_workspace_write() {
    let mut registry = ToolRegistry::new();
    registry
        .register(ToolProfile {
            tool_id: "edit_markdown".into(),
            name: "edit_markdown".into(),
            description: "edit a bounded workspace Markdown file".into(),
            input_schema: serde_json::json!({"path": "string", "expected": "string", "replacement": "string"}),
            output_schema: serde_json::json!({"changed": "boolean"}),
            side_effect_class: SideEffectClass::WorkspaceWrite,
            risk_level: RiskLevel::Medium,
            allowed_in_modes: vec![HumanSteeringMode::AutonomousLoop],
            recipe_ref: Some("markdown-edit-workspace".into()),
            version: 1,
        })
        .expect("workspace edit registers");

    let decision = PolicyGate::new().evaluate(
        &registry,
        &goal(vec!["edit_markdown"], Vec::new()),
        &request_in_mode("edit_markdown", HumanSteeringMode::AutonomousLoop),
    );

    assert_eq!(decision.decision, PolicyDecision::Allow);
    assert!(decision.reason.contains("autonomous"));
}

#[test]
fn autonomous_mode_still_requires_approval_for_high_risk_write() {
    let mut registry = ToolRegistry::new();
    registry
        .register(ToolProfile {
            tool_id: "write_file".into(),
            name: "write_file".into(),
            description: "write a file".into(),
            input_schema: serde_json::json!({"path": "string"}),
            output_schema: serde_json::json!({"changed": "boolean"}),
            side_effect_class: SideEffectClass::WorkspaceWrite,
            risk_level: RiskLevel::High,
            allowed_in_modes: vec![HumanSteeringMode::AutonomousLoop],
            recipe_ref: Some("file-write-verification".into()),
            version: 1,
        })
        .expect("high-risk write registers");

    let decision = PolicyGate::new().evaluate(
        &registry,
        &goal(vec!["write_file"], Vec::new()),
        &request_in_mode("write_file", HumanSteeringMode::AutonomousLoop),
    );

    assert_eq!(decision.decision, PolicyDecision::RequireHumanApproval);
}

#[test]
fn policy_gate_blocks_cross_project_requests() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let mut request = request("git_status");
    request.project_id = "other-project".into();
    let decision =
        PolicyGate::new().evaluate(&registry, &goal(vec!["git_status"], Vec::new()), &request);

    assert_eq!(decision.decision, PolicyDecision::Block);
    assert!(decision.reason.contains("project"));
}
