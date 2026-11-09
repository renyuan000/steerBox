#![cfg(unix)]
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::time::{Duration, Instant};
use steerbox::transcript_store::{SyncPolicy, TranscriptEntry, TranscriptKind, TranscriptStore};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("history");
    let workspace = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&workspace).unwrap();
    (temp, root, workspace)
}
fn entry(kind: TranscriptKind, content: &str) -> TranscriptEntry {
    TranscriptEntry {
        message_id: "message-1".into(),
        model_call_id: Some("call-1".into()),
        kind,
        content: content.into(),
    }
}
fn records(root: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(root.join("session-1.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn chunk_bytes_are_immediate_while_durability_requires_sync() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    let text = "你好\n🦀 exact bytes ";
    let receipt = store
        .append(entry(TranscriptKind::AssistantChunk, text))
        .unwrap();
    assert_eq!((receipt.written_sequence, receipt.durable_sequence), (1, 0));
    assert_eq!(records(&root)[0]["entry"]["content"], text);
    assert_eq!(records(&root)[0]["sequence"], 1);
    assert_eq!(records(&root)[0]["session_id"], "session-1");
    assert_eq!(store.sync().unwrap().durable_sequence, 1);
    assert_eq!(store.close().unwrap().durable_sequence, 1);
}
#[test]
fn critical_boundaries_are_synced_before_acknowledgement() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    for (index, kind) in [
        TranscriptKind::UserMessage,
        TranscriptKind::AssistantComplete,
        TranscriptKind::ModelRequest,
        TranscriptKind::ModelResponseComplete,
    ]
    .into_iter()
    .enumerate()
    {
        let receipt = store
            .append(entry(kind, "approved synthetic body"))
            .unwrap();
        assert_eq!(receipt.durable_sequence, index as u64 + 1);
    }
    store.close().unwrap();
}
#[test]
fn timer_flushes_without_another_append() {
    let (_temp, root, workspace) = fixture();
    let policy = SyncPolicy {
        interval: Duration::from_millis(30),
        ..SyncPolicy::default()
    };
    let store = TranscriptStore::create(&root, &workspace, "session-1", policy).unwrap();
    store
        .append(entry(TranscriptKind::ModelResponseChunk, "partial"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while store.status().unwrap().durable_sequence != 1 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    store.close().unwrap();
}
#[test]
fn threshold_syncs_and_close_keeps_history() {
    let (_temp, root, workspace) = fixture();
    let policy = SyncPolicy {
        byte_threshold: 1,
        ..SyncPolicy::default()
    };
    let store = TranscriptStore::create(&root, &workspace, "session-1", policy).unwrap();
    assert_eq!(
        store
            .append(entry(TranscriptKind::AssistantChunk, "chunk"))
            .unwrap()
            .durable_sequence,
        1
    );
    store.close().unwrap();
    assert_eq!(records(&root).len(), 1);
}
#[test]
fn invalid_policy_and_identity_create_no_files() {
    let (_temp, root, workspace) = fixture();
    let invalid = SyncPolicy {
        interval: Duration::ZERO,
        ..SyncPolicy::default()
    };
    assert!(TranscriptStore::create(&root, &workspace, "session-1", invalid).is_err());
    for id in ["", "../escape", "a/b"] {
        assert!(TranscriptStore::create(&root, &workspace, id, SyncPolicy::default()).is_err());
    }
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}
#[test]
fn runtime_root_is_private_external_and_not_symlinked() {
    let (temp, root, workspace) = fixture();
    let alias = temp.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert!(
        TranscriptStore::create(&alias, &workspace, "session-1", SyncPolicy::default()).is_err()
    );
    let nested = workspace.join("private");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        TranscriptStore::create(&nested, &workspace, "session-1", SyncPolicy::default()).is_err()
    );
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).is_err()
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}
#[test]
fn existing_file_and_aliases_are_never_overwritten() {
    let (temp, root, workspace) = fixture();
    let protected = temp.path().join("protected");
    fs::write(&protected, b"protected bytes").unwrap();
    for name in ["existing", "symlink", "hardlink"] {
        let target = root.join(format!("{name}.jsonl"));
        match name {
            "existing" => fs::write(target, b"old history").unwrap(),
            "symlink" => symlink(&protected, target).unwrap(),
            _ => fs::hard_link(&protected, target).unwrap(),
        }
        assert!(TranscriptStore::create(&root, &workspace, name, SyncPolicy::default()).is_err());
    }
    assert_eq!(fs::read(protected).unwrap(), b"protected bytes");
    assert_eq!(
        fs::read(root.join("existing.jsonl")).unwrap(),
        b"old history"
    );
}
#[test]
fn held_file_is_private_exclusive_and_not_redirected_after_rename() {
    let (temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    assert_eq!(
        fs::metadata(root.join("session-1.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).is_err()
    );
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("session-1.jsonl"), b"replacement").unwrap();
    store
        .append(entry(TranscriptKind::UserMessage, "pinned only"))
        .unwrap();
    store.close().unwrap();
    assert_eq!(
        fs::read(root.join("session-1.jsonl")).unwrap(),
        b"replacement"
    );
    assert_eq!(records(&moved)[0]["entry"]["content"], "pinned only");
}
#[test]
fn sensitive_transport_fields_and_oversized_body_are_not_saved() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    for text in [
        "password=synthetic-sensitive-marker",
        r#"{"api_key":"synthetic-sensitive-marker"}"#,
        r#"{"endpoint":"https://relay.example.test"}"#,
        r#"{"headers":{"authorization":"synthetic-sensitive-marker"}}"#,
    ] {
        let error = store
            .append(entry(TranscriptKind::ModelRequest, text))
            .expect_err("must refuse");
        assert!(!format!("{error:?} {error}").contains("synthetic-sensitive-marker"));
    }
    assert!(store
        .append(entry(TranscriptKind::UserMessage, &"x".repeat(65_537)))
        .is_err());
    assert!(records(&root).is_empty());
    assert_eq!(
        store
            .append(entry(TranscriptKind::UserMessage, "safe"))
            .unwrap()
            .written_sequence,
        1
    );
    store.close().unwrap();
}
#[test]
fn accepted_json_formatting_is_exact_and_debug_hides_body() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    let text = "{\n  \"messages\": [\"body-marker-only\"]\n}\n";
    let request = entry(TranscriptKind::ModelRequest, text);
    assert!(!format!("{request:?}").contains("body-marker-only"));
    store.append(request).unwrap();
    assert_eq!(records(&root)[0]["entry"]["content"], text);
    store.close().unwrap();
}

#[test]
fn duplicate_json_keys_cannot_hide_discarded_sensitive_strings() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    for text in [
        r#"{"messages":"password=synthetic-marker","messages":"safe"}"#,
        r#"{"messages":[{"text":"password=synthetic-marker","text":"safe"}]}"#,
    ] {
        assert!(store
            .append(entry(TranscriptKind::ModelRequest, text))
            .is_err());
    }
    assert!(records(&root).is_empty());
    store.close().unwrap();
}

