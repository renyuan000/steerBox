use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use steerbox::domain::{BudgetKind, GoalContract, LoopBudget, RiskLevel};
use steerbox::event_log::EventLog;
use steerbox::execution_strategy::ExecutionStrategy;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
};
use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "steerbox-strategy-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn log(&self) -> EventLog {
        EventLog::open(self.0.join("events.jsonl")).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove this test's isolated directory");
    }
}

#[derive(Default)]
struct CountingAdapter(Cell<u32>);

impl ModelAdapter for CountingAdapter {
    fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        self.0.set(self.0.get() + 1);
        Ok(ModelResponse {
            decision: ModelDecision::ContinueObservation,
            summary: "observed".into(),
        })
    }
}

fn goal() -> GoalContract {
    GoalContract::new(
        "goal",
        "project",
        "inspect",
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

#[test]
fn selected_strategy_is_audited_before_first_adapter_call_and_keeps_goal_verification() {
    let directory = TestDirectory::new();
    let mut controller = ReadOnlyLoopController::start_with_strategy(
        directory.log(),
        goal(),
        "loop",
        "task",
        budget(),
        budget(),
        features(),
    )
    .unwrap();
    assert_eq!(
        controller.strategy().unwrap().strategy,
        ExecutionStrategy::Simple
    );
    let adapter = CountingAdapter::default();
    controller
        .run_model_step(&adapter, "fixture", "step", "one")
        .unwrap();
    assert_eq!(adapter.0.get(), 1);
    let events = directory.log().read_all().unwrap();
    let selected = events
        .iter()
        .position(|e| e.event_type == "strategy_selected")
        .unwrap();
    let called = events
        .iter()
        .position(|e| e.event_type == "model_call_started")
        .unwrap();
    assert!(selected < called);
    let event = &events[selected];
    assert_eq!(event.project_id.as_deref(), Some("project"));
    assert_eq!(event.goal_id.as_deref(), Some("goal"));
    assert_eq!(event.loop_run_id.as_deref(), Some("loop"));
    assert_eq!(event.task_run_id.as_deref(), Some("task"));
    assert_eq!(event.payload["goal_verification_required"], true);
    assert_eq!(event.payload["execution_mode"], "sequential_only");
}

#[test]
fn invalid_features_do_not_start_a_loop() {
    let directory = TestDirectory::new();
    let mut input = features();
    input.uncertainty_level = f32::NAN;
    assert!(matches!(
        ReadOnlyLoopController::start_with_strategy(
            directory.log(),
            goal(),
            "loop",
            "task",
            budget(),
            budget(),
            input,
        ),
        Err(LoopControllerError::StrategySelection(_))
    ));
    assert!(directory.log().read_all().unwrap().is_empty());
}

#[test]
fn approval_is_fail_closed_for_goal_risk_irreversibility_and_explicit_request() {
    for case in 0..4 {
        let directory = TestDirectory::new();
        let mut contract = goal();
        let mut input = features();
        match case {
            0 => contract.risk_level = RiskLevel::Critical,
            1 => input.reversible = false,
            2 => input.requires_human_approval = true,
            3 => input.blast_radius = BlastRadius::Production,
            _ => unreachable!(),
        }
        let mut controller = ReadOnlyLoopController::start_with_strategy(
            directory.log(),
            contract,
            "loop",
            "task",
            budget(),
            budget(),
            input,
        )
        .unwrap();
        let adapter = CountingAdapter::default();
        assert!(matches!(
            controller.run_model_step(&adapter, "fixture", "step", "one"),
            Err(LoopControllerError::StrategyApprovalRequired { .. })
        ));
        assert_eq!(adapter.0.get(), 0);
        assert_eq!(controller.budget().steps_used, 0);
        assert_eq!(controller.budget().model_calls_used, 0);
        assert!(matches!(
            controller.select_strategy(features(), &budget()),
            Err(LoopControllerError::StrategyApprovalRequired { .. })
        ));
        let events = directory.log().read_all().unwrap();
        assert!(events
            .iter()
            .any(|e| e.event_type == "human_intervention_required"));
        assert!(!events.iter().any(|e| e.event_type == "step_started"));
    }
}

#[test]
fn research_multiplier_and_strategy_changes_cannot_extend_authorized_calls() {
    let directory = TestDirectory::new();
    let authorized = LoopBudget::new(10, 0, 2, 1, 0, 10, 10, 1);
    let mut input = features();
    input.estimated_steps = 20;
    input.interdependency_count = 5;
    input.uncertainty_level = 1.0;
    let mut controller = ReadOnlyLoopController::start_with_strategy(
        directory.log(),
        goal(),
        "loop",
        "task",
        budget(),
        authorized,
        input.clone(),
    )
    .unwrap();
    assert_eq!(
        controller.strategy().unwrap().strategy,
        ExecutionStrategy::Research
    );
    assert_eq!(controller.budget().max_model_calls, 2);
    let adapter = CountingAdapter::default();
    controller
        .run_model_step(&adapter, "fixture", "step-1", "one")
        .unwrap();
    controller.select_strategy(features(), &budget()).unwrap();
    assert_eq!(controller.budget().model_calls_used, 1);
    controller.select_strategy(input, &budget()).unwrap();
    controller
        .run_model_step(&adapter, "fixture", "step-2", "two")
        .unwrap();
    assert!(matches!(
        controller.run_model_step(&adapter, "fixture", "step-3", "three"),
        Err(LoopControllerError::BudgetExhausted(BudgetKind::ModelCalls))
    ));
    assert_eq!(adapter.0.get(), 2);
    assert_eq!(controller.budget().model_calls_used, 2);
    let events = directory.log().read_all().unwrap();
    let strategies: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "strategy_selected" || e.event_type == "strategy_adjusted")
        .collect();
    assert_eq!(strategies.len(), 3);
    assert!(strategies
        .windows(2)
        .all(|pair| pair[0].event_id != pair[1].event_id));
}

#[test]
fn changing_strategy_does_not_reset_repeat_detection() {
    let directory = TestDirectory::new();
    let authorized = LoopBudget::new(10, 0, 10, 1, 0, 10, 2, 1);
    let mut controller = ReadOnlyLoopController::start_with_strategy(
        directory.log(),
        goal(),
        "loop",
        "task",
        budget(),
        authorized,
        features(),
    )
    .unwrap();
    let adapter = CountingAdapter::default();
    controller
        .run_model_step(&adapter, "fixture", "step-1", "same")
        .unwrap();
    let mut input = features();
    input.estimated_steps = 20;
    controller.select_strategy(input, &budget()).unwrap();
    assert!(matches!(
        controller.run_model_step(&adapter, "fixture", "step-2", "same"),
        Err(LoopControllerError::BudgetExhausted(
            BudgetKind::SameActionRepetitions
        ))
    ));
    assert_eq!(adapter.0.get(), 1);
    assert!(directory
        .log()
        .read_all()
        .unwrap()
        .iter()
        .any(|e| e.event_type == "no_progress_detected"));
}

#[test]
fn zero_guard_is_not_silently_relaxed_by_strategy_start() {
    let directory = TestDirectory::new();
    let mut authorized = budget();
    authorized.max_no_progress_steps = 0;
    let mut controller = ReadOnlyLoopController::start_with_strategy(
        directory.log(),
        goal(),
        "loop",
        "task",
        budget(),
        authorized,
        features(),
    )
    .unwrap();
    let adapter = CountingAdapter::default();
    assert!(matches!(
        controller.run_model_step(&adapter, "fixture", "step", "one"),
        Err(LoopControllerError::BudgetExhausted(
            BudgetKind::NoProgressSteps
        ))
    ));
    assert_eq!(adapter.0.get(), 0);
}
