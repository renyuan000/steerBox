use super::*;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;

fn record(status: &str) -> ToolCallRecord {
    ToolCallRecord {
        tool_call_id: "call-1".into(),
        tool_id: "git_status".into(),
        project_id: "project-1".into(),
        goal_id: "goal-1".into(),
        task_run_id: "run-1".into(),
        loop_run_id: "loop-1".into(),
        step_id: "step-1".into(),
        side_effect_class: "none".into(),
        status: status.into(),
        summary: "fixture completed".into(),
        created_at: "2026-11-22T00:00:00Z".into(),
    }
}

#[derive(Default)]
struct FaultIo {
    events: Vec<&'static str>,
    fail: Option<&'static str>,
    file_identity: Option<(i32, u64, u64)>,
    contender: Option<File>,
    rename: Option<(PathBuf, PathBuf)>,
    directory_witness: Option<(u64, u64)>,
}

impl FaultIo {
    fn stage(&mut self, stage: &'static str) -> io::Result<()> {
        if let Some(contender) = &self.contender {
            assert!(matches!(
                contender.try_lock(),
                Err(fs::TryLockError::WouldBlock)
            ));
        }
        self.events.push(stage);
        if self.fail == Some(stage) {
            Err(io::Error::other("synthetic-private-tool-io"))
        } else {
            Ok(())
        }
    }
}

impl ToolStoreIo for FaultIo {
    fn after_lock(&mut self) -> io::Result<()> {
        self.stage("locked")?;
        if let Some((original, moved)) = &self.rename {
            fs::rename(original, moved)?;
            fs::create_dir(original)?;
            fs::write(original.join("tools.jsonl"), b"replacement-sentinel")?;
            fs::write(original.join("witness"), b"replacement-witness")?;
        }
        Ok(())
    }

    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()> {
        let metadata = file.metadata()?;
        self.file_identity = Some((file.as_raw_fd(), metadata.dev(), metadata.ino()));
        if self.fail == Some("write") {
            file.write_all(&encoded[..encoded.len() / 2])?;
        }
        self.stage("write")?;
        file.write_all(encoded)
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        assert_eq!(
            self.file_identity,
            Some((file.as_raw_fd(), metadata.dev(), metadata.ino()))
        );
        self.stage("file")?;
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        if let Some(expected) = self.directory_witness {
            let witness = parent.sibling(std::ffi::OsStr::new("witness"))?;
            let metadata = witness.open_read()?.unwrap().metadata()?;
            assert_eq!((metadata.dev(), metadata.ino()), expected);
        }
        self.stage("directory")?;
        parent.sync_directory()
    }
}

fn contender(path: &Path) -> File {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(PathBuf::from(lock_path))
        .unwrap()
}

#[test]
fn append_ack_holds_real_lock_until_same_fd_and_parent_are_synchronized() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tools.jsonl");
    let store = ToolCallRecordStore::open(&path).unwrap();
    let lock = contender(&path);
    lock.try_lock().unwrap();
    lock.unlock().unwrap();
    let mut io = FaultIo {
        contender: Some(lock),
        ..Default::default()
    };
    store.append_with_io(&record("completed"), &mut io).unwrap();
    assert_eq!(io.events, ["locked", "write", "file", "directory"]);
    assert_eq!(fs::read(&path).unwrap().last(), Some(&b'\n'));
    io.contender.as_ref().unwrap().try_lock().unwrap();
}

#[test]
fn append_failures_preserve_uncertain_bytes_short_circuit_and_release_lock() {
    for (stage, expected) in [
        ("locked", vec!["locked"]),
        ("write", vec!["locked", "write"]),
        ("file", vec!["locked", "write", "file"]),
        ("directory", vec!["locked", "write", "file", "directory"]),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tools.jsonl");
        let store = ToolCallRecordStore::open(&path).unwrap();
        let lock = contender(&path);
        let mut io = FaultIo {
            fail: Some(stage),
            contender: Some(lock),
            ..Default::default()
        };
        let error = store
            .append_with_io(&record("completed"), &mut io)
            .unwrap_err();
        assert!(!error.to_string().contains("synthetic-private-tool-io"));
        assert!(!format!("{error:?}").contains("synthetic-private-tool-io"));
        let ToolCallStoreError::Io(error) = error else {
            panic!("expected I/O failure")
        };
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(io.events, expected);
        if stage == "locked" {
            assert!(!path.exists());
        } else {
            let bytes = fs::read(&path).unwrap();
            let mut encoded = serde_json::to_vec(&record("completed")).unwrap();
            encoded.push(b'\n');
            let expected = if stage == "write" {
                &encoded[..encoded.len() / 2]
            } else {
                &encoded
            };
            assert_eq!(bytes, expected);
        }
        io.contender.as_ref().unwrap().try_lock().unwrap();
    }
}

#[test]
fn append_uses_pinned_parent_even_after_locked_parent_path_is_replaced() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    fs::create_dir(&original).unwrap();
    let path = original.join("tools.jsonl");
    let store = ToolCallRecordStore::open(&path).unwrap();
    fs::write(original.join("witness"), b"original-witness").unwrap();
    let witness = original.join("witness").metadata().unwrap();
    let moved = root.path().join("moved");
    let mut io = FaultIo {
        contender: Some(contender(&path)),
        rename: Some((original.clone(), moved.clone())),
        directory_witness: Some((witness.dev(), witness.ino())),
        ..Default::default()
    };
    store.append_with_io(&record("completed"), &mut io).unwrap();
    assert_eq!(io.events, ["locked", "write", "file", "directory"]);
    assert_eq!(fs::read(&path).unwrap(), b"replacement-sentinel");
    assert_eq!(
        fs::read(moved.join("tools.jsonl")).unwrap().last(),
        Some(&b'\n')
    );
    io.contender.as_ref().unwrap().try_lock().unwrap();
}

#[test]
fn tool_history_keeps_duplicate_ids_identical_records_and_uncertain_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tools.jsonl");
    let store = ToolCallRecordStore::open(&path).unwrap();
    store.append(&record("started")).unwrap();
    store.append(&record("completed")).unwrap();
    store.append(&record("completed")).unwrap();
    let mut fault = FaultIo {
        fail: Some("directory"),
        ..Default::default()
    };
    assert!(store
        .append_with_io(&record("completed"), &mut fault)
        .is_err());
    store.append(&record("completed")).unwrap();
    let records = store.read_page(0, 10).unwrap();
    assert_eq!(records.len(), 5);
    assert!(records.iter().all(|entry| entry.tool_call_id == "call-1"));
    assert_eq!(records[0].status, "started");
}

#[test]
fn append_does_not_parse_or_repair_existing_invalid_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tools.jsonl");
    fs::write(&path, b"invalid-history\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let store = ToolCallRecordStore::open(&path).unwrap();
    store.append(&record("completed")).unwrap();
    assert!(fs::read(&path).unwrap().starts_with(b"invalid-history\n{"));
    assert!(matches!(
        store.read_page(0, 1),
        Err(ToolCallStoreError::InvalidJson { line: 1, .. })
    ));
}