#[test]
fn escaped_json_string_cannot_bypass_semantic_secret_detection() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    assert!(store
        .append(entry(
            TranscriptKind::ModelRequest,
            r#""\u0070assword=synthetic-marker""#
        ))
        .is_err());
    assert!(records(&root).is_empty());
    store.close().unwrap();
}

#[test]
fn rejected_structured_parsing_never_falls_back_to_plaintext() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    let deep = format!(
        "{}{}{}",
        "[".repeat(200),
        r#"{"endpoint":"https://relay.example.test"}"#,
        "]".repeat(200)
    );
    let deep_secret = format!(
        "{}{}{}",
        "[".repeat(200),
        r#""\u0070assword=synthetic-marker""#,
        "]".repeat(200)
    );
    for text in [
        deep.as_str(),
        deep_secret.as_str(),
        r#"{"endpoint":"https://relay.example.test","number":1e999}"#,
        r#"{"endpoint":"https://relay.example.test",}"#,
        r#""\u0070assword=synthetic-marker"#,
    ] {
        assert!(store
            .append(entry(TranscriptKind::ModelRequest, text))
            .is_err());
        assert_eq!(store.status().unwrap().written_sequence, 0);
        assert!(records(&root).is_empty());
    }
    assert_eq!(
        store
            .append(entry(TranscriptKind::UserMessage, "safe afterward"))
            .unwrap()
            .durable_sequence,
        1
    );
    store.close().unwrap();
}

