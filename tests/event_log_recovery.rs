use std::fs;
use std::path::PathBuf;
use steerbox::domain::EventEnvelope;
use steerbox::event_log::{EventLog, EventLogError};

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

#[test]
fn reopen_rejects_complete_json_without_its_line_terminator() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut first = event("unterminated");
    first.sequence_no = Some(1);
    let original = serde_json::to_vec(&first).unwrap();
    fs::write(&path, &original).unwrap();

    let error = EventLog::open(&path).expect_err("an unsealed record must block append access");
    assert!(matches!(
        error,
        EventLogError::UnterminatedRecord { line: 1 }
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn reopen_preserves_the_complete_prefix_and_unterminated_second_record() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    {
        let mut log = EventLog::open(&path).unwrap();
        log.append(event("committed-prefix")).unwrap();
    }
    let mut original = fs::read(&path).unwrap();
    let mut second = event("unsealed-second");
    second.sequence_no = Some(2);
    original.extend(serde_json::to_vec(&second).unwrap());
    fs::write(&path, &original).unwrap();

    let error = EventLog::open(&path).expect_err("even valid JSON needs its record terminator");
    assert!(matches!(
        error,
        EventLogError::UnterminatedRecord { line: 2 }
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn reopen_rejects_unterminated_whitespace_instead_of_silently_repairing_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut original = serde_json::to_vec(&event("complete")).unwrap();
    original.extend_from_slice(b"\n  ");
    fs::write(&path, &original).unwrap();

    let error =
        EventLog::open(&path).expect_err("an incomplete trailing line must remain untouched");
    assert!(matches!(
        error,
        EventLogError::UnterminatedRecord { line: 2 }
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn reopen_preserves_a_truncated_json_tail_without_discarding_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut original = serde_json::to_vec(&event("complete")).unwrap();
    original.extend_from_slice(b"\n{\"event_id\":\"unfinished");
    fs::write(&path, &original).unwrap();

    let error = EventLog::open(&path).expect_err("partial JSON must not be skipped or truncated");
    assert!(matches!(
        error,
        EventLogError::UnterminatedRecord { line: 2 }
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn reopen_does_not_skip_a_corrupt_terminated_record_between_valid_records() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut original = serde_json::to_vec(&event("first")).unwrap();
    original.extend_from_slice(b"\n{not-json}\n");
    original.extend(serde_json::to_vec(&event("third")).unwrap());
    original.push(b'\n');
    fs::write(&path, &original).unwrap();

    let error = EventLog::open(&path).expect_err("interior corruption must remain fatal");
    assert!(matches!(error, EventLogError::InvalidEvent { line: 2, .. }));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn reopen_accepts_empty_files_and_complete_crlf_records_with_blank_lines() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    fs::write(&path, b"").unwrap();
    assert!(EventLog::open(&path)
        .unwrap()
        .read_all()
        .unwrap()
        .is_empty());
    let mut first = event("crlf");
    first.sequence_no = Some(7);
    let mut original = serde_json::to_vec(&first).unwrap();
    original.extend_from_slice(b"\r\n \r\n\n");
    fs::write(&path, &original).unwrap();

    let mut log = EventLog::open(&path).unwrap();
    assert_eq!(log.append(event("after-crlf")).unwrap(), 8);
    let updated = fs::read(&path).unwrap();
    assert!(updated.starts_with(&original));
    assert_eq!(log.read_all().unwrap().len(), 2);
}

#[test]
fn reopen_preserves_multibyte_text_and_continues_the_sequence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    {
        let mut log = EventLog::open(&path).unwrap();
        log.append(event("已同步-恢复-🦀")).unwrap();
    }
    let original = fs::read(&path).unwrap();

    let mut log = EventLog::open(&path).unwrap();
    assert_eq!(log.read_all().unwrap()[0].event_id, "已同步-恢复-🦀");
    assert_eq!(log.append(event("下一条")).unwrap(), 2);
    assert!(fs::read(&path).unwrap().starts_with(&original));
    assert_eq!(log.read_all().unwrap()[1].event_id, "下一条");
}

#[test]
fn reopen_rejects_invalid_utf8_without_lossy_repair() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut original = serde_json::to_vec(&event("valid-prefix")).unwrap();
    original.extend_from_slice(b"\n{\xff}\n");
    fs::write(&path, &original).unwrap();

    let error = EventLog::open(&path).expect_err("invalid UTF-8 must not be silently replaced");
    assert!(
        matches!(error, EventLogError::Io(ref io) if io.kind() == std::io::ErrorKind::InvalidData)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn killed_writer_keeps_acknowledged_events_for_reopen() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    const CHILD_ROOT: &str = "STEERBOX_EVENT_TAIL_TEST_CHILD_ROOT";
    const READY: &str = "STEERBOX_EVENT_APPEND_ACKNOWLEDGED";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let mut log = EventLog::open(PathBuf::from(root).join("events.jsonl")).unwrap();
        assert_eq!(log.append(event("before-sigkill")).unwrap(), 1);
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }

    struct ReapChild(Child);
    impl Drop for ReapChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = tempfile::tempdir().unwrap();
    let mut child = ReapChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "killed_writer_keeps_acknowledged_events_for_reopen",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sent, received) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(READY) {
                let _ = sent.send(true);
                return;
            }
        }
        let _ = sent.send(false);
    });
    assert!(received.recv_timeout(Duration::from_secs(10)).unwrap());
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    reader.join().unwrap();

    let mut reopened = EventLog::open(root.path().join("events.jsonl")).unwrap();
    assert_eq!(reopened.read_all().unwrap()[0].event_id, "before-sigkill");
    assert_eq!(reopened.append(event("after-sigkill")).unwrap(), 2);
    assert_eq!(reopened.read_all().unwrap().len(), 2);
}
