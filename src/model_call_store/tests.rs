use super::*;
use std::fs;
use std::os::unix::fs::MetadataExt;

fn success_record() -> ModelCallRecord {
    serde_json::from_value(serde_json::json!({
        "model_call_id": "call-1", "task_id": "task-1", "task_run_id": "run-1",
        "agent_role": "implementer", "provider_id": "fixture",
        "endpoint_id": "local://fixture", "model_id": "fixture-model",
        "route_decision_id": "route-1", "resolved_prompt_pack_id": "prompt-1",
        "input_context_hash": "sha256:input", "output_artifact_ref": "trace://output",
        "latency_ms": 12, "input_tokens": 20, "output_tokens": 30,
        "estimated_cost": 0.0, "streaming": false, "finish_reason": "stop",
        "tool_call_count": 0, "verification_status": "not_run", "error_class": null,
        "started_at": "2026-11-22T00:00:00Z",
        "completed_at": "2026-11-22T00:00:01Z", "created_at": "2026-11-22T00:00:01Z"
    }))
    .unwrap()
}

fn failure_record() -> ModelCallFailureRecord {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "model_call_id": "call-1", "task_id": "task-1",
        "task_run_id": "run-1", "agent_role": "implementer", "provider_id": "fixture",
        "endpoint_id": "local://fixture", "model_id": "fixture-model",
        "route_decision_id": "route-1", "resolved_prompt_pack_id": "prompt-1",
        "started_at": "2026-11-22T00:00:00Z", "ended_at": "2026-11-22T00:00:01Z",
        "failure_stage": "provider_request", "failure_class": "timeout",
        "input_context_hash": null, "output_artifact_ref": null, "finish_reason": null,
        "input_tokens": null, "output_tokens": null, "estimated_cost": null
    }))
    .unwrap()
}

#[derive(Default)]
struct FaultIo {
    events: Vec<&'static str>,
    fail: Option<&'static str>,
    rename: Option<(PathBuf, PathBuf)>,
    expected_file: Option<(u64, u64)>,
    expected_directory_witness: Option<(u64, u64)>,
    lock_contender: Option<File>,
}

impl FaultIo {
    fn at(stage: &'static str) -> Self {
        Self {
            fail: Some(stage),
            ..Self::default()
        }
    }

    fn stage(&mut self, stage: &'static str) -> io::Result<()> {
        if let Some(contender) = &self.lock_contender {
            assert!(
                matches!(contender.try_lock(), Err(fs::TryLockError::WouldBlock)),
                "real lock was released before {stage}"
            );
        }
        self.events.push(stage);
        if self.fail == Some(stage) {
            Err(io::Error::other("synthetic-private-io"))
        } else {
            Ok(())
        }
    }
}

impl ModelStoreIo for FaultIo {
    fn after_lock(&mut self) -> io::Result<()> {
        self.stage("locked")?;
        if let Some((old, pinned)) = &self.rename {
            fs::rename(old, pinned)?;
            fs::create_dir(old)?;
            fs::write(old.join("calls.jsonl"), b"replacement-sentinel")?;
            fs::write(old.join("calls.failures.jsonl"), b"replacement-sentinel")?;
        }
        Ok(())
    }

    fn write(&mut self, file: &mut File, encoded: &[u8]) -> io::Result<()> {
        let metadata = file.metadata()?;
        self.expected_file = Some((metadata.dev(), metadata.ino()));
        if self.fail == Some("write") {
            file.write_all(&encoded[..encoded.len() / 2])?;
        }
        self.stage("write")?;
        file.write_all(encoded)
    }

    fn sync_file(&mut self, file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        assert_eq!(self.expected_file, Some((metadata.dev(), metadata.ino())));
        self.stage("file")?;
        file.sync_data()
    }

    fn sync_directory(&mut self, parent: &AuditParent) -> io::Result<()> {
        if let Some(expected) = self.expected_directory_witness {
            let witness = parent.sibling(std::ffi::OsStr::new("directory-witness"))?;
            let metadata = witness.open_read()?.unwrap().metadata()?;
            assert_eq!(expected, (metadata.dev(), metadata.ino()));
        }
        self.stage("directory")?;
        parent.sync_directory()
    }
}

fn append(
    store: &ModelCallStore,
    failure: bool,
    io: &mut FaultIo,
) -> Result<(), ModelCallStoreError> {
    if failure {
        store.append_failure_with_io(&failure_record(), io)
    } else {
        store.append_with_io(&success_record(), io)
    }
}

