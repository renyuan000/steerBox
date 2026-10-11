use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
    ModelResponseWithAction, DECISION_SYSTEM_PROMPT,
};
use crate::model_contracts::ModelActionEnvelope;
use crate::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderMessage, ProviderRequest, ProviderResponse,
    ProviderStreamEvent, ProviderTransport,
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
    action: Option<ModelActionEnvelope>,
}

/// Resolved completion evidence, not a persisted ModelCallRecord. The caller
/// must still store the permitted result and bind its task/run/attempt identity.
pub struct ResolvedModelResponse {
    pub response: ModelResponse,
    pub provider_response: ProviderResponse,
    pub input_context_hash: String,
}

/// Resolved completion evidence that preserves a validated model action.
pub struct ResolvedModelResponseWithAction {
    pub response: ModelResponseWithAction,
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
    model_id: String,
    timeout_ms: u64,
}

impl<T: ProviderTransport> ProviderModelAdapter<T> {
    pub fn new(provider: OpenAiCompatibleAdapter<T>, auth_ref: Option<String>) -> Self {
        Self {
            provider,
            auth_ref,
            model_id: String::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }

    pub fn provider_id(&self) -> &str {
        self.provider.provider_id()
    }

    pub fn endpoint_base_url(&self) -> &str {
        self.provider.base_url()
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn auth_ref(&self) -> Option<&str> {
        self.auth_ref.as_deref()
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
        let resolved = self.complete_resolved_with_action(request, pack)?;
        if resolved.response.action.is_some() {
            return Err(ModelAdapterError::ProviderTextUnmappable);
        }
        Ok(ResolvedModelResponse {
            response: resolved.response.into_legacy(),
            provider_response: resolved.provider_response,
            input_context_hash: resolved.input_context_hash,
        })
    }

    pub fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ModelAdapterError> {
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
        let response = parse_decision_with_action(&provider_response.text)?;
        Ok(ResolvedModelResponseWithAction {
            response,
            provider_response,
            input_context_hash,
        })
    }

    /// Parse the strict decision envelope and retain an optional validated
    /// action proposal. The legacy `complete` method remains source-compatible
    /// for callers that only consume decision and summary.
    pub fn complete_with_action(
        &self,
        request: ModelRequest,
    ) -> Result<ModelResponseWithAction, ModelAdapterError> {
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

        parse_decision_with_action(&response.text)
    }

    /// Stream provider events using the same trusted system/user message
    /// layers as `complete`. The transport owns credential lookup; this method
    /// forwards only the opaque auth reference and never exposes credential
    /// material in the request body or returned error.
    pub fn stream(
        &self,
        request: ModelRequest,
    ) -> Result<Vec<ProviderStreamEvent>, ModelAdapterError> {
        if request.model_id.trim().is_empty() {
            return Err(ModelAdapterError::EmptyModelId);
        }
        if request.objective.trim().is_empty() {
            return Err(ModelAdapterError::EmptyObjective);
        }

        self.provider
            .stream(ProviderRequest {
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
                stream: true,
            })
            .map_err(|error| ModelAdapterError::Provider {
                class: error.class(),
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

        let response = parse_decision_with_action(&response.text)?;
        if response.action.is_some() {
            return Err(ModelAdapterError::ProviderTextUnmappable);
        }
        Ok(response.into_legacy())
    }

    fn complete_with_action(
        &self,
        request: ModelRequest,
    ) -> Result<ModelResponseWithAction, ModelAdapterError> {
        ProviderModelAdapter::complete_with_action(self, request)
    }
}

fn parse_decision_with_action(text: &str) -> Result<ModelResponseWithAction, ModelAdapterError> {
    let envelope = serde_json::from_str::<DecisionEnvelope>(text)
        .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?;
    if envelope.summary.trim().is_empty() {
        return Err(ModelAdapterError::ProviderTextUnmappable);
    }
    if let Some(action) = &envelope.action {
        action
            .validate()
            .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?;
    }
    Ok(ModelResponseWithAction {
        decision: envelope.decision,
        summary: envelope.summary,
        action: envelope.action,
    })
}
