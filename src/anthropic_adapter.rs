use crate::model_adapter::{
    ModelAdapter, ModelAdapterError, ModelDecision, ModelRequest, ModelResponse,
    ModelResponseWithAction, DECISION_SYSTEM_PROMPT,
};
use crate::model_contracts::ModelActionEnvelope;
use crate::provider_adapter::{
    ProviderMessage, ProviderRequest, ProviderResponse, ProviderUsage, TransportResponse,
    MAX_TIMEOUT_MS,
};
use crate::provider_model_adapter::{ResolvedModelAdapter, ResolvedModelResponseWithAction};
use crate::redaction::Redactor;
use crate::resolved_prompt::ResolvedPromptPack;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use thiserror::Error;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_TOKENS: u32 = 1_024;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicHeaders {
    /// Opaque lookup reference. The transport resolves it to the x-api-key
    /// header outside this crate; credential material never enters the adapter.
    pub api_key_ref: Option<String>,
    pub anthropic_version: String,
}

impl AnthropicHeaders {
    pub fn auth_header_name(&self) -> &'static str {
        "x-api-key"
    }

    pub fn version_header_name(&self) -> &'static str {
        "anthropic-version"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicTransportError {
    Timeout,
    Network,
    Credential,
    InvalidEndpoint,
    ResponseTooLarge,
    InvalidResponse,
    Redirect,
    UnsupportedStreaming,
    Other,
}

