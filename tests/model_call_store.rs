use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use steerbox::model_call_store::{ModelCallStore, ModelCallStoreError};
use steerbox::model_contracts::ModelCallRecord;

static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        for _ in 0..128 {
            let id = NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "steerbox-model-call-store-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create model call test directory: {error}"),
            }
        }
        panic!("cannot allocate a unique model call test directory");
    }

    fn file(&self) -> PathBuf {
        self.path.join("model-calls.jsonl")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        // This path was exclusively created by this instance. Drop also runs
        // during panic unwinding, but never panics itself if cleanup fails.
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn record(id: &str) -> ModelCallRecord {
    ModelCallRecord {
        model_call_id: id.into(),
        task_id: "task-1".into(),
        task_run_id: "task-run-1".into(),
        agent_role: "implementer".into(),
        provider_id: "fixture".into(),
        endpoint_id: "local://fixture".into(),
        model_id: "fixture-model".into(),
        route_decision_id: "route-1".into(),
        resolved_prompt_pack_id: "prompt-1".into(),
        input_context_hash: "sha256:input".into(),
        output_artifact_ref: format!("trace://{id}/output"),
        latency_ms: Some(12),
        input_tokens: Some(20),
        output_tokens: Some(30),
        estimated_cost: Some(0.0),
        streaming: false,
        finish_reason: "stop".into(),
        tool_call_count: 0,
        verification_status: "not_run".into(),
        error_class: None,
        started_at: "2026-10-10T00:00:00Z".into(),
        completed_at: Some("2026-10-10T00:00:01Z".into()),
        created_at: "2026-10-10T00:00:01Z".into(),
    }
}

#[test]
fn append_and_read_all_preserve_append_only_order() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");

    store.append(&record("call-1")).expect("first append");
    store.append(&record("call-2")).expect("second append");

    let records = store.read_all().expect("records read");
    assert_eq!(records, vec![record("call-1"), record("call-2")]);
    let lines = fs::read_to_string(&file).expect("jsonl exists");
    assert_eq!(lines.lines().count(), 2);
}

#[test]
fn missing_model_call_log_reads_as_empty() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");

    assert!(store.read_all().expect("empty log reads").is_empty());
    assert!(!file.exists());
}

#[test]
fn malformed_json_reports_the_one_based_line_number() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let first_line = serde_json::to_string(&record("call-1")).expect("record encodes");
    fs::write(&file, format!("{first_line}\nnot-json\n")).expect("fixture writes");
    let store = ModelCallStore::open(&file).expect("store opens");

    let error = store.read_all().expect_err("malformed line must fail");
    assert!(matches!(
        error,
        ModelCallStoreError::InvalidJson { line: 2, .. }
    ));
}

#[test]
fn append_accepts_a_record_with_the_secret_free_field_schema() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");
    let safe = record("call-safe");
    safe.validate_secret_free().expect("fixture is secret-free");

    store.append(&safe).expect("safe record appends");
    assert_eq!(store.read_all().expect("safe record reads").len(), 1);
}

#[test]
fn unknown_authentication_fields_are_rejected_on_deserialization_and_read() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");
    for field in [
        "api_key",
        "authorization",
        "credential",
        "auth_ref",
        "token",
    ] {
        let mut value = serde_json::to_value(record("call-1")).expect("record encodes");
        value[field] = serde_json::Value::String("synthetic-fixture-only".into());
        assert!(serde_json::from_value::<ModelCallRecord>(value.clone()).is_err());
        fs::write(&file, format!("{value}\n")).expect("fixture writes");
        assert!(matches!(
            store.read_all().expect_err("unknown field must fail"),
            ModelCallStoreError::InvalidJson { line: 1, .. }
        ));
    }
}

#[test]
fn blank_lines_are_skipped_without_changing_error_line_numbers() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let first_line = serde_json::to_string(&record("call-1")).expect("record encodes");
    fs::write(&file, format!("\n{first_line}\n \t\n")).expect("fixture writes");
    let store = ModelCallStore::open(&file).expect("store opens");
    assert_eq!(
        store.read_all().expect("blank lines skipped"),
        [record("call-1")]
    );

    fs::write(&file, format!("\n{first_line}\n \t\nnot-json\n")).expect("malformed fixture writes");
    assert!(matches!(
        store.read_all().expect_err("malformed line must fail"),
        ModelCallStoreError::InvalidJson { line: 4, .. }
    ));
}

