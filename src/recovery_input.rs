//! Private, opt-in recovery inputs. Correspondence never grants execution authority.
mod io;
mod reconcile;
mod validation;

use crate::domain::{GoalContract, LoopRun, NoProgressTracker, TaskRun};
use crate::model_call_store::ModelCallStore;
use crate::runtime_config::RuntimeConfig;
use crate::strategy_selector::StrategyDecision;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

const RECORD_CAP: usize = 2 * 1024 * 1024;
const EVENT_CAP: usize = 16 * 1024 * 1024;
const AUDIT_CAP: usize = 4 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecoveryInputError {
    #[error("recovery input is unsafe or unavailable")]
    UnsafeOrUnavailable,
    #[error("recovery input limit reached")]
    LimitReached,
    #[error("recovery input record is invalid")]
    InvalidRecord,
    #[error("recovery input write is unconfirmed; do not automatically retry")]
    WriteUnconfirmed,
    #[error("recovery input sources do not match")]
    SourcesChanged,
}

/// Contains private task text. Host access is not permission to send it to a model.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryState {
    pub(super) goal: GoalContract,
    pub(super) loop_run: LoopRun,
    pub(super) task_run: TaskRun,
    pub(super) next_step_no: u32,
    pub(super) no_progress: NoProgressTracker,
    pub(super) strategy: Option<StrategyDecision>,
    pub(super) strategy_revision: u64,
}

impl RecoveryState {
    pub fn goal(&self) -> &GoalContract {
        &self.goal
    }
    pub fn loop_run(&self) -> &LoopRun {
        &self.loop_run
    }
    pub fn task_run(&self) -> &TaskRun {
        &self.task_run
    }
    pub fn next_step_no(&self) -> u32 {
        self.next_step_no
    }
    pub fn no_progress(&self) -> &NoProgressTracker {
        &self.no_progress
    }
    pub fn strategy(&self) -> Option<&StrategyDecision> {
        self.strategy.as_ref()
    }
    pub fn strategy_revision(&self) -> u64 {
        self.strategy_revision
    }
}

impl fmt::Debug for RecoveryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryState { private: true }")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySource {
    Events,
    ModelCalls,
    ModelFailures,
    ToolCalls,
    SideEffects,
    Traces,
    Checkpoints,
}
const SOURCES: [RecoverySource; 7] = [
    RecoverySource::Events,
    RecoverySource::ModelCalls,
    RecoverySource::ModelFailures,
    RecoverySource::ToolCalls,
    RecoverySource::SideEffects,
    RecoverySource::Traces,
    RecoverySource::Checkpoints,
];

/// Paths supplied by the trusted host, never from the saved record or a model action.
#[derive(Clone)]
pub struct RecoveryInputPaths {
    paths: [Option<PathBuf>; 7],
}

impl fmt::Debug for RecoveryInputPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryInputPaths { private: true }")
    }
}

impl RecoveryInputPaths {
    /// All other sources are explicitly unconfigured, not implicitly audited.
    pub fn events_only(events: impl Into<PathBuf>) -> Self {
        Self {
            paths: [Some(events.into()), None, None, None, None, None, None],
        }
    }
    /// Pure path calculation: does not load configuration or credential contents.
    pub fn from_config(config: &RuntimeConfig) -> Self {
        let [model, failures, _, _] = ModelCallStore::storage_paths_for(config.model_calls_path());
        Self {
            paths: [
                Some(config.events_path()),
                Some(model),
                Some(failures),
                Some(config.tool_calls_path()),
                Some(config.side_effects_path()),
                Some(config.traces_path()),
                Some(config.checkpoints_path()),
            ],
        }
    }
    pub fn with_source(mut self, kind: RecoverySource, path: impl Into<PathBuf>) -> Self {
        let index = SOURCES
            .iter()
            .position(|source| *source == kind)
            .expect("fixed source kind");
        self.paths[index] = Some(path.into());
        self
    }
    pub(crate) fn matches_events(&self, path: &Path) -> bool {
        self.paths[0]
            .as_deref()
            .is_some_and(|events| same_path(events, path))
    }
    fn validate(&self) -> Result<(), RecoveryInputError> {
        if self.paths[0].is_none() {
            return Err(RecoveryInputError::InvalidRecord);
        }
        for (index, path) in self.paths.iter().enumerate() {
            if let Some(path) = path {
                validate_path(path)?;
                if self.paths[..index]
                    .iter()
                    .flatten()
                    .any(|prior| same_path(prior, path))
                {
                    return Err(RecoveryInputError::InvalidRecord);
                }
            }
        }
        Ok(())
    }
}

