#![cfg(unix)]

use std::ffi::CString;
use std::fs::{self, Metadata, OpenOptions};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use steerbox::user_command_inbox::{
    CommandKind, CommandStatus, UserCommand, UserCommandEntry, UserCommandInbox,
    UserCommandInboxError,
};

const PROBE_ROOT: &str = "STEERBOX_INBOX_PATH_TEST_ROOT";
const PROBE_KIND: &str = "STEERBOX_INBOX_PATH_TEST_KIND";

fn command(id: &str) -> UserCommand {
    let mut command = UserCommand::new(
        id,
        "operator-1",
        "project-1",
        "goal-1",
        "task-run-1",
        "loop-1",
        CommandKind::Pause,
        "corr-1",
        serde_json::json!({"reason": "synthetic test"}),
    )
    .unwrap();
    command.requested_at = "2026-11-22T00:00:00Z".into();
    command
}

fn history_bytes() -> Vec<u8> {
    let entry = UserCommandEntry {
        command: command("existing"),
        status: CommandStatus::Pending,
        updated_at: "2026-11-22T00:00:00Z".into(),
    };
    let mut bytes = serde_json::to_vec(&entry).unwrap();
    bytes.push(b'\n');
    bytes
}

fn identity(metadata: &Metadata) -> (u64, u64, u32, u32, u32, u64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
        metadata.nlink(),
    )
}

fn assert_io<T: std::fmt::Debug>(result: Result<T, UserCommandInboxError>) {
    assert!(
        matches!(result, Err(UserCommandInboxError::Io { .. })),
        "{result:?}"
    );
}

fn reject_public_operations(path: &Path) {
    let inbox = UserCommandInbox::open(path);
    assert_io(inbox.read_all());
    assert_io(inbox.latest("existing"));
    assert_io(inbox.pending_for_loop("project-1", "loop-1"));
    assert_io(inbox.submit(command("new")));
    assert_io(inbox.update_status("existing", CommandStatus::Applied));
}

fn reject_leaf(kind: &str) {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.jsonl");
    let leaf = root.path().join("inbox.jsonl");
    let bytes = history_bytes();
    fs::write(&target, &bytes).unwrap();
    match kind {
        "symlink" => symlink(&target, &leaf).unwrap(),
        "dangling" => symlink(root.path().join("absent.jsonl"), &leaf).unwrap(),
        "hardlink" => fs::hard_link(&target, &leaf).unwrap(),
        "directory" => {
            fs::create_dir(&leaf).unwrap();
            fs::write(leaf.join("marker"), b"untouched").unwrap();
        }
        _ => panic!("unknown leaf fixture"),
    }
    let leaf_identity = identity(&fs::symlink_metadata(&leaf).unwrap());
    let target_identity = identity(&fs::metadata(&target).unwrap());
    reject_public_operations(&leaf);
    assert_eq!(
        identity(&fs::symlink_metadata(&leaf).unwrap()),
        leaf_identity
    );
    assert_eq!(identity(&fs::metadata(&target).unwrap()), target_identity);
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(!root.path().join("absent.jsonl").exists());
    if kind == "directory" {
        assert_eq!(fs::read(leaf.join("marker")).unwrap(), b"untouched");
    }
}

#[test]
fn symlink_leaf_is_rejected_without_following_or_mutating_target() {
    reject_leaf("symlink");
}

#[test]
fn dangling_symlink_leaf_is_rejected_without_creating_target() {
    reject_leaf("dangling");
}

#[test]
fn hardlinked_leaf_is_rejected_without_mutating_shared_inode() {
    reject_leaf("hardlink");
}

#[test]
fn directory_leaf_is_rejected_without_mutating_contents() {
    reject_leaf("directory");
}

