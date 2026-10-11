use std::fs;
use std::path::PathBuf;
use steerbox::domain::GoalContract;
use steerbox::workspace_edit::{edit_markdown, verify_markdown_links, WorkspaceEditError};

fn workspace(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("steerbox-workspace-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("workspace creates");
    path
}

fn goal() -> GoalContract {
    GoalContract::new(
        "goal-edit",
        "project-edit",
        "update docs",
        vec![".".into()],
        vec![],
        vec!["markdown_link_check".into()],
    )
}

#[test]
fn markdown_edit_requires_approval_and_scope() {
    let root = workspace("approval");
    fs::write(root.join("README.md"), "old\n").expect("file writes");
    let denied = edit_markdown(&root, &goal(), "README.md", "old\n", "new\n", false)
        .expect_err("unapproved write must fail");
    assert!(matches!(denied, WorkspaceEditError::ApprovalRequired));
    let outside = edit_markdown(&root, &goal(), "../README.md", "old\n", "new\n", true)
        .expect_err("outside path must fail");
    assert!(matches!(
        outside,
        WorkspaceEditError::InvalidPath(_) | WorkspaceEditError::OutOfScope(_)
    ));
    fs::remove_dir_all(root).expect("workspace removes");
}

#[test]
fn markdown_edit_is_atomic_and_link_verification_is_reproducible() {
    let root = workspace("success");
    fs::write(root.join("README.md"), "See [guide](guide.md).\n").expect("file writes");
    fs::write(root.join("guide.md"), "guide\n").expect("guide writes");
    let result = edit_markdown(
        &root,
        &goal(),
        "README.md",
        "See [guide](guide.md).\n",
        "See [guide](guide.md) and [notes](notes.md).\n",
        true,
    )
    .expect("approved edit succeeds");
    assert_eq!(result.relative_path, "README.md");
    assert_eq!(
        fs::read_to_string(root.join("README.md")).expect("file reads"),
        "See [guide](guide.md) and [notes](notes.md).\n"
    );
    let verification = verify_markdown_links(&root, "README.md").expect("links verify");
    assert!(!verification.passed);
    assert_eq!(verification.missing_links, vec!["notes.md"]);
    fs::remove_dir_all(root).expect("workspace removes");
}

struct SecurityFixture {
    root: PathBuf,
    workspace: PathBuf,
    external: PathBuf,
}

impl SecurityFixture {
    fn new() -> Self {
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let number = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "steerbox-path-security-{}-{number}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique fixture creates without overwriting existing data");
        let workspace = root.join("workspace");
        let external = root.join("external");
        fs::create_dir(&workspace).expect("workspace creates");
        fs::create_dir(&external).expect("external fixture directory creates");
        Self {
            root,
            workspace,
            external,
        }
    }

    fn temporary(&self) -> PathBuf {
        self.workspace
            .join(format!("README.md.steerbox-tmp-{}", std::process::id()))
    }
}

impl Drop for SecurityFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("owned temporary fixture removes");
    }
}

#[cfg(unix)]
#[test]
fn markdown_edit_rejects_parent_symlink_without_writing_outside_workspace() {
    use std::os::unix::fs::symlink;
    let fixture = SecurityFixture::new();
    fs::write(fixture.external.join("README.md"), "outside-old\n").unwrap();
    symlink(&fixture.external, fixture.workspace.join("docs")).unwrap();
    let result = edit_markdown(
        &fixture.workspace,
        &goal(),
        "docs/README.md",
        "outside-old\n",
        "outside-new\n",
        true,
    );
    assert_eq!(
        fs::read_to_string(fixture.external.join("README.md")).unwrap(),
        "outside-old\n",
        "a parent symlink must not permit an external write"
    );
    assert!(result.is_err(), "a symlink ancestor must be rejected");
}

#[cfg(unix)]
#[test]
fn markdown_edit_rejects_target_symlink_without_reading_or_replacing_it() {
    use std::os::unix::fs::symlink;
    let fixture = SecurityFixture::new();
    let external = fixture.external.join("README.md");
    fs::write(&external, "external fixture\n").unwrap();
    let target = fixture.workspace.join("README.md");
    symlink(&external, &target).unwrap();
    let result = edit_markdown(
        &fixture.workspace,
        &goal(),
        "README.md",
        "external fixture\n",
        "replacement\n",
        true,
    );
    assert!(
        result.is_err(),
        "target symlinks must be rejected before reading content"
    );
    assert!(fs::symlink_metadata(&target)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(external).unwrap(), "external fixture\n");
}

