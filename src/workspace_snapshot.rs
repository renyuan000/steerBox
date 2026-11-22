//! Local, private, single-file snapshots. Not a controller recovery mechanism.
//!
//! The expected current content hash must come from a trusted caller, not a
//! model action. Authorization is enforced by the controller before it calls
//! this storage primitive; this module does not accept approval flags or
//! decide policy. Permissions do not isolate the same OS user/root; this API
//! does not provide a concurrent writer lock or a crash transaction.

use crate::audit_store::AuditParent;
use crate::domain::GoalContract;
use crate::workspace_edit::{checked_existing_file, edit_markdown, safe_relative_path, EditResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;
// A UTF-8 body may encode each byte as six JSON bytes. Metadata is bounded too.
pub const MAX_RECORD_BYTES: usize = 6 * MAX_SNAPSHOT_BYTES + 64 * 1024;
const MAX_IDENTITY_BYTES: usize = 1024;
const MAX_PATH_BYTES: usize = 4096;

// Narrow failure/operation seams; production always uses the real file I/O.
trait SnapshotIo {
    fn root_opened(&mut self) {}

    fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
        file.write_all(bytes)
    }

    fn sync_file(&mut self, file: &File) -> std::io::Result<()> {
        file.sync_all()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> std::io::Result<()> {
        parent.sync_directory()
    }
}

struct RealSnapshotIo;
impl SnapshotIo for RealSnapshotIo {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SnapshotMetadata {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub project_id: String,
    pub goal_id: String,
    pub relative_path: String,
    /// SHA256(raw file bytes), unlike the controller's path-bound hashes.
    pub before_hash: String,
    pub bytes_before: usize,
}

// Intentionally no Debug: the body and workspace binding are private on disk.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRecord {
    metadata: SnapshotMetadata,
    workspace_binding: PathBuf,
    contents: String,
}

pub(crate) struct PreparedSnapshotRestore {
    workspace: PathBuf,
    relative_path: String,
    current: String,
    replacement: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotError {
    #[error("snapshot storage requires Unix private filesystem permissions")]
    UnsupportedPlatform,
    #[error("invalid snapshot identity")]
    InvalidIdentity,
    #[error("invalid or unauthorized Markdown path")]
    InvalidPath,
    #[error("snapshot path crosses a symbolic link or is not absolute")]
    UnsafePath,
    #[error("snapshot store must be outside the workspace")]
    StoreWithinWorkspace,
    #[error("snapshot store or record is not private to the effective user")]
    NotPrivate,
    #[error("snapshot source or record is not a single-link regular file")]
    NotRegularFile,
    #[error("snapshot input exceeds its byte limit")]
    TooLarge,
    #[error("snapshot source or current file is not UTF-8")]
    InvalidUtf8,
    #[error("snapshot record is invalid or incomplete")]
    InvalidRecord,
    #[error("snapshot does not belong to this workspace, project and goal")]
    BindingMismatch,
    #[error("current content hash does not match the expected version")]
    CurrentHashMismatch,
    #[error("invalid expected current content hash")]
    InvalidHash,
    #[error("conditional workspace replacement failed; inspect current file before retrying")]
    ReplacementFailed,
    #[error("snapshot filesystem operation failed ({kind:?})")]
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone)]
pub struct WorkspaceSnapshotStore {
    root: PathBuf,
}

impl WorkspaceSnapshotStore {
    /// Does not access disk. Capture/restore validate the root on each call.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn capture(
        &self,
        workspace: &Path,
        goal: &GoalContract,
        relative_path: &str,
        snapshot_id: &str,
    ) -> Result<SnapshotMetadata, SnapshotError> {
        self.capture_with_io(
            workspace,
            goal,
            relative_path,
            snapshot_id,
            &mut RealSnapshotIo,
        )
    }

    fn capture_with_io(
        &self,
        workspace: &Path,
        goal: &GoalContract,
        relative_path: &str,
        snapshot_id: &str,
        io: &mut impl SnapshotIo,
    ) -> Result<SnapshotMetadata, SnapshotError> {
        supported_platform()?;
        validate_id(snapshot_id)?;
        validate_goal_identity(goal)?;
        let binding = checked_workspace(workspace)?;
        let relative = scoped_path(goal, relative_path)?;
        let target =
            checked_existing_file(&binding, &relative).map_err(|_| SnapshotError::InvalidPath)?;
        let root = self.checked_root(&binding, snapshot_id, true)?;
        io.root_opened();
        let contents = read_text(&target, MAX_SNAPSHOT_BYTES, false)?;
        let metadata = SnapshotMetadata {
            schema_version: 1,
            snapshot_id: snapshot_id.into(),
            project_id: goal.project_id.clone(),
            goal_id: goal.goal_id.clone(),
            relative_path: relative,
            before_hash: content_hash(contents.as_bytes()),
            bytes_before: contents.len(),
        };
        let record = SnapshotRecord {
            metadata: metadata.clone(),
            workspace_binding: binding,
            contents,
        };
        let encoded = serde_json::to_vec(&record).map_err(|_| SnapshotError::InvalidRecord)?;
        if encoded.len() > MAX_RECORD_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        // Retain ownership even if subsequent FD validation fails, so failure
        // cleanup can unlink this call's entry in the same pinned directory.
        let mut file = root.open_create_new_candidate().map_err(io_error)?;
        let persisted = (|| {
            check_regular(&file.metadata().map_err(io_error)?)?;
            check_private(&file.metadata().map_err(io_error)?)?;
            io.write(&mut file, &encoded).map_err(io_error)?;
            io.sync_file(&file).map_err(io_error)?;
            io.sync_directory(&root).map_err(io_error)
        })();
        if persisted.is_err() {
            // Best effort through the pinned directory, not a pathname. This
            // does not fence entry replacement by another same-UID writer.
            drop(file);
            let _ = root.remove_file();
            let _ = root.sync_directory();
        }
        persisted?;
        Ok(metadata)
    }

    pub fn restore(
        &self,
        workspace: &Path,
        goal: &GoalContract,
        snapshot_id: &str,
        expected_after_hash: &str,
    ) -> Result<EditResult, SnapshotError> {
        let prepared = self.prepare_restore(workspace, goal, snapshot_id, expected_after_hash)?;
        self.restore_prepared(goal, prepared)
    }

    pub(crate) fn prepare_restore(
        &self,
        workspace: &Path,
        goal: &GoalContract,
        snapshot_id: &str,
        expected_after_hash: &str,
    ) -> Result<PreparedSnapshotRestore, SnapshotError> {
        validate_content_hash(expected_after_hash)?;
        supported_platform()?;
        validate_id(snapshot_id)?;
        validate_goal_identity(goal)?;
        let binding = checked_workspace(workspace)?;
        let root = self.checked_root(&binding, snapshot_id, false)?;
        // Keep the existing static-symlink error category; the following open
        // still uses the pinned root, never this prechecked pathname.
        check_no_symlinks(&self.root.join(format!("{snapshot_id}.json")))?;
        let encoded = read_parent(&root, MAX_RECORD_BYTES, true)?;
        let record: SnapshotRecord =
            serde_json::from_slice(&encoded).map_err(|_| SnapshotError::InvalidRecord)?;
        let metadata = &record.metadata;
        if metadata.schema_version != 1
            || metadata.snapshot_id != snapshot_id
            || metadata.bytes_before != record.contents.len()
            || record.contents.len() > MAX_SNAPSHOT_BYTES
            || metadata.before_hash != content_hash(record.contents.as_bytes())
        {
            return Err(SnapshotError::InvalidRecord);
        }
        if metadata.project_id != goal.project_id
            || metadata.goal_id != goal.goal_id
            || record.workspace_binding != binding
        {
            return Err(SnapshotError::BindingMismatch);
        }
        let relative = scoped_path(goal, &metadata.relative_path)?;
        if relative != metadata.relative_path {
            return Err(SnapshotError::InvalidRecord);
        }
        let target =
            checked_existing_file(&binding, &relative).map_err(|_| SnapshotError::InvalidPath)?;
        let current = read_text(&target, MAX_SNAPSHOT_BYTES, false)?;
        if content_hash(current.as_bytes()) != expected_after_hash {
            return Err(SnapshotError::CurrentHashMismatch);
        }
        Ok(PreparedSnapshotRestore {
            workspace: binding,
            relative_path: relative,
            current,
            replacement: record.contents,
        })
    }

    pub(crate) fn restore_prepared(
        &self,
        goal: &GoalContract,
        prepared: PreparedSnapshotRestore,
    ) -> Result<EditResult, SnapshotError> {
        // The editor rechecks exact content before replacement. This is not a
        // CAS against adversarial concurrent writers, nor a rollback transaction.
        edit_markdown(
            &prepared.workspace,
            goal,
            &prepared.relative_path,
            &prepared.current,
            &prepared.replacement,
            true,
        )
        .map_err(|_| SnapshotError::ReplacementFailed)
    }

    fn checked_root(
        &self,
        workspace: &Path,
        snapshot_id: &str,
        create: bool,
    ) -> Result<AuditParent, SnapshotError> {
        check_absolute_components(&self.root)?;
        let parent = self.root.parent().ok_or(SnapshotError::UnsafePath)?;
        check_no_symlinks(parent)?;
        let parent = fs::canonicalize(parent).map_err(io_error)?;
        let resolved = parent.join(self.root.file_name().ok_or(SnapshotError::UnsafePath)?);
        if resolved.starts_with(workspace) {
            return Err(SnapshotError::StoreWithinWorkspace);
        }
        // Preserve static-path error categories, but these prechecks are not
        // the authority: all record operations use the opened directory FD.
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(SnapshotError::UnsafePath),
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let containing = AuditParent::open(&resolved, false)
            .map_err(io_error)?
            .ok_or_else(missing_file)?;
        let root = containing
            .enter_directory(OsStr::new(&format!("{snapshot_id}.json")), create)
            .map_err(io_error)?
            .ok_or_else(missing_file)?;
        check_private(&root.directory_metadata().map_err(io_error)?)?;
        Ok(root)
    }
}

