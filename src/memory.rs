use crate::redaction::Redactor;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MEMORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Project,
    Goal,
    Task,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCandidateState {
    New,
    Reviewing,
    Rejected,
    Accepted,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProvenance {
    pub producer: String,
    pub source_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub schema_version: u32,
    pub candidate_id: String,
    pub scope: MemoryScope,
    pub project_id: String,
    pub goal_id: Option<String>,
    pub task_id: Option<String>,
    pub claim_summary: String,
    pub provenance: MemoryProvenance,
    pub source_event_refs: Vec<String>,
    pub confidence: MemoryConfidence,
    pub lifecycle_state: MemoryCandidateState,
}

impl MemoryCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        candidate_id: impl Into<String>,
        scope: MemoryScope,
        project_id: impl Into<String>,
        goal_id: Option<&str>,
        task_id: Option<&str>,
        claim_summary: impl Into<String>,
        provenance: MemoryProvenance,
        source_event_refs: Vec<String>,
        confidence: MemoryConfidence,
        lifecycle_state: MemoryCandidateState,
    ) -> Result<Self, MemoryError> {
        let raw_summary = claim_summary.into();
        let claim_summary = Redactor::default().redact_text(&raw_summary).map_err(|_| {
            MemoryError::InvalidField {
                field: "claim_summary".into(),
                reason: "exceeds local redaction limit".into(),
            }
        })?;
        let candidate = Self {
            schema_version: MEMORY_SCHEMA_VERSION,
            candidate_id: candidate_id.into(),
            scope,
            project_id: project_id.into(),
            goal_id: goal_id.map(str::to_owned),
            task_id: task_id.map(str::to_owned),
            claim_summary,
            provenance,
            source_event_refs,
            confidence,
            lifecycle_state,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema_version != MEMORY_SCHEMA_VERSION {
            return Err(MemoryError::InvalidField {
                field: "schema_version".into(),
                reason: "unsupported schema version".into(),
            });
        }
        validate_non_blank("candidate_id", &self.candidate_id)?;
        validate_non_blank("project_id", &self.project_id)?;
        validate_non_blank("claim_summary", &self.claim_summary)?;
        let redacted_summary = Redactor::default()
            .redact_text(&self.claim_summary)
            .map_err(|_| MemoryError::InvalidField {
                field: "claim_summary".into(),
                reason: "exceeds local redaction limit".into(),
            })?;
        if redacted_summary != self.claim_summary {
            return Err(MemoryError::InvalidField {
                field: "claim_summary".into(),
                reason: "must be locally redacted".into(),
            });
        }
        validate_non_blank("provenance.producer", &self.provenance.producer)?;
        validate_non_blank("provenance.source_kind", &self.provenance.source_kind)?;

        if self.source_event_refs.is_empty()
            || self
                .source_event_refs
                .iter()
                .any(|reference| reference.trim().is_empty())
            || has_duplicate_strings(&self.source_event_refs)
        {
            return Err(MemoryError::InvalidField {
                field: "source_event_refs".into(),
                reason: "must contain at least one non-empty reference".into(),
            });
        }

        match self.scope {
            MemoryScope::Project => {
                if self.goal_id.is_some() {
                    return Err(MemoryError::InvalidField {
                        field: "goal_id".into(),
                        reason: "must be absent for project scope".into(),
                    });
                }
                if self.task_id.is_some() {
                    return Err(MemoryError::InvalidField {
                        field: "task_id".into(),
                        reason: "must be absent for project scope".into(),
                    });
                }
            }
            MemoryScope::Goal => {
                let goal_id = self
                    .goal_id
                    .as_deref()
                    .ok_or_else(|| MemoryError::InvalidField {
                        field: "goal_id".into(),
                        reason: "required for goal scope".into(),
                    })?;
                validate_non_blank("goal_id", goal_id)?;
                if self.task_id.is_some() {
                    return Err(MemoryError::InvalidField {
                        field: "task_id".into(),
                        reason: "must be absent for goal scope".into(),
                    });
                }
            }
            MemoryScope::Task => {
                if let Some(goal_id) = self.goal_id.as_deref() {
                    validate_non_blank("goal_id", goal_id)?;
                }
                let task_id = self
                    .task_id
                    .as_deref()
                    .ok_or_else(|| MemoryError::InvalidField {
                        field: "task_id".into(),
                        reason: "required for task scope".into(),
                    })?;
                validate_non_blank("task_id", task_id)?;
            }
        }
        Ok(())
    }

    fn visible_to(&self, query: &MemoryQuery) -> bool {
        if self.project_id != query.project_id
            || matches!(
                self.lifecycle_state,
                MemoryCandidateState::Rejected | MemoryCandidateState::Superseded
            )
        {
            return false;
        }

        match self.scope {
            MemoryScope::Project => true,
            MemoryScope::Goal => self.goal_id.as_deref() == query.goal_id.as_deref(),
            MemoryScope::Task => {
                self.goal_id.as_deref() == query.goal_id.as_deref()
                    && self.task_id.as_deref() == query.task_id.as_deref()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryQuery {
    pub project_id: String,
    pub goal_id: Option<String>,
    pub task_id: Option<String>,
}

impl MemoryQuery {
    pub fn new(
        project_id: impl Into<String>,
        goal_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<Self, MemoryError> {
        let query = Self {
            project_id: project_id.into(),
            goal_id: goal_id.map(str::to_owned),
            task_id: task_id.map(str::to_owned),
        };
        validate_non_blank("project_id", &query.project_id)?;
        if let Some(goal_id) = query.goal_id.as_deref() {
            validate_non_blank("goal_id", goal_id)?;
        }
        if let Some(task_id) = query.task_id.as_deref() {
            validate_non_blank("task_id", task_id)?;
        }
        Ok(query)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemoryCatalog {
    candidates: Vec<MemoryCandidate>,
}

impl MemoryCatalog {
    pub fn insert(&mut self, candidate: MemoryCandidate) -> Result<(), MemoryError> {
        candidate.validate()?;
        if self
            .candidates
            .iter()
            .any(|existing| existing.candidate_id == candidate.candidate_id)
        {
            return Err(MemoryError::DuplicateCandidate(candidate.candidate_id));
        }
        self.candidates.push(candidate);
        Ok(())
    }

    /// Returns scoped candidate metadata, not trusted or active memory.
    /// Results must not be automatically injected into runtime context.
    /// This does not authorize access or promote candidates; callers must enforce PolicyGate.
    pub fn retrieve(&self, query: &MemoryQuery) -> Vec<MemoryCandidate> {
        let mut visible = self
            .candidates
            .iter()
            .filter(|candidate| candidate.visible_to(query))
            .cloned()
            .collect::<Vec<_>>();
        visible.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
        visible
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryError {
    #[error("invalid memory field {field}: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("duplicate memory candidate: {0}")]
    DuplicateCandidate(String),
}

fn validate_non_blank(field: &str, value: &str) -> Result<(), MemoryError> {
    if value.trim().is_empty() {
        return Err(MemoryError::InvalidField {
            field: field.into(),
            reason: "must not be blank".into(),
        });
    }
    Ok(())
}

fn has_duplicate_strings(values: &[String]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}
