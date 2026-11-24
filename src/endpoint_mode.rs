use serde::{Deserialize, Serialize};

/// Host-controlled address interpretation; never inferred from a hostname.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMode {
    #[default]
    BaseUrl,
    FullUrl,
}

impl EndpointMode {
    pub(crate) fn is_base_url(&self) -> bool {
        *self == Self::BaseUrl
    }

    pub(crate) fn resolve(self, endpoint: &str, suffix: &str) -> String {
        match self {
            Self::BaseUrl => format!("{}{}", endpoint.trim_end_matches('/'), suffix),
            Self::FullUrl => endpoint.to_owned(),
        }
    }

    pub(crate) fn binding_url(self, endpoint: &str) -> &str {
        match self {
            Self::BaseUrl => endpoint.trim_end_matches('/'),
            Self::FullUrl => endpoint,
        }
    }
}
