use crate::memory::{MemoryCandidate, MemoryCatalog, MemoryError};
use serde_json::from_str;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryStoreError {
    #[error("memory store I/O failed: {message}")]
    Io { message: String },
    #[error("memory candidate JSON is invalid at line {line}")]
    InvalidJson { line: usize },
    #[error("memory candidate is invalid at line {line}: {field}: {reason}")]
    InvalidCandidate {
        line: usize,
        field: String,
        reason: String,
    },
    #[error("memory candidate ID is duplicated at line {line}")]
    DuplicateCandidate { line: usize },
}

/// Append-only JSONL storage for unpromoted memory candidate metadata.
/// Reading validates schema and candidate fields but never resolves source events,
/// promotes candidates, or grants runtime authorization.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    path: PathBuf,
}

impl MemoryStore {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, candidate: &MemoryCandidate) -> Result<(), MemoryStoreError> {
        validate_candidate(candidate, 0)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let _lock = self.acquire_lock()?;
        let existing = self.read_all_unlocked()?;
        if existing
            .iter()
            .any(|stored| stored.candidate_id == candidate.candidate_id)
        {
            return Err(MemoryStoreError::DuplicateCandidate { line: 0 });
        }

        let encoded = serde_json::to_string(candidate)
            .map_err(|_| MemoryStoreError::InvalidJson { line: 0 })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(io_error)?;
        writeln!(file, "{encoded}").map_err(io_error)
    }

    pub fn read_all(&self) -> Result<Vec<MemoryCandidate>, MemoryStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let _lock = self.acquire_lock()?;
        self.read_all_unlocked()
    }

    pub fn read_catalog(&self) -> Result<MemoryCatalog, MemoryStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let _lock = self.acquire_lock()?;
        self.read_catalog_unlocked()
    }

    fn read_all_unlocked(&self) -> Result<Vec<MemoryCandidate>, MemoryStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let reader = BufReader::new(File::open(&self.path).map_err(io_error)?);
        let mut records = Vec::new();
        for (index, line) in reader.lines().enumerate() {
            let line_number = index + 1;
            let line = line.map_err(io_error)?;
            if line.trim().is_empty() {
                continue;
            }
            let candidate = from_str::<MemoryCandidate>(&line)
                .map_err(|_| MemoryStoreError::InvalidJson { line: line_number })?;
            validate_candidate(&candidate, line_number)?;
            if records
                .iter()
                .any(|stored: &MemoryCandidate| stored.candidate_id == candidate.candidate_id)
            {
                return Err(MemoryStoreError::DuplicateCandidate { line: line_number });
            }
            records.push(candidate);
        }
        Ok(records)
    }

    fn read_catalog_unlocked(&self) -> Result<MemoryCatalog, MemoryStoreError> {
        if !self.path.exists() {
            return Ok(MemoryCatalog::default());
        }
        let reader = BufReader::new(File::open(&self.path).map_err(io_error)?);
        let mut catalog = MemoryCatalog::default();
        for (index, line) in reader.lines().enumerate() {
            let line_number = index + 1;
            let line = line.map_err(io_error)?;
            if line.trim().is_empty() {
                continue;
            }
            let candidate = from_str::<MemoryCandidate>(&line)
                .map_err(|_| MemoryStoreError::InvalidJson { line: line_number })?;
            validate_candidate(&candidate, line_number)?;
            catalog
                .insert(candidate)
                .map_err(|error| map_memory_error(line_number, error))?;
        }
        Ok(catalog)
    }

    fn acquire_lock(&self) -> Result<File, MemoryStoreError> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))
            .map_err(io_error)?;
        lock_file.lock().map_err(io_error)?;
        Ok(lock_file)
    }
}

fn validate_candidate(candidate: &MemoryCandidate, line: usize) -> Result<(), MemoryStoreError> {
    candidate
        .validate()
        .map_err(|error| map_memory_error(line, error))
}

fn map_memory_error(line: usize, error: MemoryError) -> MemoryStoreError {
    match error {
        MemoryError::InvalidField { field, reason } => MemoryStoreError::InvalidCandidate {
            line,
            field,
            reason,
        },
        MemoryError::DuplicateCandidate(_) => MemoryStoreError::DuplicateCandidate { line },
    }
}

fn io_error(error: std::io::Error) -> MemoryStoreError {
    MemoryStoreError::Io {
        message: error.to_string(),
    }
}
