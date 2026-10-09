use steerbox::domain::GoalContract;
use steerbox::drift_guard::{check_goal_alignment, DiffRiskSummary, DriftDecision, DriftGuard};

fn goal() -> GoalContract {
    GoalContract::new(
        "goal-drift",
        "project-drift",
        "update docs",
        vec!["docs".into()],
        Vec::new(),
        vec!["docs check".into()],
    )
}

#[test]
fn drift_guard_blocks_out_of_scope_and_escalates_stable_files() {
    let goal = goal();
    let guard = DriftGuard::default();
    let outside = guard.evaluate(
        &goal,
        &DiffRiskSummary {
            changed_files: vec!["src/main.rs".into()],
            changed_lines: 1,
            deleted_lines: 0,
            stable_artifacts: Vec::new(),
        },
    );
    assert_eq!(outside.decision, DriftDecision::Block);

    let stable = guard.evaluate(
        &goal,
        &DiffRiskSummary {
            changed_files: vec!["docs/PHASE_1_DESIGN.md".into()],
            changed_lines: 1,
            deleted_lines: 0,
            stable_artifacts: vec!["docs/PHASE_1_DESIGN.md".into()],
        },
    );
    assert_eq!(stable.decision, DriftDecision::RequireHumanApproval);
}

#[test]
fn goal_alignment_requires_stage_goal_and_in_scope_changes() {
    let goal = goal();
    let report = check_goal_alignment(&goal, Some("update docs only"), &["docs/README.md".into()]);
    assert!(report.aligned);
    let report = check_goal_alignment(&goal, None, &["src/main.rs".into()]);
    assert!(!report.aligned);
    assert!(report.missing_stage_goal);
    assert_eq!(report.out_of_scope_files, vec!["src/main.rs"]);
}
