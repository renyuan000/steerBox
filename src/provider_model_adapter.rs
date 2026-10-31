use crate::adaptive_routing::EffectLevel;
use crate::model_adapter::{
    DecisionOutputRejection, ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest,
    ModelResponse, ModelResponseWithAction, DECISION_SYSTEM_PROMPT,
};
use crate::model_contracts::ModelActionEnvelope;
use crate::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderMessage, ProviderRequest, ProviderResponse,
    ProviderStreamEvent, ProviderTransport, ReasoningEffort,
};
use crate::redaction::Redactor;
use crate::resolved_prompt::ResolvedPromptPack;
use serde::Deserialize;
use std::fmt;
use thiserror::Error;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCallEvidence {
    input_context_hash: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    finish_reason: Option<&'static str>,
}

impl ResolvedCallEvidence {
    fn from_response(input_context_hash: &str, response: &ProviderResponse) -> Option<Self> {
        let digest = input_context_hash.strip_prefix("sha256:")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        Some(Self {
            input_context_hash: input_context_hash.to_owned(),
            input_tokens: response
                .usage
                .as_ref()
                .and_then(|usage| usage.prompt_tokens),
            output_tokens: response
                .usage
                .as_ref()
                .and_then(|usage| usage.completion_tokens),
            finish_reason: match response.finish_reason.as_deref() {
                Some("stop") => Some("stop"),
                Some("length") => Some("length"),
                Some("end_turn") => Some("end_turn"),
                Some("max_tokens") => Some("max_tokens"),
                _ => None,
            },
        })
    }

    pub fn input_context_hash(&self) -> &str {
        &self.input_context_hash
    }

    pub fn input_tokens(&self) -> Option<u64> {
        self.input_tokens
    }

    pub fn output_tokens(&self) -> Option<u64> {
        self.output_tokens
    }

    pub fn finish_reason(&self) -> Option<&str> {
        self.finish_reason
    }
}

#[derive(Debug, Error)]
#[error("{cause}")]
pub struct ResolvedCallFailure {
    #[source]
    cause: ModelAdapterError,
    evidence: Option<ResolvedCallEvidence>,
}

impl ResolvedCallFailure {
    pub fn new(cause: ModelAdapterError) -> Self {
        Self {
            cause,
            evidence: None,
        }
    }

    pub(crate) fn after_response(
        cause: ModelAdapterError,
        input_context_hash: &str,
        response: &ProviderResponse,
    ) -> Self {
        Self {
            cause,
            evidence: ResolvedCallEvidence::from_response(input_context_hash, response),
        }
    }

    pub fn cause(&self) -> &ModelAdapterError {
        &self.cause
    }

    pub fn evidence(&self) -> Option<&ResolvedCallEvidence> {
        self.evidence.as_ref()
    }

    pub fn into_parts(self) -> (ModelAdapterError, Option<ResolvedCallEvidence>) {
        (self.cause, self.evidence)
    }
}

/// Common resolved-call seam used by the controller for provider protocols
/// that have different wire formats but the same audit/decision contract.
pub trait ResolvedModelAdapter {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure>;
}

/// Opaque binding minted only by the runtime profile factory after it applies
/// the declared effect policy to the concrete OpenAI-compatible adapter.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EffectProfileBinding {
    pub(crate) provider_id: String,
    pub(crate) endpoint: String,
    pub(crate) model_id: String,
    pub(crate) auth_ref: Option<String>,
    pub(crate) selected_level: EffectLevel,
    pub(crate) maximum_level: EffectLevel,
    pub(crate) mapping: std::collections::BTreeMap<EffectLevel, ReasoningEffort>,
}

/// A profile-factory-created adapter whose private binding cannot be supplied
/// or altered by a resolver closure.
pub struct EffectBoundResolvedAdapter<T: ProviderTransport> {
    inner: ProviderModelAdapter<T>,
    binding: EffectProfileBinding,
}

impl<T: ProviderTransport> EffectBoundResolvedAdapter<T> {
    pub(crate) fn from_profile(
        inner: ProviderModelAdapter<T>,
        binding: EffectProfileBinding,
    ) -> Self {
        Self { inner, binding }
    }

