//! Explicit local Markdown snapshot/restore contracts, not automatic rollback.
#![cfg(unix)]
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use steerbox::domain::GoalContract;
use steerbox::workspace_snapshot::{
    SnapshotError, SnapshotMetadata, WorkspaceSnapshotStore, MAX_RECORD_BYTES, MAX_SNAPSHOT_BYTES,
};

const BEFORE: &[u8] = b"# Snapshot fixture\r\nExact original body.\r\n";
const AFTER: &[u8] = b"# Verified change\n";

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

struct Fixture {
    _dir: tempfile::TempDir,
    workspace: PathBuf,
    store_root: PathBuf,
    goal: GoalContract,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), BEFORE).unwrap();
        let store_root = dir.path().join("private-snapshots");
        let goal = GoalContract::new(
            "goal-snapshot",
            "project-snapshot",
            "edit docs",
            vec![".".into()],
            vec![],
            vec!["verified".into()],
        );
        Self {
            _dir: dir,
            workspace,
            store_root,
            goal,
        }
    }

    fn store(&self) -> WorkspaceSnapshotStore {
        WorkspaceSnapshotStore::open(self.store_root.clone())
    }

    fn capture(&self) -> SnapshotMetadata {
        self.store()
            .capture(&self.workspace, &self.goal, "README.md", "snapshot-1")
            .unwrap()
    }

    fn assert_bytes(&self, expected: &[u8]) {
        assert_eq!(
            fs::read(self.workspace.join("README.md")).unwrap(),
            expected
        );
    }
}

#[test]
fn capture_reopen_and_explicit_restore_preserve_exact_crlf_bytes() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let metadata = store
        .capture(&fixture.workspace, &fixture.goal, "README.md", "snapshot-1")
        .unwrap();
    assert_eq!(metadata.schema_version, 1);
    assert_eq!(metadata.snapshot_id, "snapshot-1");
    assert_eq!(metadata.project_id, fixture.goal.project_id);
    assert_eq!(metadata.goal_id, fixture.goal.goal_id);
    assert_eq!(metadata.relative_path, "README.md");
    assert_eq!(metadata.before_hash, hash(BEFORE));
    assert_eq!(metadata.bytes_before, BEFORE.len());
    fixture.assert_bytes(BEFORE);
    drop(store);
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let reopened = fixture.store();
    let restored = reopened
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true,
        )
        .unwrap();
    assert_eq!(restored.relative_path, "README.md");
    assert!(restored.changed);
    assert_eq!(restored.bytes_before, AFTER.len());
    assert_eq!(restored.bytes_after, BEFORE.len());
    fixture.assert_bytes(BEFORE);
}

#[test]
fn metadata_serialization_contains_no_original_file_body() {
    let fixture = Fixture::new();
    let metadata = fixture.capture();
    let serialized = serde_json::to_string(&metadata).unwrap();
    assert!(!serialized.contains("Exact original body"));
    let value = serde_json::to_value(&metadata).unwrap();
    let fields = value.as_object().unwrap();
    for forbidden in ["contents", "content", "body", "bytes", "original"] {
        assert!(!fields.contains_key(forbidden));
    }
    assert_eq!(
        fields.len(),
        7,
        "external metadata contains only the declared contract fields"
    );
}

#[test]
fn restore_requires_explicit_approval_and_keeps_current_bytes() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            false
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_rejects_invalid_expected_hash_before_comparing_current_bytes() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    for invalid_hash in [
        "sha256:not-a-hash".to_owned(),
        format!("sha256:{}", "a".repeat(65)),
    ] {
        let error = fixture
            .store()
            .restore(
                &fixture.workspace,
                &fixture.goal,
                "snapshot-1",
                &invalid_hash,
                true,
            )
            .unwrap_err();
        assert_eq!(error, SnapshotError::InvalidHash);
        fixture.assert_bytes(AFTER);
    }
}

#[test]
fn restore_rejects_user_changed_current_file_hash() {
    let fixture = Fixture::new();
    fixture.capture();
    let user_bytes = b"# User edit\r\nPreserve my change.\r\n";
    fs::write(fixture.workspace.join("README.md"), user_bytes).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(user_bytes);
}

