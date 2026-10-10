use serde_json::json;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::domain::{BudgetKind, GoalContract, LoopBudget, LoopStatus, RiskLevel};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
};
use steerbox::policy_gate::{PolicyDecision, PolicyGate};
use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput};
use steerbox::tool_registry::ToolRegistry;
use steerbox::user_command_inbox::{CommandKind, CommandStatus, UserCommand, UserCommandInbox};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "steerbox-strategy-controls-{}-{nonce}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn log(&self) -> EventLog {
        EventLog::open(self.0.join("events.jsonl")).unwrap()
    }

    fn inbox(&self) -> UserCommandInbox {
        UserCommandInbox::open(self.0.join("commands.jsonl"))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0)
            .expect("remove this test's isolated strategy controls directory");
    }
}

#[derive(Default)]
struct CountingAdapter(Cell<u32>);

impl ModelAdapter for CountingAdapter {
    fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.0.set(self.0.get() + 1);
        Ok(ModelResponse {
            decision: ModelDecision::ContinueObservation,
            summary: "local fixture observation".into(),
        })
    }
}

fn goal() -> GoalContract {
    GoalContract::new(
        "goal",
        "project",
        "inspect local fixture",
        vec![".".into()],
        vec![],
        vec!["manual_review".into()],
    )
}

fn features() -> TaskFeatures {
    TaskFeatures {
        estimated_steps: 1,
        uncertainty_level: 0.0,
        interdependency_count: 0,
        risk_level: RiskLevel::Low,
        reversible: true,
        blast_radius: BlastRadius::Local,
        domain: ProfessionalDomain::SoftwareDevelopment,
        requires_human_approval: false,
        max_budget_multiplier: 4.0,
    }
}

fn budget() -> LoopBudget {
    LoopBudget::new(10, 10, 10, 2, 2, 10, 10, 2)
}

fn start(
    directory: &TestDirectory,
    contract: GoalContract,
    limit: LoopBudget,
) -> ReadOnlyLoopController {
    ReadOnlyLoopController::start_with_strategy(
        directory.log(),
        contract,
        "loop",
        "task",
        limit.clone(),
        limit,
        features(),
    )
    .unwrap()
}

fn submit(inbox: &UserCommandInbox, id: &str, kind: CommandKind) {
    inbox
        .submit(
            UserCommand::new(
                id,
                "local-operator",
                "project",
                "goal",
                "task",
                "loop",
                kind,
                "fixture-correlation",
                json!({"note":"fixture-only"}),
            )
            .unwrap(),
        )
        .unwrap();
}

#[test]
fn pause_and_resume_cannot_clear_strategy_approval_or_consumed_counters() {
    let directory = TestDirectory::new();
    let inbox = directory.inbox();
    let mut controller = start(&directory, goal(), budget()).with_command_inbox(inbox.clone());
    let adapter = CountingAdapter::default();
    controller
        .run_model_step(&adapter, "fixture", "observed", "initial fixture")
        .unwrap();
    let mut input = features();
    input.reversible = false;
    controller.select_strategy(input, &budget()).unwrap();
    let before = controller.budget().clone();
    assert_eq!(before.steps_used, 1);
    assert_eq!(before.model_calls_used, 1);
    assert!(controller.strategy().unwrap().approval_required);

    submit(&inbox, "pause-1", CommandKind::Pause);
    assert!(matches!(
        controller.run_model_step(&adapter, "fixture", "paused-step", "paused fixture"),
        Err(LoopControllerError::Paused)
    ));
    assert_eq!(controller.status(), LoopStatus::Paused);
    assert_eq!(
        inbox.latest("pause-1").unwrap().status,
        CommandStatus::Applied
    );
    assert_eq!(controller.budget(), &before);

    submit(&inbox, "resume-1", CommandKind::Resume);
    assert!(matches!(
        controller.run_model_step(&adapter, "fixture", "resumed-step", "resumed fixture"),
        Err(LoopControllerError::StrategyApprovalRequired { .. })
    ));
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(
        inbox.latest("resume-1").unwrap().status,
        CommandStatus::Applied
    );
    assert_eq!(controller.budget(), &before);
    assert!(controller.strategy().unwrap().approval_required);
    assert_eq!(adapter.0.get(), 1);
}

#[test]
fn failed_strategy_adjustments_leave_prior_strategy_budget_and_events_unchanged() {
    let directory = TestDirectory::new();
    let mut controller = start(&directory, goal(), budget());
    let adapter = CountingAdapter::default();
    controller
        .run_model_step(&adapter, "fixture", "initial", "fixture observation")
        .unwrap();
    let old_strategy = controller.strategy().unwrap().clone();
    let old_budget = controller.budget().clone();
    let old_events = directory.log().read_all().unwrap();

    let mut invalid = features();
    invalid.uncertainty_level = f32::NAN;
    let mut insufficient = features();
    insufficient.risk_level = RiskLevel::Critical;
    insufficient.max_budget_multiplier = 1.0;
    for input in [invalid, insufficient] {
        assert!(matches!(
            controller.select_strategy(input, &budget()),
            Err(LoopControllerError::StrategySelection(_))
        ));
        assert_eq!(controller.strategy(), Some(&old_strategy));
        assert_eq!(controller.budget(), &old_budget);
        assert_eq!(directory.log().read_all().unwrap(), old_events);
    }
}

