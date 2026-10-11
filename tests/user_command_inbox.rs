use std::collections::VecDeque;
use std::fs;
use steerbox::user_command_inbox::{
    CommandKind, CommandPollResult, CommandPoller, CommandRead, CommandStatus,
    NonBlockingCommandReader, UserCommand, UserCommandInbox, UserCommandInboxError,
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
