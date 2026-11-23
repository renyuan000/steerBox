#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use steerbox::domain::{AttemptReason, AttemptStatus, GoalContract, LoopBudget, TaskRun};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::ReadOnlyLoopController;
use steerbox::recovery_input::{
    RecoveryInput, RecoveryInputError, RecoveryInputPaths, RecoverySource, RecoveryVerdict,
};

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn controller(dir: &Path) -> (ReadOnlyLoopController, RecoveryInputPaths) {
    let events = dir.join("events.jsonl");
    let controller = ReadOnlyLoopController::start(
        EventLog::open(&events).unwrap(),
        GoalContract::new(
            "goal",
            "project",
            "private objective",
            vec![],
            vec![],
            vec![],
        ),
        "loop",
        "task-run",
        LoopBudget::new(8, 5, 4, 3, 2, 2, 2, 2),
    )
    .unwrap();
    (controller, RecoveryInputPaths::events_only(events))
}

fn rewrite_record(path: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

#[test]
fn private_round_trip_preserves_state_without_granting_resume() {
    let dir = private_dir();
    let events = dir.path().join("events.jsonl");
    let mut controller = ReadOnlyLoopController::start(
        EventLog::open(&events).unwrap(),
        GoalContract::new(
            "goal",
            "project",
            "private objective",
            vec![],
            vec![],
            vec![],
        ),
        "loop",
        "task-run",
        LoopBudget::new(8, 5, 4, 3, 2, 2, 2, 2),
    )
    .unwrap();
    controller
        .run_observation("step-1", "safe observation")
        .unwrap();
    let paths = RecoveryInputPaths::events_only(&events);
    let input = controller.capture_recovery_input(&paths).unwrap();
    let saved = dir.path().join("recovery.json");
    input.persist_new(&saved, &paths).unwrap();
    drop(controller);
    let loaded = RecoveryInput::load(&saved).unwrap();
    assert_eq!(input.state(), loaded.state());
    assert_eq!(loaded.state().task_run().budget.steps_used, 1);
    assert_eq!(loaded.state().task_run().attempts.len(), 1);
    assert_eq!(loaded.state().next_step_no(), 2);
    let assessment = loaded.assess(&paths);
    assert_eq!(assessment.verdict, RecoveryVerdict::InputsMatchOnly);
    assert!(!assessment.resume_authorized);
    let public = format!("{loaded:?} {assessment:?}");
    assert!(!public.contains("private objective"));
    assert!(!public.contains(dir.path().to_str().unwrap()));
}

#[test]
fn attempt_parent_chain_and_spent_budget_survive_new_process() {
    let dir = private_dir();
    let events = dir.path().join("events.jsonl");
    let mut task =
        TaskRun::new("task-run", "task", LoopBudget::new(8, 5, 4, 3, 2, 2, 2, 2)).unwrap();
    task.start_attempt("a1", AttemptReason::Initial, None)
        .unwrap();
    task.finish_attempt("a1", AttemptStatus::Failed, "failed", vec![], vec![])
        .unwrap();
    task.start_attempt("a2", AttemptReason::Retry, Some("a1".into()))
        .unwrap();
    let mut c = ReadOnlyLoopController::start_with_task_run(
        EventLog::open(&events).unwrap(),
        GoalContract::new(
            "goal",
            "project",
            "private objective",
            vec![],
            vec![],
            vec![],
        ),
        "loop",
        task,
    )
    .unwrap();
    c.run_observation("s1", "observed").unwrap();
    let paths = RecoveryInputPaths::events_only(&events);
    let input = c.capture_recovery_input(&paths).unwrap();
    let saved = dir.path().join("recovery.json");
    input.persist_new(&saved, &paths).unwrap();
    drop(c);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fresh_process_reload_child", "--nocapture"])
        .env("STEERBOX_RECOVERY_TEST_RECORD", &saved)
        .env("STEERBOX_RECOVERY_TEST_EVENTS", &events)
        .output()
        .unwrap();
    assert!(output.status.success(), "child reload failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn controller_strategy_revisions_and_budget_survive_private_reload() {
    use steerbox::domain::RiskLevel;
    use steerbox::task_features::{BlastRadius, ProfessionalDomain, TaskFeatures};
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    let features = TaskFeatures {
        estimated_steps: 2,
        uncertainty_level: 0.1,
        interdependency_count: 0,
        risk_level: RiskLevel::Low,
        reversible: true,
        blast_radius: BlastRadius::Local,
        domain: ProfessionalDomain::SoftwareDevelopment,
        requires_human_approval: false,
        max_budget_multiplier: 4.0,
    };
    let base = LoopBudget::new(8, 5, 4, 3, 2, 2, 2, 2);
    c.select_strategy(features.clone(), &base).unwrap();
    let decision = c.select_strategy(features, &base).unwrap();
    let input = c.capture_recovery_input(&paths).unwrap();
    let path = dir.path().join("recovery.json");
    input.persist_new(&path, &paths).unwrap();
    let restored = RecoveryInput::load(path).unwrap();
    assert_eq!(restored.state(), input.state());
    assert_eq!(restored.state().strategy(), Some(&decision));
    assert_eq!(restored.state().strategy_revision(), 2);
    assert_eq!(
        restored.assess(&paths).verdict,
        RecoveryVerdict::InputsMatchOnly
    );
}

#[test]
fn fresh_process_reload_child() {
    let Some(record) = std::env::var_os("STEERBOX_RECOVERY_TEST_RECORD") else {
        return;
    };
    let events = std::env::var_os("STEERBOX_RECOVERY_TEST_EVENTS").unwrap();
    let input = RecoveryInput::load(Path::new(&record)).unwrap();
    assert_eq!(input.state().task_run().attempts.len(), 2);
    assert_eq!(
        input.state().task_run().attempts[1]
            .parent_attempt_id
            .as_deref(),
        Some("a1")
    );
    assert_eq!(input.state().task_run().budget.attempts_used, 2);
    assert_eq!(input.state().task_run().budget.steps_used, 1);
    assert_eq!(input.state().next_step_no(), 2);
    let result = input.assess(&RecoveryInputPaths::events_only(events));
    assert_eq!(result.verdict, RecoveryVerdict::InputsMatchOnly);
    assert!(!result.resume_authorized);
    assert!(!result.complete_recovery_coverage);
}

#[test]
fn no_progress_rejection_preserves_consumed_budget_and_fingerprint() {
    use steerbox::model_adapter::{LocalFixtureModelAdapter, ModelDecision};
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    let adapter = LocalFixtureModelAdapter::new(ModelDecision::ContinueObservation, "continue");
    c.run_model_step(&adapter, "fixture-model", "s1", "private observation")
        .unwrap();
    assert!(c
        .run_model_step(&adapter, "fixture-model", "s2", "private observation")
        .is_err());
    let input = c.capture_recovery_input(&paths).unwrap();
    assert_eq!(input.state().task_run().budget.steps_used, 2);
    assert_eq!(input.state().next_step_no(), 2);
    assert_eq!(input.state().no_progress().no_progress_steps, 2);
    assert_eq!(input.state().no_progress().same_action_repetitions, 2);
    assert!(input
        .state()
        .no_progress()
        .last_action_fingerprint
        .as_ref()
        .unwrap()
        .contains("private observation"));
    let saved = dir.path().join("recovery.json");
    input.persist_new(&saved, &paths).unwrap();
    let loaded = RecoveryInput::load(&saved).unwrap();
    assert_eq!(input.state(), loaded.state());
    assert!(!format!("{loaded:?} {:?}", loaded.state()).contains("private observation"));
}

#[test]
fn same_length_source_edit_or_append_is_not_a_matching_boundary() {
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    c.run_observation("s1", "alpha").unwrap();
    let input = c.capture_recovery_input(&paths).unwrap();
    let events = dir.path().join("events.jsonl");
    let original = fs::read(&events).unwrap();
    let mut changed = original.clone();
    let offset = changed
        .windows(5)
        .position(|window| window == b"alpha")
        .unwrap();
    changed[offset..offset + 5].copy_from_slice(b"bravo");
    fs::write(&events, changed).unwrap();
    assert_eq!(
        input.assess(&paths).verdict,
        RecoveryVerdict::SourcesChanged
    );
    assert_eq!(
        input.persist_new(dir.path().join("not-created.json"), &paths),
        Err(RecoveryInputError::SourcesChanged)
    );
    assert!(!dir.path().join("not-created.json").exists());
    fs::write(&events, original).unwrap();
    c.run_observation("s2", "more").unwrap();
    assert_eq!(
        input.assess(&paths).verdict,
        RecoveryVerdict::SourcesChanged
    );
}

#[test]
fn missing_partial_duplicate_and_discontinuous_event_logs_fail_closed() {
    for problem in [
        "missing",
        "tail",
        "duplicate",
        "sequence",
        "sensitive",
        "blank",
    ] {
        let dir = private_dir();
        let (mut c, paths) = controller(dir.path());
        let input = c.capture_recovery_input(&paths).unwrap();
        let events = dir.path().join("events.jsonl");
        let mut bytes = fs::read(&events).unwrap();
        match problem {
            "missing" => fs::remove_file(&events).unwrap(),
            "tail" => {
                bytes.pop();
                fs::write(&events, bytes).unwrap();
            }
            "blank" => {
                bytes.push(b'\n');
                fs::write(&events, bytes).unwrap();
            }
            other => {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                match other {
                    "sequence" => value["sequence_no"] = 9.into(),
                    "sensitive" => value["payload"]["api_key"] = "synthetic".into(),
                    "duplicate" => {
                        bytes.extend_from_within(..);
                        fs::write(&events, bytes).unwrap();
                    }
                    _ => unreachable!(),
                }
                if other != "duplicate" {
                    let mut bytes = serde_json::to_vec(&value).unwrap();
                    bytes.push(b'\n');
                    fs::write(&events, bytes).unwrap();
                }
            }
        }
        let result = input.assess(&paths);
        assert_ne!(
            result.verdict,
            RecoveryVerdict::InputsMatchOnly,
            "{problem}"
        );
        assert!(!result.resume_authorized);
    }
}

#[test]
fn configured_missing_and_present_empty_sources_are_distinct() {
    let dir = private_dir();
    let (mut c, events_only) = controller(dir.path());
    let effects = dir.path().join("effects.jsonl");
    let paths = events_only
        .clone()
        .with_source(RecoverySource::SideEffects, &effects);
    let input = c.capture_recovery_input(&paths).unwrap();
    assert_eq!(
        input.assess(&paths).verdict,
        RecoveryVerdict::SourcesUnavailable
    );
    assert_eq!(
        input.assess(&events_only).verdict,
        RecoveryVerdict::SourcesChanged
    );
    fs::write(&effects, []).unwrap();
    assert_eq!(
        input.assess(&paths).verdict,
        RecoveryVerdict::SourcesChanged
    );
    let empty = c.capture_recovery_input(&paths).unwrap();
    assert_eq!(
        empty.assess(&paths).verdict,
        RecoveryVerdict::InputsMatchOnly
    );
    assert!(!empty.assess(&paths).complete_recovery_coverage);
}

#[test]
fn missing_envelope_loop_id_does_not_bypass_identity_checks() {
    for field in ["project_id", "attempt_id"] {
        let dir = private_dir();
        let (mut c, paths) = controller(dir.path());
        let events = dir.path().join("events.jsonl");
        rewrite_record(&events, |event| {
            event["loop_run_id"] = serde_json::Value::Null;
            event[field] = "unrelated-identity".into();
        });
        let input = c.capture_recovery_input(&paths).unwrap();
        assert_eq!(
            input.assess(&paths).verdict,
            RecoveryVerdict::InconsistentState,
            "{field}"
        );
    }
}

#[test]
fn step_allocations_must_be_nonzero_unique_and_increasing() {
    for numbers in [[2, 2], [2, 1], [0, 2]] {
        let dir = private_dir();
        let (mut c, paths) = controller(dir.path());
        c.run_observation("first", "first observation").unwrap();
        c.run_observation("second", "second observation").unwrap();
        let events = dir.path().join("events.jsonl");
        let original = fs::read_to_string(&events).unwrap();
        let mut bytes = Vec::new();
        let mut allocations = numbers.into_iter();
        for line in original.lines() {
            let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event["event_type"] == "step_started" {
                event["payload"]["step_no"] = allocations.next().unwrap().into();
            }
            serde_json::to_writer(&mut bytes, &event).unwrap();
            bytes.push(b'\n');
        }
        fs::write(&events, bytes).unwrap();
        let input = c.capture_recovery_input(&paths).unwrap();
        assert_eq!(
            input.assess(&paths).verdict,
            RecoveryVerdict::InconsistentState,
            "{numbers:?}"
        );
    }
}

#[test]
fn explicit_loop_identity_conflicts_are_not_matching_inputs() {
    for conflict in ["aggregate", "model_payload"] {
        let dir = private_dir();
        let (mut c, paths) = controller(dir.path());
        let events = dir.path().join("events.jsonl");
        if conflict == "aggregate" {
            rewrite_record(&events, |event| {
                event["aggregate_id"] = "unrelated-loop".into()
            });
        } else {
            let mut bytes = fs::read(&events).unwrap();
            let mut event: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            event["sequence_no"] = 2.into();
            event["event_id"] = "model-action".into();
            event["event_type"] = "model_action_started".into();
            event["payload"] = serde_json::json!({"loop_run_id": "unrelated-loop"});
            serde_json::to_writer(&mut bytes, &event).unwrap();
            bytes.push(b'\n');
            fs::write(&events, bytes).unwrap();
        }
        let input = c.capture_recovery_input(&paths).unwrap();
        assert_eq!(
            input.assess(&paths).verdict,
            RecoveryVerdict::InconsistentState,
            "{conflict}"
        );
    }
}

#[test]
fn pending_effects_block_and_completed_effects_still_require_join_reconciliation() {
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    let effects = dir.path().join("effects.jsonl");
    let paths = paths.with_source(RecoverySource::SideEffects, &effects);
    for status in [
        "planned",
        "started",
        "uncertain",
        "unknown",
        "completed",
        "failed",
    ] {
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "side_effect_id": "effect", "goal_id": "goal", "loop_run_id": "loop",
            "tool_id": "edit_markdown", "class": "workspace_write", "status": status,
            "summary": "synthetic effect", "created_at": "2026-11-23T00:00:00Z"
        }))
        .unwrap();
        bytes.push(b'\n');
        fs::write(&effects, bytes).unwrap();
        let input = c.capture_recovery_input(&paths).unwrap();
        let result = input.assess(&paths);
        let expected = if matches!(status, "completed" | "failed") {
            RecoveryVerdict::AuditReconciliationRequired
        } else {
            RecoveryVerdict::PendingAudit
        };
        assert_eq!(result.verdict, expected, "{status}");
        assert!(!result.resume_authorized);
    }
}