/// Transport boundary for Anthropic's independent x-api-key protocol.
/// Implementations resolve api_key_ref; this adapter never receives or logs
/// the resulting secret.
pub trait AnthropicTransport {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnthropicAdapterError {
    #[error("provider request is invalid: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("provider authentication failed")]
    Authentication,
    #[error("provider permission denied")]
    Permission,
    #[error("provider rate limit reached")]
    RateLimit,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider transport failed")]
    Transport,
    #[error("provider returned an upstream error")]
    Upstream,
    #[error("provider response could not be parsed")]
    ResponseParse,
    #[error("provider streaming response is unsupported")]
    StreamingUnsupported,
}

impl AnthropicAdapterError {
    pub fn class(&self) -> crate::provider_adapter::ProviderErrorClass {
        match self {
            Self::InvalidRequest { .. } => {
                crate::provider_adapter::ProviderErrorClass::InvalidRequest
            }
            Self::Authentication => crate::provider_adapter::ProviderErrorClass::Authentication,
            Self::Permission => crate::provider_adapter::ProviderErrorClass::Permission,
            Self::RateLimit => crate::provider_adapter::ProviderErrorClass::RateLimit,
            Self::Timeout => crate::provider_adapter::ProviderErrorClass::Timeout,
            Self::Transport => crate::provider_adapter::ProviderErrorClass::Transport,
            Self::Upstream => crate::provider_adapter::ProviderErrorClass::Upstream,
            Self::ResponseParse | Self::StreamingUnsupported => {
                crate::provider_adapter::ProviderErrorClass::ResponseParse
            }
        }
    }
}

#[derive(Debug)]
pub struct AnthropicAdapter<T: AnthropicTransport> {
    provider_id: String,
    base_url: String,
    transport: T,
    auth_ref: Option<String>,
    model_id: String,
    timeout_ms: u64,
    max_tokens: u32,
}

impl<T: AnthropicTransport> AnthropicAdapter<T> {
    pub fn new(
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        transport: T,
        auth_ref: Option<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            transport,
            auth_ref,
            model_id: String::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn auth_ref(&self) -> Option<&str> {
        self.auth_ref.as_deref()
    }

    pub fn complete(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, AnthropicAdapterError> {
        let auth_ref = request.auth_ref.clone().or_else(|| self.auth_ref.clone());
        let body = self.prepare_request(&request)?;
        let headers = AnthropicHeaders {
            api_key_ref: auth_ref,
            anthropic_version: ANTHROPIC_VERSION.to_owned(),
        };
        let response = self
            .transport
            .send(
                &self.endpoint(),
                &headers,
                &body,
                Duration::from_millis(self.timeout_ms),
                false,
            )
            .map_err(map_transport_error)?;
        if !(200..300).contains(&response.status_code) {
            return Err(classify_status(response.status_code));
        }
        if response
            .body
            .lines()
            .any(|line| line.trim_start().starts_with("data:"))
        {
            return Err(AnthropicAdapterError::StreamingUnsupported);
        }
        let parsed = parse_response(&response.body)?;
        if parsed.content.len() != 1
            || parsed.content[0].kind != "text"
            || parsed.content[0].text.trim().is_empty()
        {
            return Err(AnthropicAdapterError::ResponseParse);
        }
        let usage = parsed.usage.map(|usage| ProviderUsage {
            prompt_tokens: Some(usage.input_tokens),
            completion_tokens: Some(usage.output_tokens),
            total_tokens: usage.input_tokens.checked_add(usage.output_tokens),
        });
        Ok(ProviderResponse {
            request_id: parsed.id,
            model: parsed.model,
            text: parsed.content[0].text.clone(),
            finish_reason: parsed.stop_reason,
            usage,
        })
    }

    pub fn input_hash(&self, request: &ProviderRequest) -> Result<String, AnthropicAdapterError> {
        let body = self.prepare_request(request)?;
        Ok(format!("sha256:{:x}", Sha256::digest(body.as_bytes())))
    }

    fn prepare_request(&self, request: &ProviderRequest) -> Result<String, AnthropicAdapterError> {
        if self.provider_id.trim().is_empty() || self.base_url.trim().is_empty() {
            return Err(AnthropicAdapterError::InvalidRequest {
                reason: "provider identity and endpoint must not be empty",
            });
        }
        if request.stream {
            return Err(AnthropicAdapterError::StreamingUnsupported);
        }
        if request.model.trim().is_empty() {
            return Err(AnthropicAdapterError::InvalidRequest {
                reason: "model must not be empty",
            });
        }
        if request.messages.is_empty() {
            return Err(AnthropicAdapterError::InvalidRequest {
                reason: "messages must not be empty",
            });
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(AnthropicAdapterError::InvalidRequest {
                reason: "timeout_ms is outside the allowed boundary",
            });
        }
        validate_auth_ref(self.auth_ref.as_deref())?;
        validate_auth_ref(request.auth_ref.as_deref())?;

        let mut system_parts = Vec::new();
        let mut messages = Vec::new();
        for message in &request.messages {
            match message.role.as_str() {
                "system" | "developer" => {
                    system_parts.push(Redactor::default().redact_text(&message.content).map_err(
                        |_| AnthropicAdapterError::InvalidRequest {
                            reason: "message content redaction failed",
                        },
                    )?)
                }
                "user" | "assistant" => messages.push(AnthropicMessage {
                    role: message.role.clone(),
                    content: vec![AnthropicTextBlock {
                        kind: "text".into(),
                        text: Redactor::default()
                            .redact_text(&message.content)
                            .map_err(|_| AnthropicAdapterError::InvalidRequest {
                                reason: "message content redaction failed",
                            })?,
                    }],
                }),
                _ => {
                    return Err(AnthropicAdapterError::InvalidRequest {
                        reason: "message role is unsupported",
                    })
                }
            }
        }
        if messages.is_empty() {
            return Err(AnthropicAdapterError::InvalidRequest {
                reason: "messages must include a user or assistant message",
            });
        }
        let wire = AnthropicRequest {
            model: request.model.clone(),
            max_tokens: self.max_tokens,
            system: (!system_parts.is_empty()).then(|| system_parts.join("\n")),
            messages,
            stream: false,
        };
        serde_json::to_string(&wire).map_err(|_| AnthropicAdapterError::ResponseParse)
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{base}/messages")
        } else {
            format!("{base}/v1/messages")
        }
    }
}

impl<T: AnthropicTransport> ResolvedModelAdapter for AnthropicAdapter<T> {
    fn complete_resolved_with_action(
        &self,
        request: ModelRequest,
        pack: &ResolvedPromptPack,
    ) -> Result<ResolvedModelResponseWithAction, ModelAdapterError> {
        if pack.endpoint_protocol() != "anthropic" {
            return Err(ModelAdapterError::ResolvedBindingMismatch {
                field: "endpoint_protocol",
            });
        }
        for (actual, expected, field) in [
            (self.provider_id.as_str(), pack.provider_id(), "provider_id"),
            (
                self.base_url.trim_end_matches('/'),
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
        let provider_request = ProviderRequest {
            model: pack.model_name().to_owned(),
            messages: pack.messages().to_vec(),
            auth_ref: self.auth_ref.clone(),
            timeout_ms: self.timeout_ms,
            stream: false,
        };
        let input_context_hash =
            self.input_hash(&provider_request)
                .map_err(|error| ModelAdapterError::Provider {
                    class: error.class(),
                })?;
        let provider_response =
            self.complete(provider_request)
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
}

impl<T: AnthropicTransport> ModelAdapter for AnthropicAdapter<T> {
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelAdapterError> {
        if request.model_id.trim().is_empty() {
            return Err(ModelAdapterError::EmptyModelId);
        }
        if request.objective.trim().is_empty() {
            return Err(ModelAdapterError::EmptyObjective);
        }
        let response = self
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

    fn complete_with_action(
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
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicTextBlock>,
}

#[derive(Debug, Serialize)]
struct AnthropicTextBlock {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnthropicResponse {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    role: String,
    model: String,
    content: Vec<AnthropicResponseBlock>,
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnthropicResponseBlock {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionEnvelope {
    decision: ModelDecision,
    summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionEnvelopeWithAction {
    decision: ModelDecision,
    summary: String,
    #[serde(default)]
    action: Option<ModelActionEnvelope>,
}

fn parse_response(body: &str) -> Result<AnthropicResponse, AnthropicAdapterError> {
    let response = serde_json::from_str::<AnthropicResponse>(body)
        .map_err(|_| AnthropicAdapterError::ResponseParse)?;
    if response.kind != "message" || response.role != "assistant" {
        return Err(AnthropicAdapterError::ResponseParse);
    }
    let _ = response.stop_sequence.as_deref();
    Ok(response)
}

fn parse_decision(text: &str) -> Result<ModelResponse, ModelAdapterError> {
    let envelope = serde_json::from_str::<DecisionEnvelope>(text)
        .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?;
    if envelope.summary.trim().is_empty() {
        return Err(ModelAdapterError::ProviderTextUnmappable);
    }
    Ok(ModelResponse {
        decision: envelope.decision,
        summary: Redactor::default()
            .redact_text(&envelope.summary)
            .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?,
    })
}

fn parse_decision_with_action(text: &str) -> Result<ModelResponseWithAction, ModelAdapterError> {
    let envelope = serde_json::from_str::<DecisionEnvelopeWithAction>(text)
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
        summary: Redactor::default()
            .redact_text(&envelope.summary)
            .map_err(|_| ModelAdapterError::ProviderTextUnmappable)?,
        action: envelope.action,
    })
}

fn validate_auth_ref(auth_ref: Option<&str>) -> Result<(), AnthropicAdapterError> {
    let Some(auth_ref) = auth_ref else {
        return Ok(());
    };
    let valid_prefix = ["env:", "keychain:", "secret-ref:"]
        .iter()
        .any(|prefix| auth_ref.starts_with(prefix));
    let value = auth_ref
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or_default();
    if !valid_prefix
        || auth_ref.len() > 200
        || auth_ref
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        || value.is_empty()
        || value.contains('=')
        || value.starts_with("sk-")
    {
        return Err(AnthropicAdapterError::InvalidRequest {
            reason: "auth_ref must be an opaque credential reference",
        });
    }
    Ok(())
}

fn map_transport_error(error: AnthropicTransportError) -> AnthropicAdapterError {
    match error {
        AnthropicTransportError::Timeout => AnthropicAdapterError::Timeout,
        AnthropicTransportError::Credential => AnthropicAdapterError::Authentication,
        AnthropicTransportError::InvalidEndpoint
        | AnthropicTransportError::UnsupportedStreaming => AnthropicAdapterError::InvalidRequest {
            reason: "transport endpoint or mode is invalid",
        },
        AnthropicTransportError::Network
        | AnthropicTransportError::ResponseTooLarge
        | AnthropicTransportError::InvalidResponse
        | AnthropicTransportError::Redirect
        | AnthropicTransportError::Other => AnthropicAdapterError::Transport,
    }
}

fn classify_status(status_code: u16) -> AnthropicAdapterError {
    match status_code {
        401 => AnthropicAdapterError::Authentication,
        403 => AnthropicAdapterError::Permission,
        408 => AnthropicAdapterError::Timeout,
        429 => AnthropicAdapterError::RateLimit,
        400..=499 => AnthropicAdapterError::InvalidRequest {
            reason: "provider rejected the request",
        },
        _ => AnthropicAdapterError::Upstream,
    }
}
