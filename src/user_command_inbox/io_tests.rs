use super::*;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

fn entry() -> UserCommandEntry {
    UserCommandEntry {
        command: UserCommand::new(
            "command-1",
            "operator",
            "project",
            "goal",
            "task",
            "loop",
            CommandKind::Pause,
            "correlation",
            serde_json::json!({}),
        )
        .unwrap(),
        status: CommandStatus::Pending,
        updated_at: "2026-11-22T00:00:00Z".into(),
    }
}

#[derive(Default)]
struct FaultIo {
    fail: Option<&'static str>,
    stages: Vec<&'static str>,
    identity: Option<(u64, u64, i32)>,
    replacement: Option<(PathBuf, PathBuf)>,
    witness: Option<(u64, u64)>,
}

impl FaultIo {
    fn observe(&mut self, file: &File) {
        let metadata = file.metadata().unwrap();
        let identity = (metadata.dev(), metadata.ino(), file.as_raw_fd());
        if let Some(expected) = self.identity {
            assert_eq!(identity, expected, "append and sync must use one fd");
        } else {
            self.identity = Some(identity);
        }
    }

    fn stage(&mut self, stage: &'static str) -> std::io::Result<()> {
        self.stages.push(stage);
        if self.fail == Some(stage) {
            Err(std::io::Error::other("synthetic-private-inbox-fault"))
        } else {
            Ok(())
        }
    }
}

impl InboxIo for FaultIo {
    fn after_open(&mut self, file: &File) -> std::io::Result<()> {
        self.observe(file);
        self.stage("open")?;
        if let Some((original, moved)) = &self.replacement {
            fs::rename(original, moved)?;
            fs::create_dir(original)?;
            fs::write(original.join("inbox.jsonl"), b"replacement-inbox")?;
            fs::write(original.join("witness"), b"replacement-witness")?;
        }
        Ok(())
    }

    fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
        self.observe(file);
        if self.fail == Some("write") {
            file.write_all(&bytes[..bytes.len() / 2])?;
        }
        self.stage("write")?;
        assert_eq!(bytes.last(), Some(&b'\n'));
        serde_json::from_slice::<UserCommandEntry>(&bytes[..bytes.len() - 1]).unwrap();
        file.write_all(bytes)
    }

    fn sync_file(&mut self, file: &File) -> std::io::Result<()> {
        self.observe(file);
        self.stage("file")?;
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> std::io::Result<()> {
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
fn append_orders_json_lf_same_fd_sync_and_directory_ack() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let inbox = UserCommandInbox::open(&path);
    let value = entry();
    let mut io = FaultIo::default();
    inbox.append_record_with_io(&value, &mut io).unwrap();
    assert_eq!(io.stages, ["open", "write", "file", "directory"]);
    let mut expected = serde_json::to_vec(&value).unwrap();
    expected.push(b'\n');
    assert_eq!(fs::read(&path).unwrap(), expected);
    assert_eq!(inbox.read_all().unwrap(), [value]);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "no sidecar");
}

#[test]
fn append_failures_short_circuit_without_ack_or_rollback() {
    let value = entry();
    let mut encoded = serde_json::to_vec(&value).unwrap();
    encoded.push(b'\n');
    let stages = ["open", "write", "file", "directory"];
    for (index, failure) in stages.iter().enumerate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("inbox.jsonl");
        let mut prefix = serde_json::to_vec(&value).unwrap();
        prefix.push(b'\n');
        fs::write(&path, &prefix).unwrap();
        let inbox = UserCommandInbox::open(&path);
        let mut io = FaultIo {
            fail: Some(failure),
            ..Default::default()
        };
        let error = inbox.append_record_with_io(&value, &mut io).unwrap_err();
        assert!(matches!(error, UserCommandInboxError::Io { .. }));
        assert!(!error.to_string().contains("synthetic-private"));
        assert!(!error.to_string().contains(&path.display().to_string()));
        assert!(!format!("{error:?}").contains("synthetic-private"));
        assert!(!format!("{error:?}").contains(&path.display().to_string()));
        assert_eq!(io.stages, stages[..=index]);
        let expected = match *failure {
            "open" => &[][..],
            "write" => &encoded[..encoded.len() / 2],
            _ => &encoded[..],
        };
        prefix.extend_from_slice(expected);
        assert_eq!(fs::read(&path).unwrap(), prefix);
    }
}

