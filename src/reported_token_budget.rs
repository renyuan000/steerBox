use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Error)]
#[serde(rename_all = "snake_case")]
pub enum ReportedTokenBudgetError {
    #[error("reported token budget is exhausted")]
    Exhausted,
    #[error("model call ID is already settled")]
    CallAlreadySettled,
    #[error("model call usage conflicts with its earlier settlement")]
    ConflictingUsage,
    #[error("reported token usage is incomplete")]
    UnknownUsage,
    #[error("reported token arithmetic overflow")]
    Overflow,
    #[error("reported token budget blocked by audit or progress failure")]
    AuditFailure,
    #[error("reported token budget requires a resolved model call")]
    ResolvedCallRequired,
    #[error("reported token budget must be enabled once before model calls")]
    EnableRejected,
    #[error("invalid reported token budget state")]
    InvalidState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReportedTokenUsage {
    model_call_id: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl ReportedTokenUsage {
    pub fn model_call_id(&self) -> &str {
        &self.model_call_id
    }

    pub fn input_tokens(&self) -> Option<u64> {
        self.input_tokens
    }

    pub fn output_tokens(&self) -> Option<u64> {
        self.output_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "BudgetSnapshot")]
pub struct ReportedTokenBudget {
    limit: u64,
    known_input_tokens: u64,
    known_output_tokens: u64,
    known_total_tokens: u64,
    records: Vec<ReportedTokenUsage>,
    blocked: Option<ReportedTokenBudgetError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetSnapshot {
    limit: u64,
    known_input_tokens: u64,
    known_output_tokens: u64,
    known_total_tokens: u64,
    records: Vec<ReportedTokenUsage>,
    blocked: Option<ReportedTokenBudgetError>,
}

impl ReportedTokenBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            known_input_tokens: 0,
            known_output_tokens: 0,
            known_total_tokens: 0,
            records: Vec::new(),
            blocked: None,
        }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn known_input_tokens(&self) -> u64 {
        self.known_input_tokens
    }

    pub fn known_output_tokens(&self) -> u64 {
        self.known_output_tokens
    }

    pub fn known_total_tokens(&self) -> u64 {
        self.known_total_tokens
    }

    pub fn records(&self) -> &[ReportedTokenUsage] {
        &self.records
    }

    pub fn blocked(&self) -> Option<ReportedTokenBudgetError> {
        self.blocked
    }

    pub fn check_send(&self, model_call_id: &str) -> Result<(), ReportedTokenBudgetError> {
        if let Some(error) = self.blocked {
            return Err(error);
        }
        if model_call_id.trim().is_empty() {
            return Err(ReportedTokenBudgetError::InvalidState);
        }
        if self
            .records
            .iter()
            .any(|record| record.model_call_id == model_call_id)
        {
            return Err(ReportedTokenBudgetError::CallAlreadySettled);
        }
        if self.known_total_tokens >= self.limit {
            return Err(ReportedTokenBudgetError::Exhausted);
        }
        Ok(())
    }

    pub fn settle(
        &mut self,
        model_call_id: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<(), ReportedTokenBudgetError> {
        if model_call_id.trim().is_empty() {
            return Err(ReportedTokenBudgetError::InvalidState);
        }
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.model_call_id == model_call_id)
        {
            if record.input_tokens == input_tokens && record.output_tokens == output_tokens {
                return Ok(());
            }
            self.blocked
                .get_or_insert(ReportedTokenBudgetError::ConflictingUsage);
            return Err(ReportedTokenBudgetError::ConflictingUsage);
        }
        self.records.push(ReportedTokenUsage {
            model_call_id: model_call_id.into(),
            input_tokens,
            output_tokens,
        });
        let mut error = None;
        for (reported, known) in [
            (input_tokens, &mut self.known_input_tokens),
            (output_tokens, &mut self.known_output_tokens),
        ] {
            match reported {
                Some(tokens) => match known.checked_add(tokens) {
                    Some(total) => *known = total,
                    None => error = Some(ReportedTokenBudgetError::Overflow),
                },
                None => {
                    error.get_or_insert(ReportedTokenBudgetError::UnknownUsage);
                }
            }
        }
        match self
            .known_input_tokens
            .checked_add(self.known_output_tokens)
        {
            Some(total) => self.known_total_tokens = total,
            None => error = Some(ReportedTokenBudgetError::Overflow),
        }
        if let Some(error) = error {
            self.blocked.get_or_insert(error);
            return Err(error);
        }
        Ok(())
    }

    pub fn block_audit_failure(&mut self) {
        self.blocked
            .get_or_insert(ReportedTokenBudgetError::AuditFailure);
    }
}

impl TryFrom<BudgetSnapshot> for ReportedTokenBudget {
    type Error = ReportedTokenBudgetError;

    fn try_from(snapshot: BudgetSnapshot) -> Result<Self, Self::Error> {
        let mut reconstructed = Self::new(snapshot.limit);
        for record in &snapshot.records {
            if record.model_call_id.trim().is_empty()
                || reconstructed
                    .records
                    .iter()
                    .any(|prior| prior.model_call_id == record.model_call_id)
            {
                return Err(ReportedTokenBudgetError::InvalidState);
            }
            let _ = reconstructed.settle(
                &record.model_call_id,
                record.input_tokens,
                record.output_tokens,
            );
        }
        if reconstructed.known_input_tokens != snapshot.known_input_tokens
            || reconstructed.known_output_tokens != snapshot.known_output_tokens
            || reconstructed.known_total_tokens != snapshot.known_total_tokens
            || (reconstructed.blocked.is_some() && snapshot.blocked.is_none())
            || matches!(
                snapshot.blocked,
                Some(
                    ReportedTokenBudgetError::Exhausted
                        | ReportedTokenBudgetError::CallAlreadySettled
                        | ReportedTokenBudgetError::ResolvedCallRequired
                        | ReportedTokenBudgetError::EnableRejected
                        | ReportedTokenBudgetError::InvalidState
                )
            )
        {
            return Err(ReportedTokenBudgetError::InvalidState);
        }
        reconstructed.blocked = snapshot.blocked;
        Ok(reconstructed)
    }
}
