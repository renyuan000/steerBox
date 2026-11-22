#![cfg(unix)]

use chrono::{Duration, Utc};
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;
use steerbox::approval_contract::{
    ApprovalError, ApprovalRecord, ApprovalRequest, ApprovalStatus, ApprovalStore,
};

fn pending_history() -> Vec<u8> {
    let now = Utc::now();
    let request = ApprovalRequest::at(
        "approval-1",
        "project-1",
        "session-1",
        "task-1",
        "read_file",
        "workspace-source",
        "D1",
        "read project source",
        now,
        now + Duration::minutes(15),
    )
    .unwrap();
    let record = ApprovalRecord {
        request,
        status: ApprovalStatus::Pending,
        scope: None,
        resolved_at: None,
    };
    let mut encoded = serde_json::to_vec(&record).unwrap();
    encoded.push(b'\n');
    encoded
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn identity(path: &Path) -> (u64, u64, u32, u32, u32, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
        metadata.nlink(),
    )
}

#[test]
fn data_symlink_is_rejected_without_following_valid_pending_history() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("approvals.jsonl");
    let lock = root.path().join("approvals.jsonl.lock");
    let target = root.path().join("protected-history.jsonl");
    let history = pending_history();
    write_private(&target, &history);
    write_private(&lock, b"lock marker\n");
    symlink(&target, &data).unwrap();
    let target_before = identity(&target);
    let link_before = identity(&data);
    let lock_before = identity(&lock);

    let result = ApprovalStore::open(&data).read_all();

    assert_eq!(fs::read(&target).unwrap(), history);
    assert_eq!(identity(&target), target_before);
    assert_eq!(identity(&data), link_before);
    assert_eq!(fs::read_link(&data).unwrap(), target);
    assert_eq!(identity(&lock), lock_before);
    assert_eq!(fs::read(&lock).unwrap(), b"lock marker\n");
    assert!(
        matches!(result, Err(ApprovalError::Io(_))),
        "data symlink must fail at the I/O boundary, got {result:?}"
    );
}

#[test]
fn real_lock_symlink_is_rejected_without_locking_its_target() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("approvals.jsonl");
    let lock = root.path().join("approvals.jsonl.lock");
    let target = root.path().join("protected-lock-target");
    let history = pending_history();
    write_private(&data, &history);
    write_private(&target, &history);
    symlink(&target, &lock).unwrap();
    let data_before = identity(&data);
    let target_before = identity(&target);
    let link_before = identity(&lock);

    let result = ApprovalStore::open(&data).read_all();

    assert_eq!(fs::read(&data).unwrap(), history);
    assert_eq!(identity(&data), data_before);
    assert_eq!(fs::read(&target).unwrap(), history);
    assert_eq!(identity(&target), target_before);
    assert_eq!(identity(&lock), link_before);
    assert_eq!(fs::read_link(&lock).unwrap(), target);
    assert!(
        matches!(result, Err(ApprovalError::Io(_))),
        "real basename.lock symlink must fail at the I/O boundary, got {result:?}"
    );
}
