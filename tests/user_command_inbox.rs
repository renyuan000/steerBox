use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use steerbox::user_command_inbox::UnixCommandReader;
use steerbox::user_command_inbox::{
    CommandDrainStopReason, CommandKind, CommandPollResult, CommandPoller, CommandRead,
    CommandStatus, NonBlockingCommandReader, ReadCommandReader, UserCommand, UserCommandInbox,
    UserCommandInboxError,
};

fn command(command_id: &str, payload: serde_json::Value) -> UserCommand {
    UserCommand::new(
        command_id,
        "operator-1",
        "project-1",
        "goal-1",
        "task-run-1",
        "loop-1",
        CommandKind::Pause,
        "corr-1",
        payload,
    )
    .expect("command is valid")
}

struct ScriptedReader {
    events: VecDeque<Result<CommandRead, UserCommandInboxError>>,
}

impl ScriptedReader {
    fn new(events: impl IntoIterator<Item = Result<CommandRead, UserCommandInboxError>>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl NonBlockingCommandReader for ScriptedReader {
    fn read_available(&mut self) -> Result<CommandRead, UserCommandInboxError> {
        self.events.pop_front().unwrap_or_else(|| {
            Err(UserCommandInboxError::Io {
                message: "scripted reader exhausted".into(),
            })
        })
    }
}

struct ScriptedRead {
    events: VecDeque<io::Result<Vec<u8>>>,
}

impl Read for ScriptedRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.events.pop_front().expect("scripted read event") {
            Ok(bytes) => {
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Err(error) => Err(error),
        }
    }
}

#[test]
fn read_command_reader_maps_would_block_data_eof_and_io_error() {
    let mut reader = ReadCommandReader::new(ScriptedRead {
        events: VecDeque::from([
            Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Ok(b"data".to_vec()),
            Ok(Vec::new()),
            Err(io::Error::from(io::ErrorKind::Interrupted)),
        ]),
    });

    assert_eq!(
        reader.read_available().expect("would block maps"),
        CommandRead::NoInput
    );
    assert_eq!(
        reader.read_available().expect("data maps"),
        CommandRead::Data(b"data".to_vec())
    );
    assert_eq!(reader.read_available().expect("eof maps"), CommandRead::Eof);
    assert!(matches!(
        reader.read_available(),
        Err(UserCommandInboxError::Io { .. })
    ));
}

#[cfg(unix)]
#[test]
fn unix_command_reader_does_not_wait_for_pipe_input_and_detects_eof() {
    let (mut writer, reader_side) = UnixStream::pair().expect("pipe pair creates");
    let mut reader = UnixCommandReader::from_fd(reader_side.as_raw_fd()).expect("fd duplicates");
    drop(reader_side);

    assert_eq!(
        reader.read_available().expect("empty pipe does not block"),
        CommandRead::NoInput
    );
    writer.write_all(b"pipe-data").expect("pipe write succeeds");
    assert_eq!(
        reader.read_available().expect("pipe data reads"),
        CommandRead::Data(b"pipe-data".to_vec())
    );
    drop(writer);
    assert_eq!(
        reader.read_available().expect("closed pipe reads eof"),
        CommandRead::Eof
    );
}

#[test]
fn submit_is_idempotent_and_status_updates_append_latest_record() {
    let path = std::env::temp_dir().join(format!(
        "steerbox-user-command-inbox-{}-{}.jsonl",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = fs::remove_file(&path);
    let inbox = UserCommandInbox::open(&path);
    let first = inbox
        .submit(command(
            "command-1",
            serde_json::json!({"reason": "review"}),
        ))
        .expect("command submits");
    assert_eq!(first.command.expected_object_version, None);
    let retry = inbox
        .submit(command(
            "command-1",
            serde_json::json!({"reason": "review"}),
        ))
        .expect("retry returns prior command");
    assert_eq!(retry, first);

    let applied = inbox
        .update_status("command-1", CommandStatus::Applied)
        .expect("status updates");
    assert_eq!(applied.status, CommandStatus::Applied);
    assert_eq!(inbox.read_all().expect("records read").len(), 2);
    assert!(inbox
        .pending_for_loop("project-1", "loop-1")
        .expect("pending commands read")
        .is_empty());
    assert_eq!(inbox.latest("command-1").expect("latest exists"), applied);
    fs::remove_file(path).expect("temporary inbox removes");
}

#[test]
fn duplicate_command_id_with_different_payload_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "steerbox-user-command-conflict-{}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let inbox = UserCommandInbox::open(&path);
    inbox
        .submit(command(
            "command-1",
            serde_json::json!({"reason": "review"}),
        ))
        .expect("command submits");
    let error = inbox
        .submit(command(
            "command-1",
            serde_json::json!({"reason": "cancel"}),
        ))
        .expect_err("different command content conflicts");
    assert!(matches!(
        error,
        UserCommandInboxError::CommandIdConflict { .. }
    ));
    fs::remove_file(path).expect("temporary inbox removes");
}

#[test]
fn sensitive_command_payload_is_rejected_before_persistence() {
    let path = std::env::temp_dir().join(format!(
        "steerbox-user-command-sensitive-{}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let inbox = UserCommandInbox::open(&path);
    let mut unsafe_command = command("command-1", serde_json::json!({}));
    unsafe_command.payload = serde_json::json!({"access_token": "must-not-persist"});
    let error = inbox
        .submit(unsafe_command)
        .expect_err("sensitive payload is rejected");
    assert!(matches!(
        error,
        UserCommandInboxError::SensitivePayload { .. }
    ));
    assert!(!path.exists());
}

#[test]
fn command_must_bind_to_a_specific_runtime_target() {
    let error = UserCommand::new(
        "command-1",
        "operator-1",
        "project-1",
        "goal-1",
        "task-run-1",
        "",
        CommandKind::Pause,
        "corr-1",
        serde_json::json!({}),
    )
    .expect_err("unbound command is rejected");
    assert!(matches!(
        error,
        UserCommandInboxError::MissingField {
            field: "loop_run_id"
        }
    ));
}

#[test]
fn poll_returns_no_input_without_waiting_for_a_command() {
    let mut poller = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::NoInput)]));