#[test]
fn a_directory_as_log_target_returns_io_errors_for_append_and_read() {
    let directory = TestDirectory::new();
    let store = ModelCallStore::open(&directory.path).expect("store opens");
    assert!(matches!(
        store
            .append(&record("call-1"))
            .expect_err("directory cannot be appended"),
        ModelCallStoreError::Io(_)
    ));
    assert!(matches!(
        store
            .read_all()
            .expect_err("directory cannot be read as log"),
        ModelCallStoreError::Io(_)
    ));
}

#[test]
fn a_file_as_parent_returns_an_io_error_on_open() {
    let directory = TestDirectory::new();
    let parent_file = directory.path.join("not-a-directory");
    fs::write(&parent_file, "fixture").expect("parent fixture writes");
    assert!(matches!(
        ModelCallStore::open(parent_file.join("model-calls.jsonl"))
            .expect_err("file cannot be a parent directory"),
        ModelCallStoreError::Io(_)
    ));
}

#[test]
fn test_directories_are_unique_and_cleaned_on_drop() {
    let first = TestDirectory::new();
    let second = TestDirectory::new();
    assert_ne!(first.path, second.path);
    let path = first.path.clone();
    fs::write(first.file(), "fixture").expect("cleanup fixture writes");
    drop(first);
    assert!(!path.exists());
}

#[test]
fn test_directory_is_cleaned_during_panic_unwinding() {
    let directory = TestDirectory::new();
    let path = directory.path.clone();
    fs::write(directory.file(), "fixture").expect("cleanup fixture writes");
    let result = std::panic::catch_unwind(move || {
        let _owned_directory = directory;
        panic!("synthetic test panic");
    });
    assert!(result.is_err());
    assert!(!path.exists());
}

fn assert_invalid_cost_does_not_create_or_change_log(cost: f64) {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");
    let mut invalid = record("call-invalid");
    invalid.estimated_cost = Some(cost);

    let error = store.append(&invalid).expect_err("invalid cost must fail");
    assert!(error.to_string().contains("invalid model call contract"));
    assert!(matches!(error, ModelCallStoreError::InvalidContract(_)));
    assert!(!file.exists(), "invalid append must not create a log");

    store.append(&record("call-1")).expect("valid append");
    let bytes_before = fs::read(&file).expect("read original bytes");
    store.append(&invalid).expect_err("invalid cost must fail");
    assert_eq!(fs::read(&file).expect("read unchanged bytes"), bytes_before);
}

#[test]
fn append_rejects_negative_cost_without_creating_or_changing_log() {
    assert_invalid_cost_does_not_create_or_change_log(-1.0);
}

#[test]
fn append_rejects_nan_cost_without_creating_or_changing_log() {
    assert_invalid_cost_does_not_create_or_change_log(f64::NAN);
}

#[test]
fn append_rejects_infinite_cost_without_creating_or_changing_log() {
    assert_invalid_cost_does_not_create_or_change_log(f64::INFINITY);
}

#[test]
fn append_rejects_negative_infinite_cost_without_creating_or_changing_log() {
    assert_invalid_cost_does_not_create_or_change_log(f64::NEG_INFINITY);
}

#[test]
fn append_rejects_blank_identity_without_creating_log() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");
    let mut invalid = record("call-invalid");
    invalid.task_run_id = " \t".into();
    let error = store
        .append(&invalid)
        .expect_err("blank identity must fail");
    assert!(error.to_string().contains("invalid model call contract"));
    assert!(matches!(error, ModelCallStoreError::InvalidContract(_)));
    assert!(!file.exists());
}

#[test]
fn read_rejects_invalid_identity_or_cost_with_actual_line_and_no_record_text() {
    let directory = TestDirectory::new();
    let file = directory.file();
    let store = ModelCallStore::open(&file).expect("store opens");
    let first_line = serde_json::to_string(&record("call-1")).expect("record encodes");
    for (field, invalid_value) in [
        ("task_id", serde_json::json!(" \t")),
        ("estimated_cost", serde_json::json!(-1.0)),
    ] {
        let mut value =
            serde_json::to_value(record("synthetic-private-marker")).expect("record becomes JSON");
        value[field] = invalid_value;
        fs::write(&file, format!("{first_line}\n\n{value}\n")).expect("fixture writes");
        let error = store.read_all().expect_err("invalid contract must fail");
        let message = error.to_string();
        assert!(message.contains("invalid model call contract"));
        assert!(message.contains("line 3"));
        assert!(!message.contains("synthetic-private-marker"));
        assert!(!format!("{error:?}").contains("synthetic-private-marker"));
        assert!(matches!(
            error,
            ModelCallStoreError::InvalidContractAtLine { line: 3, .. }
        ));
    }
}
