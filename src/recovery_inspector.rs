//! Bounded, diagnostic-only inspection. Never persists, repairs or resumes a run.

mod audits;
pub use audits::{inspect_files_with_audits, AuditInspectLimits, AuditRecoveryInspectionReport};

use crate::{audit_store::AuditParent, domain::EventEnvelope, state_store::StateStore};
use serde::Serialize;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct InspectLimits {
    pub events_bytes: usize,
    /// Includes the terminating LF.
    pub line_bytes: usize,
    pub event_records: usize,
    pub state_bytes: usize,
}

impl Default for InspectLimits {
    fn default() -> Self {
        Self {
            events_bytes: 16 * 1024 * 1024,
            line_bytes: 1024 * 1024,
            event_records: 10_000,
            state_bytes: 2 * 1024 * 1024,
        }
    }
}

impl InspectLimits {
    fn validate(self) -> Result<(), InspectRequestError> {
        let max = Self::default();
        if [
            (self.events_bytes, max.events_bytes),
            (self.line_bytes, max.line_bytes),
            (self.event_records, max.event_records),
            (self.state_bytes, max.state_bytes),
        ]
        .iter()
        .any(|(value, cap)| *value == 0 || value > cap)
        {
            return Err(InspectRequestError);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("invalid recovery inspection limits")]
pub struct InspectRequestError;

// All strings below are fixed vocabulary, never input-derived text. In particular,
// neither Debug nor serialization retains paths, identities, payloads or errors.
#[derive(Debug, Serialize)]
pub struct RecoveryInspectionReport {
    schema_version: u32,
    mode: &'static str,
    coverage: &'static str,
    events: EventsReport,
    projection: ProjectionReport,
    state: StateReport,
    issues: Vec<InspectionIssue>,
}

#[derive(Debug, Serialize)]
struct EventsReport {
    status: &'static str,
    read_bytes: usize,
    sealed_prefix_bytes: usize,
    sealed_records: usize,
    last_sequence_no: Option<u64>,
    ignored_event_count: usize,
}

#[derive(Debug, Serialize)]
struct ProjectionReport {
    status: &'static str,
    loop_count: usize,
    step_count: usize,
}

#[derive(Debug, Serialize)]
struct StateReport {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct InspectionIssue {
    source: &'static str,
    code: &'static str,
    line: Option<usize>,
    byte_offset: Option<usize>,
}

impl RecoveryInspectionReport {
    pub fn has_findings(&self) -> bool {
        !self.issues.is_empty()
    }

    fn issue(
        &mut self,
        source: &'static str,
        code: &'static str,
        line: Option<usize>,
        byte_offset: Option<usize>,
    ) {
        self.issues.push(InspectionIssue {
            source,
            code,
            line,
            byte_offset,
        });
    }

    fn event_issue(&mut self, code: &'static str, line: usize, offset: usize) {
        self.events.status = code;
        self.issue("events", code, Some(line), Some(offset));
    }
}

/// Inspect explicit paths without runtime/config initialization or filesystem writes.
/// A clean report covers only the loop/step projection, not safe resumability.
pub fn inspect_files(
    events: &Path,
    state: Option<&Path>,
    limits: InspectLimits,
) -> Result<RecoveryInspectionReport, InspectRequestError> {
    limits.validate()?;
    let mut report = RecoveryInspectionReport {
        schema_version: 1,
        mode: "diagnostic_only",
        coverage: "events_state_projection_only",
        events: EventsReport {
            status: "complete",
            read_bytes: 0,
            sealed_prefix_bytes: 0,
            sealed_records: 0,
            last_sequence_no: None,
            ignored_event_count: 0,
        },
        projection: ProjectionReport {
            status: "not_rebuilt",
            loop_count: 0,
            step_count: 0,
        },
        state: StateReport {
            status: if state.is_some() {
                "not_compared"
            } else {
                "not_requested"
            },
        },
        issues: Vec::new(),
    };
    let input = read_file(events, limits.events_bytes);
    report.events.read_bytes = input.bytes.len();
    if input.status != "complete" && input.status != "limit_reached" {
        report.events.status = input.status;
        report.issue("events", input.status, None, None);
        return Ok(report);
    }
    let parsed = scan_events(
        &input.bytes,
        input.status == "complete",
        limits,
        &mut report,
    );
    if report.events.status != "complete" {
        return Ok(report);
    }
    // Do not render StateStoreError: it may contain arbitrary IDs or payload fields.
    let rebuilt = match StateStore::rebuild(&parsed) {
        Ok(rebuilt) => rebuilt,
        Err(_) => {
            report.projection.status = "invalid";
            report.issue("projection", "invalid_projection", None, None);
            return Ok(report);
        }
    };
    report.projection = ProjectionReport {
        status: "rebuilt",
        loop_count: rebuilt.loops().count(),
        step_count: rebuilt.steps().count(),
    };
    if let Some(path) = state {
        let input = read_file(path, limits.state_bytes);
        report.state.status = if input.status != "complete" {
            input.status
        } else {
            match serde_json::from_slice::<StateStore>(&input.bytes) {
                Ok(stored) if stored == rebuilt => "matches",
                Ok(_) => "mismatch",
                Err(_) => "invalid",
            }
        };
        if report.state.status != "matches" {
            report.issue("state", report.state.status, None, None);
        }
    }
    Ok(report)
}

fn scan_events(
    bytes: &[u8],
    eof: bool,
    limits: InspectLimits,
    report: &mut RecoveryInspectionReport,
) -> Vec<EventEnvelope> {
    let mut parsed = Vec::new();
    let mut offset = 0;
    let mut line_no = 1;
    while offset < bytes.len() {
        let Some(relative_end) = bytes[offset..].iter().position(|byte| *byte == b'\n') else {
            report.event_issue(
                if eof {
                    "unterminated_record"
                } else {
                    "limit_reached"
                },
                line_no,
                offset,
            );
            return parsed;
        };
        let end = offset + relative_end + 1;
        if end - offset > limits.line_bytes {
            report.event_issue("limit_reached", line_no, offset);
            return parsed;
        }
        let line = match std::str::from_utf8(&bytes[offset..end]) {
            Ok(line) => line.trim(),
            Err(_) => {
                report.event_issue("invalid_utf8", line_no, offset);
                return parsed;
            }
        };
        if !line.is_empty() {
            if parsed.len() == limits.event_records {
                report.event_issue("limit_reached", line_no, offset);
                return parsed;
            }
            let event: EventEnvelope = match serde_json::from_str(line) {
                Ok(event) => event,
                Err(_) => {
                    report.event_issue("invalid_json", line_no, offset);
                    return parsed;
                }
            };
            if event.schema_version != 1 {
                report.event_issue("unsupported_schema", line_no, offset);
                return parsed;
            }
            let expected = Some(parsed.len() as u64 + 1);
            if event.sequence_no != expected {
                report.event_issue("invalid_sequence", line_no, offset);
                return parsed;
            }
            if !matches!(
                event.event_type.as_str(),
                "loop_started"
                    | "loop_completed"
                    | "loop_failed"
                    | "loop_paused"
                    | "loop_resumed"
                    | "loop_cancelled"
                    | "step_started"
                    | "step_completed"
                    | "step_failed"
            ) {
                report.events.ignored_event_count += 1;
            }
            report.events.last_sequence_no = expected;
            parsed.push(event);
            report.events.sealed_records = parsed.len();
        }
        report.events.sealed_prefix_bytes = end;
        offset = end;
        line_no += 1;
    }
    if !eof {
        report.event_issue("limit_reached", line_no, offset);
    }
    parsed
}

struct FileRead {
    bytes: Vec<u8>,
    status: &'static str,
}

fn read_file(path: &Path, cap: usize) -> FileRead {
    let opened = AuditParent::open(path, false)
        .and_then(|parent| parent.map_or(Ok(None), |parent| parent.open_read()));
    match opened {
        Ok(Some(mut file)) => read_open_file(&mut file, cap, |_| {}),
        Ok(None) => FileRead {
            bytes: Vec::new(),
            status: "missing",
        },
        Err(error) => FileRead {
            bytes: Vec::new(),
            status: path_error(&error),
        },
    }
}

fn path_error(error: &io::Error) -> &'static str {
    let kind = error.kind();
    if matches!(
        kind,
        io::ErrorKind::InvalidInput | io::ErrorKind::NotADirectory | io::ErrorKind::IsADirectory
    ) || kind == io::Error::from_raw_os_error(libc::ELOOP).kind()
    {
        "unsafe_path"
    } else {
        "unreadable"
    }
}

// The private hook permits deterministic real-fd mutation tests. Production uses
// a no-op; metadata always comes from the same fd, never a reopened pathname.
fn read_open_file(file: &mut File, cap: usize, after_read: impl FnOnce(&File)) -> FileRead {
    let mut input = FileRead {
        bytes: Vec::new(),
        status: "unreadable",
    };
    let Ok(before) = file.metadata() else {
        return input;
    };
    let result = file.take(cap as u64).read_to_end(&mut input.bytes);
    after_read(file);
    let Ok(after) = file.metadata() else {
        return input;
    };
    if metadata_changed(&before, &after) {
        input.status = "unstable";
    } else if result.is_ok() {
        // No out-of-budget probe: filling the cap cannot prove EOF, even if the
        // length happens to equal the cap. Metadata length is not an EOF proof.
        input.status = if input.bytes.len() == cap {
            "limit_reached"
        } else {
            "complete"
        };
    }
    input
}

#[cfg(unix)]
fn metadata_changed(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let fingerprint = |meta: &std::fs::Metadata| {
        (
            meta.dev(),
            meta.ino(),
            meta.mode(),
            meta.nlink(),
            meta.len(),
            meta.uid(),
            meta.gid(),
            meta.mtime(),
            meta.mtime_nsec(),
            meta.ctime(),
            meta.ctime_nsec(),
        )
    };
    fingerprint(before) != fingerprint(after)
}

#[cfg(not(unix))]
fn metadata_changed(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs::{self, Permissions};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[test]
    fn bounded_reader_uses_the_open_fd_and_never_probes_past_cap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events");
        fs::write(&path, b"abcdef").unwrap();
        let parent = AuditParent::open(&path, false).unwrap().unwrap();
        let mut file = parent.open_read().unwrap().unwrap();
        let inode = file.metadata().unwrap().ino();
        let input = read_open_file(&mut file, 3, |actual| {
            assert_eq!(actual.metadata().unwrap().ino(), inode);
        });
        assert_eq!(input.bytes, b"abc");
        assert_eq!(input.status, "limit_reached");
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"def");
    }