#[test]
fn identity_and_capacity_rejections_do_not_allocate_sequence_or_poison() {
    let (_temp, root, workspace) = fixture();
    let policy = SyncPolicy {
        max_total_bytes: 1024,
        ..SyncPolicy::default()
    };
    let store = TranscriptStore::create(&root, &workspace, "session-1", policy).unwrap();
    let mut invalid = entry(TranscriptKind::ModelRequest, "body");
    invalid.model_call_id = None;
    assert!(store.append(invalid).is_err());
    let mut invalid = entry(TranscriptKind::UserMessage, "body");
    invalid.message_id = "not/a/safe/id".into();
    assert!(store.append(invalid).is_err());
    assert!(store
        .append(entry(TranscriptKind::AssistantChunk, &"x".repeat(2000)))
        .is_err());
    assert_eq!(store.status().unwrap().written_sequence, 0);
    assert_eq!(
        store
            .append(entry(TranscriptKind::UserMessage, "small"))
            .unwrap()
            .durable_sequence,
        1
    );
    store.close().unwrap();
    assert_eq!(records(&root).len(), 1);
}

#[test]
fn concurrent_appends_preserve_unique_file_order_sequences() {
    let (_temp, root, workspace) = fixture();
    let store =
        TranscriptStore::create(&root, &workspace, "session-1", SyncPolicy::default()).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let store = &store;
            scope.spawn(move || {
                for _ in 0..10 {
                    store
                        .append(entry(TranscriptKind::AssistantChunk, "approved chunk"))
                        .unwrap();
                }
            });
        }
    });
    assert_eq!(store.close().unwrap().durable_sequence, 40);
    let saved = records(&root);
    assert_eq!(saved.len(), 40);
    for (index, record) in saved.iter().enumerate() {
        assert_eq!(record["sequence"], index + 1);
    }
}

#[test]
fn ambiguous_paths_symlink_parents_and_overlapping_trees_create_no_files() {
    let (temp, root, workspace) = fixture();
    let alias = temp.path().join("parent-alias");
    symlink(temp.path(), &alias).unwrap();
    let paths = [
        alias.join("history"),
        root.join("."),
        root.join(".."),
        std::path::PathBuf::from("relative"),
        temp.path().join("missing"),
    ];
    for path in paths {
        assert!(
            TranscriptStore::create(&path, &workspace, "session-1", SyncPolicy::default()).is_err()
        );
    }
    // A private ancestor of the workspace also overlaps and must not hold history.
    assert!(
        TranscriptStore::create(temp.path(), &workspace, "session-1", SyncPolicy::default())
            .is_err()
    );
    let source_alias = temp.path().join("workspace-alias");
    symlink(&workspace, &source_alias).unwrap();
    assert!(
        TranscriptStore::create(&root, source_alias, "session-1", SyncPolicy::default()).is_err()
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}

#[test]
fn killed_writer_retains_acknowledged_critical_body_without_drop() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    const CHILD_ROOT: &str = "STEERBOX_TRANSCRIPT_DURABILITY_TEST_ROOT";
    const READY: &str = "STEERBOX_TRANSCRIPT_DURABLE_ACK";
    if let Some(temp) = std::env::var_os(CHILD_ROOT) {
        let temp = std::path::PathBuf::from(temp);
        let store = TranscriptStore::create(
            temp.join("history"),
            temp.join("workspace"),
            "session-1",
            SyncPolicy::default(),
        )
        .unwrap();
        let receipt = store
            .append(entry(
                TranscriptKind::ModelResponseComplete,
                "你好\nacknowledged 🦀",
            ))
            .unwrap();
        assert_eq!(receipt.durable_sequence, 1);
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (temp, root, _workspace) = fixture();
    let mut child = Reap(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "killed_writer_retains_acknowledged_critical_body_without_drop",
                "--nocapture",
            ])
            .env(CHILD_ROOT, temp.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(READY) {
                let _ = send.send(true);
                return;
            }
        }
        let _ = send.send(false);
    });
    assert!(receive.recv_timeout(Duration::from_secs(10)).unwrap());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    reader.join().unwrap();
    assert_eq!(
        records(&root)[0]["entry"]["content"],
        "你好\nacknowledged 🦀"
    );
    assert_eq!(records(&root)[0]["sequence"], 1);
    assert!(fs::read(root.join("session-1.jsonl"))
        .unwrap()
        .ends_with(b"\n"));
}
