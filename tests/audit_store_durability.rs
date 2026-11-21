use steerbox::audit_store::{
    append_jsonl, CheckpointMetadata, CheckpointStore, SideEffectLedger, SideEffectRecord,
    TraceRecord, TraceStore,
};

fn checkpoint() -> CheckpointMetadata {
    CheckpointMetadata {
        checkpoint_id: "checkpoint-synced".into(),
        goal_id: "goal-1".into(),
        loop_run_id: "loop-1".into(),
        step_id: Some("step-1".into()),
        files: vec![],
        diff_boundary: "no_workspace_write".into(),
        recoverability: "fixture".into(),
        side_effect_refs: vec![],
        created_at: "2026-11-12T00:00:00Z".into(),
    }
}

fn side_effect() -> SideEffectRecord {
    SideEffectRecord {
        side_effect_id: "effect-synced".into(),
        goal_id: "goal-1".into(),
        loop_run_id: "loop-1".into(),
        tool_id: "fixture-read".into(),
        class: "read_only".into(),
        status: "started".into(),
        summary: "synthetic audit".into(),
        created_at: "2026-11-12T00:00:00Z".into(),
    }
}

fn trace() -> TraceRecord {
    TraceRecord {
        trace_id: "trace-synced".into(),
        goal_id: Some("goal-1".into()),
        loop_run_id: Some("loop-1".into()),
        step_id: Some("step-1".into()),
        kind: "fixture".into(),
        summary: "synthetic audit".into(),
        occurred_at: "2026-11-12T00:00:00Z".into(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn actual_append_rejects_device_before_writing() {
    // Devices must be rejected before record I/O. File sync failure remains
    // covered by the injected same-descriptor tests in audit_store.
    let error = append_jsonl("/dev/null", &serde_json::json!({"synthetic": true}))
        .expect_err("devices must not be opened as audit records");
    assert!(matches!(
        error,
        steerbox::audit_store::AuditStoreError::Io { ref message }
            if message.starts_with("audit file validation failed:")
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn all_three_public_audit_stores_propagate_device_rejection() {
    let results = [
        CheckpointStore::open("/dev/null").append(&checkpoint()),
        SideEffectLedger::open("/dev/null").append(&side_effect()),
        TraceStore::open("/dev/null").append(&trace()),
    ];
    for result in results {
        assert!(matches!(
            result,
            Err(steerbox::audit_store::AuditStoreError::Io { ref message })
                if message.starts_with("audit file validation failed:")
        ));
    }
}

#[cfg(unix)]
#[test]
fn acknowledged_audit_records_survive_self_child_sigkill() {
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    const CHILD_ROOT: &str = "STEERBOX_AUDIT_SYNC_CHILD_ROOT";
    const READY: &str = "STEERBOX_ALL_AUDIT_APPENDS_ACK";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = std::path::PathBuf::from(root);
        CheckpointStore::open(root.join("checkpoints.jsonl"))
            .append(&checkpoint())
            .unwrap();
        SideEffectLedger::open(root.join("effects.jsonl"))
            .append(&side_effect())
            .unwrap();
        TraceStore::open(root.join("traces.jsonl"))
            .append(&trace())
            .unwrap();
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut child = Reap(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "acknowledged_audit_records_survive_self_child_sigkill",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(READY) {
                let _ = send.send(true);
                return;
            }
        }
        let _ = send.send(false);
    });
    assert!(receive.recv_timeout(Duration::from_secs(10)).unwrap());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    reader.join().unwrap();
    assert_eq!(
        CheckpointStore::open(root.path().join("checkpoints.jsonl"))
            .read_all()
            .unwrap(),
        vec![checkpoint()]
    );
    assert_eq!(
        SideEffectLedger::open(root.path().join("effects.jsonl"))
            .read_all()
            .unwrap(),
        vec![side_effect()]
    );
    assert_eq!(
        TraceStore::open(root.path().join("traces.jsonl"))
            .read_all()
            .unwrap(),
        vec![trace()]
    );
    for file in ["checkpoints.jsonl", "effects.jsonl", "traces.jsonl"] {
        assert!(fs::read(root.path().join(file)).unwrap().ends_with(b"\n"));
    }
}