    assert_eq!(
        poller.poll().expect("poll succeeds"),
        CommandPollResult::NoInput
    );
}

#[test]
fn poll_keeps_partial_json_until_a_complete_line_arrives() {
    let expected = command("partial-1", serde_json::json!({"reason": "continue"}));
    let encoded = serde_json::to_vec(&expected).expect("command serializes");
    let split = encoded.len() / 2;
    let mut remainder = encoded[split..].to_vec();
    remainder.push(b'\n');
    let mut poller = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(encoded[..split].to_vec())),
        Ok(CommandRead::NoInput),
        Ok(CommandRead::Data(remainder)),
    ]));

    assert_eq!(
        poller.poll().expect("partial poll succeeds"),
        CommandPollResult::NoInput
    );
    assert_eq!(
        poller.poll().expect("completed poll succeeds"),
        CommandPollResult::Command(expected)
    );
}

#[test]
fn poll_rejects_unterminated_line_over_configured_limit_before_reading_again() {
    let mut poller = CommandPoller::new_with_max_line_bytes(
        ScriptedReader::new([
            Ok(CommandRead::Data(b"12345".to_vec())),
            Err(UserCommandInboxError::Io {
                message: "reader must not be called after limit breach".into(),
            }),
        ]),
        4,
    );

    assert!(matches!(
        poller.poll(),
        Err(UserCommandInboxError::InputLineTooLong { max_bytes: 4 })
    ));
}

#[test]
fn poll_rejects_complete_line_over_configured_limit() {
    let mut poller = CommandPoller::new_with_max_line_bytes(
        ScriptedReader::new([Ok(CommandRead::Data(b"12345\n".to_vec()))]),
        4,
    );

    assert!(matches!(
        poller.poll(),
        Err(UserCommandInboxError::InputLineTooLong { max_bytes: 4 })
    ));
}

#[test]
fn poll_many_limits_flood_and_preserves_command_order() {
    let commands = [
        command("flood-1", serde_json::json!({"order": 1})),
        command("flood-2", serde_json::json!({"order": 2})),
        command("flood-3", serde_json::json!({"order": 3})),
    ];
    let mut input = Vec::new();
    for command in &commands {
        input.extend(serde_json::to_vec(command).expect("command serializes"));
        input.push(b'\n');
    }
    let mut poller = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(input)),
        Ok(CommandRead::NoInput),
    ]));

    let first = poller.poll_many(2).expect("first bounded drain succeeds");
    assert_eq!(
        first
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>(),
        vec!["flood-1", "flood-2"]
    );
    assert_eq!(first.stop_reason, CommandDrainStopReason::LimitReached);

    let second = poller.poll_many(2).expect("second bounded drain succeeds");
    assert_eq!(
        second
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>(),
        vec!["flood-3"]
    );
    assert_eq!(second.stop_reason, CommandDrainStopReason::NoInput);
}

