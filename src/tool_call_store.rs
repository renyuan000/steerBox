use crate::domain::ToolCallRecord;
use crate::redaction::Redactor;
use serde_json::Error as JsonError;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

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
    #[error("tool call store path is not a regular non-symlink file")]
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        validate_path(&path)?;
        Ok(Self { path })
    }

    pub fn append(&self, record: &ToolCallRecord) -> Result<(), ToolCallStoreError> {
        validate_path(&self.path)?;
        let mut redacted = record.clone();
        redacted.summary = Redactor::default().redact_text_fail_closed(&record.summary);
        let encoded = serde_json::to_string(&redacted)?;
        let _lock = self.acquire_lock()?;
        let mut file = open_append_file(&self.path)?;
        writeln!(file, "{encoded}")?;
        file.sync_data()?;
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
        let _lock = self.acquire_lock()?;
        let mut records = Vec::new();
        if limit == 0 || !self.path.exists() {
            return Ok(records);
        }

        let reader = BufReader::new(open_read_file(&self.path)?);
        let mut record_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line?;
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

    fn acquire_lock(&self) -> Result<File, ToolCallStoreError> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_file = open_lock_file(&PathBuf::from(lock_path))?;
        lock_file.lock()?;
        Ok(lock_file)
    }
}

fn validate_path(path: &Path) -> Result<(), ToolCallStoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    validate_metadata(&metadata)
}

fn validate_metadata(metadata: &Metadata) -> Result<(), ToolCallStoreError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ToolCallStoreError::UnsafePath);
    }
    #[cfg(unix)]
    {
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ToolCallStoreError::UnsafePermissions);
        }
    }
    Ok(())
}

fn open_append_file(path: &Path) -> Result<File, ToolCallStoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).append(true).create(true);
    configure_no_follow(&mut options, true);
    let file = open_options(&options, path)?;
    validate_metadata(&file.metadata()?)?;
    Ok(file)
}

fn open_read_file(path: &Path) -> Result<File, ToolCallStoreError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options, false);
    let file = open_options(&options, path)?;
    validate_metadata(&file.metadata()?)?;
    Ok(file)
}

fn open_lock_file(path: &Path) -> Result<File, ToolCallStoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    configure_no_follow(&mut options, true);
    let file = open_options(&options, path)?;
    validate_metadata(&file.metadata()?)?;
    Ok(file)
}

fn open_options(options: &OpenOptions, path: &Path) -> Result<File, ToolCallStoreError> {
    match options.open(path) {
        Ok(file) => Ok(file),
        Err(error) if is_symlink_open_error(&error) => Err(ToolCallStoreError::UnsafePath),
        Err(error) => Err(ToolCallStoreError::Io(error)),
    }
}

fn configure_no_follow(options: &mut OpenOptions, create: bool) {
    #[cfg(unix)]
    {
        options.custom_flags(libc::O_NOFOLLOW);
        if create {
            options.mode(0o600);
        }
    }
    let _ = create;
}

fn is_symlink_open_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ELOOP)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}
