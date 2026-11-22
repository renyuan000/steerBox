#![cfg(unix)]

use super::*;
use crate::audit_store::AuditParent;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}

fn identity(metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileObservation {
    descriptor: i32,
    identity: FileIdentity,
}

fn observe(file: &File) -> io::Result<FileObservation> {
    let metadata = file.metadata()?;
    assert!(metadata.is_file());
    assert_eq!(metadata.nlink(), 1);
    Ok(FileObservation {
        descriptor: file.as_raw_fd(),
        identity: identity(&metadata),
    })
}

fn temporary_for(directory: &Path, file: &File) -> io::Result<PathBuf> {
    let expected = identity(&file.metadata()?);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if identity(&entry.metadata()?) == expected {
            return Ok(entry.path());
        }
    }
    Err(io::Error::other("synthetic temporary file was not found"))
}

fn projection() -> StateStore {
    let mut state = StateStore::default();
    state.loops.insert(
        "synthetic-loop".into(),
        LoopState {
            loop_run_id: "synthetic-loop".into(),
            goal_id: "synthetic-goal".into(),
            task_run_id: "synthetic-task-run".into(),
            status: LoopStatus::Running,
            current_step_id: None,
            started_at: "2026-11-22T00:00:00Z".into(),
            finished_at: None,
            last_sequence_no: Some(1),
        },
    );
    state
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    PartialWrite,
    CompleteWrite,
    FileSync,
    Publish,
    DirectorySync,
}

struct FaultIo {
    failure: Failure,
    directory: PathBuf,
    stages: Vec<&'static str>,
    write_observation: Option<FileObservation>,
    sync_observation: Option<FileObservation>,
    written_bytes: Vec<u8>,
}

impl StateIo for FaultIo {
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        self.stages.push("write");
        self.write_observation = Some(observe(file)?);
        let written = if self.failure == Failure::PartialWrite {
            &bytes[..bytes.len() / 2]
        } else {
            bytes
        };
        file.write_all(written)?;
        self.written_bytes = fs::read(temporary_for(&self.directory, file)?)?;
        assert_eq!(self.written_bytes, written);
        if matches!(self.failure, Failure::PartialWrite | Failure::CompleteWrite) {
            return Err(io::Error::other("injected synthetic write failure"));
        }
        Ok(())
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        self.stages.push("file-sync");
        self.sync_observation = Some(observe(file)?);
        assert_eq!(self.sync_observation, self.write_observation);
        if self.failure == Failure::FileSync {
            return Err(io::Error::other("injected synthetic file-sync failure"));
        }
        file.sync_all()
    }

    fn publish(&mut self, parent: &AuditParent, temp_name: &OsStr) -> io::Result<()> {
        self.stages.push("publish");
        if self.failure == Failure::Publish {
            return Err(io::Error::other("injected synthetic publish failure"));
        }
        parent.rename_from(temp_name)
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        self.stages.push("directory-sync");
        if self.failure == Failure::DirectorySync {
            return Err(io::Error::other(
                "injected synthetic directory-sync failure",
            ));
        }
        parent.sync_directory()
    }
}

fn assert_failure_boundary(failure: Failure, expected_stages: &[&str]) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let old_state = StateStore::default();
    let old_bytes = serde_json::to_vec_pretty(&old_state).unwrap();
    fs::write(&path, &old_bytes).unwrap();
    let old_identity = identity(&fs::metadata(&path).unwrap());
    let state = projection();
    let unchanged_state = state.clone();
    let expected_bytes = serde_json::to_vec_pretty(&state).unwrap();
    let mut io = FaultIo {
        failure,
        directory: directory.path().to_path_buf(),
        stages: Vec::new(),
        write_observation: None,
        sync_observation: None,
        written_bytes: Vec::new(),
    };

    let result = state.persist_with_io(&path, &mut io);

    let expected_message = match failure {
        Failure::PartialWrite | Failure::CompleteWrite => "temporary write failed: Other",
        Failure::FileSync => "temporary sync failed: Other",
        Failure::Publish => "state rename failed: Other",
        Failure::DirectorySync => "parent sync failed: Other",
    };
    assert_eq!(
        result,
        Err(StateStoreError::Io {
            message: expected_message.into(),
        })
    );
    assert_eq!(io.stages, expected_stages);
    assert_eq!(state, unchanged_state);
    let expected_written = if failure == Failure::PartialWrite {
        &expected_bytes[..expected_bytes.len() / 2]
    } else {
        expected_bytes.as_slice()
    };
    assert_eq!(io.written_bytes, expected_written);
    let temporary_identity = io.write_observation.unwrap().identity;
    assert_ne!(temporary_identity.inode, old_identity.inode);
    if failure == Failure::DirectorySync {
        // Readable new bytes after an unacknowledged publish are not rollback
        // evidence, and this check does not establish OS power-loss recovery.
        assert_eq!(fs::read(&path).unwrap(), expected_bytes);
        assert_eq!(identity(&fs::metadata(&path).unwrap()), temporary_identity);
        assert_eq!(StateStore::load(&path).unwrap(), state);
    } else {
        assert_eq!(fs::read(&path).unwrap(), old_bytes);
        assert_eq!(identity(&fs::metadata(&path).unwrap()), old_identity);
        assert_eq!(StateStore::load(&path).unwrap(), old_state);
    }
    // Cleanup is best-effort in production; these ordinary synthetic entries
    // have no injected cleanup failure and must not leave a temporary behind.
    let entries: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, vec![OsString::from("state.json")]);
}

