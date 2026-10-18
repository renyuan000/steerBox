use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

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
    #[cfg(unix)]
    set_owner_only_permissions(&directory.file());

    assert!(matches!(
        store.read_page(0, 1),
        Err(ToolCallStoreError::InvalidJson { line: 1, .. })
    ));
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("owner-only permissions apply");
}

#[cfg(unix)]
#[test]
fn newly_created_store_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");
    store
        .append(&record("private", "started"))
        .expect("record appends");

    let mode = fs::metadata(directory.file())
        .expect("store metadata reads")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "store must not be group/world accessible");
}

#[cfg(unix)]
#[test]
fn existing_non_private_store_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    fs::write(directory.file(), "").expect("store fixture writes");
    fs::set_permissions(directory.file(), fs::Permissions::from_mode(0o644))
        .expect("insecure permissions apply");

    assert!(matches!(
        ToolCallRecordStore::open(directory.file()),
        Err(ToolCallStoreError::UnsafePermissions)
    ));
}

#[cfg(unix)]
#[test]
fn append_rejects_store_path_replaced_by_symlink() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let target = directory.path.join("target.jsonl");
    let store = ToolCallRecordStore::open(&file).expect("store opens before file exists");
    fs::write(&target, "").expect("target creates");
    std::os::unix::fs::symlink(&target, &file).expect("symlink creates");

    assert!(matches!(
        store.append(&record("symlink-race", "started")),
        Err(ToolCallStoreError::UnsafePath)
    ));
}

#[test]
fn concurrent_appends_produce_parseable_complete_json_lines() {
    let directory = TestDirectory::new();
    let store = ToolCallRecordStore::open(directory.file()).expect("store opens");
    let workers = 4;
    let records_per_worker = 32;
    let mut handles = Vec::new();

    for worker in 0..workers {
        let store = store.clone();
        handles.push(thread::spawn(move || {
            for sequence in 0..records_per_worker {
                let id = format!("concurrent-{worker}-{sequence}");
                store
                    .append(&record(&id, "started"))
                    .expect("concurrent record appends");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("append worker joins");
    }

    let content = fs::read_to_string(directory.file()).expect("store reads");
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), workers * records_per_worker);
    for line in lines {
        let parsed: ToolCallRecord = serde_json::from_str(line).expect("line is complete JSON");
        assert_eq!(parsed.status, "started");
    }
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

#[cfg(unix)]
#[test]
fn open_rejects_symlink_store_paths() {
    let directory = TestDirectory::new();
    let target = directory.path.join("target.jsonl");
    let link = directory.path.join("link.jsonl");
    fs::write(&target, "").expect("target creates");
    std::os::unix::fs::symlink(&target, &link).expect("symlink creates");

    assert!(matches!(
        ToolCallRecordStore::open(&link),
        Err(ToolCallStoreError::UnsafePath)
    ));
}

#[test]
fn open_rejects_non_regular_store_paths() {
    let directory = TestDirectory::new();

    assert!(matches!(
        ToolCallRecordStore::open(directory.path.clone()),
        Err(ToolCallStoreError::UnsafePath)
    ));
}

#[test]
fn concurrent_append_keeps_each_metadata_record_parseable() {
    const WRITERS: usize = 16;
    const RECORDS_PER_WRITER: usize = 8;
    let directory = TestDirectory::new();
    let store =
        Arc::new(ToolCallRecordStore::open(directory.file()).expect("concurrent store opens"));
    let barrier = Arc::new(Barrier::new(WRITERS));
    let handles = (0..WRITERS)
        .map(|writer| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for sequence in 0..RECORDS_PER_WRITER {
                    store
                        .append(&record(&format!("call-{writer}-{sequence}"), "completed"))
                        .expect("concurrent record appends");
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("append writer joins");
    }

    let records = store
        .read_page(0, WRITERS * RECORDS_PER_WRITER)
        .expect("concurrent records read");
    assert_eq!(records.len(), WRITERS * RECORDS_PER_WRITER);
    let mut ids = records
        .iter()
        .map(|record| record.tool_call_id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), WRITERS * RECORDS_PER_WRITER);
}

#[cfg(unix)]
#[test]
fn store_file_does_not_grant_group_or_other_write_permission() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ToolCallRecordStore::open(&file).expect("permission store opens");
    store
        .append(&record("call-permissions", "completed"))
        .expect("permission record appends");

    let mode = fs::metadata(file)
        .expect("permission metadata reads")
        .permissions()
        .mode();
    assert_eq!(mode & 0o022, 0, "store must not be group/other writable");
}

#[test]
fn preexisting_lock_sidecar_is_not_read_or_overwritten_as_store_data() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let lock_path = PathBuf::from(format!("{}.lock", file.display()));
    let lock_contents = "fixture lock marker\n";
    fs::write(&lock_path, lock_contents).expect("lock sidecar creates");
    #[cfg(unix)]
    set_owner_only_permissions(&lock_path);
    let store = ToolCallRecordStore::open(&file).expect("lock-boundary store opens");

    store
        .append(&record("call-lock-boundary", "completed"))
        .expect("record appends beside lock sidecar");
    let records = store.read_page(0, 1).expect("store records read");
    assert_eq!(records[0].tool_call_id, "call-lock-boundary");
    assert_eq!(
        fs::read_to_string(&lock_path).expect("lock sidecar reads"),
        lock_contents
    );
}