#[test]
fn selected_strategy_does_not_override_goal_or_registry_policy_denials() {
    let registry = ToolRegistry::baseline().unwrap();
    for (case, expected_reason) in [
        ("forbidden", "tool is forbidden by the goal"),
        ("unregistered", "tool is not registered"),
        ("not_allowed", "tool is not allowed by the goal"),
    ] {
        let directory = TestDirectory::new();
        let mut contract = goal();
        let tool_id = match case {
            "forbidden" => {
                contract.forbidden_tools = vec!["git_status".into()];
                "git_status"
            }
            "unregistered" => "fixture-unknown-tool",
            "not_allowed" => {
                contract.allowed_tools = vec!["rg_fixed".into()];
                "git_status"
            }
            _ => unreachable!(),
        };
        let mut controller = start(&directory, contract, budget());
        let error = controller
            .request_tool_call(&registry, &PolicyGate::new(), tool_id, "policy-step")
            .unwrap_err();
        assert!(
            matches!(error, LoopControllerError::PolicyDenied { decision: PolicyDecision::Block, reason } if reason == expected_reason)
        );
        let events = directory.log().read_all().unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "policy_decision"
                && event.payload["decision"] == "block"));
        assert!(!events.iter().any(|event| matches!(
            event.event_type.as_str(),
            "tool_call_requested" | "tool_call_completed"
        )));
    }
}

#[test]
fn zero_tool_budget_blocks_tool_request_before_any_tool_request_event() {
    let directory = TestDirectory::new();
    let mut limit = budget();
    limit.max_tool_calls = 0;
    let mut controller = start(&directory, goal(), limit);
    assert!(matches!(
        controller.request_tool_call(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            "git_status",
            "zero-request"
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ToolCalls))
    ));
    assert_eq!(controller.budget().tool_calls_used, 0);
    assert!(!directory
        .log()
        .read_all()
        .unwrap()
        .iter()
        .any(|event| event.event_type == "tool_call_requested"));
}

#[test]
fn zero_tool_budget_blocks_executor_before_workspace_validation() {
    let directory = TestDirectory::new();
    let mut limit = budget();
    limit.max_tool_calls = 0;
    let mut controller = start(&directory, goal(), limit);
    // Deliberately absent: an invoked executor would fail workspace validation, not run git.
    let missing_workspace = directory.0.join("missing-workspace");
    assert!(matches!(
        controller.execute_read_only_tool(
            &ToolRegistry::baseline().unwrap(),
            &PolicyGate::new(),
            &ReadOnlyToolExecutor,
            &missing_workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: json!({})
            },
            "zero-execute",
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ToolCalls))
    ));
    assert_eq!(controller.budget().tool_calls_used, 0);
    assert!(!directory
        .log()
        .read_all()
        .unwrap()
        .iter()
        .any(|event| matches!(
            event.event_type.as_str(),
            "tool_call_requested" | "tool_call_failed" | "tool_call_completed"
        )));
}

#[test]
fn one_tool_call_is_consumed_and_strategy_adjustment_cannot_refill_it() {
    let directory = TestDirectory::new();
    let mut limit = budget();
    limit.max_tool_calls = 1;
    let mut controller = start(&directory, goal(), limit.clone());
    let registry = ToolRegistry::baseline().unwrap();
    controller
        .request_tool_call(&registry, &PolicyGate::new(), "git_status", "allowed-tool")
        .unwrap();
    assert_eq!(controller.budget().tool_calls_used, 1);

    let mut input = features();
    input.estimated_steps = 16;
    controller.select_strategy(input, &limit).unwrap();
    assert_eq!(controller.budget().max_tool_calls, 1);
    assert_eq!(controller.budget().tool_calls_used, 1);
    assert!(matches!(
        controller.request_tool_call(
            &registry,
            &PolicyGate::new(),
            "git_status",
            "exhausted-tool"
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ToolCalls))
    ));
    assert_eq!(controller.budget().tool_calls_used, 1);
    assert_eq!(
        directory
            .log()
            .read_all()
            .unwrap()
            .iter()
            .filter(|event| event.event_type == "tool_call_requested")
            .count(),
        1
    );
}

#[test]
fn tightening_tool_limit_to_already_used_count_blocks_next_request() {
    let directory = TestDirectory::new();
    let mut limit = budget();
    limit.max_tool_calls = 3;
    let mut controller = start(&directory, goal(), limit);
    let registry = ToolRegistry::baseline().unwrap();
    controller
        .request_tool_call(&registry, &PolicyGate::new(), "git_status", "first-tool")
        .unwrap();
    assert_eq!(controller.budget().tool_calls_used, 1);

    let mut tightened = budget();
    tightened.max_tool_calls = 1;
    controller.select_strategy(features(), &tightened).unwrap();
    assert_eq!(controller.budget().max_tool_calls, 1);
    assert_eq!(controller.budget().tool_calls_used, 1);
    assert!(matches!(
        controller.request_tool_call(
            &registry,
            &PolicyGate::new(),
            "git_status",
            "tightened-tool"
        ),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ToolCalls))
    ));
    assert_eq!(controller.budget().tool_calls_used, 1);
}
