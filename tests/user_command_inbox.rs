use std::fs;
use steerbox::user_command_inbox::{
    CommandKind, CommandStatus, UserCommand, UserCommandInbox, UserCommandInboxError,
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
