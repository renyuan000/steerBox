use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Steer,
    Pause,
    Resume,
    Cancel,
    Approve,
    Takeover,
    NewTask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Pending,
    Classified,
    Applied,
    Rejected,
    Expired,
    WaitingHuman,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserCommand {
    pub command_id: String,
    pub actor_ref: String,
    pub project_id: String,
    pub goal_id: String,
    pub task_run_id: String,
    pub loop_run_id: String,
    pub command_kind: CommandKind,
    pub correlation_id: String,
    pub expected_object_version: Option<u64>,
    pub requested_at: String,
    pub payload: Value,
}

impl UserCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        command_id: impl Into<String>,
        actor_ref: impl Into<String>,
        project_id: impl Into<String>,
        goal_id: impl Into<String>,
        task_run_id: impl Into<String>,
        loop_run_id: impl Into<String>,
        command_kind: CommandKind,
        correlation_id: impl Into<String>,
        payload: Value,
    ) -> Result<Self, UserCommandInboxError> {
        let command_id = command_id.into();
        let actor_ref = actor_ref.into();
        let project_id = project_id.into();
        let goal_id = goal_id.into();
        let task_run_id = task_run_id.into();
        let loop_run_id = loop_run_id.into();
        let correlation_id = correlation_id.into();
        let command = Self {
            command_id,
            actor_ref,
            project_id,
            goal_id,
            task_run_id,
            loop_run_id,
            command_kind,
            correlation_id,
            expected_object_version: None,
            requested_at: Utc::now().to_rfc3339(),
            payload,
        };
        command.validate()?;
        Ok(command)
    }

    fn validate(&self) -> Result<(), UserCommandInboxError> {
        for (field, value) in [
            ("command_id", &self.command_id),
            ("actor_ref", &self.actor_ref),
            ("project_id", &self.project_id),
            ("goal_id", &self.goal_id),
            ("task_run_id", &self.task_run_id),
            ("loop_run_id", &self.loop_run_id),
            ("correlation_id", &self.correlation_id),
        ] {
            if value.trim().is_empty() {
                return Err(UserCommandInboxError::MissingField { field });
            }
        }
        if let Some(field) = sensitive_payload_field(&self.payload) {
            return Err(UserCommandInboxError::SensitivePayload { field });
        }
        Ok(())
    }

    fn same_intent(&self, other: &Self) -> bool {
        self.command_id == other.command_id
            && self.actor_ref == other.actor_ref
            && self.project_id == other.project_id
            && self.goal_id == other.goal_id
            && self.task_run_id == other.task_run_id
            && self.loop_run_id == other.loop_run_id
            && self.command_kind == other.command_kind
            && self.correlation_id == other.correlation_id
            && self.expected_object_version == other.expected_object_version
            && self.payload == other.payload
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserCommandEntry {
    pub command: UserCommand,
    pub status: CommandStatus,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum UserCommandInboxError {
    #[error("user command inbox I/O failed: {message}")]
    Io { message: String },
    #[error("user command inbox line {line} is invalid JSON: {message}")]
    InvalidJson { line: usize, message: String },
    #[error("user command contains forbidden sensitive field: {field}")]
    SensitivePayload { field: String },
    #[error("user command field is empty: {field}")]
    MissingField { field: &'static str },
    #[error("command_id already exists with different command content: {command_id}")]
    CommandIdConflict { command_id: String },
    #[error("command_id was not found: {command_id}")]
    CommandNotFound { command_id: String },
    #[error("user command input ended with an incomplete line")]
    PartialLineAtEof,
    #[error("user command line exceeds maximum size of {max_bytes} bytes")]
    InputLineTooLong { max_bytes: usize },
}

/// The result of one non-blocking read from an injected command source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRead {
    Data(Vec<u8>),
    NoInput,
    Eof,
}

/// A source that never waits for more command input.
pub trait NonBlockingCommandReader {
    fn read_available(&mut self) -> Result<CommandRead, UserCommandInboxError>;
}

/// Adapts an injected [`Read`] implementation to the non-blocking command
/// reader contract. The caller must provide a reader that is already
/// configured not to block; this type only maps the read result.
pub struct ReadCommandReader<R> {
    reader: R,
}

impl<R> ReadCommandReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: Read> NonBlockingCommandReader for ReadCommandReader<R> {
    fn read_available(&mut self) -> Result<CommandRead, UserCommandInboxError> {
        let mut bytes = [0_u8; 8192];
        match self.reader.read(&mut bytes) {
            Ok(0) => Ok(CommandRead::Eof),
            Ok(count) => Ok(CommandRead::Data(bytes[..count].to_vec())),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(CommandRead::NoInput)
            }
            Err(error) => Err(io_error(error)),
        }
    }
}

/// Unix file-descriptor adapter for stdin, pipes, and other byte streams.
///
/// The duplicated descriptor is configured with `O_NONBLOCK` and owns its
/// duplicate independently of the caller's Rust handle. Windows has no
/// implementation here yet; use [`ReadCommandReader`] with a platform-owned
/// non-blocking `Read` implementation there.
#[cfg(unix)]
pub struct UnixCommandReader {
    file: std::fs::File,
}

#[cfg(unix)]
impl UnixCommandReader {
    pub fn from_fd(fd: std::os::fd::RawFd) -> Result<Self, UserCommandInboxError> {
        use std::os::fd::FromRawFd;

        let duplicated_fd = unsafe { libc::dup(fd) };
        if duplicated_fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }

        let file = unsafe { std::fs::File::from_raw_fd(duplicated_fd) };
        let flags = unsafe { libc::fcntl(duplicated_fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        if unsafe { libc::fcntl(duplicated_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self { file })
    }

    pub fn stdin() -> Result<Self, UserCommandInboxError> {
        use std::os::fd::AsRawFd;

        Self::from_fd(std::io::stdin().as_raw_fd())
    }
}

#[cfg(unix)]
impl NonBlockingCommandReader for UnixCommandReader {
    fn read_available(&mut self) -> Result<CommandRead, UserCommandInboxError> {
        ReadCommandReader::new(&mut self.file).read_available()
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPollResult {
    NoInput,
    Command(UserCommand),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDrainStopReason {
    LimitReached,
    NoInput,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDrainResult {
    pub commands: Vec<UserCommand>,
    pub stop_reason: CommandDrainStopReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTickResult {
    pub commands: Vec<UserCommand>,
    pub stop_reason: CommandDrainStopReason,
    pub applied_count: usize,
}

#[derive(Debug)]
pub enum RuntimeTickError<E> {
    Poll(UserCommandInboxError),
    SafeBoundary {
        command_index: usize,
        command_id: String,
        applied_count: usize,
        source: E,
    },
}

pub struct RuntimeTick;

impl RuntimeTick {
    /// Drains one bounded command batch and invokes the supplied callback in
    /// input order. The callback is the caller's safe-boundary operation; this
    /// helper does not mutate controller state by itself.
    pub fn run<R, F, E>(
        poller: &mut CommandPoller<R>,
        max_commands: usize,
        mut apply_at_safe_boundary: F,
    ) -> Result<RuntimeTickResult, RuntimeTickError<E>>
    where
        R: NonBlockingCommandReader,
        F: FnMut(&UserCommand) -> Result<(), E>,
    {
        let drain = poller
            .poll_many(max_commands)
            .map_err(RuntimeTickError::Poll)?;
        let mut applied_count = 0;
        for (command_index, command) in drain.commands.iter().enumerate() {
            if let Err(source) = apply_at_safe_boundary(command) {
                return Err(RuntimeTickError::SafeBoundary {
                    command_index,
                    command_id: command.command_id.clone(),
                    applied_count,
                    source,
                });
            }
            applied_count += 1;
        }
        Ok(RuntimeTickResult {
            commands: drain.commands,
            stop_reason: drain.stop_reason,
            applied_count,
        })
    }
}

/// Maximum command line size used by [`CommandPoller::new`].
pub const DEFAULT_MAX_COMMAND_LINE_BYTES: usize = 1024 * 1024;

/// Incrementally extracts complete JSONL commands without treating partial bytes
/// as a command. The reader is injected so platform-specific descriptors remain
/// outside this domain module and the state machine is deterministic in tests.
pub struct CommandPoller<R: NonBlockingCommandReader> {
    reader: R,
    buffer: Vec<u8>,
    line_number: usize,
    max_line_bytes: usize,
    eof: bool,
}

impl<R: NonBlockingCommandReader> CommandPoller<R> {
    pub fn new(reader: R) -> Self {
        Self::new_with_max_line_bytes(reader, DEFAULT_MAX_COMMAND_LINE_BYTES)
    }

    pub fn new_with_max_line_bytes(reader: R, max_line_bytes: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            line_number: 0,
            max_line_bytes,
            eof: false,
        }
    }

    pub fn poll(&mut self) -> Result<CommandPollResult, UserCommandInboxError> {
        loop {
            if let Some(line) = self.take_complete_line()? {
                if line.trim().is_empty() {
                    continue;
                }
                let command = serde_json::from_str::<UserCommand>(&line).map_err(|error| {
                    UserCommandInboxError::InvalidJson {
                        line: self.line_number,
                        message: error.to_string(),
                    }
                })?;
                command.validate()?;
                return Ok(CommandPollResult::Command(command));
            }
            self.ensure_partial_line_within_limit()?;
            if self.eof {
                return if self.buffer.is_empty() {
                    Ok(CommandPollResult::Eof)
                } else {
                    Err(UserCommandInboxError::PartialLineAtEof)
                };
            }
            match self.reader.read_available()? {
                CommandRead::Data(bytes) if bytes.is_empty() => {
                    return Ok(CommandPollResult::NoInput);
                }
                CommandRead::Data(bytes) => self.buffer.extend(bytes),
                CommandRead::NoInput => return Ok(CommandPollResult::NoInput),
                CommandRead::Eof => self.eof = true,
            }
        }
    }

    pub fn poll_many(
        &mut self,
        max_commands: usize,
    ) -> Result<CommandDrainResult, UserCommandInboxError> {
        if max_commands == 0 {
            return Ok(CommandDrainResult {
                commands: Vec::new(),
                stop_reason: CommandDrainStopReason::LimitReached,
            });
        }

        let mut commands = Vec::with_capacity(max_commands.min(8));
        while commands.len() < max_commands {
            match self.poll()? {
                CommandPollResult::Command(command) => commands.push(command),
                CommandPollResult::NoInput => {
                    return Ok(CommandDrainResult {
                        commands,
                        stop_reason: CommandDrainStopReason::NoInput,
                    });
                }
                CommandPollResult::Eof => {
                    return Ok(CommandDrainResult {
                        commands,
                        stop_reason: CommandDrainStopReason::Eof,
                    });
                }
            }
        }

        Ok(CommandDrainResult {
            commands,
            stop_reason: CommandDrainStopReason::LimitReached,
        })
    }

    fn take_complete_line(&mut self) -> Result<Option<String>, UserCommandInboxError> {
        let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') else {
            return Ok(None);
        };
        if newline > self.max_line_bytes {
            return Err(UserCommandInboxError::InputLineTooLong {
                max_bytes: self.max_line_bytes,
            });
        }
        let line_bytes: Vec<u8> = self.buffer.drain(..=newline).collect();
        self.line_number += 1;
        let line_bytes = line_bytes.strip_suffix(b"\n").unwrap_or(&line_bytes);
        let line_bytes = line_bytes.strip_suffix(b"\r").unwrap_or(line_bytes);
        let line = String::from_utf8(line_bytes.to_vec()).map_err(|error| {
            UserCommandInboxError::InvalidJson {
                line: self.line_number,
                message: format!("input is not valid UTF-8: {error}"),
            }
        })?;
        Ok(Some(line))
    }

    fn ensure_partial_line_within_limit(&self) -> Result<(), UserCommandInboxError> {
        if !self.buffer.contains(&b'\n') && self.buffer.len() > self.max_line_bytes {
            return Err(UserCommandInboxError::InputLineTooLong {
                max_bytes: self.max_line_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UserCommandInbox {
    path: PathBuf,
}

impl UserCommandInbox {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn submit(&self, command: UserCommand) -> Result<UserCommandEntry, UserCommandInboxError> {
        command.validate()?;
        let records = self.read_records()?;
        if let Some(existing) = latest_record(&records, &command.command_id) {
            if !existing.command.same_intent(&command) {
                return Err(UserCommandInboxError::CommandIdConflict {
                    command_id: command.command_id,
                });
            }
            return Ok(existing.clone());
        }
        let entry = UserCommandEntry {
            command: command.clone(),
            status: CommandStatus::Pending,
            updated_at: Utc::now().to_rfc3339(),
        };
        self.append_record(&entry)?;
        Ok(entry)
    }

    pub fn update_status(
        &self,
        command_id: &str,
        status: CommandStatus,
    ) -> Result<UserCommandEntry, UserCommandInboxError> {
        let records = self.read_records()?;
        let Some(existing) = latest_record(&records, command_id) else {
            return Err(UserCommandInboxError::CommandNotFound {
                command_id: command_id.into(),
            });
        };
        let entry = UserCommandEntry {
            command: existing.command.clone(),
            status,
            updated_at: Utc::now().to_rfc3339(),
        };
        self.append_record(&entry)?;
        Ok(entry)
    }

    pub fn latest(&self, command_id: &str) -> Result<UserCommandEntry, UserCommandInboxError> {
        latest_record(&self.read_records()?, command_id)
            .cloned()
            .ok_or_else(|| UserCommandInboxError::CommandNotFound {
                command_id: command_id.into(),
            })
    }

    pub fn read_all(&self) -> Result<Vec<UserCommandEntry>, UserCommandInboxError> {
        Ok(self.read_records()?.into_iter().collect())
    }

    pub fn pending_for_loop(
        &self,
        project_id: &str,
        loop_run_id: &str,
    ) -> Result<Vec<UserCommandEntry>, UserCommandInboxError> {
        let records = self.read_records()?;
        let mut seen = HashSet::new();
        let mut latest_records = Vec::new();
        for record in records.into_iter().rev() {
            if seen.insert(record.command.command_id.clone()) {
                latest_records.push(record);
            }
        }
        latest_records.reverse();
        let mut commands = Vec::new();
        for record in latest_records {
            if record.command.project_id == project_id
                && record.command.loop_run_id == loop_run_id
                && matches!(
                    record.status,
                    CommandStatus::Pending | CommandStatus::Classified
                )
            {
                commands.push(record);
            }
        }
        Ok(commands)
    }

    fn read_records(&self) -> Result<Vec<UserCommandEntry>, UserCommandInboxError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&self.path).map_err(io_error)?;
        let reader = BufReader::new(file);
        reader
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line = match line {
                    Ok(line) if !line.trim().is_empty() => line,
                    Ok(_) => return None,
                    Err(error) => return Some(Err(io_error(error))),
                };
                Some(serde_json::from_str(&line).map_err(|error| {
                    UserCommandInboxError::InvalidJson {
                        line: index + 1,
                        message: error.to_string(),
                    }
                }))
            })
            .collect()
    }

    fn append_record(&self, record: &UserCommandEntry) -> Result<(), UserCommandInboxError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let encoded =
            serde_json::to_vec(record).map_err(|error| UserCommandInboxError::InvalidJson {
                line: 0,
                message: error.to_string(),
            })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(io_error)?;
        file.write_all(&encoded).map_err(io_error)?;
        file.write_all(b"\n").map_err(io_error)?;
        file.sync_data().map_err(io_error)
    }
}

fn latest_record<'a>(
    records: &'a [UserCommandEntry],
    command_id: &str,
) -> Option<&'a UserCommandEntry> {
    records
        .iter()
        .rev()
        .find(|record| record.command.command_id == command_id)
}

fn sensitive_payload_field(value: &Value) -> Option<String> {
    match value {
        Value::Object(fields) => fields.iter().find_map(|(key, value)| {
            let normalized: String = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .map(|character| character.to_ascii_lowercase())
                .collect();
            if matches!(
                normalized.as_str(),
                "apikey"
                    | "authorization"
                    | "accesstoken"
                    | "bearertoken"
                    | "clientsecret"
                    | "cookie"
                    | "password"
                    | "privatekey"
                    | "refreshtoken"
                    | "secret"
                    | "sessiontoken"
                    | "sshkey"
                    | "token"
            ) {
                Some(key.clone())
            } else {
                sensitive_payload_field(value)
            }
        }),
        Value::Array(values) => values.iter().find_map(sensitive_payload_field),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn io_error(error: std::io::Error) -> UserCommandInboxError {
    UserCommandInboxError::Io {
        message: error.to_string(),
    }
}