#[test]
fn symlink_parent_is_rejected_for_existing_and_missing_history() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    let linked = root.path().join("linked");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("inbox.jsonl"), history_bytes()).unwrap();
    symlink(&real, &linked).unwrap();
    let before = identity(&fs::metadata(real.join("inbox.jsonl")).unwrap());
    reject_public_operations(&linked.join("inbox.jsonl"));
    reject_public_operations(&linked.join("not-created").join("inbox.jsonl"));
    assert_eq!(fs::read(real.join("inbox.jsonl")).unwrap(), history_bytes());
    assert_eq!(
        identity(&fs::metadata(real.join("inbox.jsonl")).unwrap()),
        before
    );
    assert!(!real.join("not-created").exists());
    assert_eq!(fs::read_dir(&real).unwrap().count(), 1);
}

#[test]
fn invalid_full_paths_are_rejected_before_creating_any_parent() {
    let root = tempfile::tempdir().unwrap();
    let paths = [
        std::path::PathBuf::new(),
        "/".into(),
        ".".into(),
        "..".into(),
        root.path().join("absent/../inbox.jsonl"),
        root.path().join("absent/inbox.jsonl/"),
        root.path().join("absent/inbox.jsonl/."),
        root.path().join("absent/inbox.jsonl/.."),
        root.path().join(std::ffi::OsString::from_vec(
            b"absent\0/inbox.jsonl".to_vec(),
        )),
    ];
    for path in paths {
        reject_public_operations(&path);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0, "{path:?}");
    }
}

