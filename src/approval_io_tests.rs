#![cfg(unix)]

use super::*;
use chrono::Duration;
use std::ffi::OsStr;
use std::fs;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::MetadataExt;

type FileIdentity = (u64, u64, u32, u32, u32);

fn identity(metadata: &fs::Metadata) -> FileIdentity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
    )
}

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    moved_directory: PathBuf,
    path: PathBuf,
    store: ApprovalStore,
    request: ApprovalRequest,
    before: Vec<u8>,
    identity: FileIdentity,
}

impl Fixture {
    fn approved_once() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("live");
        let moved_directory = root.path().join("moved");
        let path = directory.join("approvals.jsonl");
        let store = ApprovalStore::open(&path);
        let now = Utc::now();
        let request = ApprovalRequest::at(
            "synthetic-once",
            "synthetic-project",
            "synthetic-session",
            "synthetic-task",
            "read_file",
            "workspace-source",
            "metadata_only",
            "inspect synthetic source metadata",
            now,
            now + Duration::minutes(15),
        )
        .unwrap();
        store.submit(request.clone()).unwrap();
        store
            .resolve(
                &request.request_id,
                ApprovalDecision::Approve(ApprovalScope::Once),
            )
            .unwrap();
        let before = fs::read(&path).unwrap();
        let identity = identity(&fs::metadata(&path).unwrap());
        Self {
            _root: root,
            directory,
            moved_directory,
            path,
            store,
            request,
            before,
            identity,
        }
    }

    // Inspect the cache directly: public queries would reload and conceal an
    // incorrect early in-memory update by the failed consume operation.
    fn cached_status(&self) -> ApprovalStatus {
        self.store.state.lock().unwrap().records[&self.request.request_id].status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    PartialWrite,
    CompleteWrite,
    FileSync,
    DirectorySync,
}

const REPLACEMENT_DATA: &[u8] = b"synthetic replacement data\n";
const REPLACEMENT_LOCK: &[u8] = b"synthetic replacement lock\n";
const WITNESS: &[u8] = b"pinned-directory-witness\n";

struct ObservedIo {
    fault: Option<Fault>,
    expected_identity: FileIdentity,
    actual_fd: Option<RawFd>,
    calls: Vec<&'static str>,
    encoded: Vec<u8>,
    written: usize,
    rename: Option<(PathBuf, PathBuf)>,
}

impl ObservedIo {
    fn new(fixture: &Fixture, fault: Option<Fault>) -> Self {
        Self {
            fault,
            expected_identity: fixture.identity,
            actual_fd: None,
            calls: Vec::new(),
            encoded: Vec::new(),
            written: 0,
            rename: None,
        }
    }

    fn observe_file(&mut self, file: &File) {
        assert_eq!(identity(&file.metadata().unwrap()), self.expected_identity);
        match self.actual_fd {
            Some(descriptor) => assert_eq!(file.as_raw_fd(), descriptor),
            None => self.actual_fd = Some(file.as_raw_fd()),
        }
    }

    fn injected_error() -> io::Error {
        io::Error::other("synthetic approval I/O failure")
    }
}

impl ApprovalIo for ObservedIo {
    fn write_line(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        assert!(self.calls.is_empty());
        self.observe_file(file);
        self.calls.push("write_line");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let record: ApprovalRecord = serde_json::from_slice(bytes).unwrap();
        assert_eq!(record.status, ApprovalStatus::Consumed);
        assert_eq!(record.scope, Some(ApprovalScope::Once));
        self.encoded = bytes.to_vec();

        if let Some((original, moved)) = &self.rename {
            fs::rename(original, moved)?;
            fs::create_dir(original)?;
            fs::write(original.join("approvals.jsonl"), REPLACEMENT_DATA)?;
            fs::write(original.join("approvals.jsonl.lock"), REPLACEMENT_LOCK)?;
        }

        self.written = if self.fault == Some(Fault::PartialWrite) {
            bytes.len() / 2
        } else {
            bytes.len()
        };
        file.write_all(&bytes[..self.written])?;
        if matches!(self.fault, Some(Fault::PartialWrite | Fault::CompleteWrite)) {
            return Err(Self::injected_error());
        }
        Ok(())
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        assert_eq!(self.calls, ["write_line"]);
        self.observe_file(file);
        self.calls.push("sync_file");
        if self.fault == Some(Fault::FileSync) {
            return Err(Self::injected_error());
        }
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        assert_eq!(self.calls, ["write_line", "sync_file"]);
        self.calls.push("sync_directory");
        if self.fault == Some(Fault::DirectorySync) {
            return Err(Self::injected_error());
        }
        if self.rename.is_some() {
            let mut witness = parent.sibling(OsStr::new("sync-witness"))?.open_append()?;
            witness.write_all(WITNESS)?;
            witness.sync_data()?;
        }
        parent.sync_directory()
    }
}

