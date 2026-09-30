use serde_json::json;
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput, ToolExecutionError};
use steerbox::tool_registry::ToolRegistry;

#[test]
fn git_status_recipe_executes_fixed_read_only_command() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let output = ReadOnlyToolExecutor
        .execute(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &workspace,
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: json!({}),
            },
        )
        .expect("fixture tool executes");
    assert!(output.result["executed"].as_bool().unwrap());
    assert_eq!(output.result["tool_id"], "git_status");
    assert!(output.result["stdout"].as_str().unwrap().contains("##"));
}

#[test]
fn executor_rejects_unknown_and_non_object_inputs() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let executor = ReadOnlyToolExecutor;
    assert!(matches!(
        executor.execute(
            &registry,
            std::path::Path::new("/workspace/project"),
            ToolCallInput {
                tool_id: "unknown".into(),
                arguments: json!({})
            },
        ),
        Err(ToolExecutionError::NotRegistered(_))
    ));
    assert_eq!(
        executor
            .execute(
                &registry,
                std::path::Path::new("/workspace/project"),
                ToolCallInput {
                    tool_id: "git_status".into(),
                    arguments: json!("bad")
                },
            )
            .unwrap_err(),
        ToolExecutionError::InvalidArguments
    );
}

#[test]
fn executor_rejects_missing_workspace_before_command_execution() {
    let error = ReadOnlyToolExecutor
        .execute(
            &ToolRegistry::baseline().expect("baseline tools register"),
            std::path::Path::new("/tmp/steerbox-no-such-workspace"),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: json!({}),
            },
        )
        .expect_err("missing workspace is rejected");
    assert!(matches!(error, ToolExecutionError::InvalidWorkspace(_)));
}