#[test]
fn restore_rejects_wrong_goal_without_touching_workspace() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let mut wrong_goal = fixture.goal.clone();
    wrong_goal.goal_id = "other-goal".into();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &wrong_goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_rejects_wrong_project_without_touching_workspace() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let mut wrong_goal = fixture.goal.clone();
    wrong_goal.project_id = "other-project".into();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &wrong_goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_rejects_other_workspace_even_with_same_goal_and_current_hash() {
    let fixture = Fixture::new();
    fixture.capture();
    let other = tempfile::tempdir().unwrap();
    fs::write(other.path().join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            other.path(),
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(fs::read(other.path().join("README.md")).unwrap(), AFTER);
    fixture.assert_bytes(BEFORE);
}

#[test]
fn restore_rechecks_goal_scope_after_capture() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let mut narrowed_goal = fixture.goal.clone();
    narrowed_goal.out_of_scope = vec!["README.md".into()];
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &narrowed_goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn invalid_snapshot_json_is_rejected_without_mutation() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    fs::write(fixture.store_root.join("snapshot-1.json"), b"{invalid json").unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

fn corrupt_hash(value: &mut serde_json::Value, original_hash: &str) -> bool {
    match value {
        serde_json::Value::String(text) if text == original_hash => {
            *text = hash(b"tampered body hash");
            true
        }
        serde_json::Value::Object(fields) => {
            let mut found = false;
            for child in fields.values_mut() {
                found |= corrupt_hash(child, original_hash);
            }
            found
        }
        serde_json::Value::Array(values) => {
            let mut found = false;
            for child in values {
                found |= corrupt_hash(child, original_hash);
            }
            found
        }
        _ => false,
    }
}

#[test]
fn persisted_body_hash_mismatch_is_rejected_without_mutation() {
    let fixture = Fixture::new();
    let metadata = fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let path = fixture.store_root.join("snapshot-1.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(
        corrupt_hash(&mut value, &metadata.before_hash),
        "stored snapshot must bind its body to before_hash"
    );
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn invalid_snapshot_ids_cannot_escape_or_create_records() {
    let fixture = Fixture::new();
    for id in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        "a.b",
        "has space",
        "非ASCII",
    ] {
        assert!(
            fixture
                .store()
                .capture(&fixture.workspace, &fixture.goal, "README.md", id)
                .is_err(),
            "invalid id must fail: {id}"
        );
        assert!(
            fixture
                .store()
                .restore(&fixture.workspace, &fixture.goal, id, &hash(BEFORE), true)
                .is_err(),
            "invalid restore id must fail: {id}"
        );
        fixture.assert_bytes(BEFORE);
    }
    assert!(!fixture._dir.path().join("escape.json").exists());
    assert!(fixture
        .store()
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "README.md",
            &"a".repeat(81)
        )
        .is_err());
}

#[test]
fn relative_or_workspace_internal_store_is_rejected() {
    let fixture = Fixture::new();
    let relative = WorkspaceSnapshotStore::open(PathBuf::from("relative-snapshots"));
    assert!(relative
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "README.md",
            "relative-store"
        )
        .is_err());
    let internal = WorkspaceSnapshotStore::open(fixture.workspace.join("snapshots"));
    assert!(internal
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "README.md",
            "internal-store"
        )
        .is_err());
    assert!(!fixture
        .workspace
        .join("snapshots/internal-store.json")
        .exists());
    fixture.assert_bytes(BEFORE);
}

#[test]
fn tampered_snapshot_contents_are_rejected_without_mutation() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let path = fixture.store_root.join("snapshot-1.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(value.get("contents").is_some());
    value["contents"] = serde_json::json!("changed stored body\r\n");
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn capture_rejects_traversal_non_markdown_and_out_of_scope() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("plain.txt"), BEFORE).unwrap();
    for path in [
        "../README.md",
        "/README.md",
        "docs/../../README.md",
        "plain.txt",
    ] {
        assert!(fixture
            .store()
            .capture(&fixture.workspace, &fixture.goal, path, "invalid-path")
            .is_err());
    }
    let mut scoped_goal = fixture.goal.clone();
    scoped_goal.scope = vec!["docs".into()];
    assert!(fixture
        .store()
        .capture(
            &fixture.workspace,
            &scoped_goal,
            "README.md",
            "outside-scope"
        )
        .is_err());
    fixture.assert_bytes(BEFORE);
}

