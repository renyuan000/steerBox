use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use steerbox::domain::ToolCallRecord;
use steerbox::tool_call_store::{ToolCallRecordStore, ToolCallStoreError, MAX_TOOL_CALL_PAGE_SIZE};

static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        for _ in 0..128 {
            let id = NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "steerbox-tool-call-store-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create tool call test directory: {error}"),
            }
        }
        panic!("cannot allocate a unique tool call test directory");
    }

    fn file(&self) -> PathBuf {
        self.path.join("tool-calls.jsonl")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn record(id: &str, status: &str) -> ToolCallRecord {
    ToolCallRecord {
        tool_call_id: id.into(),
        tool_id: "git_status".into(),
        project_id: "project-1".into(),
        goal_id: "goal-1".into(),
        task_run_id: "task-run-1".into(),
        loop_run_id: "loop-1".into(),
        step_id: format!("step-{id}"),
        side_effect_class: "none".into(),
        status: status.into(),
        summary: "tool call completed".into(),
        created_at: "2026-10-18T00:00:00Z".into(),
    }
}

#[test]
fn append_redacts_summary_and_read_page_preserves_metadata_only_schema() {
    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");
    let mut started = record("call-started", "started");
    started.summary = "password=synthetic-tool-secret".into();
    store.append(&started).expect("started record appends");
    store
        .append(&record("call-completed", "completed"))
        .expect("completed record appends");
    store
        .append(&record("call-failed", "failed"))
        .expect("failed record appends");

    let persisted = fs::read_to_string(directory.file()).expect("tool call log reads");
    assert!(!persisted.contains("synthetic-tool-secret"));
    assert!(persisted.contains("[REDACTED:PASSWORD]"));
    assert!(!persisted.contains("raw_output"));
    assert!(!persisted.contains("arguments"));
    assert!(!persisted.contains("error_body"));

    let records = store.read_page(0, 10).expect("records read");
    assert_eq!(records.len(), 3);
    assert_eq!(
        records
            .iter()
            .map(|record| record.status.as_str())
            .collect::<Vec<_>>(),
        ["started", "completed", "failed"]
    );

    let json = serde_json::to_value(&records[0]).expect("record serializes");
    let object = json.as_object().expect("record is an object");
    assert_eq!(object.len(), 11);
    for field in [
        "tool_call_id",
        "tool_id",
        "project_id",
        "goal_id",
        "task_run_id",
        "loop_run_id",
        "step_id",
        "side_effect_class",
        "status",
        "summary",
        "created_at",
    ] {
        assert!(
            object.contains_key(field),
            "schema field {field} is present"
        );
    }
}

#[test]
fn read_page_is_bounded_and_skips_blank_lines() {
    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");
    for (id, status) in [
        ("call-1", "started"),
        ("call-2", "completed"),
        ("call-3", "failed"),
    ] {
        store.append(&record(id, status)).expect("record appends");
    }

    let content = fs::read_to_string(directory.file()).expect("tool call log reads");
    fs::write(directory.file(), format!("\n{content}\n \t\n")).expect("blank lines append");
    assert_eq!(
        store
            .read_page(1, 1)
            .expect("bounded page reads")
            .first()
            .map(|record| record.tool_call_id.as_str()),
        Some("call-2")
    );
    assert!(store.read_page(0, 0).expect("empty page reads").is_empty());
    assert!(store
        .read_page(10, 2)
        .expect("out-of-range page reads")
        .is_empty());
}

#[test]
fn read_page_rejects_limits_above_the_hard_cap() {
    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");

    assert!(matches!(
        store.read_page(0, MAX_TOOL_CALL_PAGE_SIZE + 1),
        Err(ToolCallStoreError::PageLimitExceeded {
            requested,
            maximum: MAX_TOOL_CALL_PAGE_SIZE,
        }) if requested == MAX_TOOL_CALL_PAGE_SIZE + 1
    ));
}

#[test]
fn malformed_json_reports_one_based_line_number() {
    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");
    fs::write(directory.file(), "not-json\n").expect("malformed fixture writes");

    assert!(matches!(
        store.read_page(0, 1),
        Err(ToolCallStoreError::InvalidJson { line: 1, .. })
    ));
}

#[test]
fn missing_log_reads_as_empty_without_creating_a_file() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ToolCallRecordStore::open(&file).expect("store opens");

    assert!(store
        .read_page(0, 10)
        .expect("missing log reads")
        .is_empty());
    assert!(!file.exists());
}
