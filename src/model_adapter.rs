use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provider_adapter::ProviderErrorClass;
use std::collections::BTreeMap;

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
    #[error("provider response text did not contain a supported decision envelope")]
    ProviderTextUnmappable,
    #[error("provider call failed: {class:?}")]
    Provider { class: ProviderErrorClass },
}

pub trait ModelAdapter {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelAdapterRegistryError {
    #[error("model adapter id must not be empty")]
    EmptyModelId,
    #[error("model adapter already registered: {0}")]
    DuplicateModel(String),
}

#[derive(Default)]
pub struct ModelAdapterRegistry {
    adapters: BTreeMap<String, Box<dyn ModelAdapter>>,
}

impl ModelAdapterRegistry {
    pub fn register(
        &mut self,
        model_id: impl Into<String>,
        adapter: Box<dyn ModelAdapter>,
    ) -> Result<(), ModelAdapterRegistryError> {
        let model_id = model_id.into();
        if model_id.trim().is_empty() {
            return Err(ModelAdapterRegistryError::EmptyModelId);
        }
        if self.adapters.contains_key(&model_id) {
            return Err(ModelAdapterRegistryError::DuplicateModel(model_id));
        }
        self.adapters.insert(model_id, adapter);
        Ok(())
    }

    pub fn get(&self, model_id: &str) -> Option<&dyn ModelAdapter> {
        self.adapters.get(model_id).map(Box::as_ref)
    }
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