#[test]
fn oversized_source_is_rejected_without_snapshot_or_source_changes() {
    let fixture = Fixture::new();
    assert_eq!(MAX_SNAPSHOT_BYTES, 1024 * 1024);
    let large = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
    fs::write(fixture.workspace.join("README.md"), &large).unwrap();
    assert!(fixture
        .store()
        .capture(&fixture.workspace, &fixture.goal, "README.md", "too-large")
        .is_err());
    fixture.assert_bytes(&large);
    assert!(!fixture.store_root.join("too-large.json").exists());
}

#[test]
fn duplicate_snapshot_id_never_overwrites_existing_snapshot_or_source() {
    let fixture = Fixture::new();
    fixture.capture();
    let record = fixture.store_root.join("snapshot-1.json");
    let saved_record = fs::read(&record).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .capture(&fixture.workspace, &fixture.goal, "README.md", "snapshot-1")
        .is_err());
    assert_eq!(fs::read(&record).unwrap(), saved_record);
    fixture.assert_bytes(AFTER);
}

#[cfg(unix)]
#[test]
fn source_symlink_and_symlink_parent_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("outside.md"), BEFORE).unwrap();
    symlink(
        external.path().join("outside.md"),
        fixture.workspace.join("link.md"),
    )
    .unwrap();
    symlink(external.path(), fixture.workspace.join("linked-dir")).unwrap();
    for path in ["link.md", "linked-dir/outside.md"] {
        assert!(fixture
            .store()
            .capture(&fixture.workspace, &fixture.goal, path, "source-link")
            .is_err());
    }
    assert_eq!(
        fs::read(external.path().join("outside.md")).unwrap(),
        BEFORE
    );
}

#[cfg(unix)]
#[test]
fn symlink_store_root_is_rejected_without_external_writes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let external = tempfile::tempdir().unwrap();
    symlink(external.path(), &fixture.store_root).unwrap();
    assert!(fixture
        .store()
        .capture(&fixture.workspace, &fixture.goal, "README.md", "store-link")
        .is_err());
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    fixture.assert_bytes(BEFORE);
}

#[cfg(unix)]
#[test]
fn symlink_snapshot_record_is_rejected_for_capture_and_restore() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.capture();
    let external = tempfile::tempdir().unwrap();
    let outside = external.path().join("record.json");
    let snapshot_path = fixture.store_root.join("snapshot-1.json");
    let original_record = fs::read(&snapshot_path).unwrap();
    fs::rename(&snapshot_path, &outside).unwrap();
    symlink(&outside, &snapshot_path).unwrap();
    assert!(fixture
        .store()
        .capture(&fixture.workspace, &fixture.goal, "README.md", "snapshot-1")
        .is_err());
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(fs::read(&outside).unwrap(), original_record);
    fixture.assert_bytes(AFTER);
}

#[cfg(unix)]
#[test]
fn restore_rejects_symlink_target_without_changing_external_file() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.capture();
    let saved = fixture.workspace.join("README.saved.md");
    fs::rename(fixture.workspace.join("README.md"), &saved).unwrap();
    let external = tempfile::tempdir().unwrap();
    let outside = external.path().join("outside.md");
    fs::write(&outside, AFTER).unwrap();
    symlink(&outside, fixture.workspace.join("README.md")).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(fs::read(outside).unwrap(), AFTER);
    assert_eq!(fs::read(saved).unwrap(), BEFORE);
}

