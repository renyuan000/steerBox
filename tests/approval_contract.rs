use chrono::{Duration, Utc};
use steerbox::approval_contract::{
    ApprovalDecision, ApprovalRecord, ApprovalRequest, ApprovalScope, ApprovalStatus, ApprovalStore,
};

fn request(request_id: &str) -> ApprovalRequest {
    let now = Utc::now();
    ApprovalRequest::at(
        request_id,
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
    .expect("request is valid")
}

#[test]
fn submit_is_metadata_only_and_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    let first = store
        .submit(request("approval-1"))
        .expect("submit succeeds");
    let second = store
        .submit(request("approval-1"))
        .expect("retry is idempotent");

    assert_eq!(first, second);
    assert_eq!(first.status, ApprovalStatus::Pending);
    assert_eq!(store.read_all().expect("read succeeds").len(), 1);
}

#[test]
fn conflicting_request_id_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    store.submit(request("approval-1")).expect("first submit");
    let mut conflict = request("approval-1");
    conflict.tool_id = "write_file".into();

    let error = store
        .submit(conflict)
        .expect_err("different intent is rejected");
    assert!(error.to_string().contains("conflict"));
}

#[test]
fn approval_is_idempotent_and_once_scope_is_consumed() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    store.submit(request("approval-1")).expect("submit");

    let approved = store
        .resolve("approval-1", ApprovalDecision::Approve(ApprovalScope::Once))
        .expect("approve");
    let retried = store
        .resolve("approval-1", ApprovalDecision::Approve(ApprovalScope::Once))
        .expect("same approve is idempotent");
    assert_eq!(approved, retried);
    assert_eq!(approved.status, ApprovalStatus::Approved);

    assert!(store
        .consume(&request("approval-1"))
        .expect("consume succeeds"));
    assert!(!store
        .consume(&request("approval-1"))
        .expect("second consume succeeds"));
}

#[test]
fn task_scope_matches_only_same_task_and_metadata() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    store.submit(request("approval-1")).expect("submit");
    store
        .resolve("approval-1", ApprovalDecision::Approve(ApprovalScope::Task))
        .expect("approve");

    assert!(store
        .consume(&request("different-request"))
        .expect("same task allowed"));

    let mut different_task = request("different-task-request");
    different_task.task_run_id = "task-2".into();
    assert!(!store
        .consume(&different_task)
        .expect("different task is denied without an approval"));
}

#[test]
fn expired_approval_does_not_authorize() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    let now = Utc::now();
    let expired = ApprovalRequest::at(
        "approval-expired",
        "project-1",
        "session-1",
        "task-1",
        "read_file",
        "workspace-source",
        "D1",
        "read project source",
        now - Duration::minutes(20),
        now - Duration::minutes(5),
    )
    .expect("request is valid");
    store.submit(expired).expect("submit");
    let error = store
        .resolve(
            "approval-expired",
            ApprovalDecision::Approve(ApprovalScope::Task),
        )
        .expect_err("expired request cannot be approved");
    assert!(error.to_string().contains("terminal"));

    assert!(!store
        .consume(&request("approval-expired"))
        .expect("expired request fails closed"));
    assert_eq!(
        store
            .latest("approval-expired")
            .expect("latest exists")
            .status,
        ApprovalStatus::Expired
    );
}

#[test]
fn approval_summary_is_redacted_before_persistence() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    let now = Utc::now();
    let request = ApprovalRequest::at(
        "approval-summary",
        "project-1",
        "session-1",
        "task-1",
        "read_file",
        "workspace-source",
        "D1",
        "inspect password=do-not-leak",
        now,
        now + Duration::minutes(15),
    )
    .expect("request is valid");
    store.submit(request).expect("submit");
    let content = std::fs::read_to_string(dir.path().join("approvals.jsonl"))
        .expect("approval snapshot is readable");
    assert!(!content.contains("do-not-leak"));
    assert!(content.contains("[REDACTED:PASSWORD]"));
}

#[test]
fn reject_is_terminal_and_cannot_be_overridden() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    store.submit(request("approval-1")).expect("submit");
    store
        .resolve("approval-1", ApprovalDecision::Reject)
        .expect("reject");

    let error = store
        .resolve("approval-1", ApprovalDecision::Approve(ApprovalScope::Task))
        .expect_err("reject cannot be overridden");
    assert!(error.to_string().contains("terminal"));
    assert!(!store
        .consume(&request("approval-1"))
        .expect("reject denies"));
}

#[test]
fn once_approval_does_not_cross_session_or_task() {
    let dir = tempfile::tempdir().unwrap();
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    store.submit(request("once")).unwrap();
    store
        .resolve("once", ApprovalDecision::Approve(ApprovalScope::Once))
        .unwrap();
    let mut different_session = request("once");
    different_session.session_id = "other-session".into();
    assert!(!store.consume(&different_session).unwrap());
    let mut different_task = request("once");
    different_task.task_run_id = "other-task".into();
    assert!(!store.consume(&different_task).unwrap());
    assert!(store.consume(&request("once")).unwrap());
}

