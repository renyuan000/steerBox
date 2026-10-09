use crate::domain::EventEnvelope;
use serde_json::Error as JsonError;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("event log I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log line {line} is invalid JSON: {source}")]
    InvalidEvent { line: usize, source: JsonError },
    #[error("event log sequence number overflow")]
    SequenceOverflow,
}

#[derive(Debug)]
pub struct EventLog {
    path: PathBuf,
    next_sequence: u64,
}

impl EventLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EventLogError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        if !existed {
            OpenOptions::new().create(true).append(true).open(&path)?;
        }
        let next_sequence = if existed {
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut last_sequence = 0;
            for (index, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let event = serde_json::from_str::<EventEnvelope>(&line).map_err(|source| {
                    EventLogError::InvalidEvent {
                        line: index + 1,
                        source,
                    }
                })?;
                if let Some(sequence) = event.sequence_no {
                    last_sequence = last_sequence.max(sequence);
                }
            }
            last_sequence
                .checked_add(1)
                .ok_or(EventLogError::SequenceOverflow)?
        } else {
            1
        };
        Ok(Self {
            path,
            next_sequence,
        })
    }

    pub fn append(&mut self, mut event: EventEnvelope) -> Result<u64, EventLogError> {
        let sequence = self.next_sequence;
        event.sequence_no = Some(sequence);
        let encoded = serde_json::to_vec(&event)
            .map_err(|source| EventLogError::InvalidEvent { line: 0, source })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(EventLogError::SequenceOverflow)?;
        Ok(sequence)
    }

    pub fn read_all(&self) -> Result<Vec<EventEnvelope>, EventLogError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (index, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            events.push(
                serde_json::from_str::<EventEnvelope>(&line).map_err(|source| {
                    EventLogError::InvalidEvent {
                        line: index + 1,
                        source,
                    }
                })?,
            );
        }
        Ok(events)
    }

    pub fn read_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, EventLogError> {
        if limit == 0 || !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::with_capacity(limit);
        let mut event_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if event_index < offset {
                event_index += 1;
                continue;
            }
            events.push(
                serde_json::from_str::<EventEnvelope>(&line).map_err(|source| {
                    EventLogError::InvalidEvent {
                        line: line_index + 1,
                        source,
                    }
                })?,
            );
            event_index += 1;
            if events.len() == limit {
                break;
            }
        }
        Ok(events)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
