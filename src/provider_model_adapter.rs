use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
    DECISION_SYSTEM_PROMPT,
};
use crate::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderMessage, ProviderRequest, ProviderResponse, ProviderTransport,
};
use crate::resolved_prompt::ResolvedPromptPack;
use serde::Deserialize;
use std::fmt;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionEnvelope {
    decision: ModelDecision,
    summary: String,
}

/// Resolved completion evidence, not a persisted ModelCallRecord. The caller
/// must still store the permitted result and bind its task/run/attempt identity.
pub struct ResolvedModelResponse {
    pub response: ModelResponse,
    pub provider_response: ProviderResponse,
    pub input_context_hash: String,
}

impl fmt::Debug for ResolvedModelResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedModelResponse")
            .field("input_context_hash", &self.input_context_hash)
            .finish_non_exhaustive()
    }
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

    pub fn complete_resolved(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponse, ModelAdapterError> {
        if !matches!(pack.endpoint_protocol(), "openai" | "openai-compatible") {
            return Err(ModelAdapterError::ResolvedBindingMismatch {
                field: "endpoint_protocol",
            });
        }
        for (actual, expected, field) in [
            (
                self.provider.provider_id(),
                pack.provider_id(),
                "provider_id",
            ),
            (
                self.provider.base_url().trim_end_matches('/'),
                pack.endpoint_base_url().trim_end_matches('/'),
                "endpoint_base_url",
            ),
            (request.model_id.as_str(), pack.model_id(), "model_id"),
            (request.objective.as_str(), pack.objective(), "objective"),
            (
                request.observation.as_str(),
                pack.observation(),
                "observation",
            ),
        ] {
            if actual != expected {
                return Err(ModelAdapterError::ResolvedBindingMismatch { field });
            }
        }
        let prepared = self
            .provider
            .prepare_complete(ProviderRequest {
                model: pack.model_name().to_owned(),
                messages: pack.messages().to_vec(),
                auth_ref: self.auth_ref.clone(),
                timeout_ms: self.timeout_ms,
                stream: false,
            })
            .map_err(|error| ModelAdapterError::Provider {
                class: error.class(),
            })?;
        let input_context_hash = prepared.input_hash().to_owned();
        let provider_response = self
            .provider
            .complete_prepared(&prepared)
            .map_err(|error| ModelAdapterError::Provider {
                class: error.class(),
            })?;
        let response = parse_decision(&provider_response.text)?;
        Ok(ResolvedModelResponse {
            response,
            provider_response,
            input_context_hash,
        })
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
                    ProviderMessage::system(DECISION_SYSTEM_PROMPT),
                    ProviderMessage::user(
                        serde_json::json!({
                            "objective": request.objective,
                            "observation": request.observation,
                        })
                        .to_string(),
                    ),
                ],
                auth_ref: self.auth_ref.clone(),
                timeout_ms: self.timeout_ms,
                stream: false,
            })
            .map_err(|error| ModelAdapterError::Provider {
                class: error.class(),
            })?;

        parse_decision(&response.text)
    }
}

fn parse_decision(text: &str) -> Result<ModelResponse, ModelAdapterError> {
    let envelope = serde_json::from_str::<DecisionEnvelope>(text)
        .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?;
    if envelope.summary.trim().is_empty() {
        return Err(ModelAdapterError::ProviderTextUnmappable);
    }
    Ok(ModelResponse {
        decision: envelope.decision,
        summary: envelope.summary,
    })
}
