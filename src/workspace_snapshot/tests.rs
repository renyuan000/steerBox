use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    _dir: tempfile::TempDir,
    workspace: PathBuf,
    root: PathBuf,
    held_root: PathBuf,
    goal: GoalContract,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), b"original\r\n").unwrap();
        let root = dir.path().join("snapshots");
        let held_root = dir.path().join("held-snapshots");
        let goal = GoalContract::new(
            "goal",
            "project",
            "edit docs",
            vec![".".into()],
            vec![],
            vec!["test".into()],
        );
        Self {
            _dir: dir,
            workspace,
            root,
            held_root,
            goal,
        }
    }
}

fn replace_directory(root: &Path, held: &Path) {
    fs::rename(root, held).unwrap();
    fs::create_dir(root).unwrap();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
}

struct ReplaceRoot<'a>(&'a Fixture);
impl SnapshotIo for ReplaceRoot<'_> {
    fn root_opened(&mut self) {
        replace_directory(&self.0.root, &self.0.held_root);
        fs::write(self.0.root.join("sentinel"), b"decoy").unwrap();
    }
}

#[test]
fn capture_uses_pinned_store_after_root_path_replacement() {
    let f = Fixture::new();
    WorkspaceSnapshotStore::open(&f.root)
        .capture_with_io(
            &f.workspace,
            &f.goal,
            "README.md",
            "one",
            &mut ReplaceRoot(&f),
        )
        .unwrap();
    assert!(f.held_root.join("one.json").is_file());
    assert!(!f.root.join("one.json").exists());
    assert_eq!(fs::read(f.root.join("sentinel")).unwrap(), b"decoy");
    assert_eq!(
        fs::read(f.workspace.join("README.md")).unwrap(),
        b"original\r\n"
    );
}

#[test]
fn source_read_uses_pinned_parent_after_directory_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("source");
    let held = dir.path().join("held-source");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("README.md"), b"original").unwrap();
    let observed = read_bytes_with_hook(&parent.join("README.md"), 100, false, || {
        replace_directory(&parent, &held);
        fs::write(parent.join("README.md"), b"replacement-decoy").unwrap();
    })
    .unwrap();
    assert_eq!(observed, b"original");
    assert_eq!(
        fs::read(parent.join("README.md")).unwrap(),
        b"replacement-decoy"
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureStage {
    Write,
    FullWrite,
    FileSync,
    DirectorySync,
}

struct FailPersistence<'a> {
    fixture: &'a Fixture,
    stage: FailureStage,
    calls: Vec<&'static str>,
}

impl FailPersistence<'_> {
    fn fail(&self) -> std::io::Result<()> {
        replace_directory(&self.fixture.root, &self.fixture.held_root);
        fs::write(self.fixture.root.join("one.json"), b"do-not-remove").unwrap();
        Err(std::io::ErrorKind::StorageFull.into())
    }
}

impl SnapshotIo for FailPersistence<'_> {
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
        self.calls.push("write");
        if self.stage == FailureStage::Write {
            file.write_all(&bytes[..bytes.len() / 2])?;
            self.fail()
        } else if self.stage == FailureStage::FullWrite {
            file.write_all(bytes)?;
            self.fail()
        } else {
            file.write_all(bytes)
        }
    }

    fn sync_file(&mut self, file: &File) -> std::io::Result<()> {
        self.calls.push("file-sync");
        if self.stage == FailureStage::FileSync {
            self.fail()
        } else {
            file.sync_all()
        }
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> std::io::Result<()> {
        self.calls.push("directory-sync");
        if self.stage == FailureStage::DirectorySync {
            self.fail()
        } else {
            parent.sync_directory()
        }
    }
}

fn assert_pinned_cleanup(stage: FailureStage) {
    let f = Fixture::new();
    let mut io = FailPersistence {
        fixture: &f,
        stage,
        calls: Vec::new(),
    };
    let error = WorkspaceSnapshotStore::open(&f.root)
        .capture_with_io(&f.workspace, &f.goal, "README.md", "one", &mut io)
        .unwrap_err();
    assert_eq!(
        error,
        SnapshotError::Io {
            kind: std::io::ErrorKind::StorageFull
        }
    );
    // This observes normal persistence only. Best-effort cleanup sync bypasses
    // the seam and must not be mistaken for continuing the failed operation.
    let expected: &[&str] = match stage {
        FailureStage::Write | FailureStage::FullWrite => &["write"],
        FailureStage::FileSync => &["write", "file-sync"],
        FailureStage::DirectorySync => &["write", "file-sync", "directory-sync"],
    };
    assert_eq!(io.calls, expected);
    assert!(
        !f.held_root.join("one.json").exists(),
        "remove only this call's created record"
    );
    assert_eq!(fs::read(f.root.join("one.json")).unwrap(), b"do-not-remove");
    assert!(f.held_root.is_dir(), "do not roll back the store root");
    assert_eq!(
        fs::read(f.workspace.join("README.md")).unwrap(),
        b"original\r\n"
    );
}

