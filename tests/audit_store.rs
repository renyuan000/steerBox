use std::fs;
use steerbox::audit_store::{
    append_jsonl, read_jsonl, CheckpointMetadata, RunSummary, TraceRecord,
};

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