#[test]
fn partial_write_error_preserves_old_projection_and_cleans_temporary() {
    assert_failure_boundary(Failure::PartialWrite, &["write"]);
}

#[test]
fn complete_write_error_does_not_publish_or_sync_new_projection() {
    assert_failure_boundary(Failure::CompleteWrite, &["write"]);
}

#[test]
fn temporary_file_sync_error_preserves_old_projection_and_stops_publish() {
    assert_failure_boundary(Failure::FileSync, &["write", "file-sync"]);
}

#[test]
fn publish_error_preserves_old_projection_after_real_temporary_sync() {
    assert_failure_boundary(Failure::Publish, &["write", "file-sync", "publish"]);
}

#[test]
fn directory_sync_error_returns_error_with_new_projection_already_readable() {
    assert_failure_boundary(
        Failure::DirectorySync,
        &["write", "file-sync", "publish", "directory-sync"],
    );
}

struct RenameParentIo {
    original: PathBuf,
    moved: PathBuf,
    stages: Vec<&'static str>,
    write_observation: Option<FileObservation>,
    sync_observation: Option<FileObservation>,
    temp_name: Option<OsString>,
}

impl StateIo for RenameParentIo {
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        self.stages.push("write");
        self.write_observation = Some(observe(file)?);
        file.write_all(bytes)?;
        let temporary = temporary_for(&self.original, file)?;
        assert_eq!(fs::read(&temporary)?, bytes);
        let temp_name = temporary.file_name().unwrap().to_os_string();
        fs::rename(&self.original, &self.moved)?;
        fs::create_dir(&self.original)?;
        fs::write(
            self.original.join("state.json"),
            b"replacement target decoy",
        )?;
        fs::write(
            self.original.join(&temp_name),
            b"replacement temporary decoy",
        )?;
        self.temp_name = Some(temp_name);
        Ok(())
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        self.stages.push("file-sync");
        self.sync_observation = Some(observe(file)?);
        assert_eq!(self.sync_observation, self.write_observation);
        file.sync_all()
    }

    fn publish(&mut self, parent: &AuditParent, temp_name: &OsStr) -> io::Result<()> {
        self.stages.push("publish");
        assert_eq!(Some(temp_name), self.temp_name.as_deref());
        parent.rename_from(temp_name)
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        self.stages.push("directory-sync");
        let mut witness = parent.sibling(OsStr::new("sync-witness"))?.open_append()?;
        witness.write_all(b"pinned directory witness")?;
        witness.sync_data()?;
        parent.sync_directory()
    }
}

#[test]
fn successful_publish_uses_actual_temporary_fd_and_renamed_pinned_parent() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let moved = directory.path().join("moved");
    fs::create_dir(&original).unwrap();
    let path = original.join("state.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&StateStore::default()).unwrap(),
    )
    .unwrap();
    let old_identity = identity(&fs::metadata(&path).unwrap());
    let state = projection();
    let unchanged_state = state.clone();
    let expected_bytes = serde_json::to_vec_pretty(&state).unwrap();
    assert!(!expected_bytes.ends_with(b"\n"));
    let mut io = RenameParentIo {
        original: original.clone(),
        moved: moved.clone(),
        stages: Vec::new(),
        write_observation: None,
        sync_observation: None,
        temp_name: None,
    };

    state.persist_with_io(&path, &mut io).unwrap();

    assert_eq!(
        io.stages,
        ["write", "file-sync", "publish", "directory-sync"]
    );
    assert_eq!(io.write_observation, io.sync_observation);
    let published_identity = identity(&fs::metadata(moved.join("state.json")).unwrap());
    assert_eq!(published_identity, io.write_observation.unwrap().identity);
    assert_ne!(published_identity.inode, old_identity.inode);
    assert_eq!(fs::read(moved.join("state.json")).unwrap(), expected_bytes);
    assert_eq!(StateStore::load(moved.join("state.json")).unwrap(), state);
    assert_eq!(state, unchanged_state);
    let temp_name = io.temp_name.unwrap();
    assert!(!moved.join(&temp_name).exists());
    assert_eq!(fs::read(&path).unwrap(), b"replacement target decoy");
    assert_eq!(
        fs::read(original.join(&temp_name)).unwrap(),
        b"replacement temporary decoy"
    );
    assert_eq!(
        fs::read(moved.join("sync-witness")).unwrap(),
        b"pinned directory witness"
    );
    assert!(!original.join("sync-witness").exists());
}
