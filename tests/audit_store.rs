use std::fs;
use steerbox::audit_store::{
    append_jsonl, read_jsonl, CheckpointMetadata, CheckpointStore, RecoveryAction, RunSummary,
    SideEffectLedger, SideEffectRecord, TraceRecord, TraceStore,
};
use steerbox::tool_registry::SideEffectDisposition;

#[test]
fn trace_and_checkpoint_records_round_trip_as_jsonl() {
    let path = std::env::temp_dir().join(format!("steerbox-audit-{}.jsonl", std::process::id()));
    let _ = fs::remove_file(&path);
    let trace = TraceRecord {
        trace_id: "trace-1".into(),
        goal_id: Some("goal-1".into()),
        loop_run_id: Some("loop-1".into()),
        step_id: Some("step-1".into()),
        kind: "step_summary".into(),
        summary: "fixture summary".into(),
        occurred_at: "2026-10-08T00:00:00Z".into(),
    };
    let checkpoint = CheckpointMetadata {
        checkpoint_id: "checkpoint-1".into(),
        goal_id: "goal-1".into(),
        loop_run_id: "loop-1".into(),
        step_id: Some("step-1".into()),
        files: vec!["README.md".into()],
        diff_boundary: "README.md:+1/-0".into(),
        recoverability: "local_patch_available".into(),
        side_effect_refs: Vec::new(),
        created_at: "2026-10-08T00:00:00Z".into(),
    };
    append_jsonl(&path, &trace).expect("trace appends");
    append_jsonl(&path, &checkpoint).expect("checkpoint appends");
    let records: Vec<serde_json::Value> = read_jsonl(&path).expect("records read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["trace_id"], "trace-1");
    assert_eq!(records[1]["checkpoint_id"], "checkpoint-1");
    fs::remove_file(path).expect("temporary audit file removes");
}

#[test]
fn run_summary_round_trips_as_json() {
    let root = std::env::temp_dir().join(format!("steerbox-summary-{}", std::process::id()));
    let path = root.join("runs/loop-1/summary.json");
    let summary = RunSummary {
        schema_version: 1,
        project_id: "project-1".into(),
        goal_id: "goal-1".into(),
        loop_run_id: "loop-1".into(),
        task_run_id: "task-1".into(),
        final_status: "completed".into(),
        step_ids: vec!["step-1".into()],
        file_changes: vec!["README.md".into()],
        verification_status: "passed".into(),
        event_ids: vec!["event-1".into()],
        verification_event_ids: vec!["verification-1".into()],
        trace_ids: vec!["trace-1".into()],
        checkpoint_ids: vec!["checkpoint-1".into()],
        side_effect_ids: vec!["side-effect-1".into()],
        event_count: 7,
        generated_at: "2026-10-09T00:00:00Z".into(),
    };
    summary.write_json(&path).expect("summary writes");
    assert_eq!(
        RunSummary::read_json(&path).expect("summary reads"),
        summary
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn trace_and_side_effect_summaries_are_redacted_before_persistence() {
    let root =
        std::env::temp_dir().join(format!("steerbox-audit-redaction-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("audit directory creates");
    let summary = "login failed password=synthetic-password at https://alice:synthetic-url-secret@example.test";

    TraceStore::open(root.join("traces.jsonl"))
        .append(&TraceRecord {
            trace_id: "trace-redaction".into(),
            goal_id: Some("goal".into()),
            loop_run_id: Some("loop".into()),
            step_id: Some("step".into()),
            kind: "tool_failed".into(),
            summary: summary.into(),
            occurred_at: "2026-10-13T00:00:00Z".into(),
        })
        .expect("trace appends");
    SideEffectLedger::open(root.join("side-effects.jsonl"))
        .append(&SideEffectRecord {
            side_effect_id: "effect-redaction".into(),
            goal_id: "goal".into(),
            loop_run_id: "loop".into(),
            tool_id: "fixture-tool".into(),
            class: "read_only".into(),
            status: "failed".into(),
            summary: summary.into(),
            created_at: "2026-10-13T00:00:00Z".into(),
        })
        .expect("side effect appends");

    let traces = fs::read_to_string(root.join("traces.jsonl")).expect("traces read");
    let side_effects =
        fs::read_to_string(root.join("side-effects.jsonl")).expect("side effects read");
    for persisted in [traces, side_effects] {
        assert!(!persisted.contains("synthetic-password"));
        assert!(!persisted.contains("synthetic-url-secret"));
        assert!(persisted.contains("[REDACTED:PASSWORD]"));
        assert!(persisted.contains("[REDACTED:URL_PASSWORD]"));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn side_effect_metadata_is_derived_without_changing_json_schema() {
    let record = SideEffectRecord {
        side_effect_id: "effect-unknown".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        tool_id: "fixture-tool".into(),
        class: "unknown-external-class".into(),
        status: "future-status".into(),
        summary: "fixture summary".into(),
        created_at: "2026-10-13T00:00:00Z".into(),
    };

    assert_eq!(record.disposition(), SideEffectDisposition::ManualRequired);
    assert_eq!(record.audit_status().to_string(), "unknown");
    assert!(!record.automatic_execution_allowed());

    let json = serde_json::to_value(&record).expect("side effect record serializes");
    let object = json.as_object().expect("side effect record is an object");
    assert_eq!(object.len(), 8);
    for field in [
        "side_effect_id",
        "goal_id",
        "loop_run_id",
        "tool_id",
        "class",
        "status",
        "summary",
        "created_at",
    ] {
        assert!(
            object.contains_key(field),
            "schema field {field} is present"
        );
    }
    for derived_field in ["disposition", "audit_status", "automatic_execution_allowed"] {
        assert!(
            !object.contains_key(derived_field),
            "derived field {derived_field} is not persisted"
        );
    }
}

#[test]
fn side_effect_audit_status_maps_known_and_unknown_values() {
    let record = |status: &str| SideEffectRecord {
        side_effect_id: "effect-status".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        tool_id: "fixture-tool".into(),
        class: "read_only".into(),
        status: status.into(),
        summary: "fixture summary".into(),
        created_at: "2026-10-13T00:00:00Z".into(),
    };

    assert_eq!(record("planned").audit_status().to_string(), "planned");
    assert_eq!(record("started").audit_status().to_string(), "started");
    assert_eq!(record("completed").audit_status().to_string(), "completed");
    assert_eq!(record("failed").audit_status().to_string(), "failed");
    assert_eq!(
        record("verification_failed").audit_status().to_string(),
        "failed"
    );
    assert_eq!(record("blocked").audit_status().to_string(), "blocked");
    assert_eq!(
        record("future-status").audit_status().to_string(),
        "unknown"
    );
}

#[test]
fn latest_recovery_checkpoint_is_scoped_to_loop_and_step() {
    let root = std::env::temp_dir().join(format!("steerbox-recovery-query-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("recovery directory creates");
    let path = root.join("checkpoints.jsonl");
    let store = CheckpointStore::open(&path);
    let hash_a = "a".repeat(64);
    let hash_b = "b".repeat(64);
    let hash_c = "c".repeat(64);
    let hash_d = "d".repeat(64);
    for checkpoint in [
        CheckpointMetadata {
            checkpoint_id: "loop-a:step-1:old".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["README.md".into()],
            diff_boundary: format!("before=sha256:{hash_a};after=sha256:{hash_b}"),
            recoverability: "verified_local_workspace_restore_or_escalate".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:00:00Z".into(),
        },
        CheckpointMetadata {
            checkpoint_id: "loop-a:step-1:latest".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["README.md".into()],
            diff_boundary: format!("before=sha256:{hash_b};after=sha256:{hash_c}"),
            recoverability: "verification_failed_local_workspace_restore_or_escalate".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:01:00Z".into(),
        },
        CheckpointMetadata {
            checkpoint_id: "loop-a:step-2".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-2".into()),
            files: vec!["README.md".into()],
            diff_boundary: format!("before=sha256:{hash_c};after=sha256:{hash_d}"),
            recoverability: "verified_local_workspace_restore_or_escalate".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:02:00Z".into(),
        },
        CheckpointMetadata {
            checkpoint_id: "loop-other:step-1".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-other".into(),
            step_id: Some("step-1".into()),
            files: Vec::new(),
            diff_boundary: "no_workspace_write".into(),
            recoverability: "event_and_state_replayable".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:03:00Z".into(),
        },
        CheckpointMetadata {
            checkpoint_id: "loop-a:no-step".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: None,
            files: Vec::new(),
            diff_boundary: "no_workspace_write".into(),
            recoverability: "event_and_state_replayable".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:00:00Z".into(),
        },
    ] {
        store.append(&checkpoint).expect("checkpoint appends");
    }

    let latest = store
        .latest_for_loop_step("loop-a", "step-1")
        .expect("recovery checkpoint query succeeds")
        .expect("matching checkpoint exists");
    assert_eq!(latest.metadata.checkpoint_id, "loop-a:step-1:latest");
    assert_eq!(
        latest.before_hash.as_deref(),
        Some(format!("sha256:{hash_b}").as_str())
    );
    assert_eq!(
        latest.after_hash.as_deref(),
        Some(format!("sha256:{hash_c}").as_str())
    );
    assert_eq!(latest.action, RecoveryAction::RestoreRequired);
    let latest_for_loop = store
        .latest_for("loop-a", None)
        .expect("loop query succeeds")
        .expect("loop checkpoint exists");
    assert_eq!(latest_for_loop.metadata.checkpoint_id, "loop-a:no-step");
    assert_eq!(latest_for_loop.action, RecoveryAction::NoAction);
    assert!(latest_for_loop.hash_metadata_consistent());
    assert!(store
        .latest_for_loop_step("loop-a", "missing-step")
        .expect("unmatched step query succeeds")
        .is_none());
    assert!(store
        .latest_for_loop_step("loop-b", "step-1")
        .expect("unmatched query succeeds")
        .is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn recovery_checkpoint_rejects_incomplete_hash_metadata_without_writing() {
    let root =
        std::env::temp_dir().join(format!("steerbox-recovery-boundary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("recovery directory creates");
    let workspace_file = root.join("workspace.txt");
    fs::write(&workspace_file, "fixture workspace content").expect("workspace file writes");
    let path = root.join("checkpoints.jsonl");
    CheckpointStore::open(&path)
        .append(&CheckpointMetadata {
            checkpoint_id: "loop-a:step-1:incomplete".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["workspace.txt".into()],
            diff_boundary: format!("before=sha256:{};after=pending", "a".repeat(64)),
            recoverability: "local_workspace_restore_or_escalate".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-13T00:00:00Z".into(),
        })
        .expect("checkpoint appends");

    let checkpoint = CheckpointStore::open(&path)
        .latest_for_loop_step("loop-a", "step-1")
        .expect("recovery checkpoint query succeeds")
        .expect("matching checkpoint exists");
    assert!(!checkpoint.hash_metadata_consistent());
    assert_eq!(checkpoint.action, RecoveryAction::ManualIntervention);
    assert_eq!(
        fs::read_to_string(&workspace_file).expect("workspace file reads"),
        "fixture workspace content"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn equal_hashes_and_non_workspace_checkpoint_need_no_restore() {
    let metadata = CheckpointMetadata {
        checkpoint_id: "loop-a:step-1:no-change".into(),
        goal_id: "goal-a".into(),
        loop_run_id: "loop-a".into(),
        step_id: Some("step-1".into()),
        files: vec!["README.md".into()],
        diff_boundary: "before=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;after=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        recoverability: "verified_local_workspace_restore_or_escalate".into(),
        side_effect_refs: Vec::new(),
        created_at: "2026-10-13T00:00:00Z".into(),
    };
    let checkpoint = steerbox::audit_store::RecoveryCheckpoint::from_metadata(metadata);
    assert!(checkpoint.hash_metadata_consistent());
    assert_eq!(checkpoint.action, RecoveryAction::NoAction);

    let metadata = CheckpointMetadata {
        checkpoint_id: "loop-a:step-1:fixture".into(),
        goal_id: "goal-a".into(),
        loop_run_id: "loop-a".into(),
        step_id: Some("step-1".into()),
        files: Vec::new(),
        diff_boundary: "no_workspace_write_fixture_loop".into(),
        recoverability: "event_and_state_replayable".into(),
        side_effect_refs: Vec::new(),
        created_at: "2026-10-13T00:00:00Z".into(),
    };
    let checkpoint = steerbox::audit_store::RecoveryCheckpoint::from_metadata(metadata);
    assert!(checkpoint.hash_metadata_consistent());
    assert_eq!(checkpoint.action, RecoveryAction::NoAction);
}

#[test]
fn no_workspace_write_marker_with_files_requires_manual_intervention() {
    for marker in ["no_workspace_write", "no_workspace_write_fixture_loop"] {
        let metadata = CheckpointMetadata {
            checkpoint_id: "checkpoint-conflicting-marker".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["workspace.txt".into()],
            diff_boundary: marker.into(),
            recoverability: "event_and_state_replayable".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-14T00:00:00Z".into(),
        };

        let checkpoint = steerbox::audit_store::RecoveryCheckpoint::from_metadata(metadata);
        assert_eq!(
            checkpoint.action,
            RecoveryAction::ManualIntervention,
            "{marker}"
        );
        assert!(!checkpoint.hash_metadata_consistent(), "{marker}");
    }
}

#[test]
fn unknown_recoverability_with_changed_hashes_requires_manual_intervention() {
    for recoverability in [
        "future-recovery-mode",
        "verified_future_recovery_mode",
        "write_failed_future_recovery_mode",
        "verification_failed_future_recovery_mode",
    ] {
        let metadata = CheckpointMetadata {
            checkpoint_id: "checkpoint-unknown-recoverability".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["workspace.txt".into()],
            diff_boundary: format!(
                "before=sha256:{};after=sha256:{}",
                "a".repeat(64),
                "b".repeat(64)
            ),
            recoverability: recoverability.into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-14T00:00:00Z".into(),
        };

        let checkpoint = steerbox::audit_store::RecoveryCheckpoint::from_metadata(metadata);
        assert!(checkpoint.hash_metadata_consistent(), "{recoverability}");
        assert_eq!(
            checkpoint.action,
            RecoveryAction::ManualIntervention,
            "{recoverability}"
        );
    }
}

#[test]
fn malformed_hash_boundaries_require_manual_intervention() {
    let before = format!("sha256:{}", "a".repeat(64));
    let after = format!("sha256:{}", "b".repeat(64));
    for diff_boundary in [
        format!("before={before};before={before};after={after}"),
        format!("before={before};after={after};after={after}"),
        format!("before={before};after={after};unknown=value"),
        format!("before=sha256:{};after={after}", "a".repeat(63)),
        format!("before={before};after=sha256:{}", "b".repeat(65)),
        format!("before={before};after=sha256:{}", "g".repeat(64)),
        format!("before={before}"),
        format!("after={after}"),
        format!("before={before};after={after};"),
        "unknown_no_workspace_write".into(),
    ] {
        let metadata = CheckpointMetadata {
            checkpoint_id: "checkpoint-invalid-hashes".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: vec!["workspace.txt".into()],
            diff_boundary,
            recoverability: "verified_local_workspace_restore_or_escalate".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-14T00:00:00Z".into(),
        };

        let checkpoint = steerbox::audit_store::RecoveryCheckpoint::from_metadata(metadata);
        assert!(
            !checkpoint.hash_metadata_consistent(),
            "{:?}",
            checkpoint.metadata
        );
        assert_eq!(checkpoint.action, RecoveryAction::ManualIntervention);
    }
}

#[test]
fn latest_checkpoint_preserves_jsonl_error_lines_even_for_nonmatching_queries() {
    let path = std::env::temp_dir().join(format!(
        "steerbox-recovery-invalid-json-{}.jsonl",
        std::process::id()
    ));
    let store = CheckpointStore::open(&path);
    fs::write(&path, "\n").expect("blank first line writes");
    store
        .append(&CheckpointMetadata {
            checkpoint_id: "checkpoint-valid".into(),
            goal_id: "goal-a".into(),
            loop_run_id: "loop-a".into(),
            step_id: Some("step-1".into()),
            files: Vec::new(),
            diff_boundary: "no_workspace_write".into(),
            recoverability: "event_and_state_replayable".into(),
            side_effect_refs: Vec::new(),
            created_at: "2026-10-14T00:00:00Z".into(),
        })
        .expect("valid checkpoint appends");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("fixture opens");
    std::io::Write::write_all(&mut file, b" \nnot-json\n").expect("invalid fourth line writes");
    drop(file);

    let read_error = store.read_all().expect_err("all-records read must fail");
    for loop_run_id in ["loop-a", "missing-loop"] {
        let error = store
            .latest_for(loop_run_id, None)
            .expect_err("query must not hide invalid JSON");
        assert!(matches!(
            error,
            steerbox::audit_store::AuditStoreError::InvalidJson { line: 4, .. }
        ));
        assert_eq!(error.to_string(), read_error.to_string());
    }
    fs::remove_file(path).expect("invalid fixture removes");
}

#[test]
fn missing_checkpoint_log_is_not_created_by_recovery_queries() {
    let path = std::env::temp_dir().join(format!(
        "steerbox-recovery-missing-{}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let store = CheckpointStore::open(&path);
    assert!(store
        .latest_for("loop-a", None)
        .expect("missing log query succeeds")
        .is_none());
    assert!(store
        .latest_for_loop_step("loop-a", "step-1")
        .expect("missing log step query succeeds")
        .is_none());
    assert!(!path.exists());
}

#[test]
fn recovery_queries_interpret_recorded_hashes_without_rehashing_or_restoring_files() {
    let root = std::env::temp_dir().join(format!(
        "steerbox-recovery-metadata-only-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture directory creates");
    let workspace_file = root.join("workspace.txt");
    let missing_file = root.join("missing.txt");
    let _ = fs::remove_file(&missing_file);
    fs::write(
        &workspace_file,
        "current workspace is not the recorded snapshot",
    )
    .expect("workspace fixture writes");
    let store_path = root.join("checkpoints.jsonl");
    let _ = fs::remove_file(&store_path);
    let store = CheckpointStore::open(&store_path);
    for (step_id, recoverability, expected_action) in [
        (
            "write-failed",
            "write_failed_local_workspace_restore_or_escalate",
            RecoveryAction::RestoreRequired,
        ),
        (
            "verification-failed",
            "verification_failed_local_workspace_restore_or_escalate",
            RecoveryAction::RestoreRequired,
        ),
        (
            "verified",
            "verified_local_workspace_restore_or_escalate",
            RecoveryAction::NoAction,
        ),
    ] {
        store
            .append(&CheckpointMetadata {
                checkpoint_id: format!("checkpoint-{step_id}"),
                goal_id: "goal-a".into(),
                loop_run_id: "loop-a".into(),
                step_id: Some(step_id.into()),
                files: vec![
                    workspace_file.to_string_lossy().into_owned(),
                    missing_file.to_string_lossy().into_owned(),
                ],
                diff_boundary: format!(
                    "before=sha256:{};after=sha256:{}",
                    "a".repeat(64),
                    "b".repeat(64)
                ),
                recoverability: recoverability.into(),
                side_effect_refs: Vec::new(),
                created_at: "2026-10-14T00:00:00Z".into(),
            })
            .expect("checkpoint appends");
        let log_before_query = fs::read(&store_path).expect("checkpoint log reads");

        let checkpoint = store
            .latest_for_loop_step("loop-a", step_id)
            .expect("metadata-only query succeeds")
            .expect("checkpoint exists");

        assert!(checkpoint.hash_metadata_consistent());
        assert_eq!(checkpoint.action, expected_action);
        assert_eq!(
            fs::read_to_string(&workspace_file).expect("workspace fixture reads"),
            "current workspace is not the recorded snapshot"
        );
        assert!(!missing_file.exists());
        assert_eq!(
            fs::read(&store_path).expect("checkpoint log rereads"),
            log_before_query
        );
    }
    fs::remove_dir_all(root).expect("fixture directory removes");
}