#[cfg(unix)]
#[test]
fn private_store_and_snapshot_use_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.capture();
    assert_eq!(
        fs::metadata(&fixture.store_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fixture.store_root.join("snapshot-1.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn restore_preserves_current_private_target_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    fs::set_permissions(
        fixture.workspace.join("README.md"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true,
        )
        .unwrap();
    fixture.assert_bytes(BEFORE);
    assert_eq!(
        fs::metadata(fixture.workspace.join("README.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn existing_broad_store_permissions_are_rejected_without_chmod() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fs::create_dir(&fixture.store_root).unwrap();
    fs::set_permissions(&fixture.store_root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(fixture
        .store()
        .capture(&fixture.workspace, &fixture.goal, "README.md", "broad-root")
        .is_err());
    assert_eq!(
        fs::metadata(&fixture.store_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!fixture.store_root.join("broad-root.json").exists());
    fixture.assert_bytes(BEFORE);
}

#[test]
fn existing_broad_snapshot_file_is_rejected_without_chmod_or_restore() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.capture();
    let path = fixture.store_root.join("snapshot-1.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fixture.assert_bytes(AFTER);
}

#[test]
fn private_store_root_made_public_rejects_restore() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.capture();
    fs::set_permissions(&fixture.store_root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(
        fs::metadata(&fixture.store_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    fixture.assert_bytes(AFTER);
}

#[test]
fn capture_rejects_invalid_utf8_missing_and_nonregular_sources() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("invalid.md"), [0xff, 0xfe]).unwrap();
    fs::create_dir(fixture.workspace.join("directory.md")).unwrap();
    for path in ["invalid.md", "missing.md", "directory.md"] {
        assert!(fixture
            .store()
            .capture(&fixture.workspace, &fixture.goal, path, "bad-source")
            .is_err());
    }
    fixture.assert_bytes(BEFORE);
}

#[test]
fn restore_rejects_invalid_utf8_missing_and_nonregular_targets() {
    for kind in ["invalid-utf8", "missing", "directory"] {
        let fixture = Fixture::new();
        fixture.capture();
        let target = fixture.workspace.join("README.md");
        let saved = fixture.workspace.join("saved.md");
        fs::rename(&target, &saved).unwrap();
        match kind {
            "invalid-utf8" => fs::write(&target, [0xff, 0xfe]).unwrap(),
            "directory" => fs::create_dir(&target).unwrap(),
            _ => (),
        }
        let expected_hash = if kind == "invalid-utf8" {
            hash(&[0xff, 0xfe])
        } else {
            hash(AFTER)
        };
        assert!(fixture
            .store()
            .restore(
                &fixture.workspace,
                &fixture.goal,
                "snapshot-1",
                &expected_hash,
                true
            )
            .is_err());
        assert_eq!(fs::read(saved).unwrap(), BEFORE);
        match kind {
            "invalid-utf8" => assert_eq!(fs::read(target).unwrap(), [0xff, 0xfe]),
            "directory" => assert!(target.is_dir()),
            _ => assert!(!target.exists()),
        }
    }
}

#[test]
fn hardlinked_source_and_snapshot_records_are_rejected() {
    let fixture = Fixture::new();
    let alias = fixture.workspace.join("source-alias.md");
    fs::hard_link(fixture.workspace.join("README.md"), &alias).unwrap();
    assert!(fixture
        .store()
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "README.md",
            "hard-source"
        )
        .is_err());
    fixture.assert_bytes(BEFORE);
    let other = Fixture::new();
    other.capture();
    let path = other.store_root.join("snapshot-1.json");
    fs::hard_link(&path, other.store_root.join("record-alias.json")).unwrap();
    fs::write(other.workspace.join("README.md"), AFTER).unwrap();
    assert!(other
        .store()
        .restore(
            &other.workspace,
            &other.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    other.assert_bytes(AFTER);
}

#[test]
fn store_ancestor_symlink_is_rejected_without_external_writes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let external = tempfile::tempdir().unwrap();
    let linked_parent = fixture._dir.path().join("linked-parent");
    symlink(external.path(), &linked_parent).unwrap();
    let store = WorkspaceSnapshotStore::open(linked_parent.join("snapshots"));
    assert!(store
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "README.md",
            "ancestor-link"
        )
        .is_err());
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    fixture.assert_bytes(BEFORE);
}

#[test]
fn restore_rejects_store_root_replaced_by_symlink() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.capture();
    let saved_root = fixture._dir.path().join("saved-snapshots");
    fs::rename(&fixture.store_root, &saved_root).unwrap();
    symlink(&saved_root, &fixture.store_root).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_rejects_target_ancestor_replaced_by_symlink() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let docs = fixture.workspace.join("docs");
    fs::create_dir(&docs).unwrap();
    fs::write(docs.join("README.md"), BEFORE).unwrap();
    fixture
        .store()
        .capture(
            &fixture.workspace,
            &fixture.goal,
            "docs/README.md",
            "docs-snapshot",
        )
        .unwrap();
    let saved_docs = fixture.workspace.join("saved-docs");
    fs::rename(&docs, &saved_docs).unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("README.md"), AFTER).unwrap();
    symlink(external.path(), &docs).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "docs-snapshot",
            &hash(AFTER),
            true
        )
        .is_err());
    assert_eq!(fs::read(external.path().join("README.md")).unwrap(), AFTER);
    assert_eq!(fs::read(saved_docs.join("README.md")).unwrap(), BEFORE);
}

#[test]
fn restore_rejects_oversized_current_target_even_when_expected_hash_matches() {
    let fixture = Fixture::new();
    fixture.capture();
    let large = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
    fs::write(fixture.workspace.join("README.md"), &large).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(&large),
            true
        )
        .is_err());
    fixture.assert_bytes(&large);
}

