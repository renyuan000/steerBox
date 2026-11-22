#![cfg(unix)]

use super::*;
use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const BEFORE: &str = "Before synthetic Markdown.\n";
const AFTER: &str = "After synthetic Markdown with a longer body.\n";

fn goal() -> GoalContract {
    GoalContract::new(
        "synthetic-edit-goal",
        "synthetic-edit-project",
        "update synthetic Markdown",
        vec![".".into()],
        vec![],
        vec!["markdown_link_check".into()],
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileObservation {
    descriptor: i32,
    device: u64,
    inode: u64,
}

fn observe(file: &fs::File) -> io::Result<FileObservation> {
    let metadata = file.metadata()?;
    assert!(metadata.is_file());
    assert_eq!(metadata.nlink(), 1);
    Ok(FileObservation {
        descriptor: file.as_raw_fd(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn temporary_for(directory: &Path, observation: FileObservation) -> io::Result<PathBuf> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if (metadata.dev(), metadata.ino()) == (observation.device, observation.inode) {
            return Ok(entry.path());
        }
    }
    Err(io::Error::other("synthetic temporary entry was not found"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Read,
    PartialWrite,
    CompleteWrite,
    Permissions,
    FileSync,
    Publish,
    DirectorySync,
}

struct ObservedIo {
    failure: Option<Failure>,
    directory: PathBuf,
    expected_limit: u64,
    stages: Vec<&'static str>,
    source: Option<FileObservation>,
    source_mode: Option<u32>,
    read_bytes: Vec<u8>,
    temporary: Option<FileObservation>,
    temporary_name: Option<OsString>,
    written_bytes: Vec<u8>,
    requested_mode: Option<u32>,
    synchronized_mode: Option<u32>,
}

impl ObservedIo {
    fn new(directory: &Path, expected: &str, failure: Option<Failure>) -> Self {
        Self {
            failure,
            directory: directory.into(),
            expected_limit: expected.len() as u64 + 1,
            stages: Vec::new(),
            source: None,
            source_mode: None,
            read_bytes: Vec::new(),
            temporary: None,
            temporary_name: None,
            written_bytes: Vec::new(),
            requested_mode: None,
            synchronized_mode: None,
        }
    }
}

impl WorkspaceEditIo for ObservedIo {
    fn read(&mut self, file: &mut fs::File, limit: u64) -> io::Result<String> {
        self.stages.push("read");
        assert_eq!(limit, self.expected_limit);
        self.source = Some(observe(file)?);
        self.source_mode = Some(file.metadata()?.mode() & 0o7777);
        let mut contents = String::new();
        file.take(limit).read_to_string(&mut contents)?;
        self.read_bytes = contents.as_bytes().to_vec();
        if self.failure == Some(Failure::Read) {
            return Err(io::Error::other("injected synthetic read failure"));
        }
        Ok(contents)
    }

    fn write(&mut self, file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
        self.stages.push("write");
        let observation = observe(file)?;
        self.temporary = Some(observation);
        let temporary = temporary_for(&self.directory, observation)?;
        self.temporary_name = Some(temporary.file_name().unwrap().to_os_string());
        let written = if self.failure == Some(Failure::PartialWrite) {
            &bytes[..bytes.len() / 2]
        } else {
            bytes
        };
        file.write_all(written)?;
        self.written_bytes = fs::read(temporary)?;
        assert_eq!(self.written_bytes, written);
        if matches!(
            self.failure,
            Some(Failure::PartialWrite | Failure::CompleteWrite)
        ) {
            return Err(io::Error::other("injected synthetic write failure"));
        }
        Ok(())
    }

    fn set_permissions(&mut self, file: &fs::File, permissions: fs::Permissions) -> io::Result<()> {
        self.stages.push("permissions");
        assert_eq!(Some(observe(file)?), self.temporary);
        let mode = permissions.mode() & 0o7777;
        self.requested_mode = Some(mode);
        assert_eq!(Some(mode), self.source_mode);
        if self.failure == Some(Failure::Permissions) {
            return Err(io::Error::other("injected synthetic permissions failure"));
        }
        // This is the existing edit's ordinary mode copy, not a chmod matrix.
        file.set_permissions(permissions)?;
        assert_eq!(file.metadata()?.mode() & 0o7777, mode);
        Ok(())
    }

    fn sync_file(&mut self, file: &fs::File) -> io::Result<()> {
        self.stages.push("file-sync");
        assert_eq!(Some(observe(file)?), self.temporary);
        self.synchronized_mode = Some(file.metadata()?.mode() & 0o7777);
        assert_eq!(self.synchronized_mode, self.source_mode);
        if self.failure == Some(Failure::FileSync) {
            return Err(io::Error::other("injected synthetic file-sync failure"));
        }
        file.sync_all()
    }

    fn publish(&mut self, parent: &AuditParent, temp_name: &OsStr) -> io::Result<()> {
        self.stages.push("publish");
        assert_eq!(Some(temp_name), self.temporary_name.as_deref());
        if self.failure == Some(Failure::Publish) {
            // Fail before the real rename, not after a successful publication.
            return Err(io::Error::other("injected synthetic publish failure"));
        }
        parent.rename_from(temp_name)
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        self.stages.push("directory-sync");
        if self.failure == Some(Failure::DirectorySync) {
            return Err(io::Error::other(
                "injected synthetic directory-sync failure",
            ));
        }
        parent.sync_directory()
    }
}

fn assert_only_target(directory: &Path) {
    let names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![OsString::from("README.md")]);
}

fn assert_fault_boundary(failure: Failure, expected_stages: &[&str]) {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("README.md");
    fs::write(&target, BEFORE).unwrap();
    let old_metadata = fs::metadata(&target).unwrap();
    let mut io = ObservedIo::new(directory.path(), BEFORE, Some(failure));

    let result = edit_markdown_with_io(
        directory.path(),
        &goal(),
        "README.md",
        BEFORE,
        AFTER,
        true,
        &mut io,
    );

    assert!(matches!(result, Err(WorkspaceEditError::Io { .. })));
    assert_eq!(io.stages, expected_stages);
    assert_eq!(io.read_bytes, BEFORE.as_bytes());
    let source = io.source.unwrap();
    assert_eq!(
        (source.device, source.inode),
        (old_metadata.dev(), old_metadata.ino())
    );
    let current_metadata = fs::metadata(&target).unwrap();
    if failure == Failure::Read {
        assert!(io.temporary.is_none());
        assert!(io.written_bytes.is_empty());
    } else {
        let temporary = io.temporary.unwrap();
        assert_ne!(
            (temporary.device, temporary.inode),
            (source.device, source.inode)
        );
        let expected_written = if failure == Failure::PartialWrite {
            &AFTER.as_bytes()[..AFTER.len() / 2]
        } else {
            AFTER.as_bytes()
        };
        assert_eq!(io.written_bytes, expected_written);
    }
    if failure == Failure::DirectorySync {
        // Publication happened despite the error. Current readable bytes are
        // not a rollback or an OS power-loss durability guarantee.
        let temporary = io.temporary.unwrap();
        assert_eq!(fs::read(&target).unwrap(), AFTER.as_bytes());
        assert_eq!(
            (current_metadata.dev(), current_metadata.ino()),
            (temporary.device, temporary.inode)
        );
    } else {
        assert_eq!(fs::read(&target).unwrap(), BEFORE.as_bytes());
        assert_eq!(
            (current_metadata.dev(), current_metadata.ino()),
            (old_metadata.dev(), old_metadata.ino())
        );
    }
    assert_eq!(current_metadata.mode(), old_metadata.mode());
    // No cleanup failure is injected into these ordinary synthetic fixtures.
    assert_only_target(directory.path());
}

#[test]
fn actual_source_read_error_stops_before_creating_temporary() {
    assert_fault_boundary(Failure::Read, &["read"]);
}

#[test]
fn partial_temporary_write_error_preserves_original_and_cleans_temporary() {
    assert_fault_boundary(Failure::PartialWrite, &["read", "write"]);
}

#[test]
fn full_temporary_write_then_error_does_not_publish_or_apply_permissions() {
    assert_fault_boundary(Failure::CompleteWrite, &["read", "write"]);
}

#[test]
fn permissions_error_preserves_original_and_stops_before_file_sync() {
    assert_fault_boundary(Failure::Permissions, &["read", "write", "permissions"]);
}

#[test]
fn temporary_sync_error_preserves_original_and_stops_before_publish() {
    assert_fault_boundary(
        Failure::FileSync,
        &["read", "write", "permissions", "file-sync"],
    );
}

#[test]
fn publish_error_after_real_file_sync_preserves_original_and_cleans_temporary() {
    assert_fault_boundary(
        Failure::Publish,
        &["read", "write", "permissions", "file-sync", "publish"],
    );
}

#[test]
fn parent_sync_error_returns_failure_with_new_file_already_published() {
    assert_fault_boundary(
        Failure::DirectorySync,
        &[
            "read",
            "write",
            "permissions",
            "file-sync",
            "publish",
            "directory-sync",
        ],
    );
}

#[test]
fn success_publishes_the_synced_real_temporary_with_source_permissions() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("README.md");
    fs::write(&target, BEFORE).unwrap();
    let original = fs::metadata(&target).unwrap();
    let mut io = ObservedIo::new(directory.path(), BEFORE, None);

    let result = edit_markdown_with_io(
        directory.path(),
        &goal(),
        "README.md",
        BEFORE,
        AFTER,
        true,
        &mut io,
    )
    .unwrap();

    assert_eq!(
        result,
        EditResult {
            relative_path: "README.md".into(),
            changed: true,
            bytes_before: BEFORE.len(),
            bytes_after: AFTER.len(),
        }
    );
    assert_eq!(
        io.stages,
        [
            "read",
            "write",
            "permissions",
            "file-sync",
            "publish",
            "directory-sync"
        ]
    );
    assert_eq!(io.read_bytes, BEFORE.as_bytes());
    assert_eq!(io.written_bytes, AFTER.as_bytes());
    assert_eq!(io.requested_mode, Some(original.mode() & 0o7777));
    assert_eq!(io.synchronized_mode, io.requested_mode);
    let temporary = io.temporary.unwrap();
    let published = fs::metadata(&target).unwrap();
    assert_eq!(
        (published.dev(), published.ino()),
        (temporary.device, temporary.inode)
    );
    assert_ne!(published.ino(), original.ino());
    assert_eq!(published.mode(), original.mode());
    assert_eq!(fs::read(&target).unwrap(), AFTER.as_bytes());
    assert_only_target(directory.path());
}

#[test]
fn noop_reads_the_actual_source_without_writes_or_durable_reack() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("README.md");
    fs::write(&target, BEFORE).unwrap();
    let original = fs::metadata(&target).unwrap();
    let mut io = ObservedIo::new(directory.path(), BEFORE, None);

    let result = edit_markdown_with_io(
        directory.path(),
        &goal(),
        "README.md",
        BEFORE,
        BEFORE,
        true,
        &mut io,
    )
    .unwrap();

    assert_eq!(
        result,
        EditResult {
            relative_path: "README.md".into(),
            changed: false,
            bytes_before: BEFORE.len(),
            bytes_after: BEFORE.len(),
        }
    );
    assert_eq!(io.stages, ["read"]);
    assert!(io.temporary.is_none());
    assert_eq!(io.read_bytes, BEFORE.as_bytes());
    assert_eq!(fs::read(&target).unwrap(), BEFORE.as_bytes());
    let current = fs::metadata(&target).unwrap();
    assert_eq!(
        (current.dev(), current.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(current.mode(), original.mode());
    assert_only_target(directory.path());
}

#[test]
fn expected_content_mismatch_does_not_create_or_publish_a_temporary() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("README.md");
    fs::write(&target, BEFORE).unwrap();
    let original = fs::metadata(&target).unwrap();
    let expected = "Before synthetic Markdown?\n";
    let mut io = ObservedIo::new(directory.path(), expected, None);

    let result = edit_markdown_with_io(
        directory.path(),
        &goal(),
        "README.md",
        expected,
        AFTER,
        true,
        &mut io,
    );

    assert_eq!(result, Err(WorkspaceEditError::ExpectedContentMismatch));
    assert_eq!(io.stages, ["read"]);
    assert!(io.temporary.is_none());
    assert_eq!(io.read_bytes, BEFORE.as_bytes());
    assert_eq!(fs::read(&target).unwrap(), BEFORE.as_bytes());
    let current = fs::metadata(&target).unwrap();
    assert_eq!(
        (current.dev(), current.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(current.mode(), original.mode());
    assert_only_target(directory.path());
}
