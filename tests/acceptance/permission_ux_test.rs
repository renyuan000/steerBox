// 权限交互验收测试
//
// 测试目标：
// 1. baseline 只读工具明确 Allow，而非仅仅没有询问
// 2. 已注册的 workspace 写操作在 HumanSteerableLoop 中必须审批
// 3. PolicyGate 决策正确
// 这是固定夹具的策略测试，不是交互 UX 或总体询问率测量。

use steerbox::domain::{GoalContract, HumanSteeringMode, RiskLevel};
use steerbox::policy_gate::{PolicyDecision, PolicyGate, PolicyRequest};
use steerbox::tool_registry::{SideEffectClass, ToolRegistry};

fn test_goal() -> GoalContract {
    let mut goal = GoalContract::new(
        "goal-policy-test",
        "project-policy-test",
        "test policy decisions",
        vec![".".into()],
        vec![],
        vec!["done".into()],
    );
    goal.allowed_tools = vec!["read_file".into(), "rg_fixed".into(), "git_status".into()];
    goal
}

fn policy_request(goal: &GoalContract, tool_id: &str, step_id: &str) -> PolicyRequest {
    PolicyRequest {
        policy_event_id: format!("policy-{}", step_id),
        tool_id: tool_id.to_string(),
        project_id: goal.project_id.clone(),
        goal_id: goal.goal_id.clone(),
        task_run_id: "task-policy-test".to_string(),
        loop_run_id: "loop-policy-test".to_string(),
        step_id: step_id.to_string(),
        correlation_id: format!("corr-{}", step_id),
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    }
}

#[test]
fn safe_operations_do_not_require_approval() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let goal = test_goal();

    let safe_operations = vec!["read_file", "rg_fixed", "git_status"];

    for tool in safe_operations {
        assert!(registry.get(tool).is_some());
        let request = policy_request(&goal, tool, &format!("step-{}", tool));
        let result = gate.evaluate(&registry, &goal, &request);

        assert_eq!(result.decision, PolicyDecision::Allow);
        assert_eq!(
            result.reason,
            "registered low-risk read-only tool is allowed"
        );
    }
}

#[test]
fn dangerous_operations_require_approval() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    // 包含危险工具的 goal
    let mut goal = GoalContract::new(
        "goal-danger",
        "project-danger",
        "test dangerous ops",
        vec![".".into()],
        vec![],
        vec!["done".into()],
    );
    goal.allowed_tools = vec!["edit_markdown".into()];

    let dangerous_operations = vec!["edit_markdown"];

    for tool in dangerous_operations {
        assert!(registry.get(tool).is_some());
        let request = policy_request(&goal, tool, &format!("step-{}", tool));
        let result = gate.evaluate(&registry, &goal, &request);

        println!(
            "危险操作 {} 决策: {:?}, 理由: {}",
            tool, result.decision, result.reason
        );

        assert_eq!(result.decision, PolicyDecision::RequireHumanApproval);
        assert_eq!(
            result.reason,
            "tool requires human approval before side effects"
        );
    }
}

#[test]
fn bounded_markdown_write_requires_approval_only_in_human_steerable_mode() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let mut goal = test_goal();
    goal.allowed_tools = vec!["edit_markdown".into()];
    let profile = registry.get("edit_markdown").expect("bounded tool exists");
    assert_eq!(profile.side_effect_class, SideEffectClass::WorkspaceWrite);
    assert_eq!(profile.risk_level, RiskLevel::Medium);
    assert_eq!(
        profile.recipe_ref.as_deref(),
        Some("markdown-edit-workspace")
    );

    for (mode, expected_decision, expected_reason) in [
        (
            HumanSteeringMode::HumanSteerableLoop,
            PolicyDecision::RequireHumanApproval,
            "tool requires human approval before side effects",
        ),
        (
            HumanSteeringMode::AutonomousLoop,
            PolicyDecision::Allow,
            "bounded workspace write is allowed in autonomous mode; checkpoint and verification remain required",
        ),
    ] {
        goal.human_steering_mode = mode;
        let mut request = policy_request(&goal, "edit_markdown", "step-mode-boundary");
        request.human_steering_mode = mode;
        let result = gate.evaluate(&registry, &goal, &request);
        assert_eq!(result.decision, expected_decision);
        assert_eq!(result.reason, expected_reason);
        assert_eq!(result.side_effect_class, Some(SideEffectClass::WorkspaceWrite));
        assert_eq!(result.risk_level, Some(RiskLevel::Medium));
    }
}

#[test]
fn forbidden_tools_are_blocked() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    // 使用已注册且列入 allowed_tools 的工具，单独验证 forbidden 优先。
    let mut goal = GoalContract::new(
        "goal-forbidden",
        "project-forbidden",
        "test forbidden tools",
        vec![".".into()],
        vec![],
        vec!["done".into()],
    );
    goal.allowed_tools = vec!["read_file".into()];
    goal.forbidden_tools = vec!["read_file".into()];
    assert!(registry.get("read_file").is_some());

    let request = policy_request(&goal, "read_file", "step-forbidden");
    let result = gate.evaluate(&registry, &goal, &request);

    assert!(
        matches!(result.decision, PolicyDecision::Block),
        "禁止的工具应该被 Block，实际: {:?}",
        result.decision
    );
    assert_eq!(result.reason, "tool is forbidden by the goal");

    println!("✅ 禁止工具被正确拦截: {}", result.reason);
}

#[test]
fn approval_decision_is_clear_and_actionable() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    let mut goal = GoalContract::new(
        "goal-approval",
        "project-approval",
        "test approval",
        vec![".".into()],
        vec![],
        vec!["done".into()],
    );
    goal.allowed_tools = vec!["edit_markdown".into()];
    assert!(registry.get("edit_markdown").is_some());

    let request = policy_request(&goal, "edit_markdown", "step-approval");
    let result = gate.evaluate(&registry, &goal, &request);

    println!("决策: {:?}", result.decision);
    println!("理由: {}", result.reason);

    assert_eq!(result.decision, PolicyDecision::RequireHumanApproval);
    assert_eq!(
        result.reason,
        "tool requires human approval before side effects"
    );
}

#[test]
fn project_mismatch_is_blocked() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    let goal = test_goal();

    let mut request = policy_request(&goal, "read_file", "step-mismatch");
    request.project_id = "wrong-project".to_string();

    let result = gate.evaluate(&registry, &goal, &request);

    assert!(
        matches!(result.decision, PolicyDecision::Block),
        "项目不匹配应该被 Block"
    );
    assert_eq!(result.reason, "request project does not match goal project");

    println!("✅ 项目不匹配被正确拦截: {}", result.reason);
}

#[test]
fn unknown_tool_is_blocked() {
    let gate = PolicyGate::new();
    let registry = ToolRegistry::baseline().expect("baseline tools register");

    let goal = GoalContract::new(
        "goal-unknown",
        "project-unknown",
        "test unknown tool",
        vec![".".into()],
        vec![],
        vec!["done".into()],
    );

    assert!(registry.get("nonexistent_tool").is_none());
    let request = policy_request(&goal, "nonexistent_tool", "step-unknown");
    let result = gate.evaluate(&registry, &goal, &request);

    assert!(
        matches!(result.decision, PolicyDecision::Block),
        "未知工具应该被 Block，实际: {:?}",
        result.decision
    );
    assert_eq!(result.reason, "tool is not registered");

    println!("✅ 未知工具被正确拦截: {}", result.reason);
}