fn assert_io_error(error: ModelCallStoreError) {
    assert!(!error.to_string().contains("synthetic-private-io"));
    assert!(!format!("{error:?}").contains("synthetic-private-io"));
    let ModelCallStoreError::Io(error) = error else {
        panic!("expected typed I/O error");
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

fn uncertain_retry(failure: bool, stage: &'static str) {
    let directory = tempfile::tempdir().unwrap();
    let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
    let path = store.storage_paths()[usize::from(failure)].clone();
    let mut first = FaultIo::at(stage);
    let error = append(&store, failure, &mut first).expect_err("first barrier must fail");
    let bytes = fs::read(&path).unwrap();
    assert!(
        bytes.ends_with(b"\n"),
        "uncertain full record remains visible"
    );
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    // Check retry before diagnostics: the old early-Ok bug must be reproduced.
    let mut retry = FaultIo::at(stage);
    let metadata = fs::metadata(&path).unwrap();
    retry.expected_file = Some((metadata.dev(), metadata.ino()));
    let retry_error = append(&store, failure, &mut retry)
        .expect_err("identical retry cannot ACK while its barrier still fails");
    let expected = if stage == "file" {
        vec!["locked", "file"]
    } else {
        vec!["locked", "file", "directory"]
    };
    let mut first_expected = vec!["locked", "write"];
    first_expected.extend_from_slice(&expected[1..]);
    assert_eq!(first.events, first_expected);
    assert_eq!(retry.events, expected);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_io_error(error);
    assert_io_error(retry_error);
    let mut recovered = FaultIo {
        expected_file: retry.expected_file,
        ..FaultIo::default()
    };
    append(&store, failure, &mut recovered).unwrap();
    assert_eq!(recovered.events, ["locked", "file", "directory"]);
    assert_eq!(fs::read(path).unwrap(), bytes, "retry must never append");
}

#[test]
fn success_identical_retry_repeats_failed_file_barrier() {
    uncertain_retry(false, "file");
}

#[test]
fn failure_identical_retry_repeats_failed_file_barrier() {
    uncertain_retry(true, "file");
}

#[test]
fn success_identical_retry_repeats_failed_directory_barrier() {
    uncertain_retry(false, "directory");
}

#[test]
fn failure_identical_retry_repeats_failed_directory_barrier() {
    uncertain_retry(true, "directory");
}

#[test]
fn complete_append_orders_both_barriers_for_both_channels() {
    for failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let mut io = FaultIo::default();
        append(&store, failure, &mut io).unwrap();
        assert_eq!(io.events, ["locked", "write", "file", "directory"]);
    }
}

#[test]
fn partial_write_stops_before_barriers_and_preserves_uncertain_bytes() {
    for failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let mut io = FaultIo::at("write");
        let error = append(&store, failure, &mut io).unwrap_err();
        assert_eq!(io.events, ["locked", "write"]);
        let path = store.storage_paths()[usize::from(failure)].clone();
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        assert!(!bytes.ends_with(b"\n"));
        assert_io_error(error);
        let mut retry = FaultIo::default();
        assert!(matches!(
            append(&store, failure, &mut retry),
            Err(ModelCallStoreError::InvalidJson { line: 1, .. })
        ));
        assert_eq!(retry.events, ["locked"]);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn each_append_keeps_lock_history_write_and_sync_in_one_parent() {
    for failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("original");
        let pinned = directory.path().join("moved");
        let store = ModelCallStore::open(old.join("calls.jsonl")).unwrap();
        fs::write(old.join("directory-witness"), b"pinned-directory").unwrap();
        let metadata = fs::metadata(old.join("directory-witness")).unwrap();
        let mut io = FaultIo {
            rename: Some((old.clone(), pinned.clone())),
            expected_directory_witness: Some((metadata.dev(), metadata.ino())),
            ..FaultIo::default()
        };
        append(&store, failure, &mut io).unwrap();
        let name = if failure {
            "calls.failures.jsonl"
        } else {
            "calls.jsonl"
        };
        assert_eq!(fs::read(old.join(name)).unwrap(), b"replacement-sentinel");
        assert!(fs::read(pinned.join(name)).unwrap().ends_with(b"\n"));
        assert_eq!(io.events, ["locked", "write", "file", "directory"]);
    }
}

#[test]
fn identical_match_never_hides_corrupt_suffix() {
    for failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        append(&store, failure, &mut FaultIo::default()).unwrap();
        let paths = store.storage_paths();
        let mut bytes = fs::read(&paths[usize::from(failure)]).unwrap();
        bytes.extend_from_slice(b"not-json\n");
        fs::write(&paths[usize::from(failure)], &bytes).unwrap();
        let mut io = FaultIo::default();
        assert!(matches!(
            append(&store, failure, &mut io),
            Err(ModelCallStoreError::InvalidJson { line: 2, .. })
        ));
        assert_eq!(io.events, ["locked"]);
        assert!(store.contains_run_identity("run-1", "call-1").is_err());
        assert_eq!(fs::read(&paths[usize::from(failure)]).unwrap(), bytes);
    }
}

