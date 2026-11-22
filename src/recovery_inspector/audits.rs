//! Per-record observations only: no identity joins, lifecycle inference or recovery.

use super::{
    inspect_files, read_file, InspectLimits, InspectRequestError, RecoveryInspectionReport,
};
use crate::{
    audit_store::SideEffectRecord, domain::ToolCallRecord, tool_registry::SideEffectClass,
};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct AuditInspectLimits {
    pub tool_bytes: usize,
    pub side_effect_bytes: usize,
    /// Per auxiliary source, excluding blank lines.
    pub records: usize,
    /// Sum of requested source byte caps, including events and optional State.
    /// This bounds file input, not the process's total memory usage.
    pub total_bytes: usize,
}

impl Default for AuditInspectLimits {
    fn default() -> Self {
        Self {
            tool_bytes: 4 * 1024 * 1024,
            side_effect_bytes: 4 * 1024 * 1024,
            records: 10_000,
            total_bytes: 26 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuditRecoveryInspectionReport {
    #[serde(flatten)]
    base: RecoveryInspectionReport,
    tools: AuditReport,
    side_effects: AuditReport,
}

impl AuditRecoveryInspectionReport {
    pub fn has_findings(&self) -> bool {
        self.base.has_findings()
    }
}

#[derive(Debug, Serialize)]
struct AuditReport {
    status: &'static str,
    read_bytes: usize,
    sealed_prefix_bytes: usize,
    sealed_records: usize,
    status_counts: StatusCounts,
    class_counts: ClassCounts,
}

impl Default for AuditReport {
    fn default() -> Self {
        Self {
            status: "not_requested",
            read_bytes: 0,
            sealed_prefix_bytes: 0,
            sealed_records: 0,
            status_counts: StatusCounts::default(),
            class_counts: ClassCounts::default(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
struct StatusCounts {
    planned: usize,
    started: usize,
    completed: usize,
    failed: usize,
    blocked: usize,
    uncertain: usize,
    verification_failed: usize,
    unknown: usize,
}

#[derive(Debug, Default, Serialize)]
struct ClassCounts {
    none: usize,
    workspace_read: usize,
    workspace_write: usize,
    external_network: usize,
    external_system: usize,
    unknown: usize,
}

/// Inspect only explicitly supplied inputs. Existing events/State behavior is
/// unchanged, including State being skipped when projection cannot be rebuilt.
/// Auxiliary files are scanned independently even if events/State have findings.
/// A complete scan is not lifecycle reconciliation, a coherent multi-file snapshot,
/// proof that an action succeeded, or authority to resume/retry it.
pub fn inspect_files_with_audits(
    events: &Path,
    state: Option<&Path>,
    tools: Option<&Path>,
    side_effects: Option<&Path>,
    limits: InspectLimits,
    audits: AuditInspectLimits,
) -> Result<AuditRecoveryInspectionReport, InspectRequestError> {
    limits.validate()?;
    let max = AuditInspectLimits::default();
    if [
        (audits.tool_bytes, max.tool_bytes),
        (audits.side_effect_bytes, max.side_effect_bytes),
        (audits.records, max.records),
        (audits.total_bytes, max.total_bytes),
    ]
    .iter()
    .any(|(value, cap)| *value == 0 || value > cap)
    {
        return Err(InspectRequestError);
    }
    // Each explicit source is read at most once. Even equal paths consume their
    // own caps. Validate this worst-case aggregate before any filesystem I/O.
    let total = [
        state.map(|_| limits.state_bytes),
        tools.map(|_| audits.tool_bytes),
        side_effects.map(|_| audits.side_effect_bytes),
    ]
    .into_iter()
    .flatten()
    .try_fold(limits.events_bytes, usize::checked_add)
    .ok_or(InspectRequestError)?;
    if total > audits.total_bytes {
        return Err(InspectRequestError);
    }

    let mut base = inspect_files(events, state, limits)?;
    base.coverage = "events_state_projection_and_audit_observations_only";
    let tools = inspect_audit(tools, "tools", audits.tool_bytes, limits, audits, &mut base);
    let side_effects = inspect_audit(
        side_effects,
        "side_effects",
        audits.side_effect_bytes,
        limits,
        audits,
        &mut base,
    );
    Ok(AuditRecoveryInspectionReport {
        base,
        tools,
        side_effects,
    })
}

fn inspect_audit(
    path: Option<&Path>,
    source: &'static str,
    cap: usize,
    limits: InspectLimits,
    audits: AuditInspectLimits,
    base: &mut RecoveryInspectionReport,
) -> AuditReport {
    let Some(path) = path else {
        return AuditReport::default();
    };
    let input = read_file(path, cap);
    scan_audit(input, source, limits, audits, base)
}

fn scan_audit(
    input: super::FileRead,
    source: &'static str,
    limits: InspectLimits,
    audits: AuditInspectLimits,
    base: &mut RecoveryInspectionReport,
) -> AuditReport {
    let mut report = AuditReport {
        status: input.status,
        read_bytes: input.bytes.len(),
        ..AuditReport::default()
    };
    if !matches!(input.status, "complete" | "limit_reached") {
        base.issue(source, input.status, None, None);
        return report;
    }
    let eof = input.status == "complete";
    let mut offset = 0;
    let mut line_no = 1;
    while offset < input.bytes.len() {
        let Some(relative_end) = input.bytes[offset..].iter().position(|byte| *byte == b'\n')
        else {
            scan_issue(
                &mut report,
                base,
                source,
                if eof {
                    "unterminated_record"
                } else {
                    "limit_reached"
                },
                line_no,
                offset,
            );
            return report;
        };
        let end = offset + relative_end + 1;
        if end - offset > limits.line_bytes {
            scan_issue(&mut report, base, source, "limit_reached", line_no, offset);
            return report;
        }
        let line = match std::str::from_utf8(&input.bytes[offset..end]) {
            Ok(line) => line.trim(),
            Err(_) => {
                scan_issue(&mut report, base, source, "invalid_utf8", line_no, offset);
                return report;
            }
        };
        if !line.is_empty() {
            if report.sealed_records == audits.records {
                scan_issue(&mut report, base, source, "limit_reached", line_no, offset);
                return report;
            }
            // Do not apply EventEnvelope schema/sequence requirements to these
            // distinct wire records. Drop all input text after fixed observations.
            let fields = if source == "tools" {
                serde_json::from_str::<ToolCallRecord>(line)
                    .map(|record| (record.status, record.side_effect_class))
            } else {
                serde_json::from_str::<SideEffectRecord>(line)
                    .map(|record| (record.status, record.class))
            };
            let (status, class) = match fields {
                Ok(fields) => fields,
                Err(_) => {
                    scan_issue(&mut report, base, source, "invalid_json", line_no, offset);
                    return report;
                }
            };
            report.sealed_records += 1;
            observe(&status, &class, &mut report, base, source, line_no, offset);
        }
        report.sealed_prefix_bytes = end;
        offset = end;
        line_no += 1;
    }
    if !eof {
        scan_issue(&mut report, base, source, "limit_reached", line_no, offset);
    }
    report
}

fn scan_issue(
    report: &mut AuditReport,
    base: &mut RecoveryInspectionReport,
    source: &'static str,
    code: &'static str,
    line: usize,
    offset: usize,
) {
    report.status = code;
    base.issue(source, code, Some(line), Some(offset));
}

fn observe(
    status: &str,
    class: &str,
    report: &mut AuditReport,
    base: &mut RecoveryInspectionReport,
    source: &'static str,
    line: usize,
    offset: usize,
) {
    // These are record counts, not a state machine or counts of distinct actions.
    let counts = &mut report.status_counts;
    let (counter, code) = match status.trim().to_ascii_lowercase().as_str() {
        "planned" => (&mut counts.planned, None),
        "started" => (&mut counts.started, None),
        "completed" => (&mut counts.completed, None),
        "failed" => (&mut counts.failed, Some("failed_status_observed")),
        "blocked" => (&mut counts.blocked, Some("blocked_status_observed")),
        "uncertain" | "side_effect_uncertain" => {
            (&mut counts.uncertain, Some("uncertain_status_observed"))
        }
        "verification_failed" => (
            &mut counts.verification_failed,
            Some("verification_failed_status_observed"),
        ),
        _ => (&mut counts.unknown, Some("unknown_status_observed")),
    };
    *counter += 1;
    if let Some(code) = code {
        observation_issue(base, source, code, line, offset);
    }
    let counter = match SideEffectClass::from_audit_label(class) {
        SideEffectClass::None => &mut report.class_counts.none,
        SideEffectClass::WorkspaceRead => &mut report.class_counts.workspace_read,
        SideEffectClass::WorkspaceWrite => &mut report.class_counts.workspace_write,
        SideEffectClass::ExternalNetwork => &mut report.class_counts.external_network,
        SideEffectClass::ExternalSystem => &mut report.class_counts.external_system,
        SideEffectClass::Unknown => {
            observation_issue(base, source, "unknown_class_observed", line, offset);
            &mut report.class_counts.unknown
        }
    };
    *counter += 1;
}

fn observation_issue(
    base: &mut RecoveryInspectionReport,
    source: &'static str,
    code: &'static str,
    line: usize,
    offset: usize,
) {
    // Only the first position per source/code is needed; counters retain totals.
    if !base
        .issues
        .iter()
        .any(|issue| issue.source == source && issue.code == code)
    {
        base.issue(source, code, Some(line), Some(offset));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs::{self, File};

    #[test]
    fn auxiliary_scan_never_promotes_bytes_from_a_changed_real_fd() {
        let root = tempfile::tempdir().unwrap();
        let events = root.path().join("events");
        let path = root.path().join("audit");
        fs::write(&events, []).unwrap();
        let bytes = serde_json::to_vec(&SideEffectRecord {
            side_effect_id: "private-identity".into(),
            goal_id: "private-identity".into(),
            loop_run_id: "private-identity".into(),
            tool_id: "private-identity".into(),
            class: "none".into(),
            status: "completed".into(),
            summary: "private-body".into(),
            created_at: "private-time".into(),
        })
        .unwrap();
        let mut sealed = bytes;
        sealed.push(b'\n');
        for source in ["tools", "side_effects"] {
            fs::write(&path, &sealed).unwrap();
            let mut file = File::open(&path).unwrap();
            let input = super::super::read_open_file(&mut file, 4096, |_| {
                fs::write(&path, b"changed\n").unwrap();
            });
            assert_eq!(input.status, "unstable");
            assert_eq!(input.bytes, sealed);
            let mut base = inspect_files(&events, None, InspectLimits::default()).unwrap();
            let report = scan_audit(
                input,
                source,
                InspectLimits::default(),
                AuditInspectLimits::default(),
                &mut base,
            );
            assert_eq!(report.status, "unstable");
            assert_eq!(report.read_bytes, sealed.len());
            assert_eq!(report.sealed_records, 0);
            assert_eq!(report.sealed_prefix_bytes, 0);
            assert_eq!(base.issues.len(), 1);
            assert_eq!(base.issues[0].code, "unstable");
            assert_eq!(base.issues[0].source, source);
        }
    }
}