#[test]
fn poll_many_reports_no_input_and_zero_limit_without_reading() {
    let mut no_input = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::NoInput)]));
    let result = no_input.poll_many(2).expect("no input drain succeeds");
    assert!(result.commands.is_empty());
    assert_eq!(result.stop_reason, CommandDrainStopReason::NoInput);

    let mut zero_limit =
        CommandPoller::new(ScriptedReader::new([Err(UserCommandInboxError::Io {
            message: "reader must not be called for zero limit".into(),
        })]));
    let result = zero_limit.poll_many(0).expect("zero limit is a no-op");
    assert!(result.commands.is_empty());
    assert_eq!(result.stop_reason, CommandDrainStopReason::LimitReached);
}

#[test]
fn poll_many_reports_eof_after_draining_available_commands() {
    let expected = command("eof-1", serde_json::json!({"reason": "done"}));
    let mut input = serde_json::to_vec(&expected).expect("command serializes");
    input.push(b'\n');
    let mut poller = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(input)),
        Ok(CommandRead::Eof),
    ]));

    let result = poller.poll_many(2).expect("eof drain succeeds");
    assert_eq!(result.commands, vec![expected]);
    assert_eq!(result.stop_reason, CommandDrainStopReason::Eof);
}

#[test]
fn poll_returns_one_command_per_complete_line() {
    let first = command("line-1", serde_json::json!({"reason": "first"}));
    let second = command("line-2", serde_json::json!({"reason": "second"}));
    let mut input = serde_json::to_vec(&first).expect("first command serializes");
    input.push(b'\n');
    input.extend(serde_json::to_vec(&second).expect("second command serializes"));
    input.push(b'\n');
    let mut poller = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(input))]));

    assert_eq!(
        poller.poll().expect("first poll succeeds"),
        CommandPollResult::Command(first)
    );
    assert_eq!(
        poller.poll().expect("second poll succeeds"),
        CommandPollResult::Command(second)
    );
}

#[test]
fn poll_distinguishes_clean_eof_from_partial_line_at_eof() {
    let mut clean = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Eof)]));
    assert_eq!(
        clean.poll().expect("clean eof succeeds"),
        CommandPollResult::Eof
    );

    let mut partial = CommandPoller::new(ScriptedReader::new([
        Ok(CommandRead::Data(b"{\"command_id\":".to_vec())),
        Ok(CommandRead::Eof),
    ]));
    assert!(matches!(
        partial.poll(),
        Err(UserCommandInboxError::PartialLineAtEof)
    ));
}

#[test]
fn poll_reports_invalid_json_with_physical_line_number() {
    let mut poller = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(
        b"\nnot-json\n".to_vec(),
    ))]));

    let error = poller.poll().expect_err("invalid JSON is rejected");
    assert!(matches!(
        error,
        UserCommandInboxError::InvalidJson { line: 2, .. }
    ));
}

#[test]
fn poll_rejects_sensitive_payload_before_returning_a_command() {
    let mut value = serde_json::to_value(command("sensitive-1", serde_json::json!({})))
        .expect("command converts to JSON");
    value["payload"] = serde_json::json!({"access_token": "redacted"});
    let mut input = serde_json::to_vec(&value).expect("command serializes");
    input.push(b'\n');
    let mut poller = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(input))]));

    assert!(matches!(
        poller.poll(),
        Err(UserCommandInboxError::SensitivePayload { field }) if field == "access_token"
    ));
}

#[test]
fn poll_rejects_invalid_utf8_instead_of_replacing_bytes() {
    let mut poller = CommandPoller::new(ScriptedReader::new([Ok(CommandRead::Data(vec![
        0xff, b'\n',
    ]))]));

    assert!(matches!(
        poller.poll(),
        Err(UserCommandInboxError::InvalidJson { line: 1, message })
            if message.contains("UTF-8")
    ));
}
