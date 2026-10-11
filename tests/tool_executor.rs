use serde_json::json;
use steerbox::domain::{HumanSteeringMode, RiskLevel};
use steerbox::model_contracts::{ActionPathScope, ModelActionEnvelope, VerificationContract};
use steerbox::tool_executor::{
    ReadOnlyToolExecutor, StructuredToolAction, ToolCallInput, ToolExecutionError,
};
use steerbox::tool_registry::{SideEffectClass, ToolProfile, ToolRegistry};

fn action(tool_id: &str, parameters: serde_json::Value, path: &str) -> ModelActionEnvelope {
    ModelActionEnvelope {
        schema_version: steerbox::model_contracts::MODEL_ACTION_SCHEMA_VERSION,
        action_id: "action-test".into(),
        tool_id: tool_id.into(),
        parameters,
        path_scope: ActionPathScope {
            root: std::env::current_dir()
                .expect("current directory exists")
                .display()
                .to_string(),
            allowed_paths: vec![path.into()],
        },
        verification: VerificationContract {
            verifier_tool_id: "markdown_link_check".into(),
            parameters: json!({"path": path}),
        },
    }
}

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
fn baseline_read_only_recipes_execute_without_shell_arguments() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let executor = ReadOnlyToolExecutor;

    let search = executor
        .execute(
            &registry,
            &workspace,
            ToolCallInput {
                tool_id: "rg_fixed".into(),
                arguments: json!({"query": "workspace", "path": "README.md"}),
            },
        )
        .expect("fixed search executes");
    assert_eq!(search.result["tool_id"], "rg_fixed");
    assert!(search.result["executed"].as_bool().unwrap());
    assert_eq!(search.result["query"], "workspace");

    let links = executor
        .execute(
            &registry,
            &workspace,
            ToolCallInput {
                tool_id: "markdown_link_check".into(),
                arguments: json!({"path": "README.md"}),
            },
        )
        .expect("markdown link check executes");
    assert_eq!(links.result["tool_id"], "markdown_link_check");
    assert!(links.result["executed"].as_bool().unwrap());

    let file = executor
        .execute(
            &registry,
            &workspace,
            ToolCallInput {
                tool_id: "read_file".into(),
                arguments: json!({"path": "README.md"}),
            },
        )
        .expect("bounded file read executes");
    assert_eq!(file.result["tool_id"], "read_file");
    assert!(file.result["executed"].as_bool().unwrap());
    assert!(file.result["content"]
        .as_str()
        .unwrap()
        .contains("steerBox"));
}

#[test]
fn executor_rejects_registry_profile_that_is_not_read_only() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let mut registry = ToolRegistry::new();
    registry
        .register(ToolProfile {
            tool_id: "unsafe_status".into(),
            name: "unsafe status".into(),
            description: "fixture".into(),
            input_schema: json!({"arguments": "object"}),
            output_schema: json!({"summary": "string"}),
            side_effect_class: SideEffectClass::WorkspaceWrite,
            risk_level: RiskLevel::High,
            allowed_in_modes: vec![HumanSteeringMode::AutonomousLoop],
            recipe_ref: Some("git-status-readonly".into()),
            version: 1,
        })
        .expect("unsafe fixture registers");

    assert!(matches!(
        ReadOnlyToolExecutor.map_action(
            &registry,
            &workspace,
            &action("unsafe_status", json!({"scope": "workspace"}), "README.md"),
        ),
        Err(ToolExecutionError::UnsafeToolProfile(_))
    ));
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

#[test]
fn structured_action_maps_git_status_without_execution() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let action = action("git_status", json!({"scope": "workspace"}), "Cargo.toml");
    let mapped = ReadOnlyToolExecutor
        .map_action(
            &ToolRegistry::baseline().expect("baseline tools register"),
            &workspace,
            &action,
        )
        .expect("git status maps");
    assert_eq!(mapped, StructuredToolAction::GitStatus);
}

#[test]
fn structured_action_maps_fixed_rg_and_markdown_link_check() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let rg = action(
        "rg_fixed",
        json!({"query": "workspace", "path": "README.md"}),
        "README.md",
    );
    assert_eq!(
        ReadOnlyToolExecutor
            .map_action(&registry, &workspace, &rg)
            .expect("rg maps"),
        StructuredToolAction::RgFixed {
            query: "workspace".into(),
            path: "README.md".into(),
        }
    );

    let links = action(
        "markdown_link_check",
        json!({"path": "README.md"}),
        "README.md",
    );
    assert_eq!(
        ReadOnlyToolExecutor
            .map_action(&registry, &workspace, &links)
            .expect("link check maps"),
        StructuredToolAction::MarkdownLinkCheck {
            path: "README.md".into(),
        }
    );

    let read = action("read_file", json!({"path": "README.md"}), "README.md");
    assert_eq!(
        ReadOnlyToolExecutor
            .map_action(&registry, &workspace, &read)
            .expect("read file maps"),
        StructuredToolAction::ReadFile {
            path: "README.md".into(),
        }
    );
}

#[test]
fn structured_action_rejects_shell_non_object_and_unregistered_inputs() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let shell = action(
        "rg_fixed",
        json!({"command": "git status", "path": "README.md"}),
        "README.md",
    );
    assert!(matches!(
        ReadOnlyToolExecutor.map_action(&registry, &workspace, &shell),
        Err(ToolExecutionError::InvalidArguments)
    ));

    let mut non_object = action(
        "rg_fixed",
        json!({"query": "x", "path": "README.md"}),
        "README.md",
    );
    non_object.parameters = json!("git status");
    assert!(matches!(
        ReadOnlyToolExecutor.map_action(&registry, &workspace, &non_object),
        Err(ToolExecutionError::InvalidArguments)
    ));

    let unknown = action("unknown", json!({"scope": "workspace"}), "README.md");
    assert!(matches!(
        ReadOnlyToolExecutor.map_action(&registry, &workspace, &unknown),
        Err(ToolExecutionError::NotRegistered(_))
    ));
}

#[test]
fn structured_action_rejects_workspace_mismatch_and_path_escape() {
    let workspace = std::env::current_dir().expect("current directory exists");
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let mut mismatch = action("git_status", json!({"scope": "workspace"}), "Cargo.toml");
    mismatch.path_scope.root = "/tmp/other-workspace".into();
    assert!(matches!(
        ReadOnlyToolExecutor.map_action(&registry, &workspace, &mismatch),
        Err(ToolExecutionError::OutsideWorkspace(_))
    ));

    let escape = action(
        "rg_fixed",
        json!({"query": "x", "path": "../outside"}),
        "../outside",
    );
    assert!(matches!(
        ReadOnlyToolExecutor.map_action(&registry, &workspace, &escape),
        Err(ToolExecutionError::InvalidArguments)
    ));
}