// Retry/reopen assertions cover currently readable bytes, not power-loss recovery.
fn assert_failed_consume(fault: Fault, expected_calls: &[&str]) {
    let fixture = Fixture::approved_once();
    let mut io = ObservedIo::new(&fixture, Some(fault));
    let error = fixture
        .store
        .consume_with_io(&fixture.request, &mut io)
        .unwrap_err();
    assert!(matches!(error, ApprovalError::Io(ref error) if error.kind() == io::ErrorKind::Other));
    assert_eq!(io.calls.as_slice(), expected_calls);
    assert_eq!(fixture.cached_status(), ApprovalStatus::Approved);

    let after = fs::read(&fixture.path).unwrap();
    let mut expected = fixture.before.clone();
    expected.extend_from_slice(&io.encoded[..io.written]);
    assert_eq!(after, expected);
    assert_eq!(
        identity(&fs::metadata(&fixture.path).unwrap()),
        fixture.identity
    );

    if fault == Fault::PartialWrite {
        assert!(!after.ends_with(b"\n"));
        assert!(matches!(
            fixture.store.read_all(),
            Err(ApprovalError::InvalidJson { line: 3 })
        ));
        assert!(fixture.store.consume(&fixture.request).is_err());
        assert_eq!(fs::read(&fixture.path).unwrap(), after);
        let reopened = ApprovalStore::open(&fixture.path);
        assert!(reopened.read_all().is_err());
        assert!(reopened.consume(&fixture.request).is_err());
    } else {
        let records: Vec<ApprovalRecord> = after
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[2].status, ApprovalStatus::Consumed);
        assert_eq!(records[2].request, fixture.request);
        assert!(!fixture.store.consume(&fixture.request).unwrap());
        assert_eq!(fixture.cached_status(), ApprovalStatus::Consumed);
        assert_eq!(fs::read(&fixture.path).unwrap(), after);
        let reopened = ApprovalStore::open(&fixture.path);
        assert!(!reopened.consume(&fixture.request).unwrap());
        assert_eq!(reopened.read_all().unwrap(), vec![records[2].clone()]);
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), after);
}

#[test]
fn partial_consumed_write_stops_sync_and_cannot_authorize_from_a_bad_tail() {
    assert_failed_consume(Fault::PartialWrite, &["write_line"]);
}

#[test]
fn complete_consumed_write_then_error_is_not_reauthorized_by_retry_or_reopen() {
    assert_failed_consume(Fault::CompleteWrite, &["write_line"]);
}

#[test]
fn consumed_file_sync_failure_stops_parent_sync_and_is_not_reauthorized() {
    assert_failed_consume(Fault::FileSync, &["write_line", "sync_file"]);
}

#[test]
fn consumed_parent_sync_failure_is_not_reauthorized_by_retry_or_reopen() {
    assert_failed_consume(
        Fault::DirectorySync,
        &["write_line", "sync_file", "sync_directory"],
    );
}

#[test]
fn successful_consume_syncs_actual_inode_and_original_pinned_parent_after_rename() {
    let fixture = Fixture::approved_once();
    let directory_identity = identity(&fs::metadata(&fixture.directory).unwrap());
    let mut io = ObservedIo::new(&fixture, None);
    io.rename = Some((fixture.directory.clone(), fixture.moved_directory.clone()));

    assert!(fixture
        .store
        .consume_with_io(&fixture.request, &mut io)
        .unwrap());
    assert_eq!(io.calls, ["write_line", "sync_file", "sync_directory"]);
    assert_eq!(fixture.cached_status(), ApprovalStatus::Consumed);
    assert_eq!(
        identity(&fs::metadata(&fixture.moved_directory).unwrap()),
        directory_identity
    );
    let moved_data = fixture.moved_directory.join("approvals.jsonl");
    assert_eq!(
        identity(&fs::metadata(&moved_data).unwrap()),
        fixture.identity
    );
    let mut expected = fixture.before.clone();
    expected.extend_from_slice(&io.encoded);
    assert_eq!(fs::read(&moved_data).unwrap(), expected);
    assert_eq!(fs::read(&fixture.path).unwrap(), REPLACEMENT_DATA);
    assert_eq!(
        fs::read(fixture.directory.join("approvals.jsonl.lock")).unwrap(),
        REPLACEMENT_LOCK
    );
    assert_eq!(
        fs::read(fixture.moved_directory.join("sync-witness")).unwrap(),
        WITNESS
    );
    assert!(!fixture.directory.join("sync-witness").exists());
}
