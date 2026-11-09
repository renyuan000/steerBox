//! Opt-in private body history; not wired into a provider, CLI, or model-visible tool.
mod private_file;
mod writer;
pub use writer::{DurabilityReceipt, SyncPolicy, WriterError};

use crate::redaction::Redactor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use writer::TimedWriter;

pub const MAX_CONTENT_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptKind {
    UserMessage,
    AssistantChunk,
    AssistantComplete,
    ModelRequest,
    ModelResponseChunk,
    ModelResponseComplete,
}

impl TranscriptKind {
    fn immediate(self) -> bool {
        !matches!(self, Self::AssistantChunk | Self::ModelResponseChunk)
    }

    fn requires_call(self) -> bool {
        matches!(
            self,
            Self::ModelRequest | Self::ModelResponseChunk | Self::ModelResponseComplete
        )
    }
}

/// Trusted callers must approve whole-body/whole-stream safety before submitting chunks.
/// No transport/configuration fields are provided. The store validates content;
/// the string type itself is not proof that a caller supplied safe text.
#[derive(Serialize)]
pub struct TranscriptEntry {
    pub message_id: String,
    pub model_call_id: Option<String>,
    pub kind: TranscriptKind,
    pub content: String,
}

impl fmt::Debug for TranscriptEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranscriptEntry")
            .field("kind", &self.kind)
            .field("content_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum TranscriptError {
    #[error("invalid transcript identity or body")]
    InvalidEntry,
    #[error("transcript body rejected by privacy gate")]
    SensitiveBody,
    #[error("invalid private transcript directory")]
    InvalidRoot,
    #[error("private transcript storage is unsupported on this platform")]
    Unsupported,
    #[error("transcript encoding failed")]
    Encoding,
    #[error(transparent)]
    Writer(#[from] WriterError),
}

/// A fresh-session write-only store. There is deliberately no reopen, read or export API.
pub struct TranscriptStore {
    session_id: String,
    append_lock: Mutex<()>,
    writer: TimedWriter,
}

#[derive(Serialize)]
struct Record<'a> {
    schema_version: u32,
    session_id: &'a str,
    sequence: u64,
    entry: &'a TranscriptEntry,
}

impl TranscriptStore {
    /// Root and workspace must be existing absolute directories. No file is created for
    /// invalid policy/identity. Failures after exclusive creation may leave a fresh file;
    /// callers must not retry by deleting or overwriting it.
    pub fn create(
        root: impl AsRef<Path>,
        workspace: impl AsRef<Path>,
        session_id: &str,
        policy: SyncPolicy,
    ) -> Result<Self, TranscriptError> {
        policy.validate()?;
        if !valid_id(session_id) {
            return Err(TranscriptError::InvalidEntry);
        }
        let file = private_file::create(root.as_ref(), workspace.as_ref(), session_id)?;
        Ok(Self {
            session_id: session_id.into(),
            append_lock: Mutex::new(()),
            writer: TimedWriter::new(file, policy)?,
        })
    }

    pub fn append(&self, entry: TranscriptEntry) -> Result<DurabilityReceipt, TranscriptError> {
        validate_entry(&entry)?;
        // Allocation, encoding and write share this lock: concurrent callers cannot
        // encode the same sequence. The sync worker never allocates sequences.
        let _guard = self
            .append_lock
            .lock()
            .map_err(|_| WriterError::Unavailable)?;
        let sequence = self
            .writer
            .status()?
            .written_sequence
            .checked_add(1)
            .ok_or(WriterError::CapacityExceeded)?;
        let mut encoded = serde_json::to_vec(&Record {
            schema_version: 1,
            session_id: &self.session_id,
            sequence,
            entry: &entry,
        })
        .map_err(|_| TranscriptError::Encoding)?;
        encoded.push(b'\n');
        Ok(self.writer.append(encoded, entry.kind.immediate())?)
    }

    pub fn status(&self) -> Result<DurabilityReceipt, TranscriptError> {
        Ok(self.writer.status()?)
    }

    pub fn sync(&self) -> Result<DurabilityReceipt, TranscriptError> {
        Ok(self.writer.sync()?)
    }

    /// Explicit close joins the timer and reports flush failures; Drop is best effort only.
    pub fn close(self) -> Result<DurabilityReceipt, TranscriptError> {
        Ok(self.writer.close()?)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_entry(entry: &TranscriptEntry) -> Result<(), TranscriptError> {
    if !valid_id(&entry.message_id)
        || entry
            .model_call_id
            .as_deref()
            .is_some_and(|id| !valid_id(id))
        || (entry.kind.requires_call() && entry.model_call_id.is_none())
        || entry.content.len() > MAX_CONTENT_BYTES
    {
        return Err(TranscriptError::InvalidEntry);
    }
    let redactor = Redactor::with_max_input_bytes(MAX_CONTENT_BYTES);
    match serde_json::from_str::<Value>(&entry.content) {
        Ok(original) => {
            // Value alone collapses duplicate keys and could hide sensitive discarded
            // strings. Validate every object for duplicates before semantic comparison.
            serde_json::from_str::<UniqueJson>(&entry.content)
                .map_err(|_| TranscriptError::SensitiveBody)?;
            // Inspect decoded strings too, including a top-level JSON string whose
            // sensitive assignment may be hidden behind Unicode escapes on disk.
            let safe = redactor
                .redact_json_value(&original)
                .map_err(|_| TranscriptError::SensitiveBody)?;
            if original != safe || transport_fields(&original) {
                return Err(TranscriptError::SensitiveBody);
            }
        }
        Err(_) if entry.content.trim_start().starts_with(['{', '[', '"']) => {
            // Malformed, excessively nested or unsupported-number JSON must not
            // bypass decoded-string/transport checks by falling back to raw text.
            return Err(TranscriptError::SensitiveBody);
        }
        Err(_) => {
            let redacted = redactor
                .redact_text(&entry.content)
                .map_err(|_| TranscriptError::SensitiveBody)?;
            if entry.content != redacted {
                return Err(TranscriptError::SensitiveBody);
            }
        }
    }
    Ok(())
}

fn transport_fields(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            let key: String = key
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect();
            matches!(
                key.as_str(),
                "headers"
                    | "auth"
                    | "authref"
                    | "authorization"
                    | "endpoint"
                    | "endpointurl"
                    | "baseurl"
                    | "apikey"
                    | "credential"
                    | "credentials"
                    | "credentialref"
                    | "transportconfig"
            ) || transport_fields(value)
        }),
        Value::Array(values) => values.iter().any(transport_fields),
        _ => false,
    }
}

// A validation-only deserializer. It retains keys only for each current object,
// respects serde_json's recursion limit, and never copies body text into errors.
struct UniqueJson;
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<UniqueJson, A::Error> {
                let mut keys = std::collections::HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                    map.next_value::<UniqueJson>()?;
                }
                Ok(UniqueJson)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<UniqueJson, A::Error> {
                while seq.next_element::<UniqueJson>()?.is_some() {}
                Ok(UniqueJson)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
        }
        d.deserialize_any(Visitor)
    }
}
