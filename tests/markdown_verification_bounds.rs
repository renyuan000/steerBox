use serde_json::json;
use std::{fs, path::Path};
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput, ToolExecutionError};
use steerbox::tool_registry::ToolRegistry;

const CAP: usize = 2 * 1024 * 1024;

fn verify(workspace: &Path) -> Result<steerbox::tool_executor::ToolCallOutput, ToolExecutionError> {
    ReadOnlyToolExecutor.execute(
        &ToolRegistry::baseline().unwrap(),
        workspace,
        ToolCallInput {
            tool_id: "markdown_link_check".into(),
            arguments: json!({"path":"README.md"}),
        },
    )
}

#[test]
fn verifier_detects_missing_link_after_display_limit_and_across_boundary() {
    for padding in [16 * 1024, 16 * 1024 - 4] {
        let root = tempfile::tempdir().unwrap();
        let contents = format!("{}[broken](missing.md)\n", "a".repeat(padding));
        fs::write(root.path().join("README.md"), contents.as_bytes()).unwrap();
        let result = verify(root.path()).unwrap();
        assert_eq!(result.result["missing_local_links"], json!(["missing.md"]));
        assert_eq!(
            fs::read(root.path().join("README.md")).unwrap(),
            contents.as_bytes()
        );
    }
}

#[test]
fn verifier_accepts_exact_budget_and_checks_last_link_without_exposing_body() {
    let root = tempfile::tempdir().unwrap();
    let suffix = "[valid](target.md)\n";
    let contents = format!("{}{suffix}", "x".repeat(CAP - suffix.len()));
    fs::write(root.path().join("README.md"), &contents).unwrap();
    fs::write(root.path().join("target.md"), "target\n").unwrap();
    let result = verify(root.path()).unwrap();
    assert_eq!(result.result["missing_local_links"], json!([]));
    assert!(!result.result.to_string().contains("xxxx"));
    assert!(!result.summary.contains("truncated"));
    fs::remove_file(root.path().join("target.md")).unwrap();
    let result = verify(root.path()).unwrap();
    assert_eq!(result.result["missing_local_links"], json!(["target.md"]));
}

#[test]
fn verifier_rejects_budget_plus_one_instead_of_verifying_prefix() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), vec![b'x'; CAP + 1]).unwrap();
    assert_eq!(
        verify(root.path()),
        Err(ToolExecutionError::OutputCollectionFailed)
    );
}

#[test]
fn verifier_rejects_invalid_utf8_beyond_display_limit() {
    let root = tempfile::tempdir().unwrap();
    let mut contents = vec![b'x'; 16 * 1024 + 1];
    contents.push(0xff);
    fs::write(root.path().join("README.md"), &contents).unwrap();
    assert_eq!(
        verify(root.path()),
        Err(ToolExecutionError::OutputCollectionFailed)
    );
}

#[test]
fn display_read_still_truncates_at_original_limit() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "x".repeat(16 * 1024 + 1)).unwrap();
    let output = ReadOnlyToolExecutor
        .execute(
            &ToolRegistry::baseline().unwrap(),
            root.path(),
            ToolCallInput {
                tool_id: "read_file".into(),
                arguments: json!({"path":"README.md"}),
            },
        )
        .unwrap();
    assert_eq!(
        output.result["content"],
        format!("{}\n[truncated]", "x".repeat(16 * 1024))
    );
}

#[cfg(unix)]
#[test]
fn verifier_preserves_root_alias_and_regular_hardlink_compatibility() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("workspace")).unwrap();
    fs::write(root.path().join("original.md"), "# fixture\n").unwrap();
    fs::hard_link(
        root.path().join("original.md"),
        root.path().join("workspace/README.md"),
    )
    .unwrap();
    std::os::unix::fs::symlink(root.path().join("workspace"), root.path().join("alias")).unwrap();
    assert_eq!(
        verify(&root.path().join("alias")).unwrap().result["missing_local_links"],
        json!([])
    );
}
