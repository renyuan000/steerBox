//! Durable, metadata-only approval requests.
//!
//! This module deliberately stores only the request metadata needed to match a
//! later action.  It does not accept arbitrary payloads or credentials.  The
//! JSONL file contains immutable snapshots; reopening the store folds the
//! snapshots by `request_id` and therefore preserves the latest state without
//! requiring an in-memory database.

use crate::redaction::Redactor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub project_id: String,
    pub session_id: String,
    pub task_run_id: String,
    pub tool_id: String,
    pub capability: String,
    pub data_class: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ApprovalRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn at(
        request_id: impl Into<String>,
        project_id: impl Into<String>,
        session_id: impl Into<String>,
        task_run_id: impl Into<String>,
        tool_id: impl Into<String>,
        capability: impl Into<String>,
        data_class: impl Into<String>,
        summary: impl Into<String>,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, ApprovalError> {
        let raw_summary = summary.into();
        let summary = Redactor::default()
            .redact_text(&raw_summary)
            .map_err(|_| ApprovalError::InvalidRequest("summary exceeds redaction limit".into()))?;
        let request = Self {
            request_id: request_id.into(),
            project_id: project_id.into(),
            session_id: session_id.into(),
            task_run_id: task_run_id.into(),
            tool_id: tool_id.into(),
            capability: capability.into(),
            data_class: data_class.into(),
            summary,
            created_at,
            expires_at,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), ApprovalError> {
        for (name, value) in [
            ("request_id", &self.request_id),
            ("project_id", &self.project_id),
            ("session_id", &self.session_id),
            ("task_run_id", &self.task_run_id),
            ("tool_id", &self.tool_id),
            ("capability", &self.capability),
            ("data_class", &self.data_class),
        ] {
            if value.trim().is_empty() {
                return Err(ApprovalError::InvalidRequest(format!(
                    "{name} must not be empty"
                )));
            }
        }
        if self.expires_at <= self.created_at {
            return Err(ApprovalError::InvalidRequest(
                "expires_at must be after created_at".into(),
            ));
        }
        if self.summary.trim().is_empty() {
            return Err(ApprovalError::InvalidRequest(
                "summary must not be empty".into(),
            ));
        }
        let redacted_summary = Redactor::default()
            .redact_text(&self.summary)
            .map_err(|_| ApprovalError::InvalidRequest("summary exceeds redaction limit".into()))?;
        if redacted_summary != self.summary {
            return Err(ApprovalError::InvalidRequest(
                "summary must be locally redacted".into(),
            ));
        }
        Ok(())
    }

    fn intent_fingerprint(&self) -> String {
        // Timestamps are intentionally excluded: retrying the same request
        // creates a fresh envelope, but must remain idempotent.
        let material = [
            self.project_id.as_str(),
            self.session_id.as_str(),
            self.task_run_id.as_str(),
            self.tool_id.as_str(),
            self.capability.as_str(),
            self.data_class.as_str(),
            self.summary.as_str(),
        ]
        .join("\u{1f}");
        let mut digest = Sha256::new();
        digest.update(material.as_bytes());
        format!("{:x}", digest.finalize())
    }

    fn action_fingerprint(&self) -> String {
        let material = [
            self.project_id.as_str(),
            self.tool_id.as_str(),
            self.capability.as_str(),
            self.data_class.as_str(),
            self.summary.as_str(),
        ]
        .join("\u{1f}");
        let mut digest = Sha256::new();
        digest.update(material.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    Task,
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve(ApprovalScope),
    Reject,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Consumed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub request: ApprovalRequest,
    pub status: ApprovalStatus,
    pub scope: Option<ApprovalScope>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl ApprovalRecord {
    fn pending(request: ApprovalRequest) -> Self {
        Self {
            request,
            status: ApprovalStatus::Pending,
            scope: None,
            resolved_at: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ApprovalError {
    #[error("invalid approval request: {0}")]
    InvalidRequest(String),
    #[error("approval request id conflict")]
    Conflict,
    #[error("approval request is terminal")]
    Terminal,
    #[error("approval request store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("approval request line {line} is invalid JSON")]
    InvalidJson { line: usize },
    #[error("approval request history is invalid at line {line}")]
    InvalidHistory { line: usize },
    #[error("approval request store initialization failed: {0}")]
    Initialization(String),
}

#[derive(Debug)]
struct ApprovalState {
    records: BTreeMap<String, ApprovalRecord>,
    initialization_error: Option<String>,
}

struct ApprovalStateGuard<'a> {
    state: std::sync::MutexGuard<'a, ApprovalState>,
    // Dropping the file releases the OS lock, including after a crash. Keep the
    // lock file itself: unlinking it permits parallel processes to lock two inodes.
    _lock_file: File,
}

impl std::ops::Deref for ApprovalStateGuard<'_> {
    type Target = ApprovalState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for ApprovalStateGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

#[derive(Debug)]
pub struct ApprovalStore {
    path: PathBuf,
    state: Mutex<ApprovalState>,
}

impl ApprovalStore {
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let loaded = load_records(&path);
        let (records, initialization_error) = match loaded {
            Ok(records) => (records, None),
            Err(error) => (BTreeMap::new(), Some(error.to_string())),
        };
        Self {
            path,
            state: Mutex::new(ApprovalState {
                records,
                initialization_error,
            }),
        }
    }

    pub fn submit(&self, request: ApprovalRequest) -> Result<ApprovalRecord, ApprovalError> {
        request.validate()?;
        let mut state = self.lock_state()?;
        self.ensure_initialized(&state)?;
        if let Some(existing) = state.records.get(&request.request_id) {
            if existing.request.intent_fingerprint() != request.intent_fingerprint() {
                return Err(ApprovalError::Conflict);
            }
            return Ok(existing.clone());
        }
        let record = ApprovalRecord::pending(request);
        append_snapshot(&self.path, &record)?;
        state
            .records
            .insert(record.request.request_id.clone(), record.clone());
        Ok(record)
    }

    pub fn resolve(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<ApprovalRecord, ApprovalError> {
        let mut state = self.lock_state()?;
        self.ensure_initialized(&state)?;
        let current = state
            .records
            .get(request_id)
            .cloned()
            .ok_or_else(|| ApprovalError::InvalidRequest("unknown request_id".into()))?;

        match (&current.status, decision) {
            (ApprovalStatus::Pending, ApprovalDecision::Approve(scope)) => {
                if current.request.expires_at <= Utc::now() {
                    let mut expired = current;
                    expired.status = ApprovalStatus::Expired;
                    append_snapshot(&self.path, &expired)?;
                    state.records.insert(request_id.to_owned(), expired);
                    return Err(ApprovalError::Terminal);
                }
                let mut approved = current;
                approved.status = ApprovalStatus::Approved;
                approved.scope = Some(scope);
                approved.resolved_at = Some(Utc::now());
                append_snapshot(&self.path, &approved)?;
                state
                    .records
                    .insert(request_id.to_owned(), approved.clone());
                Ok(approved)
            }
            (ApprovalStatus::Pending, ApprovalDecision::Reject) => {
                let mut rejected = current;
                rejected.status = ApprovalStatus::Rejected;
                rejected.resolved_at = Some(Utc::now());
                append_snapshot(&self.path, &rejected)?;
                state
                    .records
                    .insert(request_id.to_owned(), rejected.clone());
                Ok(rejected)
            }
            (ApprovalStatus::Approved, ApprovalDecision::Approve(scope))
                if current.scope == Some(scope) =>
            {
                Ok(current)
            }
            (ApprovalStatus::Rejected, _)
            | (ApprovalStatus::Consumed, _)
            | (ApprovalStatus::Expired, _) => Err(ApprovalError::Terminal),
            _ => Err(ApprovalError::Conflict),
        }
    }

    pub fn consume(&self, request: &ApprovalRequest) -> Result<bool, ApprovalError> {
        request.validate()?;
        let mut state = self.lock_state()?;
        self.ensure_initialized(&state)?;
        let action_fingerprint = request.action_fingerprint();
        let matching_id = state.records.iter().find_map(|(request_id, record)| {
            if record.status != ApprovalStatus::Approved
                || record.request.project_id != request.project_id
                || record.request.action_fingerprint() != action_fingerprint
            {
                return None;
            }
            match record.scope {
                Some(ApprovalScope::Once)
                    if record.request.request_id == request.request_id
                        && record.request.session_id == request.session_id
                        && record.request.task_run_id == request.task_run_id =>
                {
                    Some(request_id.clone())
                }
                Some(ApprovalScope::Task)
                    if record.request.task_run_id == request.task_run_id
                        && record.request.session_id == request.session_id =>
                {
                    Some(request_id.clone())
                }
                Some(ApprovalScope::Session) if record.request.session_id == request.session_id => {
                    Some(request_id.clone())
                }
                _ => None,
            }
        });
        let Some(matching_id) = matching_id else {
            return Ok(false);
        };
        let current = state.records.get(&matching_id).cloned().ok_or_else(|| {
            ApprovalError::Initialization("approval state changed unexpectedly".into())
        })?;
        if current.request.expires_at <= Utc::now() {
            let mut expired = current;
            expired.status = ApprovalStatus::Expired;
            append_snapshot(&self.path, &expired)?;
            state.records.insert(matching_id, expired);
            return Ok(false);
        }
        if current.scope == Some(ApprovalScope::Once) {
            let mut consumed = current;
            consumed.status = ApprovalStatus::Consumed;
            append_snapshot(&self.path, &consumed)?;
            state.records.insert(matching_id, consumed);
        }
        Ok(true)
    }

    pub fn latest(&self, request_id: &str) -> Option<ApprovalRecord> {
        let state = self.lock_state().ok()?;
        state.records.get(request_id).cloned()
    }

    pub fn read_all(&self) -> Result<Vec<ApprovalRecord>, ApprovalError> {
        let state = self.lock_state()?;
        self.ensure_initialized(&state)?;
        Ok(state.records.values().cloned().collect())
    }

    fn lock_state(&self) -> Result<ApprovalStateGuard<'_>, ApprovalError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ApprovalError::Initialization("approval store mutex poisoned".into()))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))?;
        lock_file.try_lock().map_err(|_| {
            ApprovalError::Initialization("approval store is busy or locking is unsupported".into())
        })?;
        // Refresh under the same OS lock as submit/resolve/consume, so separate
        // stores and restarted workers cannot consume a stale approved snapshot.
        state.records = load_records(&self.path)?;
        state.initialization_error = None;
        Ok(ApprovalStateGuard {
            state,
            _lock_file: lock_file,
        })
    }

    fn ensure_initialized(&self, state: &ApprovalState) -> Result<(), ApprovalError> {
        state
            .initialization_error
            .as_ref()
            .map(|error| Err(ApprovalError::Initialization(error.clone())))
            .unwrap_or(Ok(()))
    }
}

fn load_records(path: &Path) -> Result<BTreeMap<String, ApprovalRecord>, ApprovalError> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(path)?;
        return Ok(BTreeMap::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = BTreeMap::new();
    for (line_index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let line_number = line_index + 1;
        let record = serde_json::from_str::<ApprovalRecord>(&line)
            .map_err(|_| ApprovalError::InvalidJson { line: line_number })?;
        record.request.validate()?;
        if let Some(previous) = records.get(&record.request.request_id) {
            if previous == &record {
                continue;
            }
            validate_replay_transition(previous, &record, line_number)?;
        } else {
            validate_initial_snapshot(&record, line_number)?;
        }
        records.insert(record.request.request_id.clone(), record);
    }
    Ok(records)
}

fn append_snapshot(path: &Path, record: &ApprovalRecord) -> Result<(), ApprovalError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_vec(record).map_err(|_| ApprovalError::InvalidJson { line: 0 })?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

fn validate_initial_snapshot(record: &ApprovalRecord, line: usize) -> Result<(), ApprovalError> {
    if record.status != ApprovalStatus::Pending
        || record.scope.is_some()
        || record.resolved_at.is_some()
    {
        return Err(ApprovalError::InvalidHistory { line });
    }
    Ok(())
}

fn validate_replay_transition(
    previous: &ApprovalRecord,
    next: &ApprovalRecord,
    line: usize,
) -> Result<(), ApprovalError> {
    if previous.request != next.request {
        return Err(ApprovalError::InvalidHistory { line });
    }
    validate_snapshot_shape(next, line)?;
    let legal = matches!(
        (previous.status, next.status),
        (ApprovalStatus::Pending, ApprovalStatus::Approved)
            | (ApprovalStatus::Pending, ApprovalStatus::Rejected)
            | (ApprovalStatus::Pending, ApprovalStatus::Expired)
            | (ApprovalStatus::Approved, ApprovalStatus::Consumed)
            | (ApprovalStatus::Approved, ApprovalStatus::Expired)
    );
    if !legal {
        return Err(ApprovalError::InvalidHistory { line });
    }
    // Consuming or expiring a grant preserves its original scope and decision.
    // Expiring an undecided request must not manufacture a decision either.
    if (previous.status == ApprovalStatus::Approved || next.status == ApprovalStatus::Expired)
        && (previous.scope != next.scope || previous.resolved_at != next.resolved_at)
    {
        return Err(ApprovalError::InvalidHistory { line });
    }
    Ok(())
}

fn validate_snapshot_shape(record: &ApprovalRecord, line: usize) -> Result<(), ApprovalError> {
    let valid = match record.status {
        ApprovalStatus::Pending => record.scope.is_none() && record.resolved_at.is_none(),
        ApprovalStatus::Approved => record.scope.is_some() && record.resolved_at.is_some(),
        ApprovalStatus::Rejected => record.scope.is_none() && record.resolved_at.is_some(),
        ApprovalStatus::Consumed => {
            record.scope == Some(ApprovalScope::Once) && record.resolved_at.is_some()
        }
        ApprovalStatus::Expired => {
            (record.scope.is_none() && record.resolved_at.is_none())
                || (record.scope.is_some() && record.resolved_at.is_some())
        }
    };
    if valid {
        Ok(())
    } else {
        Err(ApprovalError::InvalidHistory { line })
    }
}
