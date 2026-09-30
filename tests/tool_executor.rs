use serde_json::json;
use steerbox::tool_executor::{ReadOnlyToolExecutor, ToolCallInput, ToolExecutionError};
use steerbox::tool_registry::ToolRegistry;

#[test]
fn git_status_recipe_returns_secret_free_fixture_output() {
    let output = ReadOnlyToolExecutor
        .execute(
            &ToolRegistry::baseline().expect("baseline tools register"),
            std::path::Path::new("/workspace/project"),
            ToolCallInput {
                tool_id: "git_status".into(),
                arguments: json!({}),
            },
        )
        .expect("fixture tool executes");
    assert!(!output.result["executed"].as_bool().unwrap());
    assert_eq!(output.result["tool_id"], "git_status");
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