fn validate_id(id: &str) -> Result<(), SnapshotError> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(SnapshotError::InvalidIdentity);
    }
    Ok(())
}

fn validate_goal_identity(goal: &GoalContract) -> Result<(), SnapshotError> {
    for id in [&goal.project_id, &goal.goal_id] {
        if id.is_empty() || id.len() > MAX_IDENTITY_BYTES || id.chars().any(char::is_control) {
            return Err(SnapshotError::InvalidIdentity);
        }
    }
    Ok(())
}

fn scoped_path(goal: &GoalContract, path: &str) -> Result<String, SnapshotError> {
    if path.len() > MAX_PATH_BYTES || path.chars().any(char::is_control) {
        return Err(SnapshotError::InvalidPath);
    }
    let relative = safe_relative_path(path).ok_or(SnapshotError::InvalidPath)?;
    if !goal.is_within_scope(&relative)
        || (!relative.ends_with(".md") && !relative.ends_with(".markdown"))
    {
        return Err(SnapshotError::InvalidPath);
    }
    Ok(relative)
}

fn checked_workspace(workspace: &Path) -> Result<PathBuf, SnapshotError> {
    let absolute = if workspace.is_absolute() {
        workspace.to_owned()
    } else {
        std::env::current_dir().map_err(io_error)?.join(workspace)
    };
    check_no_symlinks(&absolute)?;
    if !fs::metadata(&absolute).map_err(io_error)?.is_dir() {
        return Err(SnapshotError::UnsafePath);
    }
    let canonical = fs::canonicalize(absolute).map_err(io_error)?;
    if canonical.as_os_str().len() > MAX_PATH_BYTES {
        return Err(SnapshotError::UnsafePath);
    }
    Ok(canonical)
}