#[test]
fn tampered_record_metadata_is_rejected_without_mutating_either_file() {
    for (field, replacement) in [
        ("schema_version", serde_json::json!(2)),
        ("snapshot_id", serde_json::json!("other-id")),
        ("relative_path", serde_json::json!("../outside.md")),
        ("bytes_before", serde_json::json!(BEFORE.len() + 1)),
    ] {
        let fixture = Fixture::new();
        fixture.capture();
        let outside = fixture._dir.path().join("outside.md");
        fs::write(&outside, AFTER).unwrap();
        fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
        let path = fixture.store_root.join("snapshot-1.json");
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(record.get("metadata").is_some());
        record["metadata"][field] = replacement;
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(
            fixture
                .store()
                .restore(
                    &fixture.workspace,
                    &fixture.goal,
                    "snapshot-1",
                    &hash(AFTER),
                    true
                )
                .is_err(),
            "tampered metadata field: {field}"
        );
        fixture.assert_bytes(AFTER);
        assert_eq!(fs::read(outside).unwrap(), AFTER);
    }
}

#[test]
fn record_copied_to_other_id_is_not_an_authorized_snapshot() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::copy(
        fixture.store_root.join("snapshot-1.json"),
        fixture.store_root.join("snapshot-2.json"),
    )
    .unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-2",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn oversized_record_is_rejected_without_mutation() {
    let fixture = Fixture::new();
    fixture.capture();
    assert_eq!(MAX_RECORD_BYTES, 6 * MAX_SNAPSHOT_BYTES + 65536);
    fs::write(
        fixture.store_root.join("snapshot-1.json"),
        vec![b' '; MAX_RECORD_BYTES + 1],
    )
    .unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn valid_json_record_with_consistent_but_oversized_body_is_rejected() {
    let fixture = Fixture::new();
    fixture.capture();
    let path = fixture.store_root.join("snapshot-1.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let large = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
    record["contents"] = serde_json::json!(std::str::from_utf8(&large).unwrap());
    record["metadata"]["before_hash"] = serde_json::json!(hash(&large));
    record["metadata"]["bytes_before"] = serde_json::json!(large.len());
    let encoded = serde_json::to_vec(&record).unwrap();
    assert!(encoded.len() < MAX_RECORD_BYTES);
    fs::write(path, encoded).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        )
        .is_err());
    fixture.assert_bytes(AFTER);
}

#[test]
fn maximum_sized_body_with_json_escaping_roundtrips_without_truncation() {
    let fixture = Fixture::new();
    let escaped = vec![b'\0'; MAX_SNAPSHOT_BYTES];
    fs::write(fixture.workspace.join("README.md"), &escaped).unwrap();
    let metadata = fixture.capture();
    assert_eq!(metadata.bytes_before, escaped.len());
    assert_eq!(metadata.before_hash, hash(&escaped));
    let record_bytes = fs::metadata(fixture.store_root.join("snapshot-1.json"))
        .unwrap()
        .len() as usize;
    assert!(record_bytes > MAX_SNAPSHOT_BYTES);
    assert!(record_bytes <= MAX_RECORD_BYTES);
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true,
        )
        .unwrap();
    fixture.assert_bytes(&escaped);
}

