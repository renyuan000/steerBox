use std::fs;
use steerbox::audit_store::{
    append_jsonl, read_jsonl, CheckpointMetadata, RunSummary, SideEffectLedger, SideEffectRecord,
    TraceRecord, TraceStore,
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