#[test]
fn independent_stores_cannot_consume_the_same_once_grant_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.jsonl");
    let first = ApprovalStore::open(&path);
    first.submit(request("once")).unwrap();
    first
        .resolve("once", ApprovalDecision::Approve(ApprovalScope::Once))
        .unwrap();
    let second = ApprovalStore::open(&path);
    assert!(first.consume(&request("once")).unwrap());
    assert!(!second.consume(&request("once")).unwrap());
    let reopened = ApprovalStore::open(&path);
    assert_eq!(
        reopened.latest("once").unwrap().status,
        ApprovalStatus::Consumed
    );
}

#[test]
fn once_approval_expires_before_consumption() {
    let dir = tempfile::tempdir().unwrap();
    let store = ApprovalStore::open(dir.path().join("approvals.jsonl"));
    let now = Utc::now();
    let mut short = request("short");
    short.created_at = now - Duration::seconds(1);
    short.expires_at = now + Duration::milliseconds(200);
    store.submit(short.clone()).unwrap();
    store
        .resolve("short", ApprovalDecision::Approve(ApprovalScope::Once))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(!store.consume(&short).unwrap());
    assert_eq!(
        store.latest("short").unwrap().status,
        ApprovalStatus::Expired
    );
}

fn replay_snapshot(request: &ApprovalRequest, status: ApprovalStatus) -> ApprovalRecord {
    ApprovalRecord {
        request: request.clone(),
        status,
        scope: match status {
            ApprovalStatus::Approved | ApprovalStatus::Consumed => Some(ApprovalScope::Once),
            _ => None,
        },
        resolved_at: match status {
            ApprovalStatus::Approved | ApprovalStatus::Consumed | ApprovalStatus::Rejected => {
                Some(request.created_at + Duration::seconds(1))
            }
            _ => None,
        },
    }
}

fn write_replay_snapshots(path: &std::path::Path, records: &[ApprovalRecord]) {
    let mut encoded = String::new();
    for record in records {
        encoded.push_str(&serde_json::to_string(record).unwrap());
        encoded.push('\n');
    }
    std::fs::write(path, encoded).unwrap();
}

