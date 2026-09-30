use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequest {
    pub model_id: String,
    pub goal_id: String,
    pub loop_run_id: String,
    pub step_id: String,
    pub objective: String,
    pub observation: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelDecision {
    ContinueObservation,
    CompleteLoop,
    RequestHuman,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelResponse {
    pub decision: ModelDecision,
    pub summary: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelAdapterError {
    #[error("model request model_id must not be empty")]
    EmptyModelId,
    #[error("model request objective must not be empty")]
    EmptyObjective,
}

pub trait ModelAdapter {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError>;
}

#[derive(Debug, Clone)]
pub struct LocalFixtureModelAdapter {
    response: ModelResponse,
}

impl LocalFixtureModelAdapter {
    pub fn new(decision: ModelDecision, summary: impl Into<String>) -> Self {
        Self {
            response: ModelResponse {
                decision,
                summary: summary.into(),
            },
        }
    }
}

impl ModelAdapter for LocalFixtureModelAdapter {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        if request.model_id.trim().is_empty() {
            return Err(ModelAdapterError::EmptyModelId);
        }
        if request.objective.trim().is_empty() {
            return Err(ModelAdapterError::EmptyObjective);
        }
        Ok(self.response.clone())
    }
}
