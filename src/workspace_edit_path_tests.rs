use super::*;
use std::os::unix::fs::MetadataExt;

struct ReplaceParentAfterRead {
    parent: PathBuf,
    moved: PathBuf,
    source_identity: Option<(u64, u64)>,
    decoy_identity: Option<(u64, u64)>,
    fail_write: bool,
    write_failed: bool,
}

impl WorkspaceEditIo for ReplaceParentAfterRead {
    fn read(&mut self, file: &mut fs::File, limit: u64) -> io::Result<String> {
        let metadata = file.metadata()?;
        self.source_identity = Some((metadata.dev(), metadata.ino()));
        let mut contents = String::new();
        file.take(limit).read_to_string(&mut contents)?;
        fs::rename(&self.parent, &self.moved)?;
        fs::create_dir(&self.parent)?;
        let decoy = self.parent.join("README.md");
        fs::write(&decoy, b"replacement directory decoy\n")?;
        let metadata = fs::metadata(decoy)?;
        self.decoy_identity = Some((metadata.dev(), metadata.ino()));
        if self.fail_write {
            fs::write(
                self.parent
                    .join(format!("README.md.steerbox-tmp-{}", std::process::id())),
                b"replacement directory temp sentinel\n",
            )?;
        }
        Ok(contents)
    }

    fn write(&mut self, file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
        if self.fail_write {
            file.write_all(&bytes[..bytes.len() / 2])?;
            self.write_failed = true;
            return Err(io::Error::other("synthetic partial write failure"));
        }
        file.write_all(bytes)
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        let mut witness = parent
            .sibling(OsStr::new("sync-witness"))?
            .open_create_new()?;
        witness.write_all(b"same pinned directory\n")?;
        parent.sync_directory()
    }
}

#[test]
fn replacement_and_directory_sync_stay_with_the_parent_opened_before_read() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("docs");
    let moved = root.path().join("moved-docs");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("README.md"), b"original\n").unwrap();
    let goal = GoalContract::new(
        "synthetic-goal",
        "synthetic-project",
        "edit an ordinary fixture",
        vec![".".into()],
        vec![],
        vec![],
    );
    let mut io = ReplaceParentAfterRead {
        parent: parent.clone(),
        moved: moved.clone(),
        source_identity: None,
        decoy_identity: None,
        fail_write: false,
        write_failed: false,
    };

    let result = edit_markdown_with_io(
        root.path(),
        &goal,
        "docs/README.md",
        "original\n",
        "updated\n",
        true,
        &mut io,
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(fs::read(moved.join("README.md")).unwrap(), b"updated\n");
    assert_eq!(
        fs::read(parent.join("README.md")).unwrap(),
        b"replacement directory decoy\n"
    );
    let decoy = fs::metadata(parent.join("README.md")).unwrap();
    assert_eq!(Some((decoy.dev(), decoy.ino())), io.decoy_identity);
    let published = fs::metadata(moved.join("README.md")).unwrap();
    assert_ne!(Some((published.dev(), published.ino())), io.source_identity);
    assert_eq!(
        fs::read(moved.join("sync-witness")).unwrap(),
        b"same pinned directory\n"
    );
    assert!(!parent.join("sync-witness").exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 2);
}

#[test]
fn failed_write_cleans_only_the_owned_temporary_in_the_pinned_parent() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("docs");
    let moved = root.path().join("moved-docs");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("README.md"), b"original\n").unwrap();
    let original = fs::metadata(parent.join("README.md")).unwrap();
    let goal = GoalContract::new(
        "synthetic-goal",
        "synthetic-project",
        "edit an ordinary fixture",
        vec![".".into()],
        vec![],
        vec![],
    );
    let mut io = ReplaceParentAfterRead {
        parent: parent.clone(),
        moved: moved.clone(),
        source_identity: None,
        decoy_identity: None,
        fail_write: true,
        write_failed: false,
    };

    let result = edit_markdown_with_io(
        root.path(),
        &goal,
        "docs/README.md",
        "original\n",
        "updated\n",
        true,
        &mut io,
    );

    assert!(matches!(result, Err(WorkspaceEditError::Io { .. })));
    assert!(io.write_failed);
    assert_eq!(fs::read(moved.join("README.md")).unwrap(), b"original\n");
    let current = fs::metadata(moved.join("README.md")).unwrap();
    assert_eq!(
        (current.dev(), current.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(
        fs::read(parent.join("README.md")).unwrap(),
        b"replacement directory decoy\n"
    );
    assert_eq!(
        fs::read(parent.join(format!("README.md.steerbox-tmp-{}", std::process::id()))).unwrap(),
        b"replacement directory temp sentinel\n"
    );
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 2);
}
