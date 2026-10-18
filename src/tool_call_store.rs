use crate::domain::ToolCallRecord;
use crate::redaction::Redactor;
use serde_json::Error as JsonError;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

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
        Ok(Self { path })
    }

    pub fn append(&self, record: &ToolCallRecord) -> Result<(), ToolCallStoreError> {
        let mut redacted = record.clone();
        redacted.summary = Redactor::default().redact_text_fail_closed(&record.summary);
        let encoded = serde_json::to_string(&redacted)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
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
        let mut records = Vec::new();
        if limit == 0 || !self.path.exists() {
            return Ok(records);
        }

        let reader = BufReader::new(File::open(&self.path)?);
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
}