fn check_absolute_components(path: &Path) -> Result<(), SnapshotError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err(SnapshotError::UnsafePath);
    }
    Ok(())
}

fn check_no_symlinks(path: &Path) -> Result<(), SnapshotError> {
    check_absolute_components(path)?;
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(SnapshotError::UnsafePath);
        }
    }
    Ok(())
}

fn read_bytes(path: &Path, limit: usize, private: bool) -> Result<Vec<u8>, SnapshotError> {
    read_bytes_with_hook(path, limit, private, || {})
}

fn read_bytes_with_hook(
    path: &Path,
    limit: usize,
    private: bool,
    after_parent: impl FnOnce(),
) -> Result<Vec<u8>, SnapshotError> {
    check_no_symlinks(path)?;
    let parent = AuditParent::open(path, false)
        .map_err(io_error)?
        .ok_or_else(missing_file)?;
    after_parent();
    read_parent(&parent, limit, private)
}

fn read_parent(
    parent: &AuditParent,
    limit: usize,
    private: bool,
) -> Result<Vec<u8>, SnapshotError> {
    let file = parent
        .open_read_candidate()
        .map_err(io_error)?
        .ok_or_else(missing_file)?;
    let metadata = file.metadata().map_err(io_error)?;
    check_regular(&metadata)?;
    if private {
        check_private(&metadata)?;
    }
    if metadata.len() > limit as u64 {
        return Err(SnapshotError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(SnapshotError::TooLarge);
    }
    Ok(bytes)
}

fn read_text(path: &Path, limit: usize, private: bool) -> Result<String, SnapshotError> {
    String::from_utf8(read_bytes(path, limit, private)?).map_err(|_| SnapshotError::InvalidUtf8)
}

fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn validate_content_hash(value: &str) -> Result<(), SnapshotError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(SnapshotError::InvalidHash);
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SnapshotError::InvalidHash);
    }
    Ok(())
}

#[cfg(unix)]
fn supported_platform() -> Result<(), SnapshotError> {
    Ok(())
}
#[cfg(not(unix))]
fn supported_platform() -> Result<(), SnapshotError> {
    Err(SnapshotError::UnsupportedPlatform)
}

fn check_regular(metadata: &fs::Metadata) -> Result<(), SnapshotError> {
    if !metadata.is_file() {
        return Err(SnapshotError::NotRegularFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(SnapshotError::NotRegularFile);
        }
    }
    Ok(())
}

fn check_private(metadata: &fs::Metadata) -> Result<(), SnapshotError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // geteuid has no arguments and cannot fail.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7077 != 0 {
            return Err(SnapshotError::NotPrivate);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err(SnapshotError::UnsupportedPlatform)
    }
}

fn missing_file() -> SnapshotError {
    SnapshotError::Io {
        kind: std::io::ErrorKind::NotFound,
    }
}

fn io_error(error: std::io::Error) -> SnapshotError {
    SnapshotError::Io { kind: error.kind() }
}

#[cfg(all(test, unix))]
mod tests;
