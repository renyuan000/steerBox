use super::*;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

fn summary() -> RunSummary {
    RunSummary {
        schema_version: 1,
        project_id: "project-summary".into(),
        goal_id: "goal-summary".into(),
        loop_run_id: "loop-summary".into(),
        task_run_id: "task-summary".into(),
        final_status: "completed".into(),
        step_ids: vec!["step-1".into()],
        file_changes: vec![],
        verification_status: "passed".into(),
        event_ids: vec!["event-1".into()],
        verification_event_ids: vec!["verification-1".into()],
        trace_ids: vec![],
        checkpoint_ids: vec![],
        side_effect_ids: vec![],
        event_count: 2,
        generated_at: "2026-11-22T00:00:00Z".into(),
    }
}

#[derive(Default)]
struct FaultIo {
    fail: Option<&'static str>,
    events: Vec<&'static str>,
    identity: Option<(u64, u64, i32)>,
    rename: Option<(PathBuf, PathBuf)>,
    witness: Option<(u64, u64)>,
}

impl FaultIo {
    fn observe(&mut self, file: &File) {
        let metadata = file.metadata().unwrap();
        let identity = (metadata.dev(), metadata.ino(), file.as_raw_fd());
        if let Some(expected) = self.identity {
            assert_eq!(identity, expected, "summary stages must share one real fd");
        } else {
            self.identity = Some(identity);
        }
    }

    fn stage(&mut self, stage: &'static str) -> io::Result<()> {
        self.events.push(stage);
        if self.fail == Some(stage) {
            Err(io::Error::other("synthetic-private-summary-fault"))
        } else {
            Ok(())
        }
    }
}

impl SummaryIo for FaultIo {
    fn after_open(&mut self, file: &File) -> io::Result<()> {
        self.observe(file);
        self.stage("open")?;
        if let Some((original, moved)) = &self.rename {
            fs::rename(original, moved)?;
            fs::create_dir(original)?;
            fs::write(original.join("summary.json"), b"replacement-summary")?;
            fs::write(original.join("witness"), b"replacement-witness")?;
        }
        Ok(())
    }

    fn truncate(&mut self, file: &File) -> io::Result<()> {
        self.observe(file);
        self.stage("truncate")?;
        file.set_len(0)
    }

    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        self.observe(file);
        if self.fail == Some("write") {
            file.write_all(&bytes[..bytes.len() / 2])?;
        }
        self.stage("write")?;
        file.write_all(bytes)
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        self.observe(file);
        self.stage("file")?;
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        if let Some(expected) = self.witness {
            let witness = parent.sibling(std::ffi::OsStr::new("witness"))?;
            let metadata = witness.open_read()?.unwrap().metadata()?;
            assert_eq!((metadata.dev(), metadata.ino()), expected);
        }
        self.stage("directory")?;
        parent.sync_directory()
    }
}

#[test]
fn summary_rewrite_orders_real_fd_barriers_and_preserves_inode_and_mode() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("summary.json");
    fs::write(&path, vec![b'x'; 4096]).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let value = summary();
    let mut io = FaultIo::default();
    value.write_json_with_io(&path, &mut io).unwrap();
    assert_eq!(
        io.events,
        ["open", "truncate", "write", "file", "directory"]
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(after.mode(), before.mode());
    assert_eq!(after.nlink(), 1);
    assert_eq!(
        fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&value).unwrap()
    );
    assert_eq!(RunSummary::read_json(&path).unwrap(), value);
}

#[test]
fn summary_faults_short_circuit_without_ack_or_rollback_of_real_bytes() {
    let value = summary();
    let encoded = serde_json::to_vec_pretty(&value).unwrap();
    let stages = ["open", "truncate", "write", "file", "directory"];
    for (index, stage) in stages.iter().copied().enumerate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("summary.json");
        let original = b"previous-summary-sentinel";
        fs::write(&path, original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let before = fs::metadata(&path).unwrap();
        let mut io = FaultIo {
            fail: Some(stage),
            ..Default::default()
        };
        let error = value.write_json_with_io(&path, &mut io).unwrap_err();
        assert!(matches!(&error, AuditStoreError::Io { .. }));
        assert!(!error
            .to_string()
            .contains("synthetic-private-summary-fault"));
        assert!(!format!("{error:?}").contains("synthetic-private-summary-fault"));
        assert_eq!(io.events, stages[..=index]);
        let expected: &[u8] = match stage {
            "open" | "truncate" => original,
            "write" => &encoded[..encoded.len() / 2],
            _ => &encoded,
        };
        assert_eq!(fs::read(&path).unwrap(), expected, "failed stage: {stage}");
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (after.dev(), after.ino(), after.mode()),
            (before.dev(), before.ino(), before.mode())
        );
    }
}

#[test]
fn summary_after_open_failure_can_leave_a_new_empty_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("summary.json");
    let mut io = FaultIo {
        fail: Some("open"),
        ..Default::default()
    };
    assert!(summary().write_json_with_io(&path, &mut io).is_err());
    assert_eq!(io.events, ["open"]);
    let metadata = fs::metadata(&path).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.len(), 0);
    assert_eq!(metadata.mode() & 0o077, 0);
}

#[test]
fn summary_directory_barrier_uses_original_parent_after_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let moved = root.path().join("moved");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("summary.json"), b"old-summary").unwrap();
    fs::write(original.join("witness"), b"original-witness").unwrap();
    let witness = fs::metadata(original.join("witness")).unwrap();
    let before = fs::metadata(original.join("summary.json")).unwrap();
    let mut io = FaultIo {
        rename: Some((original.clone(), moved.clone())),
        witness: Some((witness.dev(), witness.ino())),
        ..Default::default()
    };
    let value = summary();
    value
        .write_json_with_io(&original.join("summary.json"), &mut io)
        .unwrap();
    assert_eq!(
        io.events,
        ["open", "truncate", "write", "file", "directory"]
    );
    assert_eq!(
        fs::read(original.join("summary.json")).unwrap(),
        b"replacement-summary"
    );
    assert_eq!(
        fs::read(original.join("witness")).unwrap(),
        b"replacement-witness"
    );
    assert_eq!(
        fs::read(moved.join("summary.json")).unwrap(),
        serde_json::to_vec_pretty(&value).unwrap()
    );
    let after = fs::metadata(moved.join("summary.json")).unwrap();
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
}

#[test]
fn helper_open_write_neither_truncates_nor_changes_existing_inode_or_mode() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("summary.json");
    fs::write(&path, b"must-survive-open-write").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let parent = AuditParent::open(&path, false).unwrap().unwrap();
    let file = parent.open_write().unwrap();
    let opened = file.metadata().unwrap();
    assert_eq!(
        (opened.dev(), opened.ino(), opened.mode()),
        (before.dev(), before.ino(), before.mode())
    );
    assert_eq!(fs::read(&path).unwrap(), b"must-survive-open-write");
}
