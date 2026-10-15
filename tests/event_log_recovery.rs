use std::fs;
use std::path::PathBuf;
use steerbox::domain::EventEnvelope;
use steerbox::event_log::EventLog;

fn test_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "steerbox-event-log-recovery-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ))
}

fn event(id: &str) -> EventEnvelope {
    EventEnvelope {
        event_id: id.into(),
        event_type: "recovery_probe".into(),
        schema_version: 1,
        sequence_no: None,
        aggregate_type: "loop_run".into(),
        aggregate_id: "loop-recovery".into(),
        portfolio_id: None,
        project_id: Some("project-recovery".into()),
        goal_id: Some("goal-recovery".into()),
        task_id: None,
        task_run_id: Some("task-recovery".into()),
        attempt_id: None,
        loop_run_id: Some("loop-recovery".into()),
        step_id: None,
        correlation_id: "loop-recovery".into(),
        causation_id: None,
        producer: "event-log-recovery-test".into(),
        occurred_at: "2026-10-15T00:00:00Z".into(),
        recorded_at: "2026-10-15T00:00:00Z".into(),
        payload: serde_json::json!({"probe": id}),
        redaction_class: "public".into(),
    }
}

#[test]
fn reopen_continues_event_sequence_without_rewriting_history() {
    let path = test_path();

    let first_sequence = {
        let mut log = EventLog::open(&path).expect("event log opens");
        assert_eq!(log.append(event("first")).expect("first event appends"), 1);
        log.append(event("second")).expect("second event appends")
    };
    assert_eq!(first_sequence, 2);

    let second_sequence = {
        let mut reopened = EventLog::open(&path).expect("event log reopens");
        assert_eq!(
            reopened.read_all().expect("history reads").len(),
            2,
            "reopen must preserve both prior events"
        );
        reopened
            .append(event("after-reopen"))
            .expect("event appends after reopen")
    };
    assert_eq!(second_sequence, 3);

    let events = EventLog::open(&path)
        .expect("event log opens for final read")
        .read_all()
        .expect("final history reads");
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence_no)
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3)]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "after-reopen"]
    );

    fs::remove_file(path).expect("temporary event log removes");
}
