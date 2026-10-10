use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub provider_id: String,
    pub display_name: String,
    #[serde(default)]
    pub auth_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EndpointProfile {
    pub endpoint_id: String,
    pub provider_id: String,
    pub base_url: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    pub model_id: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub model_name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelRegistryError {
    #[error("{entity} id must not be empty")]
    EmptyId { entity: &'static str },
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("provider already registered: {0}")]
    DuplicateProvider(String),
    #[error("endpoint already registered: {0}")]
    DuplicateEndpoint(String),
    #[error("model already registered: {0}")]
    DuplicateModel(String),
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("endpoint not found: {0}")]
    EndpointNotFound(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("endpoint provider does not match its registered provider")]
    EndpointProviderMismatch,
    #[error("{entity} map key does not match its profile id")]
    MapIdMismatch { entity: &'static str },
}

impl ProviderProfile {
    pub fn new(
        provider_id: impl Into<String>,
        display_name: impl Into<String>,
        auth_ref: Option<String>,
    ) -> Result<Self, ModelRegistryError> {
        let profile = Self {
            provider_id: provider_id.into(),
            display_name: display_name.into(),
            auth_ref,
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), ModelRegistryError> {
        validate_id(&self.provider_id, "provider")?;
        validate_field(&self.display_name, "display_name")?;
        if let Some(auth_ref) = &self.auth_ref {
            validate_field(auth_ref, "auth_ref")?;
        }
        Ok(())
    }
}

impl EndpointProfile {
    pub fn new(
        endpoint_id: impl Into<String>,
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        protocol: impl Into<String>,
    ) -> Result<Self, ModelRegistryError> {
        let profile = Self {
            endpoint_id: endpoint_id.into(),
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            protocol: protocol.into(),
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), ModelRegistryError> {
        validate_id(&self.endpoint_id, "endpoint")?;
        validate_id(&self.provider_id, "provider")?;
        validate_field(&self.base_url, "base_url")?;
        validate_field(&self.protocol, "protocol")
    }
}

impl ModelProfile {
    pub fn new(
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        model_name: impl Into<String>,
        capabilities: Vec<String>,
    ) -> Result<Self, ModelRegistryError> {
        let profile = Self {
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            endpoint_id: endpoint_id.into(),
            model_name: model_name.into(),
            capabilities,
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), ModelRegistryError> {
        validate_id(&self.model_id, "model")?;
        validate_id(&self.provider_id, "provider")?;
        validate_id(&self.endpoint_id, "endpoint")?;
        validate_field(&self.model_name, "model_name")
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelRegistry {
    providers: BTreeMap<String, ProviderProfile>,
    endpoints: BTreeMap<String, EndpointProfile>,
    models: BTreeMap<String, ModelProfile>,
}

// Preserve the persisted map layout while routing decoded profiles through the
// same validation and association checks as programmatic registration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModelRegistry {
    providers: BTreeMap<String, ProviderProfile>,
    endpoints: BTreeMap<String, EndpointProfile>,
    models: BTreeMap<String, ModelProfile>,
}

impl<'de> Deserialize<'de> for ModelRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawModelRegistry::deserialize(deserializer)?;
        let mut registry = Self::default();
        for (id, profile) in raw.providers {
            if id != profile.provider_id {
                return Err(serde::de::Error::custom(
                    ModelRegistryError::MapIdMismatch { entity: "provider" },
                ));
            }
            registry
                .register_provider(profile)
                .map_err(serde::de::Error::custom)?;
        }
        for (id, profile) in raw.endpoints {
            if id != profile.endpoint_id {
                return Err(serde::de::Error::custom(
                    ModelRegistryError::MapIdMismatch { entity: "endpoint" },
                ));
            }
            registry
                .register_endpoint(profile)
                .map_err(serde::de::Error::custom)?;
        }
        for (id, profile) in raw.models {
            if id != profile.model_id {
                return Err(serde::de::Error::custom(
                    ModelRegistryError::MapIdMismatch { entity: "model" },
                ));
            }
            registry
                .register_model(profile)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(registry)
    }
}

impl ModelRegistry {
    pub fn register_provider(
        &mut self,
        profile: ProviderProfile,
    ) -> Result<(), ModelRegistryError> {
        profile.validate()?;
        if self.providers.contains_key(&profile.provider_id) {
            return Err(ModelRegistryError::DuplicateProvider(profile.provider_id));
        }
        self.providers.insert(profile.provider_id.clone(), profile);
        Ok(())
    }

    pub fn register_endpoint(
        &mut self,
        profile: EndpointProfile,
    ) -> Result<(), ModelRegistryError> {
        profile.validate()?;
        if !self.providers.contains_key(&profile.provider_id) {
            return Err(ModelRegistryError::ProviderNotFound(profile.provider_id));
        }
        if self.endpoints.contains_key(&profile.endpoint_id) {
            return Err(ModelRegistryError::DuplicateEndpoint(profile.endpoint_id));
        }
        self.endpoints.insert(profile.endpoint_id.clone(), profile);
        Ok(())
    }

    pub fn register_model(&mut self, profile: ModelProfile) -> Result<(), ModelRegistryError> {
        profile.validate()?;
        if !self.providers.contains_key(&profile.provider_id) {
            return Err(ModelRegistryError::ProviderNotFound(profile.provider_id));
        }
        let endpoint = self
            .endpoints
            .get(&profile.endpoint_id)
            .ok_or_else(|| ModelRegistryError::EndpointNotFound(profile.endpoint_id.clone()))?;
        if endpoint.provider_id != profile.provider_id {
            return Err(ModelRegistryError::EndpointProviderMismatch);
        }
        if self.models.contains_key(&profile.model_id) {
            return Err(ModelRegistryError::DuplicateModel(profile.model_id));
        }
        self.models.insert(profile.model_id.clone(), profile);
        Ok(())
    }

    pub fn provider(&self, provider_id: &str) -> Option<&ProviderProfile> {
        self.providers.get(provider_id)
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<&EndpointProfile> {
        self.endpoints.get(endpoint_id)
    }

    pub fn model(&self, model_id: &str) -> Option<&ModelProfile> {
        self.models.get(model_id)
    }

    pub fn providers(&self) -> impl Iterator<Item = &ProviderProfile> {
        self.providers.values()
    }

    pub fn endpoints(&self) -> impl Iterator<Item = &EndpointProfile> {
        self.endpoints.values()
    }

    pub fn models(&self) -> impl Iterator<Item = &ModelProfile> {
        self.models.values()
    }
}

fn validate_id(value: &str, entity: &'static str) -> Result<(), ModelRegistryError> {
    if value.trim().is_empty() {
        Err(ModelRegistryError::EmptyId { entity })
    } else {
        Ok(())
    }
}

fn validate_field(value: &str, field: &'static str) -> Result<(), ModelRegistryError> {
    if value.trim().is_empty() {
        Err(ModelRegistryError::EmptyField { field })
    } else {
        Ok(())
    }
}
