use std::io::{self, Write};

/// The runtime object whose progress is being reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressSubject {
    Step,
    Tool,
    Model,
    Action,
    Loop,
}

impl ProgressSubject {
    fn label(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Tool => "tool",
            Self::Model => "model",
            Self::Action => "action",
            Self::Loop => "loop",
        }
    }
}

/// A safe, user-visible lifecycle state. It deliberately has no payload field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStatus {
    Started,
    Progress,
    Completed,
    Failed,
    Paused,
    Resumed,
    Cancelled,
}

impl ProgressStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Progress => "progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Paused => "paused",
            Self::Resumed => "resumed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A compact event for progress sinks. Summary is expected to be a human-facing status,
/// not a model response or a tool payload; StreamReporter sanitizes it before output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    pub subject: ProgressSubject,
    pub subject_id: String,
    pub status: ProgressStatus,
    pub summary: Option<String>,
    pub progress: Option<(u32, u32)>,
}

/// Output boundary used by the runtime and replaceable by a CLI, TUI, API, or test sink.
pub trait ProgressSink {
    fn report(&mut self, event: ProgressEvent) -> io::Result<()>;
}

/// A deliberately small, synchronous reporter. It does not own a global stdout/stderr handle
/// and does not attempt to make output thread-safe; callers choose the writer and lifecycle.
pub struct StreamReporter<W: Write> {
    writer: W,
    enabled: bool,
}

impl<W: Write> StreamReporter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn started(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Started,
            summary,
            progress,
        )
    }

    pub fn completed(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Completed,
            summary,
            progress,
        )
    }

    pub fn progress(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: (u32, u32),
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Progress,
            summary,
            Some(progress),
        )
    }

    pub fn failed(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Failed,
            summary,
            progress,
        )
    }

    pub fn paused(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Paused,
            summary,
            progress,
        )
    }

    pub fn resumed(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Resumed,
            summary,
            progress,
        )
    }

    pub fn cancelled(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        self.report_event(
            subject,
            subject_id,
            ProgressStatus::Cancelled,
            summary,
            progress,
        )
    }

    fn report_event(
        &mut self,
        subject: ProgressSubject,
        subject_id: &str,
        status: ProgressStatus,
        summary: Option<&str>,
        progress: Option<(u32, u32)>,
    ) -> io::Result<()> {
        if let Some((current, total)) = progress {
            if current > total {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "progress current value must not exceed total",
                ));
            }
        }
        self.report(ProgressEvent {
            subject,
            subject_id: subject_id.to_owned(),
            status,
            summary: summary.map(str::to_owned),
            progress,
        })
    }
}

impl<W: Write> ProgressSink for StreamReporter<W> {
    fn report(&mut self, event: ProgressEvent) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let subject_id = sanitize_field(&event.subject_id, 96);
        let mut line = format!(
            "{} {} {}",
            event.subject.label(),
            subject_id,
            event.status.label()
        );

        if let Some((current, total)) = event.progress {
            if current > total {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "progress current value must not exceed total",
                ));
            }
            line.push_str(&format!(" [{current}/{total}]"));
        }
        if let Some(summary) = event.summary.as_deref().and_then(sanitize_summary) {
            line.push_str(" - ");
            line.push_str(&summary);
        }
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()
    }
}

fn sanitize_summary(summary: &str) -> Option<String> {
    let normalized = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }

    let lower = normalized.to_ascii_lowercase();
    let reasoning_start = [
        "chain-of-thought",
        "chain of thought",
        "reasoning",
        "internal analysis",
        "scratchpad",
    ]
    .iter()
    .filter_map(|marker| lower.find(marker))
    .min();
    let normalized = if let Some(index) = reasoning_start {
        normalized[..index].trim_end().to_owned()
    } else {
        normalized
    };
    if normalized.is_empty() {
        return None;
    }

    let tokens = normalized.split(' ').collect::<Vec<_>>();
    let mut redacted = String::with_capacity(normalized.len());
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        if !redacted.is_empty() {
            redacted.push(' ');
        }
        redacted.push_str(&redact_token(token));
        if sensitive_key(token).is_some() && token.ends_with(['=', ':']) {
            index += 1;
        }
        index += 1;
    }
    Some(truncate(&redacted, 240))
}

fn redact_token(token: &str) -> String {
    let Some(key) = sensitive_key(token) else {
        return token.to_owned();
    };
    format!("{key}=[REDACTED]")
}

fn sensitive_key(token: &str) -> Option<&str> {
    let separator = token.find(['=', ':'])?;
    let key = &token[..separator];
    let normalized_key = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>()
        .to_ascii_lowercase();
    if [
        "token",
        "accesstoken",
        "refreshtoken",
        "sessiontoken",
        "apikey",
        "api_key",
        "authorization",
        "password",
        "secret",
        "clientsecret",
        "cookie",
        "privatekey",
        "sshkey",
    ]
    .contains(&normalized_key.as_str())
    {
        Some(key)
    } else {
        None
    }
}

fn sanitize_field(value: &str, max_len: usize) -> String {
    truncate(
        &value
            .chars()
            .filter(|character| !character.is_control() || *character == '\t')
            .collect::<String>(),
        max_len,
    )
}

fn truncate(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}
