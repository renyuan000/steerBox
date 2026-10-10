use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
};
use crate::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderMessage, ProviderRequest, ProviderTransport,
};
use serde::Deserialize;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionEnvelope {
    decision: ModelDecision,
    summary: String,
}

pub struct ProviderModelAdapter<T: ProviderTransport> {
    provider: OpenAiCompatibleAdapter<T>,
    auth_ref: Option<String>,
    timeout_ms: u64,
}

impl<T: ProviderTransport> ProviderModelAdapter<T> {
    pub fn new(provider: OpenAiCompatibleAdapter<T>, auth_ref: Option<String>) -> Self {
        Self {
            provider,
            auth_ref,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

impl<T: ProviderTransport> ModelAdapter for ProviderModelAdapter<T> {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        if request.model_id.trim().is_empty() {
            return Err(ModelAdapterError::EmptyModelId);
        }
        if request.objective.trim().is_empty() {
            return Err(ModelAdapterError::EmptyObjective);
        }

        let response = self
            .provider
            .complete(ProviderRequest {
                model: request.model_id,
                messages: vec![
                    ProviderMessage::system(request.objective),
                    ProviderMessage::user(request.observation),
                ],
                auth_ref: self.auth_ref.clone(),
                timeout_ms: self.timeout_ms,
                stream: false,
            })
            .map_err(|error| ModelAdapterError::Provider {
                class: error.class(),
            })?;

        let envelope = serde_json::from_str::<DecisionEnvelope>(&response.text)
            .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?;
        if envelope.summary.trim().is_empty() {
            return Err(ModelAdapterError::ProviderTextUnmappable);
        }
        Ok(ModelResponse {
            decision: envelope.decision,
            summary: envelope.summary,
        })
    }
}
