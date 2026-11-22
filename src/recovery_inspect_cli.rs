use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use steerbox::recovery_inspector::{
    inspect_files, inspect_files_with_audits, AuditInspectLimits, InspectLimits,
};

/// Signals findings only after the complete diagnostic report was written.
#[derive(Debug)]
pub(super) struct FindingsReported;

impl fmt::Display for FindingsReported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recovery inspection reported findings")
    }
}

impl Error for FindingsReported {}

pub(super) fn run(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    // Finish syntax validation before opening any explicit diagnostic path.
    let events = args
        .next()
        .filter(|path| !path.is_empty() && !path.starts_with("--"))
        .ok_or("invalid recovery inspect arguments")?;
    let mut state = None;
    let mut tools = None;
    let mut side_effects = None;
    while let Some(flag) = args.next() {
        let destination = match flag.as_str() {
            "--state" => &mut state,
            "--tools" => &mut tools,
            "--side-effects" => &mut side_effects,
            _ => return Err("invalid recovery inspect arguments".into()),
        };
        if destination.is_some() {
            return Err("invalid recovery inspect arguments".into());
        }
        let path = args
            .next()
            .filter(|path| !path.is_empty() && !path.starts_with("--"))
            .ok_or("invalid recovery inspect arguments")?;
        *destination = Some(path);
    }

    let (findings, mut encoded) = if tools.is_some() || side_effects.is_some() {
        let report = inspect_files_with_audits(
            Path::new(&events),
            state.as_deref().map(Path::new),
            tools.as_deref().map(Path::new),
            side_effects.as_deref().map(Path::new),
            InspectLimits::default(),
            AuditInspectLimits::default(),
        )
        .map_err(|_| "recovery inspection request failed")?;
        let encoded =
            serde_json::to_vec(&report).map_err(|_| "recovery inspection output failed")?;
        (report.has_findings(), encoded)
    } else {
        let report = inspect_files(
            Path::new(&events),
            state.as_deref().map(Path::new),
            InspectLimits::default(),
        )
        .map_err(|_| "recovery inspection request failed")?;
        let encoded =
            serde_json::to_vec(&report).map_err(|_| "recovery inspection output failed")?;
        (report.has_findings(), encoded)
    };
    encoded.push(b'\n');
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output
        .write_all(&encoded)
        .map_err(|_| "recovery inspection output failed")?;
    output
        .flush()
        .map_err(|_| "recovery inspection output failed")?;

    if findings {
        return Err(Box::new(FindingsReported));
    }
    Ok(())
}
