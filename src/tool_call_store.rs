use crate::audit_store::AuditParent;
use crate::domain::ToolCallRecord;
use crate::redaction::Redactor;
use serde_json::Error as JsonError;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub const MAX_TOOL_CALL_PAGE_SIZE: usize = 1024;

#[derive(Debug, Error)]
pub enum ToolCallStoreError {
    #[error("tool call store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("tool call record line {line} is invalid JSON: {source}")]
    InvalidJson { line: usize, source: JsonError },
    #[error("tool call record serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("tool call page limit {requested} exceeds maximum {maximum}")]
    PageLimitExceeded { requested: usize, maximum: usize },
    #[error("tool call store path is not a single-linked regular non-symlink file")]
    UnsafePath,
    #[error("tool call store path must be owner-only")]
    UnsafePermissions,
}

/// Metadata-only JSONL persistence for tool-call audit records.
///
/// This store persists only the fixed `ToolCallRecord` schema. Raw tool output,
/// arguments, and original error bodies are not accepted by this API.
#[derive(Debug, Clone)]
pub struct ToolCallRecordStore {
    path: PathBuf,
}

impl ToolCallRecordStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ToolCallStoreError> {
        let path = path.as_ref().to_path_buf();
        AuditParent::open(&path, true)?
            .ok_or_else(|| tool_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        // Preserve immediate metadata-only diagnostics, including for 0000
        // files. This path precheck is not the authority for subsequent I/O:
        // each operation below validates its actual opened descriptor again.
        validate_path(&path)?;
        Ok(Self { path })
    }

    pub fn append(&self, record: &ToolCallRecord) -> Result<(), ToolCallStoreError> {
        self.append_with_io(record, &mut ToolStoreDisk)
    }

    fn append_with_io(
        &self,
        record: &ToolCallRecord,
        io: &mut impl ToolStoreIo,
    ) -> Result<(), ToolCallStoreError> {
        validate_path(&self.path)?;
        let mut redacted = record.clone();
        redacted.summary = Redactor::default().redact_text_fail_closed(&record.summary);
        let mut encoded = serde_json::to_vec(&redacted)?;
        encoded.push(b'\n');
        let (parent, _lock) = self.acquire_lock()?;
        io.after_lock()
            .map_err(|error| tool_io_error("locked operation", error))?;
        let mut file = parent.open_append().map_err(leaf_error)?;
        validate_opened_file(&file)?;
        io.write(&mut file, &encoded)
            .map_err(|error| tool_io_error("append write", error))?;
        io.sync_file(&file)
            .map_err(|error| tool_io_error("append file sync", error))?;
        io.sync_directory(&parent)
            .map_err(|error| tool_io_error("append directory sync", error))?;
        Ok(())
    }

    /// Read a bounded page of non-blank JSONL records without loading the full log.
    pub fn read_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ToolCallRecord>, ToolCallStoreError> {
        if limit > MAX_TOOL_CALL_PAGE_SIZE {
            return Err(ToolCallStoreError::PageLimitExceeded {
                requested: limit,
                maximum: MAX_TOOL_CALL_PAGE_SIZE,
            });
        }
        let (parent, _lock) = self.acquire_lock()?;
        let mut records = Vec::new();
        if limit == 0 {
            return Ok(records);
        }

        let Some(file) = parent.open_read().map_err(leaf_error)? else {
            return Ok(records);
        };
        validate_opened_file(&file)?;
        let reader = BufReader::new(file);
        let mut record_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line.map_err(|error| tool_io_error("history read", error))?;
            if line.trim().is_empty() {
                continue;
            }

            let record = serde_json::from_str::<ToolCallRecord>(&line).map_err(|source| {
                ToolCallStoreError::InvalidJson {
                    line: line_index + 1,
                    source,
                }
            })?;
            if record_index >= offset {
                records.push(record);
                if records.len() >= limit {
                    break;
                }
            }
            record_index += 1;
        }
        Ok(records)
    }

    fn acquire_lock(&self) -> Result<(AuditParent, File), ToolCallStoreError> {
        let parent = AuditParent::open(&self.path, false)?
            .ok_or_else(|| tool_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        let mut lock_name = self
            .path
            .file_name()
            .ok_or_else(|| tool_io_error("path validation", io::ErrorKind::InvalidInput.into()))?
            .to_os_string();
        lock_name.push(".lock");
        let lock_parent = parent.sibling(&lock_name)?;
        let lock = lock_parent.open_lock().map_err(leaf_error)?;
        validate_opened_file(&lock)?;
        lock.lock()
            .map_err(|error| tool_io_error("lock acquire", error))?;
        Ok((parent, lock))
    }
}

fn validate_path(path: &Path) -> Result<(), ToolCallStoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(tool_io_error("path metadata", error).into()),
    };
    validate_metadata(&metadata)
}

fn validate_metadata(metadata: &Metadata) -> Result<(), ToolCallStoreError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ToolCallStoreError::UnsafePath);
    }
    #[cfg(unix)]
    {
        if metadata.nlink() != 1 {
            return Err(ToolCallStoreError::UnsafePath);
        }
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ToolCallStoreError::UnsafePermissions);
        }
    }
    Ok(())
}

fn validate_opened_file(file: &File) -> Result<(), ToolCallStoreError> {
    let metadata = file
        .metadata()
        .map_err(|error| tool_io_error("file metadata", error))?;
    validate_metadata(&metadata)
}

// Only leaf opens map shape/nlink refusals to the store's policy error.
// Path parsing, OS permissions and sync failures keep their I/O classification.
fn leaf_error(error: io::Error) -> ToolCallStoreError {
    let unsafe_path = matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::IsADirectory
    );
    #[cfg(unix)]
    let unsafe_path = {
        // The shared wrapper preserves ErrorKind but intentionally drops errno.
        let loop_kind = io::Error::from_raw_os_error(libc::ELOOP).kind();
        unsafe_path || (loop_kind != io::ErrorKind::Other && error.kind() == loop_kind)
    };
    if unsafe_path {
        ToolCallStoreError::UnsafePath
    } else {
        ToolCallStoreError::Io(error)
    }
}

fn tool_io_error(stage: &str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("tool call store {stage} failed: {:?}", error.kind()),
    )
}

trait ToolStoreIo {
    fn after_lock(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()>;
    fn sync_file(&mut self, file: &File) -> io::Result<()>;
    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()>;
}

struct ToolStoreDisk;

impl ToolStoreIo for ToolStoreDisk {
    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()> {
        file.write_all(encoded)
    }
    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        file.sync_data()
    }
    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        parent.sync_directory()
    }
}

#[cfg(all(test, unix))]
#[path = "tool_call_store/tests.rs"]
mod tests;