#[test]
fn record_destinations_cannot_alias_even_missing_configured_sources() {
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    let missing = dir.path().join("missing.jsonl");
    let paths = paths.with_source(RecoverySource::ToolCalls, &missing);
    let input = c.capture_recovery_input(&paths).unwrap();
    assert_eq!(
        input.persist_new(&missing, &paths),
        Err(RecoveryInputError::UnsafeOrUnavailable)
    );
    assert!(!missing.exists());
    assert!(c
        .capture_recovery_input(&RecoveryInputPaths::events_only(
            dir.path().join("other.jsonl")
        ))
        .is_err());
}

#[test]
fn malformed_saved_state_never_loads_and_errors_do_not_echo_private_content() {
    for corruption in [
        "schema",
        "parent",
        "unknown",
        "next_step",
        "counter",
        "token",
    ] {
        let dir = private_dir();
        let (mut c, paths) = controller(dir.path());
        let saved = dir.path().join("recovery.json");
        c.capture_recovery_input(&paths)
            .unwrap()
            .persist_new(&saved, &paths)
            .unwrap();
        rewrite_record(&saved, |value| match corruption {
            "schema" => value["schema_version"] = 22.into(),
            "parent" => {
                value["state"]["task_run"]["attempts"][0]["parent_attempt_id"] = "self".into()
            }
            "unknown" => value["state"]["task_run"]["budget"]["ignore_safety"] = true.into(),
            "next_step" => value["state"]["next_step_no"] = 0.into(),
            "counter" => value["state"]["task_run"]["budget"]["attempts_used"] = 0.into(),
            "token" => {
                value["state"]["task_run"]["reported_token_budget"] = serde_json::json!({"limit":2})
            }
            _ => unreachable!(),
        });
        let error = RecoveryInput::load(&saved).unwrap_err();
        assert_eq!(error, RecoveryInputError::InvalidRecord, "{corruption}");
        assert!(!error.to_string().contains("private objective"));
        assert!(!error.to_string().contains(dir.path().to_str().unwrap()));
    }
}

#[test]
fn inconsistent_event_identity_is_diagnostic_not_resume_authority() {
    let dir = private_dir();
    let (mut c, paths) = controller(dir.path());
    let events = dir.path().join("events.jsonl");
    rewrite_record(&events, |value| {
        value["project_id"] = "different-project".into()
    });
    let input = c.capture_recovery_input(&paths).unwrap();
    let result = input.assess(&paths);
    assert_eq!(result.verdict, RecoveryVerdict::InconsistentState);
    assert!(!result.resume_authorized);
}
