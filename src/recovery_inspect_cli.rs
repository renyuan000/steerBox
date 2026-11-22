use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use steerbox::recovery_inspector::{inspect_files, InspectLimits};

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
    // Finish syntax validation before opening either explicit diagnostic path.
    let events = args
        .next()
        .filter(|path| !path.is_empty() && !path.starts_with("--"))
        .ok_or("invalid recovery inspect arguments")?;
    let state = match args.next() {
        None => None,
        Some(flag) if flag == "--state" => Some(
            args.next()
                .filter(|path| !path.is_empty() && !path.starts_with("--"))
                .ok_or("invalid recovery inspect arguments")?,
        ),
        Some(_) => return Err("invalid recovery inspect arguments".into()),
    };
    if args.next().is_some() {
        return Err("invalid recovery inspect arguments".into());
    }

    let report = inspect_files(
        Path::new(&events),
        state.as_deref().map(Path::new),
        InspectLimits::default(),
    )
    .map_err(|_| "recovery inspection request failed")?;
    let findings = report.has_findings();
    let mut encoded =
        serde_json::to_vec(&report).map_err(|_| "recovery inspection output failed")?;
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