    pub(crate) fn binding(&self) -> &EffectProfileBinding {
        &self.binding
    }
}

impl<T: ProviderTransport> ResolvedModelAdapter for EffectBoundResolvedAdapter<T> {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure> {
        self.inner.complete_resolved_with_action(request, pack)
    }
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
    ) -> Result<ResolvedModelResponse, ResolvedCallFailure> {
        let resolved = self.complete_resolved_with_action(request, pack)?;
        if resolved.response.action.is_some() {
            return Err(ResolvedCallFailure::after_response(
                ModelAdapterError::ProviderTextUnmappable,
                &resolved.input_context_hash,
                &resolved.provider_response,
            ));
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
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure> {
        if !matches!(pack.endpoint_protocol(), "openai" | "openai-compatible") {
            return Err(ResolvedCallFailure::new(
                ModelAdapterError::ResolvedBindingMismatch {
                    field: "endpoint_protocol",
                },
            ));
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
                return Err(ResolvedCallFailure::new(
                    ModelAdapterError::ResolvedBindingMismatch { field },
                ));
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
            .map_err(|error| {
                ResolvedCallFailure::new(ModelAdapterError::Provider {
                    class: error.class(),
                })
            })?;
        let input_context_hash = prepared.input_hash().to_owned();
        let provider_response = self
            .provider
            .complete_prepared(&prepared)
            .map_err(|error| {
                ResolvedCallFailure::new(ModelAdapterError::Provider {
                    class: error.class(),
                })
            })?;
        let response = parse_decision_with_action(&provider_response.text).map_err(|cause| {
            ResolvedCallFailure::after_response(cause, &input_context_hash, &provider_response)
        })?;
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

pub(crate) fn parse_decision_with_action(
    text: &str,
) -> Result<ModelResponseWithAction, ModelAdapterError> {
    let envelope = serde_json::from_str::<DecisionEnvelope>(text).map_err(|error| {
        ModelAdapterError::DecisionOutputRejected {
            reason: match error.classify() {
                serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                    DecisionOutputRejection::JsonSyntaxInvalid
                }
                serde_json::error::Category::Data | serde_json::error::Category::Io => {
                    DecisionOutputRejection::EnvelopeInvalid
                }
            },
        }
    })?;
    if envelope.summary.trim().is_empty() {
        return Err(ModelAdapterError::DecisionOutputRejected {
            reason: DecisionOutputRejection::EmptySummary,
        });
    }
    if let Some(action) = &envelope.action {
        action
            .validate()
            .map_err(|_| ModelAdapterError::DecisionOutputRejected {
                reason: DecisionOutputRejection::ActionRejected,
            })?;
    }
    Ok(ModelResponseWithAction {
        decision: envelope.decision,
        summary: Redactor::default()
            .redact_text(&envelope.summary)
            .map_err(|_| ModelAdapterError::DecisionOutputRejected {
                reason: DecisionOutputRejection::SummaryRedactionRejected,
            })?,
        action: envelope.action,
    })
}

impl<T: ProviderTransport> ResolvedModelAdapter for ProviderModelAdapter<T> {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ResolvedCallFailure> {
        ProviderModelAdapter::complete_resolved_with_action(self, request, pack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_evidence_rejects_non_digest_hashes_without_retaining_them() {
        for hash in [
            "fixture-private-hash".to_owned(),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "g".repeat(64)),
            format!("sha256:{}", "0".repeat(63)),
            format!("sha256:{}", "0".repeat(65)),
        ] {
            let response = ProviderResponse::text(
                "fixture-private-id",
                "fixture-private-model",
                "fixture-private-body",
            );
            let failure = ResolvedCallFailure::after_response(
                ModelAdapterError::ProviderTextUnmappable,
                &hash,
                &response,
            );
            assert!(failure.evidence().is_none());
            assert!(!format!("{failure}\n{failure:?}").contains("fixture-private"));
        }
    }
}
