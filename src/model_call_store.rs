use crate::model_contracts::{ModelCallFailureRecord, ModelCallRecord, ModelContractError};
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
    #[error("duplicate model call id with different record content")]
    DuplicateModelCallId,
    #[error("model call record serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct ModelCallStore {
    path: PathBuf,
    failure_path: PathBuf,
}

impl ModelCallStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ModelCallStoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let failure_path = path.with_extension("failures.jsonl");
        Ok(Self { path, failure_path })
    }

    /// Return every path used by this store, including the two advisory lock
    /// files. Callers that place the store beside mutable workspace data must
    /// validate all four paths before starting a side-effecting run.
    pub fn storage_paths(&self) -> [PathBuf; 4] {
        let mut main_lock = self.path.as_os_str().to_os_string();
        main_lock.push(".lock");
        let mut failure_lock = self.failure_path.as_os_str().to_os_string();
        failure_lock.push(".lock");
        [
            self.path.clone(),
            self.failure_path.clone(),
            PathBuf::from(main_lock),
            PathBuf::from(failure_lock),
        ]
    }

    /// Validate identifiers, cost and field schema before serialization or log
    /// creation. This does not scan arbitrary string values for credentials.
    pub fn append(&self, record: &ModelCallRecord) -> Result<(), ModelCallStoreError> {
        record.validate()?;
        let _lock = self.acquire_lock()?;
        let mut identical_match = false;
        for existing in self.read_all_unlocked()? {
            if existing.model_call_id == record.model_call_id {
                if existing == *record {
                    identical_match = true;
                    continue;
                }
                return Err(ModelCallStoreError::DuplicateModelCallId);
            }
        }
        if identical_match {
            return Ok(());
        }
        let encoded = serde_json::to_string(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{encoded}")?;
        file.sync_data()?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let _lock = self.acquire_lock()?;
        self.read_all_unlocked()
    }

    /// Append a provider invocation failure without inventing unavailable
    /// response fields or persisting the provider's raw error body.
    pub fn append_failure(
        &self,
        record: &ModelCallFailureRecord,
    ) -> Result<(), ModelCallStoreError> {
        record.validate()?;
        let _lock = self.acquire_failure_lock()?;
        let mut identical_match = false;
        for existing in self.read_failures_unlocked()? {
            if existing.model_call_id == record.model_call_id {
                if existing == *record {
                    identical_match = true;
                    continue;
                }
                return Err(ModelCallStoreError::DuplicateModelCallId);
            }
        }
        if identical_match {
            return Ok(());
        }
        let encoded = serde_json::to_string(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.failure_path)?;
        writeln!(file, "{encoded}")?;
        file.sync_data()?;
        Ok(())
    }

    pub fn read_failures(&self) -> Result<Vec<ModelCallFailureRecord>, ModelCallStoreError> {
        let _lock = self.acquire_failure_lock()?;
        self.read_failures_unlocked()
    }

    fn read_failures_unlocked(&self) -> Result<Vec<ModelCallFailureRecord>, ModelCallStoreError> {
        if !self.failure_path.exists() {
            return Ok(Vec::new());
        }
        let reader = BufReader::new(File::open(&self.failure_path)?);
        reader
            .lines()
            .enumerate()
            .filter_map(|(index, line)| match line {
                Ok(line) if line.trim().is_empty() => None,
                Ok(line) => Some(
                    serde_json::from_str::<ModelCallFailureRecord>(&line)
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

    fn read_all_unlocked(&self) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
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

    /// Read a bounded page of non-blank JSONL records without loading the full log.
    pub fn read_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let _lock = self.acquire_lock()?;
        self.read_page_unlocked(offset, limit)
    }

    fn read_page_unlocked(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let mut records = Vec::new();
        if limit == 0 || !self.path.exists() {
            return Ok(records);
        }

        let reader = BufReader::new(File::open(&self.path)?);
        let mut record_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line.map_err(ModelCallStoreError::Io)?;
            if line.trim().is_empty() {
                continue;
            }

            let record = serde_json::from_str::<ModelCallRecord>(&line).map_err(|source| {
                ModelCallStoreError::InvalidJson {
                    line: line_index + 1,
                    source,
                }
            })?;
            record
                .validate()
                .map_err(|source| ModelCallStoreError::InvalidContractAtLine {
                    line: line_index + 1,
                    source,
                })?;

            if record_index >= offset {
                records.push(record);
                if records.len() >= limit {
                    break;
                }
            }
            record_index += 1;
        }

        Ok(records)
    }

    fn acquire_lock(&self) -> Result<File, ModelCallStoreError> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))?;
        lock_file.lock()?;
        Ok(lock_file)
    }

    fn acquire_failure_lock(&self) -> Result<File, ModelCallStoreError> {
        let mut lock_path = self.failure_path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))?;
        lock_file.lock()?;
        Ok(lock_file)
    }
}