#[test]
fn missing_history_queries_create_neither_directories_nor_sidecars() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        root.path().join("inbox.jsonl"),
        root.path().join("absent/inbox.jsonl"),
    ] {
        let inbox = UserCommandInbox::open(path);
        assert!(inbox.read_all().unwrap().is_empty());
        assert!(inbox
            .pending_for_loop("project-1", "loop-1")
            .unwrap()
            .is_empty());
        assert!(matches!(
            inbox.latest("existing"),
            Err(UserCommandInboxError::CommandNotFound { .. })
        ));
        assert!(matches!(
            inbox.update_status("existing", CommandStatus::Applied),
            Err(UserCommandInboxError::CommandNotFound { .. })
        ));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn identical_intent_ignores_requested_at_and_returns_latest_without_append() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let inbox = UserCommandInbox::open(&path);
    let original = command("existing");
    inbox.submit(original.clone()).unwrap();
    let applied = inbox
        .update_status("existing", CommandStatus::Applied)
        .unwrap();
    let before = fs::read(&path).unwrap();
    let mut retry = original.clone();
    retry.requested_at = "2026-11-22T01:00:00Z".into();
    assert_eq!(inbox.submit(retry).unwrap(), applied);
    assert_eq!(inbox.latest("existing").unwrap(), applied);
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut different_intent = original;
    different_intent.expected_object_version = Some(1);
    assert!(matches!(
        inbox.submit(different_intent),
        Err(UserCommandInboxError::CommandIdConflict { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(inbox.read_all().unwrap().len(), 2);
}

#[test]
fn latest_state_and_pending_order_follow_last_occurrence_without_losing_history() {
    let root = tempfile::tempdir().unwrap();
    let inbox = UserCommandInbox::open(root.path().join("inbox.jsonl"));
    inbox.submit(command("a")).unwrap();
    inbox.submit(command("b")).unwrap();
    inbox.update_status("a", CommandStatus::Classified).unwrap();
    let pending_ids = || {
        inbox
            .pending_for_loop("project-1", "loop-1")
            .unwrap()
            .into_iter()
            .map(|entry| entry.command.command_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(pending_ids(), ["b", "a"]);
    // Repeating a status still appends a historical entry.
    inbox.update_status("a", CommandStatus::Classified).unwrap();
    assert_eq!(inbox.read_all().unwrap().len(), 4);
    let applied = inbox.update_status("a", CommandStatus::Applied).unwrap();
    assert_eq!(inbox.latest("a").unwrap(), applied);
    assert_eq!(pending_ids(), ["b"]);
    let mut other_project = command("other-project");
    other_project.project_id = "project-2".into();
    inbox.submit(other_project).unwrap();
    let mut other_loop = command("other-loop");
    other_loop.loop_run_id = "loop-2".into();
    inbox.submit(other_loop).unwrap();
    assert_eq!(pending_ids(), ["b"]);
    assert_eq!(inbox.read_all().unwrap().len(), 7);
}

#[test]
fn blank_history_lines_are_ignored_without_rewriting_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let mut bytes = b"\n \t\n".to_vec();
    bytes.extend(history_bytes());
    bytes.extend(b"\t\n");
    fs::write(&path, &bytes).unwrap();
    let inbox = UserCommandInbox::open(&path);
    let entries = inbox.read_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].command.command_id, "existing");
    assert_eq!(inbox.latest("existing").unwrap(), entries[0]);
    assert_eq!(inbox.submit(command("existing")).unwrap(), entries[0]);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn invoke(path: &Path, operation: &str) -> Result<(), UserCommandInboxError> {
    let inbox = UserCommandInbox::open(path);
    match operation {
        "read" => inbox.read_all().map(|_| ()),
        "latest" => inbox.latest("existing").map(|_| ()),
        "pending" => inbox.pending_for_loop("project-1", "loop-1").map(|_| ()),
        "submit" => inbox.submit(command("existing")).map(|_| ()),
        "update" => inbox
            .update_status("existing", CommandStatus::Applied)
            .map(|_| ()),
        _ => panic!("unknown inbox operation"),
    }
}

#[test]
fn malformed_history_keeps_physical_line_numbers_and_every_api_scans_the_full_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let mut bytes = b"\n".to_vec();
    bytes.extend(history_bytes());
    bytes.extend(b" \t\n{broken}\n");
    fs::write(&path, &bytes).unwrap();
    for operation in ["read", "latest", "pending", "submit", "update"] {
        assert!(
            matches!(
                invoke(&path, operation),
                Err(UserCommandInboxError::InvalidJson { line: 4, .. })
            ),
            "{operation}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn invalid_utf8_remains_an_io_error_without_appending() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let mut bytes = history_bytes();
    bytes.extend(b"\xff\n");
    fs::write(&path, &bytes).unwrap();
    reject_public_operations(&path);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn final_json_without_lf_is_readable_and_append_does_not_repair_the_tail() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inbox.jsonl");
    let mut bytes = history_bytes();
    assert_eq!(bytes.pop(), Some(b'\n'));
    fs::write(&path, &bytes).unwrap();
    let inbox = UserCommandInbox::open(&path);
    let original = inbox.read_all().unwrap().pop().unwrap();
    assert_eq!(inbox.submit(command("existing")).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    // Characterize the existing format contract: no separator is synthesized.
    let appended = inbox.submit(command("new")).unwrap();
    bytes.extend(serde_json::to_vec(&appended).unwrap());
    bytes.push(b'\n');
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        inbox.read_all(),
        Err(UserCommandInboxError::InvalidJson { line: 1, .. })
    ));
}

#[test]
fn newly_created_parents_and_history_request_private_modes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new/history/inbox.jsonl");
    let inbox = UserCommandInbox::open(&path);
    inbox.submit(command("new")).unwrap();
    for directory in [root.path().join("new"), root.path().join("new/history")] {
        assert_eq!(fs::metadata(directory).unwrap().mode() & 0o077, 0);
    }
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o077, 0);
    assert_eq!(inbox.read_all().unwrap().len(), 1);
}

struct ProbeChild(Option<Child>);

impl ProbeChild {
    fn spawn(root: &Path, kind: &str) -> Self {
        Self(Some(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "inbox_process_probe", "--nocapture"])
                .env(PROBE_ROOT, root)
                .env(PROBE_KIND, kind)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }

    fn finish(mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = self.0.take().unwrap().wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "inbox probe failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                return;
            }
            assert!(Instant::now() < deadline, "inbox operation timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn fifo_probe(root: &Path) {
    let path = root.join("inbox.jsonl");
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let before = identity(&fs::symlink_metadata(&path).unwrap());
    reject_public_operations(&path);
    assert_eq!(identity(&fs::symlink_metadata(&path).unwrap()), before);
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn fifo_operations_are_rejected_within_a_bounded_child_process() {
    let root = tempfile::tempdir().unwrap();
    ProbeChild::spawn(root.path(), "fifo").finish();
}

fn drop_root_privileges(path: &Path) {
    if unsafe { libc::geteuid() } == 0 {
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        // Ownership changes are limited to this probe's synthetic history file.
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 65534, 65534) }, 0);
        assert_eq!(unsafe { libc::setgroups(0, std::ptr::null()) }, 0);
        assert_eq!(unsafe { libc::setgid(65534) }, 0);
        assert_eq!(unsafe { libc::setuid(65534) }, 0);
        assert_eq!(unsafe { libc::geteuid() }, 65534);
        assert_eq!(unsafe { libc::getegid() }, 65534);
        assert_eq!(unsafe { libc::getgroups(0, std::ptr::null_mut()) }, 0);
    }
}

fn permission_probe(root: &Path, scenario: &str) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
    let parent = root.join("history");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let path = parent.join("inbox.jsonl");
    let original_bytes = history_bytes();
    fs::write(&path, &original_bytes).unwrap();
    let mode = match scenario {
        "0200" => 0o200,
        "0400" => 0o400,
        "0644" => 0o644,
        _ => panic!("unknown permission scenario"),
    };
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    drop_root_privileges(&path);
    let before = identity(&fs::metadata(&path).unwrap());
    let inbox = UserCommandInbox::open(&path);
    match scenario {
        "0200" => {
            assert_eq!(
                OpenOptions::new()
                    .read(true)
                    .open(&path)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            // A writable file cannot bypass submit/update's initial history read.
            drop(OpenOptions::new().write(true).open(&path).unwrap());
            reject_public_operations(&path);
        }
        "0400" => {
            assert_eq!(
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            let original = inbox.read_all().unwrap().pop().unwrap();
            assert_eq!(inbox.latest("existing").unwrap(), original);
            assert_eq!(inbox.submit(command("existing")).unwrap(), original);
            assert_io(inbox.submit(command("new")));
            assert_io(inbox.update_status("existing", CommandStatus::Applied));
        }
        "0644" => {
            assert_eq!(inbox.read_all().unwrap().len(), 1);
            let new = inbox.submit(command("new")).unwrap();
            let applied = inbox
                .update_status("existing", CommandStatus::Applied)
                .unwrap();
            let entries = inbox.read_all().unwrap();
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[1], new);
            assert_eq!(entries[2], applied);
            assert_eq!(inbox.latest("existing").unwrap(), applied);
        }
        _ => unreachable!(),
    }
    assert_eq!(identity(&fs::metadata(&path).unwrap()), before);
    assert_eq!(fs::metadata(&parent).unwrap().mode() & 0o777, 0o755);
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    // Restore readability only after proving the inbox preserved its mode.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let bytes = fs::read(&path).unwrap();
    if scenario == "0644" {
        assert!(bytes.starts_with(&original_bytes));
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 3);
    } else {
        assert_eq!(bytes, original_bytes);
    }
}

#[test]
fn write_only_history_fails_at_the_initial_read_even_when_os_write_is_allowed() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    ProbeChild::spawn(root.path(), "0200").finish();
}

#[test]
fn read_only_history_allows_read_and_idempotent_retry_but_not_status_or_new_append() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    ProbeChild::spawn(root.path(), "0400").finish();
}

#[test]
fn successful_appends_preserve_existing_0644_mode_owner_and_inode() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    ProbeChild::spawn(root.path(), "0644").finish();
}

#[test]
fn inbox_process_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let kind = std::env::var(PROBE_KIND).unwrap();
    match kind.as_str() {
        "fifo" => fifo_probe(&root),
        "0200" | "0400" | "0644" => permission_probe(&root, &kind),
        _ => panic!("unknown inbox probe kind"),
    }
}
