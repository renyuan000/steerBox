use crate::redaction::Redactor;
use crate::tool_registry::{SideEffectClass, SideEffectDisposition};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditStoreError {
    #[error("audit store I/O failed: {message}")]
    Io { message: String },
    #[error("audit record JSON is invalid at line {line}: {message}")]
    InvalidJson { line: usize, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRecord {
    pub trace_id: String,
    pub goal_id: Option<String>,
    pub loop_run_id: Option<String>,
    pub step_id: Option<String>,
    pub kind: String,
    pub summary: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointMetadata {
    pub checkpoint_id: String,
    pub goal_id: String,
    pub loop_run_id: String,
    pub step_id: Option<String>,
    pub files: Vec<String>,
    pub diff_boundary: String,
    pub recoverability: String,
    pub side_effect_refs: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SideEffectRecord {
    pub side_effect_id: String,
    pub goal_id: String,
    pub loop_run_id: String,
    pub tool_id: String,
    pub class: String,
    pub status: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffectAuditStatus {
    Planned,
    Started,
    Completed,
    Failed,
    Blocked,
    Unknown,
}

impl std::fmt::Display for SideEffectAuditStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Planned => "planned",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        };
        formatter.write_str(label)
    }
}

impl SideEffectRecord {
    pub fn disposition(&self) -> SideEffectDisposition {
        SideEffectClass::from_audit_label(&self.class).disposition()
    }

    pub fn audit_status(&self) -> SideEffectAuditStatus {
        match self.status.trim().to_ascii_lowercase().as_str() {
            "planned" => SideEffectAuditStatus::Planned,
            "started" => SideEffectAuditStatus::Started,
            "completed" => SideEffectAuditStatus::Completed,
            "failed" | "verification_failed" => SideEffectAuditStatus::Failed,
            "blocked" => SideEffectAuditStatus::Blocked,
            _ => SideEffectAuditStatus::Unknown,
        }
    }

    pub fn automatic_execution_allowed(&self) -> bool {
        SideEffectClass::from_audit_label(&self.class).automatic_execution_allowed()
            && !matches!(self.audit_status(), SideEffectAuditStatus::Unknown)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunSummary {
    pub schema_version: u32,
    pub project_id: String,
    pub goal_id: String,
    pub loop_run_id: String,
    pub task_run_id: String,
    pub final_status: String,
    pub step_ids: Vec<String>,
    pub file_changes: Vec<String>,
    pub verification_status: String,
    pub event_ids: Vec<String>,
    pub verification_event_ids: Vec<String>,
    pub trace_ids: Vec<String>,
    pub checkpoint_ids: Vec<String>,
    pub side_effect_ids: Vec<String>,
    pub event_count: usize,
    pub generated_at: String,
}

impl RunSummary {
    pub fn write_json(&self, path: impl AsRef<Path>) -> Result<(), AuditStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let encoded =
            serde_json::to_vec_pretty(self).map_err(|error| AuditStoreError::InvalidJson {
                line: 0,
                message: error.to_string(),
            })?;
        fs::write(path, encoded).map_err(io_error)
    }

    pub fn read_json(path: impl AsRef<Path>) -> Result<Self, AuditStoreError> {
        let contents = fs::read_to_string(path).map_err(io_error)?;
        serde_json::from_str(&contents).map_err(|error| AuditStoreError::InvalidJson {
            line: 0,
            message: error.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TraceStore {
    path: std::path::PathBuf,
}

impl TraceStore {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn append(&self, record: &TraceRecord) -> Result<(), AuditStoreError> {
        let mut redacted = record.clone();
        redacted.summary = Redactor::default().redact_text_fail_closed(&record.summary);
        append_jsonl(&self.path, &redacted)
    }
    pub fn read_all(&self) -> Result<Vec<TraceRecord>, AuditStoreError> {
        read_jsonl(&self.path)
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointStore {
    path: std::path::PathBuf,
}

impl CheckpointStore {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn append(&self, record: &CheckpointMetadata) -> Result<(), AuditStoreError> {
        append_jsonl(&self.path, record)
    }
    pub fn read_all(&self) -> Result<Vec<CheckpointMetadata>, AuditStoreError> {
        read_jsonl(&self.path)
    }
}

#[derive(Debug, Clone)]
pub struct SideEffectLedger {
    path: std::path::PathBuf,
}

impl SideEffectLedger {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn append(&self, record: &SideEffectRecord) -> Result<(), AuditStoreError> {
        let mut redacted = record.clone();
        redacted.summary = Redactor::default().redact_text_fail_closed(&record.summary);
        append_jsonl(&self.path, &redacted)
    }
    pub fn read_all(&self) -> Result<Vec<SideEffectRecord>, AuditStoreError> {
        read_jsonl(&self.path)
    }
}

pub fn append_jsonl<T: Serialize>(
    path: impl AsRef<Path>,
    record: &T,
) -> Result<(), AuditStoreError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let encoded = serde_json::to_string(record).map_err(|error| AuditStoreError::InvalidJson {
        line: 0,
        message: error.to_string(),
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    writeln!(file, "{encoded}").map_err(io_error)
}

pub fn read_jsonl<T: for<'de> Deserialize<'de>>(
    path: impl AsRef<Path>,
) -> Result<Vec<T>, AuditStoreError> {
    if !path.as_ref().exists() {
        return Ok(Vec::new());
    }
    let reader = BufReader::new(File::open(path).map_err(io_error)?);
    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => {
                Some(
                    serde_json::from_str(&line).map_err(|error| AuditStoreError::InvalidJson {
                        line: index + 1,
                        message: error.to_string(),
                    }),
                )
            }
            Err(error) => Some(Err(io_error(error))),
        })
        .collect()
}

fn io_error(error: std::io::Error) -> AuditStoreError {
    AuditStoreError::Io {
        message: error.to_string(),
    }
}