    #[test]
    fn actual_fd_change_after_read_is_unstable_including_same_length_mode_and_links() {
        for mutation in ["length", "mode", "link"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("events");
            fs::write(&path, b"sealed\n").unwrap();
            fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
            let parent = AuditParent::open(&path, false).unwrap().unwrap();
            let mut file = parent.open_read().unwrap().unwrap();
            let input = read_open_file(&mut file, 100, |_| match mutation {
                "length" => fs::write(&path, b"longer bytes\n").unwrap(),
                "mode" => fs::set_permissions(&path, Permissions::from_mode(0o400)).unwrap(),
                "link" => fs::hard_link(&path, root.path().join("alias")).unwrap(),
                _ => unreachable!(),
            });
            assert_eq!(input.bytes, b"sealed\n");
            assert_eq!(input.status, "unstable", "{mutation}");
        }
    }

    #[test]
    fn unchanged_fd_reaches_eof_and_reads_do_not_make_atime_a_finding() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("events");
        fs::write(&path, b"sealed\n").unwrap();
        let parent = AuditParent::open(&path, false).unwrap().unwrap();
        let mut file = parent.open_read().unwrap().unwrap();
        let input = read_open_file(&mut file, 100, |_| {});
        assert_eq!(input.status, "complete");
        assert_eq!(input.bytes, b"sealed\n");
    }
}
