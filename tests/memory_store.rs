use std::fs;

use steerbox::memory::{
    MemoryCandidate, MemoryCandidateState, MemoryConfidence, MemoryProvenance, MemoryScope,
};
use steerbox::memory_store::{MemoryStore, MemoryStoreError};

fn test_directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("memory store test directory creates")
}

fn candidate(candidate_id: &str, summary: &str) -> MemoryCandidate {
    MemoryCandidate::new(
        candidate_id,
        MemoryScope::Project,
        "project-memory-store",
        None,
        None,
        summary,
        MemoryProvenance {
            producer: "test-fixture".into(),
            source_kind: "verification".into(),
        },
        vec![format!("event:{candidate_id}")],
        MemoryConfidence::Medium,
        MemoryCandidateState::New,
    )
    .expect("memory candidate is valid")
}

#[test]
fn memory_store_round_trips_redacted_candidates_without_promotion() {
    let directory = test_directory();
    let path = directory.path().join("memory.jsonl");
    let store = MemoryStore::open(&path);
    let original = candidate(
        "candidate-1",
        "verified password=synthetic-password token=synthetic-token",
    );

    store.append(&original).expect("candidate appends");
    let persisted = fs::read_to_string(&path).expect("persisted candidate reads");
    assert!(!persisted.contains("synthetic-password"));
    assert!(!persisted.contains("synthetic-token"));
    assert!(persisted.contains("[REDACTED:PASSWORD]"));
    assert!(persisted.contains("[REDACTED:TOKEN]"));

    let loaded = store.read_all().expect("candidates read");
    assert_eq!(loaded, vec![original.clone()]);
    assert_eq!(loaded[0].lifecycle_state, MemoryCandidateState::New);
    assert_eq!(loaded[0].source_event_refs, vec!["event:candidate-1"]);
    assert!(!path.is_dir());
}

#[test]
fn memory_store_missing_file_read_does_not_create_it() {
    let directory = test_directory();
    let path = directory.path().join("nested/memory.jsonl");
    let store = MemoryStore::open(&path);

    assert!(store
        .read_all()
        .expect("missing memory file reads")
        .is_empty());
    assert!(!path.exists());
    assert!(path.parent().expect("memory parent exists").exists());
}

#[test]
fn memory_store_rejects_duplicate_candidate_ids_without_appending() {
    let directory = test_directory();
    let path = directory.path().join("memory.jsonl");
    let store = MemoryStore::open(&path);
    let original = candidate("duplicate-candidate", "first claim");
    store.append(&original).expect("first candidate appends");
    let before = fs::read(&path).expect("memory file reads");

    let error = store
        .append(&candidate("duplicate-candidate", "second claim"))
        .expect_err("duplicate candidate must fail");
    assert!(matches!(
        error,
        MemoryStoreError::DuplicateCandidate { line: 0 }
    ));
    assert_eq!(fs::read(&path).expect("memory file rereads"), before);
    assert_eq!(store.read_all().expect("candidates reread"), vec![original]);
}

#[test]
fn memory_store_reports_invalid_json_line_without_echoing_sensitive_text() {
    let directory = test_directory();
    let path = directory.path().join("memory.jsonl");
    fs::write(
        &path,
        "\nnot-json\n{\"claim_summary\":\"password=synthetic-secret\"}\n",
    )
    .expect("invalid memory fixture writes");

    let error = MemoryStore::open(&path)
        .read_all()
        .expect_err("invalid JSON must fail");
    assert!(matches!(error, MemoryStoreError::InvalidJson { line: 2 }));
    assert_eq!(
        error.to_string(),
        "memory candidate JSON is invalid at line 2"
    );
    assert!(!error.to_string().contains("synthetic-secret"));
}

#[test]
fn memory_store_reports_invalid_candidate_line_and_schema_without_echoing_summary() {
    let directory = test_directory();
    let path = directory.path().join("memory.jsonl");
    let mut invalid = serde_json::to_value(candidate(
        "invalid-schema",
        "password=synthetic-schema-secret",
    ))
    .expect("candidate serializes");
    invalid["schema_version"] = serde_json::json!(2);
    fs::write(
        &path,
        serde_json::to_string(&invalid).expect("invalid candidate serializes"),
    )
    .expect("invalid candidate fixture writes");

    let error = MemoryStore::open(&path)
        .read_all()
        .expect_err("unsupported schema must fail");
    assert!(matches!(
        error,
        MemoryStoreError::InvalidCandidate { line: 1, .. }
    ));
    assert!(!error.to_string().contains("synthetic-schema-secret"));
}

#[test]
fn memory_store_propagates_later_invalid_candidate_line_after_valid_records() {
    let directory = test_directory();
    let path = directory.path().join("memory.jsonl");
    let valid = serde_json::to_string(&candidate("valid-before-error", "valid claim"))
        .expect("valid candidate serializes");
    fs::write(&path, format!("{valid}\n{{\"schema_version\":2}}\n"))
        .expect("mixed memory fixture writes");

    let error = MemoryStore::open(&path)
        .read_all()
        .expect_err("later invalid candidate must fail");
    assert!(matches!(error, MemoryStoreError::InvalidJson { line: 2 }));
}
