use chrono::{Duration, Utc};
use steerbox::approval_contract::{
    ApprovalDecision, ApprovalRequest, ApprovalScope, ApprovalStatus, ApprovalStore,
};

fn request(request_id: &str) -> ApprovalRequest {
    let now = Utc::now();
    ApprovalRequest::at(
        request_id,
        "project-compat",
        "session-compat",
        "task-compat",
        "read_file",
        "workspace-source",
        "D1",
        "inspect synthetic project source",
        now - Duration::minutes(1),
        now + Duration::days(1),
    )
    .expect("synthetic request is valid")
}

fn changed_field(original: &ApprovalRequest, field: &str) -> ApprovalRequest {
    let mut changed = original.clone();
    match field {
        "project_id" => changed.project_id = "other-project".into(),
        "session_id" => changed.session_id = "other-session".into(),
        "task_run_id" => changed.task_run_id = "other-task".into(),
        "tool_id" => changed.tool_id = "other-tool".into(),
        "capability" => changed.capability = "other-capability".into(),
        "data_class" => changed.data_class = "D2".into(),
        "summary" => changed.summary = "inspect different synthetic source".into(),
        _ => unreachable!("test only changes listed contract fields"),
    }
    changed
}

#[test]
fn submit_retry_with_changed_timestamps_returns_original_without_appending() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let path = dir.path().join("approvals.jsonl");
    let store = ApprovalStore::open(&path);
    let original = request("submit-retry");
    let first = store.submit(original.clone()).expect("initial submit");
    let before = std::fs::read(&path).expect("read pending history");
    let mut retried = original.clone();
    retried.created_at += Duration::minutes(5);
    retried.expires_at += Duration::days(1);
    assert_ne!(retried.created_at, original.created_at);
    assert_ne!(retried.expires_at, original.expires_at);

    let second = store.submit(retried.clone()).expect("same intent retry");

    assert_eq!(second, first);
    assert_eq!(second.request, original);
    assert_eq!(std::fs::read(&path).expect("read retry history"), before);

    store
        .resolve(
            "submit-retry",
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .expect("approve original request");
    assert!(store.consume(&original).expect("consume once grant"));
    let consumed = store.latest("submit-retry").expect("consumed record");
    assert_eq!(consumed.status, ApprovalStatus::Consumed);
    let consumed_bytes = std::fs::read(&path).expect("read consumed history");

    assert_eq!(
        store.submit(retried).expect("terminal same intent retry"),
        consumed
    );
    assert_eq!(
        std::fs::read(&path).expect("read terminal retry history"),
        consumed_bytes
    );
}

#[test]
fn same_scope_resolve_retry_preserves_record_and_history_bytes() {
    for scope in [
        ApprovalScope::Once,
        ApprovalScope::Task,
        ApprovalScope::Session,
    ] {
        let dir = tempfile::tempdir().expect("tempdir creates");
        let path = dir.path().join("approvals.jsonl");
        let store = ApprovalStore::open(&path);
        store.submit(request("resolve-retry")).expect("submit");
        let approved = store
            .resolve("resolve-retry", ApprovalDecision::Approve(scope))
            .expect("initial approval");
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(approved.scope, Some(scope));
        assert!(approved.resolved_at.is_some());
        let before = std::fs::read(&path).expect("read approved history");

        assert_eq!(
            store
                .resolve("resolve-retry", ApprovalDecision::Approve(scope))
                .expect("same scope retry"),
            approved
        );
        assert_eq!(std::fs::read(&path).expect("read retry history"), before);
        let reopened = ApprovalStore::open(&path);
        assert_eq!(
            reopened
                .resolve("resolve-retry", ApprovalDecision::Approve(scope))
                .expect("same scope retry after reopen"),
            approved
        );
        assert_eq!(std::fs::read(&path).expect("read reopened history"), before);
    }
}

#[test]
fn session_scope_allows_repeated_cross_task_consumption_without_appending() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let path = dir.path().join("approvals.jsonl");
    let store = ApprovalStore::open(&path);
    let original = request("session-grant");
    store.submit(original.clone()).expect("submit");
    let approved = store
        .resolve(
            "session-grant",
            ApprovalDecision::Approve(ApprovalScope::Session),
        )
        .expect("approve session");
    let before = std::fs::read(&path).expect("read approved history");
    let mut next_task = original;
    next_task.request_id = "session-action-other-id".into();
    next_task.task_run_id = "other-task".into();

    assert!(store.consume(&next_task).expect("same session other task"));
    assert!(store.consume(&next_task).expect("repeat session action"));
    next_task.request_id = "session-action-third-id".into();
    next_task.task_run_id = "third-task".into();
    assert!(store.consume(&next_task).expect("another task in session"));
    assert_eq!(store.latest("session-grant"), Some(approved.clone()));
    assert_eq!(std::fs::read(&path).expect("read consumed history"), before);
    assert_eq!(
        ApprovalStore::open(&path).latest("session-grant"),
        Some(approved)
    );
}

#[test]
fn session_scope_rejects_other_session_and_changed_action_metadata() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let path = dir.path().join("approvals.jsonl");
    let store = ApprovalStore::open(&path);
    let original = request("session-negative");
    store.submit(original.clone()).expect("submit");
    let approved = store
        .resolve(
            "session-negative",
            ApprovalDecision::Approve(ApprovalScope::Session),
        )
        .expect("approve session");
    let before = std::fs::read(&path).expect("read approved history");

    for field in [
        "session_id",
        "project_id",
        "tool_id",
        "capability",
        "data_class",
        "summary",
    ] {
        let changed = changed_field(&original, field);
        assert!(
            !store.consume(&changed).expect("valid request is denied"),
            "{field}"
        );
        assert_eq!(
            std::fs::read(&path).expect("read denied history"),
            before,
            "{field}"
        );
        assert_eq!(
            store.latest("session-negative"),
            Some(approved.clone()),
            "{field}"
        );
    }
    assert!(store
        .consume(&original)
        .expect("original grant remains usable"));
    assert_eq!(std::fs::read(&path).expect("read final history"), before);
}

#[test]
fn task_scope_rejects_changed_scope_or_action_metadata_without_appending() {
    let dir = tempfile::tempdir().expect("tempdir creates");
    let path = dir.path().join("approvals.jsonl");
    let store = ApprovalStore::open(&path);
    let original = request("task-negative");
    store.submit(original.clone()).expect("submit");
    let approved = store
        .resolve(
            "task-negative",
            ApprovalDecision::Approve(ApprovalScope::Task),
        )
        .expect("approve task");
    let before = std::fs::read(&path).expect("read approved history");
    let mut action = original;
    action.request_id = "task-action-other-id".into();
    assert!(store.consume(&action).expect("same task other request id"));

    for field in [
        "session_id",
        "task_run_id",
        "project_id",
        "tool_id",
        "capability",
        "data_class",
        "summary",
    ] {
        let changed = changed_field(&action, field);
        assert!(
            !store.consume(&changed).expect("valid request is denied"),
            "{field}"
        );
        assert_eq!(
            std::fs::read(&path).expect("read denied history"),
            before,
            "{field}"
        );
        assert_eq!(
            store.latest("task-negative"),
            Some(approved.clone()),
            "{field}"
        );
    }
    assert!(store.consume(&action).expect("repeat original task action"));
    assert_eq!(std::fs::read(&path).expect("read final history"), before);
    assert_eq!(store.latest("task-negative"), Some(approved));
}