#[test]
fn metadata_debug_and_restore_error_do_not_echo_original_file_body() {
    let fixture = Fixture::new();
    let metadata = fixture.capture();
    assert!(!format!("{metadata:?}").contains("Exact original body"));
    let path = fixture.store_root.join("snapshot-1.json");
    fs::write(path, b"{invalid json Exact original body.").unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let error = fixture
        .store()
        .restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true,
        )
        .unwrap_err();
    assert!(!error.to_string().contains("Exact original body"));
    assert!(!format!("{error:?}").contains("Exact original body"));
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_errors_distinguish_approval_current_version_and_record_failures() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(matches!(
        fixture.store().restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            false
        ),
        Err(SnapshotError::ApprovalRequired)
    ));
    assert!(matches!(
        fixture.store().restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(BEFORE),
            true
        ),
        Err(SnapshotError::CurrentHashMismatch)
    ));
    let mut wrong = fixture.goal.clone();
    wrong.goal_id = "other-goal".into();
    assert!(matches!(
        fixture
            .store()
            .restore(&fixture.workspace, &wrong, "snapshot-1", &hash(AFTER), true),
        Err(SnapshotError::BindingMismatch)
    ));
    fs::write(fixture.store_root.join("snapshot-1.json"), b"{invalid-json").unwrap();
    assert!(matches!(
        fixture.store().restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        ),
        Err(SnapshotError::InvalidRecord)
    ));
    fixture.assert_bytes(AFTER);
}

#[test]
fn restore_rejects_hardlinked_current_target_with_matching_hash() {
    let fixture = Fixture::new();
    fixture.capture();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    let alias = fixture.workspace.join("after-alias.md");
    fs::hard_link(fixture.workspace.join("README.md"), &alias).unwrap();
    assert!(matches!(
        fixture.store().restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        ),
        Err(SnapshotError::NotRegularFile)
    ));
    fixture.assert_bytes(AFTER);
    assert_eq!(fs::read(alias).unwrap(), AFTER);
}

#[test]
fn restore_rejects_store_ancestor_replaced_by_symlink() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let parent = fixture._dir.path().join("store-parent");
    fs::create_dir(&parent).unwrap();
    let store = WorkspaceSnapshotStore::open(parent.join("snapshots"));
    store
        .capture(&fixture.workspace, &fixture.goal, "README.md", "snapshot-1")
        .unwrap();
    let saved = fixture._dir.path().join("saved-parent");
    fs::rename(&parent, &saved).unwrap();
    symlink(&saved, &parent).unwrap();
    fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
    assert!(matches!(
        store.restore(
            &fixture.workspace,
            &fixture.goal,
            "snapshot-1",
            &hash(AFTER),
            true
        ),
        Err(SnapshotError::UnsafePath)
    ));
    fixture.assert_bytes(AFTER);
}

#[test]
fn foreign_owned_private_store_and_record_are_rejected_when_running_as_root() {
    use std::os::unix::ffi::OsStrExt;
    // Root-only ownership changes are restricted to directories/files created
    // in this test's TempDir; normal-user CI does not attempt chown.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    for root_is_foreign in [true, false] {
        let fixture = Fixture::new();
        fixture.capture();
        let path = if root_is_foreign {
            fixture.store_root.clone()
        } else {
            fixture.store_root.join("snapshot-1.json")
        };
        let owned_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // The path is an existing fixture-owned non-symlink path. Keep gid.
        assert_eq!(
            unsafe { libc::chown(owned_path.as_ptr(), 65534, !0 as libc::gid_t) },
            0
        );
        fs::write(fixture.workspace.join("README.md"), AFTER).unwrap();
        assert!(matches!(
            fixture.store().restore(
                &fixture.workspace,
                &fixture.goal,
                "snapshot-1",
                &hash(AFTER),
                true
            ),
            Err(SnapshotError::NotPrivate)
        ));
        fixture.assert_bytes(AFTER);
    }
}
