use crate::model_contracts::{ModelCallRecord, ModelContractError};
use serde_json::Error as JsonError;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelCallStoreError {
    #[error("model call store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model call line {line} is invalid JSON: {source}")]
    InvalidJson { line: usize, source: JsonError },
    #[error("invalid model call contract: {0}")]
    InvalidContract(#[from] ModelContractError),
    #[error("invalid model call contract at line {line}: {source}")]
    InvalidContractAtLine {
        line: usize,
        source: ModelContractError,
    },
    #[error("model call record serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct ModelCallStore {
    path: PathBuf,
}

impl ModelCallStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ModelCallStoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }

    /// Validate identifiers, cost and field schema before serialization or log
    /// creation. This does not scan arbitrary string values for credentials.
    pub fn append(&self, record: &ModelCallRecord) -> Result<(), ModelCallStoreError> {
        record.validate()?;
        let encoded = serde_json::to_string(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{encoded}")?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let reader = BufReader::new(File::open(&self.path)?);
        reader
            .lines()
            .enumerate()
            .filter_map(|(index, line)| match line {
                Ok(line) if line.trim().is_empty() => None,
                Ok(line) => Some(
                    serde_json::from_str::<ModelCallRecord>(&line)
                        .map_err(|source| ModelCallStoreError::InvalidJson {
                            line: index + 1,
                            source,
                        })
                        .and_then(|record| {
                            record.validate().map_err(|source| {
                                ModelCallStoreError::InvalidContractAtLine {
                                    line: index + 1,
                                    source,
                                }
                            })?;
                            Ok(record)
                        }),
                ),
                Err(error) => Some(Err(ModelCallStoreError::Io(error))),
            })
            .collect()
    }
}
