use steerbox::budget_calculator::BudgetCalculator;
use steerbox::domain::{BudgetKind, LoopBudget};
use steerbox::execution_strategy::ExecutionStrategy;

fn budget(allocation: u32, guard: u32) -> LoopBudget {
    LoopBudget::new(
        allocation, allocation, allocation, allocation, allocation, guard, guard, guard,
    )
}

fn allocation_limits(budget: &LoopBudget) -> [u32; 5] {
    [
        budget.max_steps,
        budget.max_tool_calls,
        budget.max_model_calls,
        budget.max_attempts,
        budget.max_goal_revisions,
    ]
}

fn guard_limits(budget: &LoopBudget) -> [u32; 3] {
    [
        budget.max_no_progress_steps,
        budget.max_same_action_repetitions,
        budget.max_consecutive_verification_failures,
    ]
}

fn counters(budget: &LoopBudget) -> [u32; 8] {
    [
        budget.steps_used,
        budget.tool_calls_used,
        budget.model_calls_used,
        budget.attempts_used,
        budget.goal_revisions_used,
        budget.no_progress_steps_used,
        budget.same_action_repetitions_used,
        budget.consecutive_verification_failures,
    ]
}

#[test]
fn fixed_strategy_ratios_scale_allocations_and_floor_odd_values() {
    let base = budget(3, 2);
    let current = budget(100, 10);
    for (strategy, expected, multiplier, approval) in [
        (ExecutionStrategy::Simple, 3, 1.0, false),
        (ExecutionStrategy::Guided, 4, 1.5, false),
        (ExecutionStrategy::Collaborative, 7, 2.5, false),
        (ExecutionStrategy::HighStakes, 9, 3.0, true),
        (ExecutionStrategy::Research, 12, 4.0, false),
        (ExecutionStrategy::SecurityAssessment, 9, 3.0, true),
        (ExecutionStrategy::IncidentResponse, 7, 2.5, false),
        (ExecutionStrategy::RefactorSafely, 4, 1.5, false),
        (ExecutionStrategy::ComplianceAudit, 9, 3.0, true),
    ] {
        let calculated = BudgetCalculator::calculate(&base, &current, strategy);
        assert_eq!(
            allocation_limits(&calculated),
            [expected; 5],
            "{}",
            strategy.id()
        );
        assert_eq!(guard_limits(&calculated), [2; 3]);
        assert_eq!(strategy.budget_multiplier(), multiplier);
        assert_eq!(strategy.requires_approval(), approval);
    }
}

#[test]
fn strategy_cannot_expand_any_current_authorization_limit() {
    let base = budget(u32::MAX, u32::MAX);
    let mut current = budget(1, 1);
    current.max_tool_calls = 2;
    current.max_model_calls = 3;
    current.max_attempts = 4;
    current.max_goal_revisions = 5;
    current.max_same_action_repetitions = 2;
    current.max_consecutive_verification_failures = 3;
    let calculated = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Research);
    assert_eq!(calculated, current);
}

#[test]
fn guard_thresholds_use_minimum_without_scaling() {
    let mut base = budget(3, 2);
    base.max_same_action_repetitions = 8;
    base.max_consecutive_verification_failures = 5;
    let mut current = budget(100, 9);
    current.max_same_action_repetitions = 3;
    current.max_consecutive_verification_failures = 7;
    let calculated = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Research);
    assert_eq!(guard_limits(&calculated), [2, 3, 5]);
}

#[test]
fn zero_allocation_and_guard_limits_stay_zero() {
    let zero = budget(0, 0);
    let available = budget(10, 10);
    assert_eq!(
        BudgetCalculator::calculate(&zero, &available, ExecutionStrategy::Research),
        zero
    );
    assert_eq!(
        BudgetCalculator::calculate(&available, &zero, ExecutionStrategy::Research),
        zero
    );
}

#[test]
fn maximum_integer_allocations_clamp_without_overflow() {
    let maximum = budget(u32::MAX, u32::MAX);
    for strategy in [
        ExecutionStrategy::Guided,
        ExecutionStrategy::Collaborative,
        ExecutionStrategy::HighStakes,
        ExecutionStrategy::Research,
    ] {
        let calculated = BudgetCalculator::calculate(&maximum, &maximum, strategy);
        assert_eq!(calculated, maximum);
    }
}

#[test]
fn every_consumed_counter_is_preserved_from_current_not_base() {
    let mut base = budget(3, 2);
    base.steps_used = 99;
    base.model_calls_used = 88;
    let mut current = budget(20, 20);
    current.steps_used = 1;
    current.tool_calls_used = 2;
    current.model_calls_used = 3;
    current.attempts_used = 4;
    current.goal_revisions_used = 5;
    current.no_progress_steps_used = 6;
    current.same_action_repetitions_used = 7;
    current.consecutive_verification_failures = 8;
    let calculated = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Research);
    assert_eq!(counters(&calculated), counters(&current));
    assert_eq!(allocation_limits(&calculated), [12; 5]);
    assert_eq!(guard_limits(&calculated), [2; 3]);
}

#[test]
fn overused_current_budget_stays_exhausted_after_calculation() {
    let base = budget(3, 2);
    let mut current = budget(1, 1);
    current.steps_used = 5;
    current.tool_calls_used = 5;
    current.model_calls_used = 5;
    current.attempts_used = 5;
    current.goal_revisions_used = 5;
    current.no_progress_steps_used = 5;
    current.same_action_repetitions_used = 5;
    current.consecutive_verification_failures = 5;
    let mut calculated = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Research);
    assert_eq!(calculated, current);
    assert_eq!(calculated.consume_step(), Err(BudgetKind::Steps));
    assert_eq!(calculated.consume_tool_call(), Err(BudgetKind::ToolCalls));
    assert_eq!(calculated.consume_model_call(), Err(BudgetKind::ModelCalls));
    assert_eq!(calculated.start_attempt(), Err(BudgetKind::Attempts));
    assert_eq!(
        calculated.consume_goal_revision(),
        Err(BudgetKind::GoalRevisions)
    );
    assert_eq!(
        calculated.consume_no_progress_step(),
        Err(BudgetKind::NoProgressSteps)
    );
    assert_eq!(
        calculated.consume_same_action_repetition(),
        Err(BudgetKind::SameActionRepetitions)
    );
    assert_eq!(
        calculated.consume_verification_failure(),
        Err(BudgetKind::VerificationFailures)
    );
    assert_eq!(counters(&calculated), counters(&current));
}

#[test]
fn exhausted_budget_is_not_refilled_by_strategy_switching() {
    let base = budget(1, 8);
    let mut current = budget(4, 8);
    current.consume_model_call().expect("first call consumes");
    let mut simple = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Simple);
    assert_eq!(simple.consume_model_call(), Err(BudgetKind::ModelCalls));
    let mut research = BudgetCalculator::calculate(&base, &simple, ExecutionStrategy::Research);
    assert_eq!(research.max_model_calls, 1);
    assert_eq!(research.model_calls_used, 1);
    assert_eq!(research.consume_model_call(), Err(BudgetKind::ModelCalls));
}

#[test]
fn calculation_does_not_mutate_base_or_current() {
    let base = budget(3, 2);
    let mut current = budget(9, 5);
    current.steps_used = 1;
    let base_before = base.clone();
    let current_before = current.clone();
    let _ = BudgetCalculator::calculate(&base, &current, ExecutionStrategy::Guided);
    assert_eq!(base, base_before);
    assert_eq!(current, current_before);
}
