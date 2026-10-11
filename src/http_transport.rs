//! A bounded, non-streaming HTTP transport for OpenAI-compatible providers.
//!
//! This module owns network I/O only. Provider request construction and response
//! schema validation remain in `provider_adapter`; credentials are resolved by an
//! injected resolver and are never serialized or included in transport errors.

use crate::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use crate::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct ResolvedCredential(String);

impl ResolvedCredential {
    pub fn bearer(token: impl Into<String>) -> Result<Self, CredentialError> {
        let token = token.into();
        if token.trim().is_empty() || token.chars().any(|character| character.is_control()) {
            return Err(CredentialError::Rejected);
        }
        Ok(Self(token))
    }
}

impl fmt::Debug for ResolvedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResolvedCredential([REDACTED])")
    }
}

pub trait CredentialResolver: Send + Sync {
    fn resolve(&self, auth_ref: &str) -> Result<ResolvedCredential, CredentialError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    Missing,
    Rejected,
    Unavailable,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "credential missing",
            Self::Rejected => "credential rejected",
            Self::Unavailable => "credential unavailable",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpTransportBuildError {
    Client,
}

impl fmt::Display for HttpTransportBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HTTP transport client could not be constructed")
    }
}

pub struct HttpProviderTransport<R: CredentialResolver> {
    client: Client,
    resolver: Arc<R>,
    max_response_bytes: usize,
    allow_loopback_http: bool,
}

impl<R: CredentialResolver> fmt::Debug for HttpProviderTransport<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpProviderTransport")
            .field("max_response_bytes", &self.max_response_bytes)
            .field("allow_loopback_http", &self.allow_loopback_http)
            .finish_non_exhaustive()
    }
}

impl<R: CredentialResolver> HttpProviderTransport<R> {
    pub fn new(resolver: R) -> Result<Self, HttpTransportBuildError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .no_proxy()
            .build()
            .map_err(|_| HttpTransportBuildError::Client)?;
        Ok(Self {
            client,
            resolver: Arc::new(resolver),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            allow_loopback_http: false,
        })
    }

    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes.max(1);
        self
    }

    /// Enable plain HTTP only for explicitly controlled loopback test servers.
    /// This must never be used for a production provider endpoint.
    pub fn allow_loopback_http_for_tests(mut self) -> Self {
        self.allow_loopback_http = true;
        self
    }

    fn validate_endpoint(&self, endpoint: &str) -> Result<reqwest::Url, TransportError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| TransportError::InvalidEndpoint)?;
        if url.username() != "" || url.password().is_some() || url.fragment().is_some() {
            return Err(TransportError::InvalidEndpoint);
        }
        match url.scheme() {
            "https" => Ok(url),
            "http" if self.allow_loopback_http && is_loopback_host(url.host_str()) => Ok(url),
            _ => Err(TransportError::InvalidEndpoint),
        }
    }

    fn read_bounded_body(&self, response: Response) -> Result<String, TransportError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(TransportError::ResponseTooLarge);
        }
        let mut bytes = Vec::with_capacity(self.max_response_bytes.min(64 * 1024));
        let mut limited = response.take(self.max_response_bytes as u64 + 1);
        limited
            .read_to_end(&mut bytes)
            .map_err(|_| TransportError::Network)?;
        if bytes.len() > self.max_response_bytes {
            return Err(TransportError::ResponseTooLarge);
        }
        String::from_utf8(bytes).map_err(|_| TransportError::InvalidResponse)
    }
}

impl<R: CredentialResolver> ProviderTransport for HttpProviderTransport<R> {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        if streaming {
            return Err(TransportError::UnsupportedStreaming);
        }
        if timeout.is_zero() {
            return Err(TransportError::Timeout);
        }
        let endpoint = self.validate_endpoint(endpoint)?;
        let mut request = self
            .client
            .post(endpoint)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .body(request_body.to_owned());
        if let Some(auth_ref) = auth_ref {
            let credential = self
                .resolver
                .resolve(auth_ref)
                .map_err(|_| TransportError::Credential)?;
            request = request.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", credential.0),
            );
        }
        let response = request.send().map_err(|error| {
            if error.is_timeout() {
                TransportError::Timeout
            } else {
                TransportError::Network
            }
        })?;
        let status_code = response.status().as_u16();
        if (300..400).contains(&status_code) {
            return Err(TransportError::Redirect);
        }
        let body = self.read_bounded_body(response)?;
        Ok(TransportResponse::body(status_code, body))
    }
}