#[test]
fn replay_rejects_changes_to_every_immutable_request_field() {
    let original = request("replay-immutable");
    for field in [
        "project_id",
        "session_id",
        "task_run_id",
        "tool_id",
        "capability",
        "data_class",
        "summary",
        "created_at",
        "expires_at",
    ] {
        let mut changed = original.clone();
        match field {
            "project_id" => changed.project_id = "tampered-project".into(),
            "session_id" => changed.session_id = "tampered-session".into(),
            "task_run_id" => changed.task_run_id = "tampered-task".into(),
            "tool_id" => changed.tool_id = "tampered-tool".into(),
            "capability" => changed.capability = "tampered-capability".into(),
            "data_class" => changed.data_class = "tampered-class".into(),
            "summary" => changed.summary = "tampered-summary".into(),
            "created_at" => changed.created_at += Duration::seconds(1),
            "expires_at" => changed.expires_at += Duration::days(1),
            _ => unreachable!(),
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.jsonl");
        write_replay_snapshots(
            &path,
            &[
                replay_snapshot(&original, ApprovalStatus::Pending),
                replay_snapshot(&changed, ApprovalStatus::Approved),
            ],
        );
        let store = ApprovalStore::open(&path);
        let error = store
            .read_all()
            .expect_err("changed intent must fail closed");
        assert!(!error.to_string().contains("tampered"));
        assert!(store.latest(&original.request_id).is_none());
        assert!(
            store.consume(&changed).is_err(),
            "replayed change in {field}"
        );
    }
}

#[test]
fn replay_rejects_every_illegal_state_transition() {
    let original = request("replay-transition");
    let states = [
        ApprovalStatus::Pending,
        ApprovalStatus::Approved,
        ApprovalStatus::Rejected,
        ApprovalStatus::Consumed,
        ApprovalStatus::Expired,
    ];
    for from in states {
        for to in states {
            if from == to
                || matches!(
                    (from, to),
                    (
                        ApprovalStatus::Pending,
                        ApprovalStatus::Approved
                            | ApprovalStatus::Rejected
                            | ApprovalStatus::Expired
                    ) | (
                        ApprovalStatus::Approved,
                        ApprovalStatus::Consumed | ApprovalStatus::Expired
                    )
                )
            {
                continue;
            }
            let mut records = vec![replay_snapshot(&original, ApprovalStatus::Pending)];
            if matches!(from, ApprovalStatus::Approved | ApprovalStatus::Consumed) {
                records.push(replay_snapshot(&original, ApprovalStatus::Approved));
            }
            records.push(replay_snapshot(&original, from));
            records.push(replay_snapshot(&original, to));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("approvals.jsonl");
            write_replay_snapshots(&path, &records);
            let store = ApprovalStore::open(&path);
            assert!(
                store.read_all().is_err(),
                "illegal transition {from:?} -> {to:?}"
            );
            assert!(store.consume(&original).is_err());
        }
    }
}

#[test]
fn replay_rejects_orphan_decisions_and_inconsistent_snapshot_shapes() {
    let original = request("replay-shape");
    let pending = replay_snapshot(&original, ApprovalStatus::Pending);
    let approved = replay_snapshot(&original, ApprovalStatus::Approved);
    let mut missing_scope = approved.clone();
    missing_scope.scope = None;
    let mut missing_resolution = approved.clone();
    missing_resolution.resolved_at = None;
    let mut pending_grant = pending.clone();
    pending_grant.scope = Some(ApprovalScope::Session);
    let mut rejected_grant = replay_snapshot(&original, ApprovalStatus::Rejected);
    rejected_grant.scope = Some(ApprovalScope::Once);
    let mut consumed_task = replay_snapshot(&original, ApprovalStatus::Consumed);
    consumed_task.scope = Some(ApprovalScope::Task);
    for records in [
        vec![approved.clone()],
        vec![pending.clone(), missing_scope],
        vec![pending.clone(), missing_resolution],
        vec![pending_grant],
        vec![pending.clone(), rejected_grant],
        vec![pending, approved, consumed_task],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.jsonl");
        write_replay_snapshots(&path, &records);
        assert!(ApprovalStore::open(path).read_all().is_err());
    }
}

#[test]
fn replay_rejects_scope_or_resolution_changes_after_approval() {
    let original = request("replay-resolution");
    let pending = replay_snapshot(&original, ApprovalStatus::Pending);
    let approved = replay_snapshot(&original, ApprovalStatus::Approved);
    let mut broader = approved.clone();
    broader.scope = Some(ApprovalScope::Session);
    let mut new_resolution = approved.clone();
    new_resolution.resolved_at = Some(original.created_at + Duration::seconds(2));
    let mut changed_consumption = replay_snapshot(&original, ApprovalStatus::Consumed);
    changed_consumption.resolved_at = Some(original.created_at + Duration::seconds(2));
    let mut changed_expiry = approved.clone();
    changed_expiry.status = ApprovalStatus::Expired;
    changed_expiry.scope = None;
    changed_expiry.resolved_at = None;
    for changed in [broader, new_resolution, changed_consumption, changed_expiry] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.jsonl");
        write_replay_snapshots(&path, &[pending.clone(), approved.clone(), changed]);
        assert!(ApprovalStore::open(path).read_all().is_err());
    }
}

#[test]
fn replay_allows_exact_duplicate_snapshots_and_legitimate_api_histories() {
    let original = request("replay-duplicates");
    let pending = replay_snapshot(&original, ApprovalStatus::Pending);
    let approved = replay_snapshot(&original, ApprovalStatus::Approved);
    let consumed = replay_snapshot(&original, ApprovalStatus::Consumed);
    let rejected = replay_snapshot(&original, ApprovalStatus::Rejected);
    let expired_pending = replay_snapshot(&original, ApprovalStatus::Expired);
    let mut expired_approved = approved.clone();
    expired_approved.status = ApprovalStatus::Expired;
    for records in [
        vec![pending.clone(), pending.clone()],
        vec![
            pending.clone(),
            approved.clone(),
            approved.clone(),
            consumed.clone(),
            consumed,
        ],
        vec![pending.clone(), rejected.clone(), rejected],
        vec![pending.clone(), expired_pending.clone(), expired_pending],
        vec![
            pending,
            approved,
            expired_approved.clone(),
            expired_approved,
        ],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.jsonl");
        write_replay_snapshots(&path, &records);
        let store = ApprovalStore::open(&path);
        assert_eq!(
            store.read_all().unwrap(),
            vec![records.last().unwrap().clone()]
        );
    }
}

#[test]
fn replay_errors_do_not_echo_invalid_record_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.jsonl");
    let mut record = serde_json::to_value(replay_snapshot(
        &request("bad-record"),
        ApprovalStatus::Pending,
    ))
    .unwrap();
    record["status"] = serde_json::json!("synthetic-private-marker");
    std::fs::write(&path, format!("{record}\n")).unwrap();
    let error = ApprovalStore::open(path).read_all().unwrap_err();
    assert!(!error.to_string().contains("synthetic-private-marker"));
    assert!(!format!("{error:?}").contains("synthetic-private-marker"));
}