#[test]
fn append_and_directory_ack_stay_on_pinned_parent_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let moved = root.path().join("moved");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("witness"), b"original-witness").unwrap();
    let metadata = fs::metadata(original.join("witness")).unwrap();
    let mut io = FaultIo {
        replacement: Some((original.clone(), moved.clone())),
        witness: Some((metadata.dev(), metadata.ino())),
        ..Default::default()
    };
    let value = entry();
    UserCommandInbox::open(original.join("inbox.jsonl"))
        .append_record_with_io(&value, &mut io)
        .unwrap();
    assert_eq!(io.stages, ["open", "write", "file", "directory"]);
    assert_eq!(
        fs::read(original.join("inbox.jsonl")).unwrap(),
        b"replacement-inbox"
    );
    assert_eq!(
        UserCommandInbox::open(moved.join("inbox.jsonl"))
            .read_all()
            .unwrap(),
        [value]
    );
}

#[test]
fn append_retry_after_directory_failure_still_appends_instead_of_deduplicating() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let inbox = UserCommandInbox::open(&path);
    let value = entry();
    let mut io = FaultIo {
        fail: Some("directory"),
        ..Default::default()
    };
    assert!(inbox.append_record_with_io(&value, &mut io).is_err());
    assert_eq!(
        inbox.read_all().unwrap().as_slice(),
        std::slice::from_ref(&value)
    );
    inbox.append_record(&value).unwrap();
    assert_eq!(inbox.read_all().unwrap(), [value.clone(), value]);
}

#[test]
fn public_same_intent_retry_returns_record_after_uncertain_append_without_appending() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let inbox = UserCommandInbox::open(&path);
    let value = entry();
    let mut io = FaultIo {
        fail: Some("directory"),
        ..Default::default()
    };
    assert!(inbox.append_record_with_io(&value, &mut io).is_err());
    let before = fs::read(&path).unwrap();
    assert_eq!(inbox.submit(value.command.clone()).unwrap(), value);
    assert_eq!(fs::read(&path).unwrap(), before);
    // This is the existing public retry path, not a re-synchronization promise.
}

#[test]
fn direct_append_rejects_unsafe_leaf_even_without_a_preceding_history_read() {
    for kind in ["symlink", "dangling", "hardlink", "directory"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("inbox.jsonl");
        let target = root.path().join("target");
        let missing = root.path().join("missing");
        fs::write(&target, b"synthetic-sentinel").unwrap();
        match kind {
            "symlink" => std::os::unix::fs::symlink(&target, &path).unwrap(),
            "dangling" => std::os::unix::fs::symlink(&missing, &path).unwrap(),
            "hardlink" => fs::hard_link(&target, &path).unwrap(),
            "directory" => fs::create_dir(&path).unwrap(),
            _ => unreachable!(),
        }
        let before = fs::symlink_metadata(&path).unwrap();
        let mut io = FaultIo::default();
        let result = UserCommandInbox::open(&path).append_record_with_io(&entry(), &mut io);
        assert!(matches!(result, Err(UserCommandInboxError::Io { .. })));
        assert!(io.stages.is_empty(), "must reject before writing");
        let after = fs::symlink_metadata(&path).unwrap();
        assert_eq!(
            (before.dev(), before.ino(), before.mode(), before.nlink()),
            (after.dev(), after.ino(), after.mode(), after.nlink())
        );
        assert_eq!(fs::read(&target).unwrap(), b"synthetic-sentinel");
        assert!(!missing.exists());
    }
}