#[test]
fn partial_write_failure_cleans_pinned_record_not_replacement_decoy() {
    assert_pinned_cleanup(FailureStage::Write);
}

#[test]
fn file_sync_failure_cleans_pinned_record_not_replacement_decoy() {
    assert_pinned_cleanup(FailureStage::FileSync);
}

#[test]
fn directory_sync_failure_cleans_pinned_record_not_replacement_decoy() {
    assert_pinned_cleanup(FailureStage::DirectorySync);
}

#[test]
fn full_write_error_cleans_pinned_record_not_replacement_decoy() {
    assert_pinned_cleanup(FailureStage::FullWrite);
}

struct FailCleanup<'a>(&'a Fixture);
impl SnapshotIo for FailCleanup<'_> {
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
        file.write_all(&bytes[..bytes.len() / 2])?;
        // This controlled entry substitution makes best-effort unlink fail.
        fs::remove_file(self.0.root.join("one.json")).unwrap();
        fs::create_dir(self.0.root.join("one.json")).unwrap();
        Err(std::io::ErrorKind::StorageFull.into())
    }
}

#[test]
fn cleanup_failure_preserves_the_original_persistence_error() {
    let f = Fixture::new();
    let error = WorkspaceSnapshotStore::open(&f.root)
        .capture_with_io(
            &f.workspace,
            &f.goal,
            "README.md",
            "one",
            &mut FailCleanup(&f),
        )
        .unwrap_err();
    assert_eq!(
        error,
        SnapshotError::Io {
            kind: std::io::ErrorKind::StorageFull
        }
    );
    assert!(f.root.join("one.json").is_dir());
    assert!(f.root.is_dir());
    assert_eq!(
        fs::read(f.workspace.join("README.md")).unwrap(),
        b"original\r\n"
    );
}

#[test]
fn record_read_uses_pinned_store_after_root_path_replacement() {
    let f = Fixture::new();
    let store = WorkspaceSnapshotStore::open(&f.root);
    store
        .capture(&f.workspace, &f.goal, "README.md", "one")
        .unwrap();
    let expected = fs::read(f.root.join("one.json")).unwrap();
    let root = store.checked_root(&f.workspace, "one", false).unwrap();
    replace_directory(&f.root, &f.held_root);
    fs::write(f.root.join("one.json"), b"invalid-decoy-record").unwrap();
    assert_eq!(
        read_parent(&root, MAX_RECORD_BYTES, true).unwrap(),
        expected
    );
    assert_eq!(
        fs::read(f.root.join("one.json")).unwrap(),
        b"invalid-decoy-record"
    );
}

#[test]
fn capture_orders_real_same_file_and_pinned_directory_barriers_before_success() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    struct Witness<'a> {
        fixture: &'a Fixture,
        written_file: Option<(i32, u64, u64)>,
        calls: Vec<&'static str>,
    }

    impl SnapshotIo for Witness<'_> {
        fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
            self.calls.push("write");
            let metadata = file.metadata()?;
            self.written_file = Some((file.as_raw_fd(), metadata.dev(), metadata.ino()));
            file.write_all(bytes)?;
            replace_directory(&self.fixture.root, &self.fixture.held_root);
            fs::write(self.fixture.root.join("one.json"), b"same-name-decoy")?;
            Ok(())
        }

        fn sync_file(&mut self, file: &File) -> std::io::Result<()> {
            self.calls.push("file-sync");
            let metadata = file.metadata()?;
            assert_eq!(
                Some((file.as_raw_fd(), metadata.dev(), metadata.ino())),
                self.written_file
            );
            let held = fs::metadata(self.fixture.held_root.join("one.json"))?;
            assert_eq!((metadata.dev(), metadata.ino()), (held.dev(), held.ino()));
            file.sync_all()
        }

        fn sync_directory(&mut self, parent: &AuditParent) -> std::io::Result<()> {
            self.calls.push("directory-sync");
            let actual = parent.directory_metadata()?;
            let held = fs::metadata(&self.fixture.held_root)?;
            let decoy = fs::metadata(&self.fixture.root)?;
            assert_eq!((actual.dev(), actual.ino()), (held.dev(), held.ino()));
            assert_ne!((actual.dev(), actual.ino()), (decoy.dev(), decoy.ino()));
            parent.sync_directory()
        }
    }

    let f = Fixture::new();
    let mut io = Witness {
        fixture: &f,
        written_file: None,
        calls: Vec::new(),
    };
    let metadata = WorkspaceSnapshotStore::open(&f.root)
        .capture_with_io(&f.workspace, &f.goal, "README.md", "one", &mut io)
        .unwrap();
    assert_eq!(io.calls, ["write", "file-sync", "directory-sync"]);
    let record: SnapshotRecord =
        serde_json::from_slice(&fs::read(f.held_root.join("one.json")).unwrap()).unwrap();
    assert_eq!(record.metadata, metadata);
    assert_eq!(record.contents.as_bytes(), b"original\r\n");
    assert_eq!(
        fs::read(f.root.join("one.json")).unwrap(),
        b"same-name-decoy"
    );
}
