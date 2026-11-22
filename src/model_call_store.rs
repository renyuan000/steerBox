use crate::audit_store::AuditParent;
use crate::model_contracts::{ModelCallFailureRecord, ModelCallRecord, ModelContractError};
use serde_json::Error as JsonError;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
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
        AuditParent::open(&path, true)?
            .ok_or_else(|| model_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        let failure_path = Self::storage_paths_for(&path)[1].clone();
        Ok(Self { path, failure_path })
    }

    /// Return every path used by this store, including the two advisory lock
    /// files. Callers that place the store beside mutable workspace data must
    /// validate all four paths before starting a side-effecting run.
    pub fn storage_paths(&self) -> [PathBuf; 4] {
        Self::storage_paths_for(&self.path)
    }

    /// Derive success, failure, and advisory-lock names without filesystem I/O
    /// or creating parent directories. This does not validate their safety.
    pub fn storage_paths_for(path: impl AsRef<Path>) -> [PathBuf; 4] {
        let path = path.as_ref();
        let failure_path = path.with_extension("failures.jsonl");
        let mut main_lock = path.as_os_str().to_os_string();
        main_lock.push(".lock");
        let mut failure_lock = failure_path.as_os_str().to_os_string();
        failure_lock.push(".lock");
        [
            path.to_path_buf(),
            failure_path,
            PathBuf::from(main_lock),
            PathBuf::from(failure_lock),
        ]
    }

    /// Validate identifiers, cost and field schema before serialization or log
    /// creation. This does not scan arbitrary string values for credentials.
    pub fn append(&self, record: &ModelCallRecord) -> Result<(), ModelCallStoreError> {
        self.append_with_io(record, &mut ModelStoreDisk)
    }

    fn append_with_io(
        &self,
        record: &ModelCallRecord,
        io: &mut impl ModelStoreIo,
    ) -> Result<(), ModelCallStoreError> {
        record.validate()?;
        let (parent, _lock) = self.acquire_lock(false)?;
        io.after_lock()
            .map_err(|error| model_io_error("locked operation", error))?;
        let history = parent.open_read()?;
        let mut identical_match = false;
        for existing in Self::read_all_unlocked(history.as_ref())? {
            if existing.model_call_id == record.model_call_id {
                if existing == *record {
                    identical_match = true;
                    continue;
                }
                return Err(ModelCallStoreError::DuplicateModelCallId);
            }
        }
        if identical_match {
            // Sync the exact history fd that was validated, including on a
            // retry after readable bytes were left by an uncertain append.
            return confirm_durable(
                history.as_ref().expect("matching history was opened"),
                &parent,
                io,
            );
        }
        let mut encoded = serde_json::to_vec(record)?;
        encoded.push(b'\n');
        let mut file = parent.open_append()?;
        io.write(&mut file, &encoded)
            .map_err(|error| model_io_error("append write", error))?;
        confirm_durable(&file, &parent, io)
    }

    pub fn read_all(&self) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let (parent, _lock) = self.acquire_lock(false)?;
        let file = parent.open_read()?;
        Self::read_all_unlocked(file.as_ref())
    }

    /// Append a provider invocation failure without inventing unavailable
    /// response fields or persisting the provider's raw error body.
    pub fn append_failure(
        &self,
        record: &ModelCallFailureRecord,
    ) -> Result<(), ModelCallStoreError> {
        self.append_failure_with_io(record, &mut ModelStoreDisk)
    }

    fn append_failure_with_io(
        &self,
        record: &ModelCallFailureRecord,
        io: &mut impl ModelStoreIo,
    ) -> Result<(), ModelCallStoreError> {
        record.validate()?;
        let (parent, _lock) = self.acquire_lock(true)?;
        io.after_lock()
            .map_err(|error| model_io_error("locked operation", error))?;
        let history = parent.open_read()?;
        let mut identical_match = false;
        for existing in Self::read_failures_unlocked(history.as_ref())? {
            if existing.model_call_id == record.model_call_id {
                if existing == *record {
                    identical_match = true;
                    continue;
                }
                return Err(ModelCallStoreError::DuplicateModelCallId);
            }
        }
        if identical_match {
            return confirm_durable(
                history.as_ref().expect("matching history was opened"),
                &parent,
                io,
            );
        }
        let mut encoded = serde_json::to_vec(record)?;
        encoded.push(b'\n');
        let mut file = parent.open_append()?;
        io.write(&mut file, &encoded)
            .map_err(|error| model_io_error("append write", error))?;
        confirm_durable(&file, &parent, io)
    }

    pub fn read_failures(&self) -> Result<Vec<ModelCallFailureRecord>, ModelCallStoreError> {
        let (parent, _lock) = self.acquire_lock(true)?;
        let file = parent.open_read()?;
        Self::read_failures_unlocked(file.as_ref())
    }

    fn read_failures_unlocked(
        file: Option<&File>,
    ) -> Result<Vec<ModelCallFailureRecord>, ModelCallStoreError> {
        let Some(file) = file else {
            return Ok(Vec::new());
        };
        let reader = BufReader::new(file);
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
                Err(error) => Some(Err(model_io_error("history read", error).into())),
            })
            .collect()
    }

    fn read_all_unlocked(file: Option<&File>) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let Some(file) = file else {
            return Ok(Vec::new());
        };
        let reader = BufReader::new(file);
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
                Err(error) => Some(Err(model_io_error("history read", error).into())),
            })
            .collect()
    }

    pub(crate) fn contains_run_identity(
        &self,
        task_run_id: &str,
        model_call_id: &str,
    ) -> Result<bool, ModelCallStoreError> {
        // Per-file read locks are not an atomic claim across the run history set.
        let mut found = false;
        for failure in [false, true] {
            let (parent, _lock) = self.acquire_lock(failure)?;
            let file = match parent.open_read()? {
                Some(file) => file,
                None => continue,
            };
            for (index, line) in BufReader::new(file).lines().enumerate() {
                let line = line.map_err(|error| model_io_error("history read", error))?;
                if line.trim().is_empty() {
                    continue;
                }
                let invalid_json = |source| ModelCallStoreError::InvalidJson {
                    line: index + 1,
                    source,
                };
                let invalid_contract = |source| ModelCallStoreError::InvalidContractAtLine {
                    line: index + 1,
                    source,
                };
                let (record_task, record_call) = if failure {
                    let record: ModelCallFailureRecord =
                        serde_json::from_str(&line).map_err(invalid_json)?;
                    record.validate().map_err(invalid_contract)?;
                    (record.task_run_id, record.model_call_id)
                } else {
                    let record: ModelCallRecord =
                        serde_json::from_str(&line).map_err(invalid_json)?;
                    record.validate().map_err(invalid_contract)?;
                    (record.task_run_id, record.model_call_id)
                };
                found |= record_task == task_run_id || record_call == model_call_id;
            }
        }
        Ok(found)
    }

    /// Read a bounded page of non-blank JSONL records without loading the full log.
    pub fn read_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ModelCallRecord>, ModelCallStoreError> {
        let (parent, _lock) = self.acquire_lock(false)?;
        let mut records = Vec::new();
        // Query locks are intentional, even for a zero page. Do not open the
        // data leaf after that short circuit.
        if limit == 0 {
            return Ok(records);
        }
        let Some(file) = parent.open_read()? else {
            return Ok(records);
        };
        let reader = BufReader::new(file);
        let mut record_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line.map_err(|error| model_io_error("history read", error))?;
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

    fn acquire_lock(&self, failure: bool) -> Result<(AuditParent, File), ModelCallStoreError> {
        // Missing parents remain errors after construction. Each operation
        // pins once, then opens both the real lock and its data as siblings.
        let mut parent = AuditParent::open(&self.path, false)?
            .ok_or_else(|| model_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        if failure {
            parent = parent.sibling(self.failure_path.file_name().ok_or_else(|| {
                model_io_error("path validation", io::ErrorKind::InvalidInput.into())
            })?)?;
        }
        let paths = self.storage_paths();
        let lock_parent = parent.sibling(
            paths[if failure { 3 } else { 2 }]
                .file_name()
                .ok_or_else(|| {
                    model_io_error("path validation", io::ErrorKind::InvalidInput.into())
                })?,
        )?;
        let lock = lock_parent.open_lock()?;
        lock.lock()
            .map_err(|error| model_io_error("lock acquire", error))?;
        Ok((parent, lock))
    }
}

fn confirm_durable(
    file: &File,
    parent: &AuditParent,
    io: &mut impl ModelStoreIo,
) -> Result<(), ModelCallStoreError> {
    io.sync_file(file)
        .map_err(|error| model_io_error("append file sync", error))?;
    io.sync_directory(parent)
        .map_err(|error| model_io_error("append directory sync", error))?;
    Ok(())
}

fn model_io_error(stage: &str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("model call store {stage} failed: {:?}", error.kind()),
    )
}

// Per-call seam: fault tests exercise the real validation/dedup/ACK control
// flow using real files and locks, without process-wide injection settings.
trait ModelStoreIo {
    fn after_lock(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()>;
    fn sync_file(&mut self, file: &File) -> io::Result<()>;
    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()>;
}

struct ModelStoreDisk;

impl ModelStoreIo for ModelStoreDisk {
    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()> {
        file.write_all(encoded)
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        parent.sync_directory()
    }
}

#[cfg(all(test, unix))]
#[path = "model_call_store/tests.rs"]
mod tests;
