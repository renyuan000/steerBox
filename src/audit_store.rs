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

/// A recovery recommendation derived only from recorded checkpoint metadata.
/// This does not prove current workspace contents or authorize or perform a restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    NoAction,
    RestoreRequired,
    ManualIntervention,
}

impl std::fmt::Display for RecoveryAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::NoAction => "no_action",
            Self::RestoreRequired => "restore_required",
            Self::ManualIntervention => "manual_intervention",
        };
        formatter.write_str(label)
    }
}

/// A read-only interpretation of a checkpoint's recorded hashes and recovery status.
/// No workspace files are read, rehashed, or restored, and no restore snapshot is validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCheckpoint {
    pub metadata: CheckpointMetadata,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub action: RecoveryAction,
    hash_metadata_consistent: bool,
}

impl RecoveryCheckpoint {
    pub fn from_metadata(metadata: CheckpointMetadata) -> Self {
        let parsed = parse_hash_boundary(&metadata.diff_boundary);
        let hash_metadata_consistent =
            parsed.consistent && (!parsed.explicit_no_workspace_write || metadata.files.is_empty());
        let action = if !hash_metadata_consistent {
            RecoveryAction::ManualIntervention
        } else if parsed.explicit_no_workspace_write || parsed.before_hash == parsed.after_hash {
            RecoveryAction::NoAction
        } else {
            match metadata.recoverability.as_str() {
                "write_failed_local_workspace_restore_or_escalate"
                | "verification_failed_local_workspace_restore_or_escalate" => {
                    RecoveryAction::RestoreRequired
                }
                "verified_local_workspace_restore_or_escalate" => RecoveryAction::NoAction,
                _ => RecoveryAction::ManualIntervention,
            }
        };
        Self {
            metadata,
            before_hash: parsed.before_hash,
            after_hash: parsed.after_hash,
            action,
            hash_metadata_consistent,
        }
    }

    /// Whether the recorded boundary has valid, unambiguous SHA-256 metadata,
    /// or a recognized no-write marker with an empty file list.
    /// This does not compare the recorded hashes with the current workspace.
    pub fn hash_metadata_consistent(&self) -> bool {
        self.hash_metadata_consistent
    }
}

#[derive(Debug, Default)]
struct ParsedHashBoundary {
    before_hash: Option<String>,
    after_hash: Option<String>,
    consistent: bool,
    explicit_no_workspace_write: bool,
}

fn parse_hash_boundary(diff_boundary: &str) -> ParsedHashBoundary {
    if diff_boundary.trim() == "no_workspace_write_fixture_loop"
        || diff_boundary.trim() == "no_workspace_write"
    {
        return ParsedHashBoundary {
            consistent: true,
            explicit_no_workspace_write: true,
            ..ParsedHashBoundary::default()
        };
    }

    let mut before_hash = None;
    let mut after_hash = None;
    let mut malformed = false;
    for component in diff_boundary.split(';') {
        let Some((key, value)) = component.split_once('=') else {
            malformed = true;
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "before" if before_hash.is_none() && is_sha256_hash(value) => {
                before_hash = Some(value.to_owned());
            }
            "after" if after_hash.is_none() && is_sha256_hash(value) => {
                after_hash = Some(value.to_owned());
            }
            _ => malformed = true,
        }
    }
    ParsedHashBoundary {
        consistent: !malformed && before_hash.is_some() && after_hash.is_some(),
        before_hash,
        after_hash,
        explicit_no_workspace_write: false,
    }
}

fn is_sha256_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
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

    /// Interprets the last appended matching checkpoint without reading workspace files.
    /// A missing step filter matches any step, including records without a step ID.
    /// Scans the full JSONL log, retaining only the latest match and propagating any
    /// malformed record, even one outside the requested loop or step.
    pub fn latest_for(
        &self,
        loop_run_id: &str,
        step_id: Option<&str>,
    ) -> Result<Option<RecoveryCheckpoint>, AuditStoreError> {
        let mut latest = None;
        for_each_jsonl(&self.path, |checkpoint: CheckpointMetadata| {
            if checkpoint.loop_run_id == loop_run_id
                && step_id.is_none_or(|expected| checkpoint.step_id.as_deref() == Some(expected))
            {
                latest = Some(checkpoint);
            }
        })?;
        Ok(latest.map(RecoveryCheckpoint::from_metadata))
    }

    pub fn latest_for_loop_step(
        &self,
        loop_run_id: &str,
        step_id: &str,
    ) -> Result<Option<RecoveryCheckpoint>, AuditStoreError> {
        self.latest_for(loop_run_id, Some(step_id))
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
    let mut records = Vec::new();
    for_each_jsonl(path, |record| records.push(record))?;
    Ok(records)
}

fn for_each_jsonl<T: for<'de> Deserialize<'de>>(
    path: impl AsRef<Path>,
    mut visit: impl FnMut(T),
) -> Result<(), AuditStoreError> {
    if !path.as_ref().exists() {
        return Ok(());
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
        .try_for_each(|record| {
            visit(record?);
            Ok(())
        })
}

fn io_error(error: std::io::Error) -> AuditStoreError {
    AuditStoreError::Io {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_records_are_visited_before_a_later_decode_error() {
        let path = std::env::temp_dir().join(format!(
            "steerbox-jsonl-streaming-{}.jsonl",
            std::process::id()
        ));
        fs::write(&path, "1\nnot-json\n").expect("streaming fixture writes");
        let mut visited = Vec::new();

        let error = for_each_jsonl::<u64>(&path, |record| visited.push(record))
            .expect_err("invalid JSON must fail");

        fs::remove_file(path).expect("streaming fixture removes");
        assert_eq!(visited, vec![1]);
        assert!(matches!(
            error,
            AuditStoreError::InvalidJson { line: 2, .. }
        ));
    }
}