#[test]
fn success_identity_match_does_not_hide_corrupt_failure_history() {
    for corrupt in [b"not-json\n".as_slice(), b"\xff\n", b"{}\n"] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        store.append(&success_record()).unwrap();
        fs::write(&store.failure_path, corrupt).unwrap();
        assert!(store.contains_run_identity("run-1", "call-1").is_err());
        assert_eq!(fs::read(&store.failure_path).unwrap(), corrupt);
    }
}

#[test]
fn invalid_inputs_fail_before_lock_or_any_io_seam() {
    let directory = tempfile::tempdir().unwrap();
    let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
    let mut success = success_record();
    success.task_id.clear();
    let mut failure = failure_record();
    failure.task_id.clear();
    let mut io = FaultIo::default();
    assert!(matches!(
        store.append_with_io(&success, &mut io),
        Err(ModelCallStoreError::InvalidContract(_))
    ));
    assert!(matches!(
        store.append_failure_with_io(&failure, &mut io),
        Err(ModelCallStoreError::InvalidContract(_))
    ));
    assert!(io.events.is_empty());
    assert!(store.storage_paths().iter().all(|path| !path.exists()));
}

#[test]
fn real_lock_is_held_through_new_and_retry_barriers_and_released_after_errors() {
    for failure in [false, true] {
        for retry in [false, true] {
            for fault in [None, Some("write"), Some("file"), Some("directory")] {
                if retry && fault == Some("write") {
                    continue; // Identical retry must not call write at all.
                }
                let directory = tempfile::tempdir().unwrap();
                let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
                let paths = store.storage_paths();
                if retry {
                    append(&store, failure, &mut FaultIo::default()).unwrap();
                }
                let lock_path = &paths[if failure { 3 } else { 2 }];
                let open_contender = || {
                    fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(lock_path)
                        .unwrap()
                };
                let available = open_contender();
                available
                    .try_lock()
                    .expect("locking is supported before operation");
                drop(available);
                let expected_file = if retry {
                    let metadata = fs::metadata(&paths[usize::from(failure)]).unwrap();
                    Some((metadata.dev(), metadata.ino()))
                } else {
                    None
                };
                let mut io = FaultIo {
                    fail: fault,
                    expected_file,
                    lock_contender: Some(open_contender()),
                    ..FaultIo::default()
                };
                let result = append(&store, failure, &mut io);
                assert_eq!(result.is_ok(), fault.is_none());
                assert_eq!(io.events.contains(&"file"), fault != Some("write"));
                io.lock_contender
                    .as_ref()
                    .unwrap()
                    .try_lock()
                    .expect("store must release its real lock on success and failure");
            }
        }
    }
}

#[test]
fn complete_unsealed_history_remains_readable_and_retry_does_not_repair_it() {
    for failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap();
        let path = store.storage_paths()[usize::from(failure)].clone();
        let bytes = if failure {
            serde_json::to_vec(&failure_record()).unwrap()
        } else {
            serde_json::to_vec(&success_record()).unwrap()
        };
        fs::write(&path, &bytes).unwrap();
        append(
            &store,
            failure,
            &mut FaultIo {
                expected_file: {
                    let metadata = fs::metadata(&path).unwrap();
                    Some((metadata.dev(), metadata.ino()))
                },
                ..FaultIo::default()
            },
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(store.contains_run_identity("run-1", "call-1").unwrap());
        // Characterize, do not silently fix: a different append after an
        // unsealed complete value still concatenates JSON and damages history.
        if failure {
            let mut next = failure_record();
            next.model_call_id = "call-2".into();
            store.append_failure(&next).unwrap();
            assert!(matches!(
                store.read_failures(),
                Err(ModelCallStoreError::InvalidJson { line: 1, .. })
            ));
        } else {
            let mut next = success_record();
            next.model_call_id = "call-2".into();
            store.append(&next).unwrap();
            assert!(matches!(
                store.read_all(),
                Err(ModelCallStoreError::InvalidJson { line: 1, .. })
            ));
        }
    }
}
