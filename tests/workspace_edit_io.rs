use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use steerbox::domain::GoalContract;
use steerbox::workspace_edit::{edit_markdown, WorkspaceEditError};

const ORIGINAL: &[u8] = b"original\r\n";
const SENTINEL: &[u8] = b"another operation owns this ordinary temporary\n";

fn goal() -> GoalContract {
    GoalContract::new(
        "synthetic-edit-goal",
        "synthetic-edit-project",
        "update synthetic Markdown",
        vec![".".into()],
        vec![],
        vec!["markdown_link_check".into()],
    )
}

fn directory_entries(path: &Path) -> Vec<OsString> {
    let mut entries: Vec<_> = fs::read_dir(path)
        .expect("inspect owned synthetic directory")
        .map(|entry| entry.expect("read synthetic directory entry").file_name())
        .collect();
    entries.sort();
    entries
}

fn target_with_temporary_sentinel() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().expect("synthetic workspace");
    let target = root.path().join("README.md");
    let temporary = root
        .path()
        .join(format!("README.md.steerbox-tmp-{}", std::process::id()));
    fs::write(&target, ORIGINAL).expect("write ordinary Markdown fixture");
    fs::write(&temporary, SENTINEL).expect("write ordinary temporary sentinel");
    (root, target, temporary)
}

#[test]
fn approval_precedes_path_validation_and_does_not_create_the_workspace() {
    let root = tempfile::tempdir().expect("synthetic parent");
    let workspace = root.path().join("missing-workspace");

    for relative in ["README.md", "../not-markdown.txt"] {
        assert!(matches!(
            edit_markdown(&workspace, &goal(), relative, "old", "new", false),
            Err(WorkspaceEditError::ApprovalRequired)
        ));
        assert!(directory_entries(root.path()).is_empty());
    }
}

#[test]
fn noop_preserves_a_preexisting_ordinary_temporary_sentinel() {
    let (root, target, temporary) = target_with_temporary_sentinel();
    let entries = directory_entries(root.path());
    let original = std::str::from_utf8(ORIGINAL).expect("synthetic UTF-8");

    let result = edit_markdown(
        root.path(),
        &goal(),
        "./README.md",
        original,
        original,
        true,
    )
    .expect("matching no-op succeeds despite an occupied temporary name");

    assert_eq!(result.relative_path, "README.md");
    assert!(!result.changed);
    assert_eq!(result.bytes_before, ORIGINAL.len());
    assert_eq!(result.bytes_after, ORIGINAL.len());
    assert_eq!(fs::read(&target).expect("read unchanged target"), ORIGINAL);
    assert_eq!(
        fs::read(&temporary).expect("read untouched sentinel"),
        SENTINEL
    );
    assert_eq!(directory_entries(root.path()), entries);
}

#[test]
fn expected_mismatch_precedes_an_ordinary_temporary_collision() {
    let (root, target, temporary) = target_with_temporary_sentinel();
    let entries = directory_entries(root.path());

    assert!(matches!(
        edit_markdown(
            root.path(),
            &goal(),
            "README.md",
            "stale\r\n",
            "replacement\r\n",
            true
        ),
        Err(WorkspaceEditError::ExpectedContentMismatch)
    ));
    assert_eq!(fs::read(&target).expect("read protected target"), ORIGINAL);
    assert_eq!(
        fs::read(&temporary).expect("read protected sentinel"),
        SENTINEL
    );
    assert_eq!(directory_entries(root.path()), entries);
}

#[test]
fn ordinary_temporary_collision_is_io_and_preserves_both_files() {
    let (root, target, temporary) = target_with_temporary_sentinel();
    let entries = directory_entries(root.path());

    assert!(matches!(
        edit_markdown(
            root.path(),
            &goal(),
            "README.md",
            std::str::from_utf8(ORIGINAL).expect("synthetic UTF-8"),
            "replacement\r\n",
            true
        ),
        Err(WorkspaceEditError::Io { .. })
    ));
    assert_eq!(fs::read(&target).expect("read protected target"), ORIGINAL);
    assert_eq!(
        fs::read(&temporary).expect("read protected sentinel"),
        SENTINEL
    );
    assert_eq!(directory_entries(root.path()), entries);
}

#[test]
fn unicode_crlf_edit_preserves_exact_bytes_and_counts_utf8_bytes() {
    let root = tempfile::tempdir().expect("synthetic workspace");
    let target = root.path().join("README.markdown");
    let before = "旧😀\r\n";
    let after = "新内容🙂\r\n";
    fs::write(&target, before.as_bytes()).expect("write Unicode CRLF fixture");

    let result = edit_markdown(root.path(), &goal(), "README.markdown", before, after, true)
        .expect("ordinary Unicode CRLF edit succeeds");

    assert_eq!(result.relative_path, "README.markdown");
    assert!(result.changed);
    assert_eq!(result.bytes_before, 9);
    assert_eq!(result.bytes_after, 15);
    assert_eq!(
        fs::read(&target).expect("read exact replacement"),
        after.as_bytes()
    );
    assert_eq!(
        directory_entries(root.path()),
        vec![OsString::from("README.markdown")]
    );
}

#[test]
fn missing_target_is_io_and_does_not_create_a_file() {
    let root = tempfile::tempdir().expect("synthetic workspace");

    assert!(matches!(
        edit_markdown(root.path(), &goal(), "missing.md", "", "new\n", true),
        Err(WorkspaceEditError::Io { .. })
    ));
    assert!(directory_entries(root.path()).is_empty());
}

#[test]
fn missing_parent_is_io_and_does_not_create_directories() {
    let root = tempfile::tempdir().expect("synthetic workspace");

    assert!(matches!(
        edit_markdown(
            root.path(),
            &goal(),
            "missing-parent/README.md",
            "",
            "new\n",
            true
        ),
        Err(WorkspaceEditError::Io { .. })
    ));
    assert!(directory_entries(root.path()).is_empty());
}
