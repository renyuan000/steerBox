use crate::domain::LoopBudget;
use crate::execution_strategy::ExecutionStrategy;

/// Pure strategy allocation. `current` supplies the existing authorization
/// ceiling and consumed counters; `base` is a requested allocation, not a grant.
pub struct BudgetCalculator;

impl BudgetCalculator {
    pub fn calculate(
        base: &LoopBudget,
        current: &LoopBudget,
        strategy: ExecutionStrategy,
    ) -> LoopBudget {
        let (numerator, denominator) = match strategy {
            ExecutionStrategy::Simple => (1, 1),
            ExecutionStrategy::Guided | ExecutionStrategy::RefactorSafely => (3, 2),
            ExecutionStrategy::Collaborative | ExecutionStrategy::IncidentResponse => (5, 2),
            ExecutionStrategy::HighStakes
            | ExecutionStrategy::SecurityAssessment
            | ExecutionStrategy::ComplianceAudit => (3, 1),
            ExecutionStrategy::Research => (4, 1),
        };
        // Do not use LoopBudget::new: strategy selection is not a fresh attempt
        // and must preserve even already-overused counters rather than refill them.
        let mut budget = current.clone();
        budget.max_steps = scaled_limit(base.max_steps, current.max_steps, numerator, denominator);
        budget.max_tool_calls = scaled_limit(
            base.max_tool_calls,
            current.max_tool_calls,
            numerator,
            denominator,
        );
        budget.max_model_calls = scaled_limit(
            base.max_model_calls,
            current.max_model_calls,
            numerator,
            denominator,
        );
        budget.max_attempts = scaled_limit(
            base.max_attempts,
            current.max_attempts,
            numerator,
            denominator,
        );
        budget.max_goal_revisions = scaled_limit(
            base.max_goal_revisions,
            current.max_goal_revisions,
            numerator,
            denominator,
        );
        // Progress/failure guards are safety thresholds, not resources to scale.
        budget.max_no_progress_steps = base
            .max_no_progress_steps
            .min(current.max_no_progress_steps);
        budget.max_same_action_repetitions = base
            .max_same_action_repetitions
            .min(current.max_same_action_repetitions);
        budget.max_consecutive_verification_failures = base
            .max_consecutive_verification_failures
            .min(current.max_consecutive_verification_failures);
        budget
    }
}

fn scaled_limit(base: u32, cap: u32, numerator: u64, denominator: u64) -> u32 {
    // Fixed strategy ratios have numerator <= 5 and denominator >= 1, so the
    // u32 input product fits u64. Clamp to a u32 ceiling before narrowing.
    let scaled = u64::from(base) * numerator / denominator;
    scaled.min(u64::from(cap)) as u32
}
