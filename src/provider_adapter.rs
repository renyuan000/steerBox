use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::Duration;
use thiserror::Error;

/// Upper bound applied before a transport is called. The adapter owns this
/// boundary; a concrete HTTP transport may apply a smaller operational limit.
pub const MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderMessage {
    pub role: String,
    pub content: String,
}

impl ProviderMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_owned(),
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_owned(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: content.into(),
        }
    }
}

/// Input accepted by a provider adapter. `auth_ref` is intentionally omitted
/// from serialized provider JSON: it is only an opaque lookup reference that a
/// transport may use to resolve credentials outside the runtime contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub model: String,
    pub messages: Vec<ProviderMessage>,
    #[serde(skip)]
    pub auth_ref: Option<String>,
    pub timeout_ms: u64,
    pub stream: bool,
}

/// A validated completion snapshot. The hash covers the exact body sent to the
/// transport, not auth references, headers, or operational timeout settings.
#[derive(Clone)]
pub struct PreparedProviderRequest {
    request: ProviderRequest,
    body: String,
    input_hash: String,
    provider_id: String,
    base_url: String,
}

impl PreparedProviderRequest {
    pub fn input_hash(&self) -> &str {
        &self.input_hash
    }
}

impl fmt::Debug for PreparedProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedProviderRequest")
            .field("input_hash", &self.input_hash)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    pub request_id: String,
    pub model: String,
    pub text: String,
    pub finish_reason: Option<String>,
    pub usage: Option<ProviderUsage>,
}

