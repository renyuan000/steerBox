use crate::audit_store::AuditParent;
use crate::domain::EventEnvelope;
use serde_json::Error as JsonError;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("event log I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log line {line} is invalid JSON: {source}")]
    InvalidEvent { line: usize, source: JsonError },
    #[error("event log line {line} is unterminated; history was not modified")]
    UnterminatedRecord { line: usize },
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
        let parent = AuditParent::open(&path, true)?
            .ok_or_else(|| event_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        // Existing histories need read access only. Creation is separately
        // synchronized, then scanned through the same pinned parent directory.
        let file = match parent.open_read()? {
            Some(file) => file,
            None => {
                let file = parent.open_append()?;
                file.sync_data()
                    .map_err(|error| event_io_error("create sync", error))?;
                parent.sync_directory()?;
                parent
                    .open_read()?
                    .ok_or_else(|| event_io_error("history open", io::ErrorKind::NotFound.into()))?
            }
        };
        let next_sequence = {
            let mut reader = BufReader::new(file);
            let mut last_sequence = 0;
            let mut line_number = 0;
            let mut line = String::new();
            loop {
                line.clear();
                if reader
                    .read_line(&mut line)
                    .map_err(|error| event_io_error("history read", error))?
                    == 0
                {
                    break;
                }
                line_number += 1;
                // A complete JSON value without its newline is not a sealed
                // record: appending would concatenate the next event onto it.
                // Refuse to open for append, preserving all recovery evidence.
                if !line.ends_with('\n') {
                    return Err(EventLogError::UnterminatedRecord { line: line_number });
                }
                if line.trim().is_empty() {
                    continue;
                }
                let event = serde_json::from_str::<EventEnvelope>(&line).map_err(|source| {
                    EventLogError::InvalidEvent {
                        line: line_number,
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
        };
        Ok(Self {
            path,
            next_sequence,
        })
    }

    pub fn append(&mut self, event: EventEnvelope) -> Result<u64, EventLogError> {
        let parent = AuditParent::open(&self.path, true)?
            .ok_or_else(|| event_io_error("parent open", io::ErrorKind::NotFound.into()))?;
        let mut file = parent.open_append()?;
        self.append_with_file(event, &mut file, || parent.sync_directory())
    }

    fn append_with_file(
        &mut self,
        mut event: EventEnvelope,
        file: &mut impl EventAppendFile,
        sync_directory: impl FnOnce() -> io::Result<()>,
    ) -> Result<u64, EventLogError> {
        let sequence = self.next_sequence;
        event.sequence_no = Some(sequence);
        let mut encoded = serde_json::to_vec(&event)
            .map_err(|source| EventLogError::InvalidEvent { line: 0, source })?;
        encoded.push(b'\n');
        file.write_all(&encoded)
            .map_err(|error| event_io_error("append write", error))?;
        file.sync_data()
            .map_err(|error| event_io_error("append file sync", error))?;
        sync_directory().map_err(|error| event_io_error("append directory sync", error))?;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(EventLogError::SequenceOverflow)?;
        Ok(sequence)
    }

    pub fn read_all(&self) -> Result<Vec<EventEnvelope>, EventLogError> {
        let Some(file) = self.open_history()? else {
            return Ok(Vec::new());
        };
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (index, line) in reader.lines().enumerate() {
            let line = line.map_err(|error| event_io_error("history read", error))?;
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

    pub(crate) fn contains_run_identity(
        &self,
        loop_run_id: &str,
        task_run_id: &str,
    ) -> Result<bool, EventLogError> {
        // Serial history query only; it does not atomically claim an identity.
        let Some(file) = self.open_history()? else {
            return Ok(false);
        };
        let mut found = false;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| event_io_error("history read", error))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: EventEnvelope =
                serde_json::from_str(&line).map_err(|source| EventLogError::InvalidEvent {
                    line: index + 1,
                    source,
                })?;
            found |= event.loop_run_id.as_deref() == Some(loop_run_id)
                || event.task_run_id.as_deref() == Some(task_run_id)
                || (event.aggregate_type == "loop_run" && event.aggregate_id == loop_run_id)
                || (event.aggregate_type == "task_run" && event.aggregate_id == task_run_id);
        }
        Ok(found)
    }

    pub fn read_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, EventLogError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let Some(file) = self.open_history()? else {
            return Ok(Vec::new());
        };
        let reader = BufReader::new(file);
        let mut events = Vec::with_capacity(limit);
        let mut event_index = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line.map_err(|error| event_io_error("history read", error))?;
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

    fn open_history(&self) -> io::Result<Option<File>> {
        let Some(parent) = AuditParent::open(&self.path, false)? else {
            return Ok(None);
        };
        parent.open_read()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

trait EventAppendFile: Write {
    fn sync_data(&mut self) -> io::Result<()>;
}

impl EventAppendFile for File {
    fn sync_data(&mut self) -> io::Result<()> {
        File::sync_data(self)
    }
}

fn event_io_error(stage: &str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("event log {stage} failed: {:?}", error.kind()),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn event() -> EventEnvelope {
        EventEnvelope::new(
            "loop_run",
            "event",
            "probe",
            "loop",
            "correlation",
            serde_json::json!({}),
        )
        .unwrap()
    }

    #[test]
    fn identity_query_rejects_replaced_leaf_and_parent_without_reading_target() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("events.jsonl");
        let log = EventLog::open(&path).unwrap();
        let target = root.path().join("target.jsonl");
        fs::write(
            &target,
            format!("{}\n", serde_json::to_string(&event()).unwrap()),
        )
        .unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&target, &path).unwrap();
        assert!(log.contains_run_identity("loop", "task").is_err());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&parent).unwrap();
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::copy(&target, other.join("events.jsonl")).unwrap();
        symlink(&other, &parent).unwrap();
        assert!(log.contains_run_identity("loop", "task").is_err());
    }

    #[test]
    fn identity_query_missing_history_does_not_create_nodes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("parent/events.jsonl");
        let log = EventLog::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
        assert!(!log.contains_run_identity("loop", "task").unwrap());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn identity_match_does_not_hide_later_corrupt_record() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events.jsonl");
        let mut log = EventLog::open(&path).unwrap();
        log.append(event()).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\ninvalid\n")
            .unwrap();
        assert!(matches!(
            log.contains_run_identity("loop", "task"),
            Err(EventLogError::InvalidEvent { line: 3, .. })
        ));
    }

    #[test]
    fn pages_skip_prior_json_but_keep_physical_line_numbers_and_stop_at_limit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events.jsonl");
        let log = EventLog::open(&path).unwrap();
        fs::write(
            &path,
            format!(
                "\ninvalid\n\n{}\ninvalid\n",
                serde_json::to_string(&event()).unwrap()
            ),
        )
        .unwrap();
        assert_eq!(log.read_page(1, 1).unwrap().len(), 1);
        assert!(matches!(
            log.read_page(1, 2),
            Err(EventLogError::InvalidEvent { line: 5, .. })
        ));
        assert!(matches!(
            log.read_all(),
            Err(EventLogError::InvalidEvent { line: 2, .. })
        ));
    }

    #[test]
    fn queries_keep_unsealed_tail_semantics_distinct_from_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events.jsonl");
        let log = EventLog::open(&path).unwrap();
        let recorded = event();
        fs::write(&path, serde_json::to_vec(&recorded).unwrap()).unwrap();
        assert_eq!(log.read_all().unwrap(), vec![recorded.clone()]);
        assert_eq!(log.read_page(0, 1).unwrap(), vec![recorded]);
        assert!(log.contains_run_identity("loop", "task").unwrap());
        assert!(matches!(
            EventLog::open(&path),
            Err(EventLogError::UnterminatedRecord { line: 1 })
        ));
    }

    #[test]
    fn query_utf8_errors_are_not_skipped_before_offset_or_after_identity_match() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events.jsonl");
        let log = EventLog::open(&path).unwrap();
        let mut bytes = serde_json::to_vec(&event()).unwrap();
        bytes.extend_from_slice(b"\n\xff\n");
        fs::write(&path, &bytes).unwrap();
        assert_eq!(log.read_page(0, 1).unwrap().len(), 1);
        for error in [
            log.read_page(2, 1).unwrap_err(),
            log.contains_run_identity("loop", "task").unwrap_err(),
        ] {
            assert!(
                matches!(error, EventLogError::Io(ref source) if source.kind() == io::ErrorKind::InvalidData)
            );
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[derive(Default)]
    struct AppendProbe {
        bytes: Vec<u8>,
        synced_bytes: Vec<u8>,
        write_limit: Option<usize>,
        fail_sync: bool,
        calls: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    }

    impl Write for AppendProbe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls.borrow_mut().push("write");
            let count = match self.write_limit {
                Some(limit) if self.bytes.len() >= limit => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "synthetic private write detail",
                    ))
                }
                Some(limit) => bytes.len().min(limit - self.bytes.len()),
                None => bytes.len(),
            };
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl EventAppendFile for AppendProbe {
        fn sync_data(&mut self) -> io::Result<()> {
            self.calls.borrow_mut().push("file sync");
            if self.fail_sync {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic private sync detail",
                ));
            }
            self.synced_bytes = self.bytes.clone();
            Ok(())
        }
    }

    #[test]
    fn append_ack_and_sequence_follow_complete_record_file_sync_and_directory_sync() {
        let mut log = EventLog {
            path: PathBuf::new(),
            next_sequence: 7,
        };
        let mut probe = AppendProbe::default();
        let calls = probe.calls.clone();
        let sequence = log
            .append_with_file(event(), &mut probe, || {
                calls.borrow_mut().push("directory sync");
                Ok(())
            })
            .unwrap();
        assert_eq!(sequence, 7);
        assert_eq!(log.next_sequence, 8);
        assert_eq!(
            *calls.borrow(),
            vec!["write", "file sync", "directory sync"]
        );
        assert!(probe.bytes.ends_with(b"\n"));
        assert_eq!(probe.synced_bytes, probe.bytes);
        assert_eq!(
            serde_json::from_slice::<EventEnvelope>(&probe.bytes)
                .unwrap()
                .sequence_no,
            Some(7)
        );
    }

    #[test]
    fn append_write_and_file_sync_failures_short_circuit_without_sequence_advance() {
        for write_failure in [true, false] {
            let mut log = EventLog {
                path: PathBuf::new(),
                next_sequence: 7,
            };
            let mut probe = AppendProbe {
                write_limit: write_failure.then_some(1),
                fail_sync: !write_failure,
                ..AppendProbe::default()
            };
            let error = log
                .append_with_file(event(), &mut probe, || {
                    panic!("directory sync must not run")
                })
                .unwrap_err();
            assert_eq!(log.next_sequence, 7);
            assert!(!probe.bytes.is_empty());
            let expected = if write_failure {
                "append write"
            } else {
                "append file sync"
            };
            assert!(
                matches!(error, EventLogError::Io(ref source) if source.kind() == io::ErrorKind::PermissionDenied)
            );
            assert!(error.to_string().contains(expected));
            assert!(!format!("{error:?}").contains("synthetic private"));
            assert_eq!(probe.calls.borrow().contains(&"file sync"), !write_failure);
        }
    }

    #[test]
    fn append_directory_sync_failure_keeps_uncertain_record_and_does_not_advance_sequence() {
        let mut log = EventLog {
            path: PathBuf::new(),
            next_sequence: 7,
        };
        let mut probe = AppendProbe::default();
        let calls = probe.calls.clone();
        let error = log
            .append_with_file(event(), &mut probe, || {
                calls.borrow_mut().push("directory sync");
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic private directory detail",
                ))
            })
            .unwrap_err();
        assert_eq!(log.next_sequence, 7);
        assert_eq!(probe.synced_bytes, probe.bytes);
        assert!(probe.bytes.ends_with(b"\n"));
        assert_eq!(
            *calls.borrow(),
            vec!["write", "file sync", "directory sync"]
        );
        assert!(
            matches!(error, EventLogError::Io(ref source) if source.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(error.to_string().contains("append directory sync"));
        assert!(!format!("{error:?}").contains("synthetic private"));
    }

    #[test]
    fn existing_sequence_exhaustion_can_fail_after_writing_max_record() {
        // Characterization only: this slice does not move the overflow check.
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events.jsonl");
        let mut previous = event();
        previous.sequence_no = Some(u64::MAX - 1);
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&previous).unwrap()),
        )
        .unwrap();
        let mut log = EventLog::open(&path).unwrap();
        assert!(matches!(
            log.append(event()),
            Err(EventLogError::SequenceOverflow)
        ));
        assert_eq!(log.next_sequence, u64::MAX);
        assert_eq!(
            log.read_all().unwrap().last().unwrap().sequence_no,
            Some(u64::MAX)
        );
        let bytes = fs::read(&path).unwrap();
        assert!(matches!(
            EventLog::open(&path),
            Err(EventLogError::SequenceOverflow)
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