/// A bounded, non-streaming HTTP transport for Anthropic's x-api-key protocol.
/// Credential lookup remains injected and the resolved secret never enters a
/// public error, debug, or audit value.
pub struct HttpAnthropicTransport<R: CredentialResolver> {
    client: Client,
    resolver: Arc<R>,
    max_response_bytes: usize,
    allow_loopback_http: bool,
}

impl<R: CredentialResolver> fmt::Debug for HttpAnthropicTransport<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpAnthropicTransport")
            .field("max_response_bytes", &self.max_response_bytes)
            .field("allow_loopback_http", &self.allow_loopback_http)
            .finish_non_exhaustive()
    }
}

impl<R: CredentialResolver> HttpAnthropicTransport<R> {
    pub fn new(resolver: R) -> Result<Self, HttpTransportBuildError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .no_proxy()
            .build()
            .map_err(|_| HttpTransportBuildError::Client)?;
        Ok(Self {
            client,
            resolver: Arc::new(resolver),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            allow_loopback_http: false,
        })
    }

    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes.max(1);
        self
    }

    pub fn allow_loopback_http_for_tests(mut self) -> Self {
        self.allow_loopback_http = true;
        self
    }

    fn validate_endpoint(&self, endpoint: &str) -> Result<reqwest::Url, AnthropicTransportError> {
        let url =
            reqwest::Url::parse(endpoint).map_err(|_| AnthropicTransportError::InvalidEndpoint)?;
        if url.username() != "" || url.password().is_some() || url.fragment().is_some() {
            return Err(AnthropicTransportError::InvalidEndpoint);
        }
        match url.scheme() {
            "https" => Ok(url),
            "http" if self.allow_loopback_http && is_loopback_host(url.host_str()) => Ok(url),
            _ => Err(AnthropicTransportError::InvalidEndpoint),
        }
    }

    fn read_bounded_body(&self, response: Response) -> Result<String, AnthropicTransportError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(AnthropicTransportError::ResponseTooLarge);
        }
        let mut bytes = Vec::with_capacity(self.max_response_bytes.min(64 * 1024));
        let mut limited = response.take(self.max_response_bytes as u64 + 1);
        limited
            .read_to_end(&mut bytes)
            .map_err(|_| AnthropicTransportError::Network)?;
        if bytes.len() > self.max_response_bytes {
            return Err(AnthropicTransportError::ResponseTooLarge);
        }
        String::from_utf8(bytes).map_err(|_| AnthropicTransportError::InvalidResponse)
    }
}

impl<R: CredentialResolver> AnthropicTransport for HttpAnthropicTransport<R> {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        request_body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        if streaming {
            return Err(AnthropicTransportError::UnsupportedStreaming);
        }
        if timeout.is_zero() {
            return Err(AnthropicTransportError::Timeout);
        }
        let endpoint = self.validate_endpoint(endpoint)?;
        let credential = headers
            .api_key_ref
            .as_deref()
            .ok_or(AnthropicTransportError::Credential)
            .and_then(|auth_ref| {
                self.resolver
                    .resolve(auth_ref)
                    .map_err(|_| AnthropicTransportError::Credential)
            })?;
        let request = self
            .client
            .post(endpoint)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::HeaderName::from_static("x-api-key"),
                credential.0,
            )
            .header(
                reqwest::header::HeaderName::from_static("anthropic-version"),
                headers.anthropic_version.as_str(),
            )
            .body(request_body.to_owned());
        let response = request.send().map_err(|error| {
            if error.is_timeout() {
                AnthropicTransportError::Timeout
            } else {
                AnthropicTransportError::Network
            }
        })?;
        let status_code = response.status().as_u16();
        if (300..400).contains(&status_code) {
            return Err(AnthropicTransportError::Redirect);
        }
        let body = self.read_bounded_body(response)?;
        Ok(TransportResponse::body(status_code, body))
    }
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost" | "127.0.0.1" | "[::1]" | "::1"))
}