impl ProviderResponse {
    pub fn text(
        request_id: impl Into<String>,
        model: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            model: model.into(),
            text: text.into(),
            finish_reason: None,
            usage: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
pub enum ProviderStreamEvent {
    Delta {
        request_id: String,
        model: String,
        text: String,
    },
    Completed {
        request_id: String,
        model: String,
        finish_reason: String,
    },
}

impl ProviderStreamEvent {
    pub fn delta(
        request_id: impl Into<String>,
        model: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self::Delta {
            request_id: request_id.into(),
            model: model.into(),
            text: text.into(),
        }
    }

    pub fn completed(
        request_id: impl Into<String>,
        model: impl Into<String>,
        finish_reason: impl Into<String>,
    ) -> Self {
        Self::Completed {
            request_id: request_id.into(),
            model: model.into(),
            finish_reason: finish_reason.into(),
        }
    }
}

/// The only boundary an adapter needs from an HTTP or local transport.
/// Implementations own credential lookup and network I/O; this crate does not
/// read, print, or persist credential material.
pub trait ProviderTransport {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportResponse {
    pub status_code: u16,
    pub body: String,
}

impl TransportResponse {
    pub fn body(status_code: u16, body: impl Into<String>) -> Self {
        Self {
            status_code,
            body: body.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    Network,
    Other,
}

impl TransportError {
    pub fn timeout(_message: impl AsRef<str>) -> Self {
        Self::Timeout
    }

    pub fn network(_message: impl AsRef<str>) -> Self {
        Self::Network
    }

    pub fn other(_message: impl AsRef<str>) -> Self {
        Self::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorClass {
    InvalidRequest,
    Authentication,
    Permission,
    RateLimit,
    Timeout,
    Transport,
    Upstream,
    ResponseParse,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProviderAdapterError {
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
}

impl ProviderAdapterError {
    pub fn class(&self) -> ProviderErrorClass {
        match self {
            Self::InvalidRequest { .. } => ProviderErrorClass::InvalidRequest,
            Self::Authentication => ProviderErrorClass::Authentication,
            Self::Permission => ProviderErrorClass::Permission,
            Self::RateLimit => ProviderErrorClass::RateLimit,
            Self::Timeout => ProviderErrorClass::Timeout,
            Self::Transport => ProviderErrorClass::Transport,
            Self::Upstream => ProviderErrorClass::Upstream,
            Self::ResponseParse => ProviderErrorClass::ResponseParse,
        }
    }
}

#[derive(Debug)]
pub struct OpenAiCompatibleAdapter<T: ProviderTransport> {
    provider_id: String,
    base_url: String,
    transport: T,
}

impl<T: ProviderTransport> OpenAiCompatibleAdapter<T> {
    pub fn new(provider_id: impl Into<String>, base_url: impl Into<String>, transport: T) -> Self {
        Self {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            transport,
        }
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn prepare_complete(
        &self,
        request: ProviderRequest,
    ) -> Result<PreparedProviderRequest, ProviderAdapterError> {
        if self.provider_id.trim().is_empty() || self.base_url.trim().is_empty() {
            return Err(ProviderAdapterError::InvalidRequest {
                reason: "provider identity and endpoint must not be empty",
            });
        }
        if request.stream {
            return Err(ProviderAdapterError::InvalidRequest {
                reason: "complete request must set stream=false",
            });
        }
        let body = self.prepare_request(&request, false)?;
        let input_hash = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
        Ok(PreparedProviderRequest {
            request,
            body,
            input_hash,
            provider_id: self.provider_id.clone(),
            base_url: self.base_url.trim_end_matches('/').to_owned(),
        })
    }

    pub fn complete_prepared(
        &self,
        prepared: &PreparedProviderRequest,
    ) -> Result<ProviderResponse, ProviderAdapterError> {
        if prepared.provider_id != self.provider_id
            || prepared.base_url != self.base_url.trim_end_matches('/')
        {
            return Err(ProviderAdapterError::InvalidRequest {
                reason: "prepared request belongs to a different provider or endpoint",
            });
        }
        let response = self.send(&prepared.request, &prepared.body, false)?;
        parse_completion_response(&response.body)
    }

    pub fn complete(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderAdapterError> {
        let prepared = self.prepare_complete(request)?;
        self.complete_prepared(&prepared)
    }

    pub fn stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Vec<ProviderStreamEvent>, ProviderAdapterError> {
        if !request.stream {
            return Err(ProviderAdapterError::InvalidRequest {
                reason: "stream request must set stream=true",
            });
        }
        let body = self.prepare_request(&request, true)?;
        let response = self.send(&request, &body, true)?;
        parse_stream_response(&response.body)
    }

    fn prepare_request(
        &self,
        request: &ProviderRequest,
        stream: bool,
    ) -> Result<String, ProviderAdapterError> {
        validate_request(request)?;
        let wire_request = WireRequest {
            model: &request.model,
            messages: &request.messages,
            stream,
        };
        serde_json::to_string(&wire_request).map_err(|_| ProviderAdapterError::ResponseParse)
    }

    fn send(
        &self,
        request: &ProviderRequest,
        body: &str,
        streaming: bool,
    ) -> Result<TransportResponse, ProviderAdapterError> {
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .transport
            .send(
                &endpoint,
                request.auth_ref.as_deref(),
                body,
                Duration::from_millis(request.timeout_ms),
                streaming,
            )
            .map_err(|error| match error {
                TransportError::Timeout => ProviderAdapterError::Timeout,
                TransportError::Network => ProviderAdapterError::Transport,
                TransportError::Other => ProviderAdapterError::Transport,
            })?;
        if !(200..300).contains(&response.status_code) {
            return Err(match response.status_code {
                401 => ProviderAdapterError::Authentication,
                403 => ProviderAdapterError::Permission,
                408 => ProviderAdapterError::Timeout,
                429 => ProviderAdapterError::RateLimit,
                400..=499 => ProviderAdapterError::InvalidRequest {
                    reason: "provider rejected the request",
                },
                _ => ProviderAdapterError::Upstream,
            });
        }
        Ok(response)
    }
}

#[derive(Debug, Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: &'a [ProviderMessage],
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct WireCompletionResponse {
    id: String,
    model: String,
    choices: Vec<WireChoice>,
    usage: Option<ProviderUsage>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    message: Option<WireMessage>,
    delta: Option<WireMessage>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireMessage {
    content: Option<String>,
}

fn validate_request(request: &ProviderRequest) -> Result<(), ProviderAdapterError> {
    if request.model.trim().is_empty() {
        return Err(ProviderAdapterError::InvalidRequest {
            reason: "model must not be empty",
        });
    }
    if request.messages.is_empty() {
        return Err(ProviderAdapterError::InvalidRequest {
            reason: "messages must not be empty",
        });
    }
    if request.timeout_ms == 0 || request.timeout_ms > MAX_TIMEOUT_MS {
        return Err(ProviderAdapterError::InvalidRequest {
            reason: "timeout_ms is outside the allowed boundary",
        });
    }
    if let Some(auth_ref) = request.auth_ref.as_deref() {
        validate_auth_ref(auth_ref)?;
    }
    Ok(())
}

fn validate_auth_ref(auth_ref: &str) -> Result<(), ProviderAdapterError> {
    let valid_prefix = ["env:", "keychain:", "secret-ref:"]
        .iter()
        .any(|prefix| auth_ref.starts_with(prefix));
    if !valid_prefix
        || auth_ref.len() > 200
        || auth_ref
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(ProviderAdapterError::InvalidRequest {
            reason: "auth_ref must be an opaque credential reference",
        });
    }
    let value = auth_ref
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or_default();
    if value.is_empty() || value.contains('=') || value.starts_with("sk-") {
        return Err(ProviderAdapterError::InvalidRequest {
            reason: "auth_ref must not contain credential material",
        });
    }
    Ok(())
}

fn parse_completion_response(body: &str) -> Result<ProviderResponse, ProviderAdapterError> {
    let response = serde_json::from_str::<WireCompletionResponse>(body)
        .map_err(|_| ProviderAdapterError::ResponseParse)?;
    let choice = response
        .choices
        .first()
        .ok_or(ProviderAdapterError::ResponseParse)?;
    let text = choice
        .message
        .as_ref()
        .and_then(|message| message.content.as_deref())
        .ok_or(ProviderAdapterError::ResponseParse)?;
    Ok(ProviderResponse {
        request_id: response.id,
        model: response.model,
        text: text.to_owned(),
        finish_reason: choice.finish_reason.clone(),
        usage: response.usage,
    })
}

fn parse_stream_response(body: &str) -> Result<Vec<ProviderStreamEvent>, ProviderAdapterError> {
    let mut events = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let data = line
            .strip_prefix("data:")
            .map(str::trim)
            .ok_or(ProviderAdapterError::ResponseParse)?;
        if data == "[DONE]" {
            break;
        }
        let chunk = serde_json::from_str::<WireCompletionResponse>(data)
            .map_err(|_| ProviderAdapterError::ResponseParse)?;
        let choice = chunk
            .choices
            .first()
            .ok_or(ProviderAdapterError::ResponseParse)?;
        if let Some(text) = choice
            .delta
            .as_ref()
            .and_then(|message| message.content.as_deref())
        {
            if !text.is_empty() {
                events.push(ProviderStreamEvent::delta(&chunk.id, &chunk.model, text));
            }
        }
        if let Some(reason) = choice.finish_reason.as_deref() {
            events.push(ProviderStreamEvent::completed(
                &chunk.id,
                &chunk.model,
                reason,
            ));
        }
    }
    if events.is_empty() {
        return Err(ProviderAdapterError::ResponseParse);
    }
    Ok(events)
}
