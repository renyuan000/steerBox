use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptBinding {
    pub role: String,
    pub task: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptProfile {
    pub prompt_id: String,
    pub version: u32,
    pub binding: PromptBinding,
    pub template: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptRegistryError {
    #[error("prompt_id must not be empty")]
    EmptyPromptId,
    #[error("prompt version must be greater than zero")]
    InvalidVersion,
    #[error("prompt binding field must not be empty: {field}")]
    EmptyBinding { field: &'static str },
    #[error("prompt template must not be empty")]
    EmptyTemplate,
    #[error("prompt already registered: {prompt_id} v{version}")]
    DuplicatePrompt { prompt_id: String, version: u32 },
    #[error("prompt JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("prompt map key does not match its profile id")]
    MapIdMismatch,
    #[error("prompt version map key does not match its profile version")]
    MapVersionMismatch,
}

impl PromptProfile {
    pub fn new(
        prompt_id: impl Into<String>,
        version: u32,
        role: impl Into<String>,
        task: impl Into<String>,
        template: impl Into<String>,
    ) -> Result<Self, PromptRegistryError> {
        let profile = Self {
            prompt_id: prompt_id.into(),
            version,
            binding: PromptBinding {
                role: role.into(),
                task: task.into(),
            },
            template: template.into(),
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn from_json_str(contents: &str) -> Result<Self, PromptRegistryError> {
        let profile = serde_json::from_str::<Self>(contents)
            .map_err(|error| PromptRegistryError::InvalidJson(error.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), PromptRegistryError> {
        if self.prompt_id.trim().is_empty() {
            return Err(PromptRegistryError::EmptyPromptId);
        }
        if self.version == 0 {
            return Err(PromptRegistryError::InvalidVersion);
        }
        if self.binding.role.trim().is_empty() {
            return Err(PromptRegistryError::EmptyBinding { field: "role" });
        }
        if self.binding.task.trim().is_empty() {
            return Err(PromptRegistryError::EmptyBinding { field: "task" });
        }
        if self.template.trim().is_empty() {
            return Err(PromptRegistryError::EmptyTemplate);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptRegistry {
    profiles: BTreeMap<String, BTreeMap<u32, PromptProfile>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPromptRegistry {
    profiles: BTreeMap<String, BTreeMap<u32, PromptProfile>>,
}

impl<'de> Deserialize<'de> for PromptRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPromptRegistry::deserialize(deserializer)?;
        let mut registry = Self::default();
        for (id, versions) in raw.profiles {
            if id.trim().is_empty() {
                return Err(serde::de::Error::custom(PromptRegistryError::EmptyPromptId));
            }
            for (version, profile) in versions {
                if id != profile.prompt_id {
                    return Err(serde::de::Error::custom(PromptRegistryError::MapIdMismatch));
                }
                if version != profile.version {
                    return Err(serde::de::Error::custom(
                        PromptRegistryError::MapVersionMismatch,
                    ));
                }
                registry
                    .register(profile)
                    .map_err(serde::de::Error::custom)?;
            }
        }
        Ok(registry)
    }
}

impl PromptRegistry {
    pub fn register(&mut self, profile: PromptProfile) -> Result<(), PromptRegistryError> {
        profile.validate()?;
        let versions = self.profiles.entry(profile.prompt_id.clone()).or_default();
        if versions.contains_key(&profile.version) {
            return Err(PromptRegistryError::DuplicatePrompt {
                prompt_id: profile.prompt_id,
                version: profile.version,
            });
        }
        versions.insert(profile.version, profile);
        Ok(())
    }

    pub fn get(&self, prompt_id: &str, version: u32) -> Option<&PromptProfile> {
        self.profiles.get(prompt_id)?.get(&version)
    }

    pub fn latest_for(&self, prompt_id: &str, role: &str, task: &str) -> Option<&PromptProfile> {
        self.profiles
            .get(prompt_id)?
            .values()
            .rev()
            .find(|profile| profile.binding.role == role && profile.binding.task == task)
    }

    pub fn profiles(&self) -> impl Iterator<Item = &PromptProfile> {
        self.profiles.values().flat_map(BTreeMap::values)
    }
}
