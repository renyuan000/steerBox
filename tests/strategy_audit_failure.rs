use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use steerbox::domain::{GoalContract, LoopBudget, LoopStatus, RiskLevel};
use steerbox::event_log::EventLog;
use steerbox::execution_strategy::ExecutionStrategy;
use steerbox::loop_controller::{LoopControllerError, ReadOnlyLoopController};
use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        for _ in 0..128 {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "steerbox-strategy-audit-failure-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create isolated audit test directory: {error}"),
            }
        }
        panic!("cannot allocate unique audit test directory");
    }

    fn events_path(&self) -> PathBuf {
        self.0.join("events.jsonl")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Remove only the directory exclusively created by this instance, also
        // during panic unwinding. Never cause a second panic during cleanup.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn low_risk_features() -> TaskFeatures {
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

#[test]
fn failed_strategy_audit_preserves_state_and_next_successful_revision() {
    let directory = TestDirectory::new();
    let events_path = directory.events_path();
    let goal = GoalContract::new(
        "goal-audit",
        "project-audit",
        "inspect synthetic local state",
        vec![".".into()],
        vec![],
        vec!["manual_review".into()],
    );
    let budget = LoopBudget::new(10, 10, 10, 2, 2, 10, 10, 2);
    let mut controller = ReadOnlyLoopController::start_with_strategy(
        EventLog::open(&events_path).expect("initial event log opens"),
        goal,
        "loop-audit",
        "task-audit",
        budget.clone(),
        budget,
        low_risk_features(),
    )
    .expect("low-risk strategy starts");
    let old_decision = controller.strategy().expect("selected strategy").clone();
    let old_budget = controller.budget().clone();
    let old_events = EventLog::open(&events_path)
        .expect("event log opens for snapshot")
        .read_all()
        .expect("events read before fault");
    let old_bytes = fs::read(&events_path).expect("capture original audit bytes");
    assert_eq!(old_decision.strategy, ExecutionStrategy::Simple);
    assert!(!old_decision.approval_required);
    assert_eq!(old_events.len(), 2);
    assert_eq!(old_events[1].payload["strategy_revision"], 1);

    // The fault stays entirely inside this test's owned directory: an existing
    // directory cannot be opened as an append-only file, even when run as root.
    let backup_path = directory.0.join("events-backup.jsonl");
    fs::rename(&events_path, &backup_path).expect("preserve original audit log");
    fs::create_dir(&events_path).expect("create deterministic append failure target");

    let mut new_features = low_risk_features();
    new_features.estimated_steps = 10;
    new_features.uncertainty_level = 0.5;
    new_features.risk_level = RiskLevel::High;
    let smaller_budget = LoopBudget::new(2, 2, 2, 1, 1, 2, 2, 1);
    let error = controller
        .select_strategy(new_features.clone(), &smaller_budget)
        .expect_err("strategy selection must propagate audit append failure");
    assert!(matches!(error, LoopControllerError::EventLog(_)));
    assert_eq!(controller.strategy(), Some(&old_decision));
    assert_eq!(controller.budget(), &old_budget);
    assert_eq!(controller.status(), LoopStatus::Running);
    assert_eq!(
        fs::read(&backup_path).expect("preserved audit bytes"),
        old_bytes
    );
    let preserved_events = EventLog::open(&backup_path)
        .expect("preserved log opens")
        .read_all()
        .expect("preserved events read");
    assert_eq!(
        serde_json::to_value(preserved_events).expect("preserved events serialize"),
        serde_json::to_value(old_events).expect("original events serialize")
    );

    fs::remove_dir(&events_path).expect("remove only the empty fault directory");
    fs::rename(&backup_path, &events_path).expect("restore original audit log path");
    controller
        .run_observation("step-after-fault", "synthetic local observation")
        .expect("old low-risk policy remains executable after failed selection");
    assert_eq!(controller.strategy(), Some(&old_decision));
    assert_eq!(controller.budget().steps_used, old_budget.steps_used + 1);

    new_features.risk_level = RiskLevel::Low;
    let new_decision = controller
        .select_strategy(new_features, &smaller_budget)
        .expect("new low-risk strategy is audited after recovery");
    assert_eq!(new_decision.strategy, ExecutionStrategy::Guided);
    let events = EventLog::open(&events_path)
        .expect("recovered audit log opens")
        .read_all()
        .expect("recovered events read");
    let strategy_events: Vec<_> = events
        .iter()
        .filter(|event| {
            event.event_type == "strategy_selected" || event.event_type == "strategy_adjusted"
        })
        .collect();
    assert_eq!(strategy_events.len(), 2);
    assert_eq!(strategy_events[1].event_type, "strategy_adjusted");
    assert_eq!(strategy_events[1].event_id, "loop-audit:strategy:2");
    assert_eq!(strategy_events[1].payload["strategy_revision"], 2);
    assert!(!events
        .iter()
        .any(|event| event.event_type == "human_intervention_required"));
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.sequence_no, Some(index as u64 + 1));
    }
}