fn validate_path(path: &Path) -> Result<(), RecoveryInputError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(RecoveryInputError::UnsafeOrUnavailable);
    }
    Ok(())
}
fn same_path(left: &Path, right: &Path) -> bool {
    left.components().eq(right.components())
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, tag = "presence", rename_all = "snake_case")]
enum SourceContent {
    Unconfigured,
    Missing,
    Present {
        bytes: u64,
        sha256: String,
        records: u32,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SourceBoundary {
    kind: RecoverySource,
    content: SourceContent,
}

/// Host-only record. There is deliberately no resumed-controller constructor.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryInput {
    schema_version: u32,
    state: RecoveryState,
    sources: Vec<SourceBoundary>,
    event_sequence: u64,
}
impl fmt::Debug for RecoveryInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryInput { private: true, resume_authorized: false }")
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryVerdict {
    InputsMatchOnly,
    SourcesChanged,
    SourcesUnavailable,
    InvalidSources,
    InconsistentState,
    PendingAudit,
    AuditReconciliationRequired,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecoveryAssessment {
    pub verdict: RecoveryVerdict,
    pub resume_authorized: bool,
    pub complete_recovery_coverage: bool,
}

impl RecoveryInput {
    pub(crate) fn capture(
        state: RecoveryState,
        paths: &RecoveryInputPaths,
    ) -> Result<Self, RecoveryInputError> {
        validation::validate(&state)?;
        let scan = reconcile::scan(paths, &state)?;
        let record = Self {
            schema_version: 1,
            state,
            sources: scan.sources,
            event_sequence: scan.event_sequence,
        };
        record.validate()?;
        Ok(record)
    }
    pub fn state(&self) -> &RecoveryState {
        &self.state
    }
    /// New private file only. Failure may leave uncertain bytes; never blindly retry.
    pub fn persist_new(
        &self,
        path: impl AsRef<Path>,
        sources: &RecoveryInputPaths,
    ) -> Result<(), RecoveryInputError> {
        let path = path.as_ref();
        validate_path(path)?;
        sources.validate()?;
        if sources
            .paths
            .iter()
            .flatten()
            .any(|source| same_path(path, source))
        {
            return Err(RecoveryInputError::UnsafeOrUnavailable);
        }
        self.validate()?;
        let scan = reconcile::scan(sources, &self.state)?;
        if scan.sources != self.sources || scan.event_sequence != self.event_sequence {
            return Err(RecoveryInputError::SourcesChanged);
        }
        let mut bytes = serde_json::to_vec(self).map_err(|_| RecoveryInputError::InvalidRecord)?;
        bytes.push(b'\n');
        if bytes.len() >= RECORD_CAP {
            return Err(RecoveryInputError::LimitReached);
        }
        io::persist_new(path, &bytes)
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RecoveryInputError> {
        validate_path(path.as_ref())?;
        let bytes = io::read_private(path.as_ref(), RECORD_CAP)?;
        if !bytes.ends_with(b"\n") {
            return Err(RecoveryInputError::InvalidRecord);
        }
        let record: Self = validation::decode(&bytes)?;
        record.validate()?;
        Ok(record)
    }
    fn validate(&self) -> Result<(), RecoveryInputError> {
        if self.schema_version != 1
            || self.sources.len() != SOURCES.len()
            || self.event_sequence == 0
        {
            return Err(RecoveryInputError::InvalidRecord);
        }
        for (boundary, kind) in self.sources.iter().zip(SOURCES) {
            if boundary.kind != kind {
                return Err(RecoveryInputError::InvalidRecord);
            }
            if let SourceContent::Present {
                bytes,
                sha256,
                records,
            } = &boundary.content
            {
                let cap = if kind == RecoverySource::Events {
                    EVENT_CAP
                } else {
                    AUDIT_CAP
                };
                if *bytes >= cap as u64
                    || *records > reconcile::RECORD_LIMIT
                    || sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                {
                    return Err(RecoveryInputError::InvalidRecord);
                }
                if kind == RecoverySource::Events
                    && (*records == 0 || u64::from(*records) != self.event_sequence)
                {
                    return Err(RecoveryInputError::InvalidRecord);
                }
            } else if kind == RecoverySource::Events {
                return Err(RecoveryInputError::InvalidRecord);
            }
        }
        validation::validate(&self.state)
    }
    pub fn assess(&self, paths: &RecoveryInputPaths) -> RecoveryAssessment {
        let verdict = if self.validate().is_err() {
            RecoveryVerdict::InconsistentState
        } else {
            match reconcile::scan(paths, &self.state) {
                Err(RecoveryInputError::UnsafeOrUnavailable | RecoveryInputError::LimitReached) => {
                    RecoveryVerdict::SourcesUnavailable
                }
                Err(_) => RecoveryVerdict::InvalidSources,
                Ok(scan)
                    if scan.sources != self.sources
                        || scan.event_sequence != self.event_sequence =>
                {
                    RecoveryVerdict::SourcesChanged
                }
                Ok(scan) => scan.verdict,
            }
        };
        RecoveryAssessment {
            verdict,
            resume_authorized: false,
            complete_recovery_coverage: false,
        }
    }
}
