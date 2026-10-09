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