#[cfg(unix)]
#[test]
fn markdown_edit_does_not_follow_or_remove_a_preexisting_temporary_symlink() {
    use std::os::unix::fs::symlink;
    let fixture = SecurityFixture::new();
    fs::write(fixture.workspace.join("README.md"), "original\n").unwrap();
    let external = fixture.external.join("protected.md");
    fs::write(&external, "protected fixture\n").unwrap();
    let temporary = fixture.temporary();
    symlink(&external, &temporary).unwrap();
    let result = edit_markdown(
        &fixture.workspace,
        &goal(),
        "README.md",
        "original\n",
        "replacement\n",
        true,
    );
    assert_eq!(
        fs::read_to_string(&external).unwrap(),
        "protected fixture\n",
        "preexisting temporary symlink must never redirect the write"
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "original\n"
    );
    assert!(fs::symlink_metadata(&temporary)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn markdown_edit_preserves_a_preexisting_temporary_regular_file() {
    let fixture = SecurityFixture::new();
    fs::write(fixture.workspace.join("README.md"), "original\n").unwrap();
    let temporary = fixture.temporary();
    fs::write(&temporary, "other operation's data\n").unwrap();
    let result = edit_markdown(
        &fixture.workspace,
        &goal(),
        "README.md",
        "original\n",
        "replacement\n",
        true,
    );
    assert!(
        result.is_err(),
        "exclusive creation must reject existing temporary files"
    );
    assert_eq!(
        fs::read_to_string(&temporary).unwrap(),
        "other operation's data\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "original\n"
    );
}

#[cfg(unix)]
#[test]
fn verification_rejects_source_file_and_parent_symlinks() {
    use std::os::unix::fs::symlink;
    let fixture = SecurityFixture::new();
    let external = fixture.external.join("README.md");
    fs::write(&external, "external fixture has no links\n").unwrap();
    symlink(&external, fixture.workspace.join("README.md")).unwrap();
    symlink(&fixture.external, fixture.workspace.join("docs")).unwrap();
    for path in ["README.md", "docs/README.md"] {
        assert!(
            verify_markdown_links(&fixture.workspace, path).is_err(),
            "source {path} must not escape via symlink"
        );
    }
}

#[test]
fn verification_marks_parent_traversal_and_absolute_external_links_as_failed() {
    let fixture = SecurityFixture::new();
    let external = fixture.external.join("guide.md");
    fs::write(&external, "outside fixture\n").unwrap();
    let absolute = external.to_str().unwrap();
    fs::write(
        fixture.workspace.join("README.md"),
        format!("[traversal](../external/guide.md) [absolute]({absolute})\n"),
    )
    .unwrap();
    let verification = verify_markdown_links(&fixture.workspace, "README.md").unwrap();
    assert!(
        !verification.passed,
        "existing files outside the workspace cannot validate links"
    );
    assert_eq!(
        verification.missing_links,
        ["../external/guide.md", absolute]
    );
}

#[cfg(unix)]
#[test]
fn verification_marks_file_and_directory_symlink_escapes_as_failed() {
    use std::os::unix::fs::symlink;
    let fixture = SecurityFixture::new();
    fs::write(fixture.external.join("guide.md"), "outside fixture\n").unwrap();
    symlink(
        fixture.external.join("guide.md"),
        fixture.workspace.join("alias.md"),
    )
    .unwrap();
    symlink(&fixture.external, fixture.workspace.join("external-docs")).unwrap();
    fs::write(
        fixture.workspace.join("README.md"),
        "[file](alias.md) [directory](external-docs/guide.md)\n",
    )
    .unwrap();
    let verification = verify_markdown_links(&fixture.workspace, "README.md").unwrap();
    assert!(
        !verification.passed,
        "symlink targets outside workspace must not pass verification"
    );
    assert_eq!(
        verification.missing_links,
        ["alias.md", "external-docs/guide.md"]
    );
}

#[test]
fn verification_preserves_relative_parent_links_resolved_inside_workspace() {
    let fixture = SecurityFixture::new();
    fs::create_dir(fixture.workspace.join("docs")).unwrap();
    fs::write(fixture.workspace.join("guide.md"), "guide\n").unwrap();
    fs::write(
        fixture.workspace.join("docs/README.md"),
        "[guide](../guide.md#intro)\n",
    )
    .unwrap();
    let verification = verify_markdown_links(&fixture.workspace, "docs/README.md").unwrap();
    assert!(verification.passed);
    assert_eq!(verification.checked_links, ["../guide.md"]);
}

#[test]
fn markdown_edit_requires_an_existing_regular_markdown_target_and_matching_content() {
    let fixture = SecurityFixture::new();
    fs::write(fixture.workspace.join("README.md"), "original\n").unwrap();
    let mismatch = edit_markdown(
        &fixture.workspace,
        &goal(),
        "README.md",
        "stale\n",
        "new\n",
        true,
    );
    assert!(matches!(
        mismatch,
        Err(WorkspaceEditError::ExpectedContentMismatch)
    ));
    assert!(!fixture.temporary().exists());
    assert!(edit_markdown(&fixture.workspace, &goal(), "missing.md", "", "new\n", true).is_err());
    fs::create_dir(fixture.workspace.join("folder.md")).unwrap();
    assert!(edit_markdown(&fixture.workspace, &goal(), "folder.md", "", "new\n", true).is_err());
    fs::write(fixture.workspace.join("note.txt"), "original\n").unwrap();
    assert!(matches!(
        edit_markdown(
            &fixture.workspace,
            &goal(),
            "note.txt",
            "original\n",
            "new\n",
            true
        ),
        Err(WorkspaceEditError::NotMarkdown(_))
    ));
    assert!(verify_markdown_links(&fixture.workspace, "note.txt").is_err());
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "original\n"
    );
}
