use steerbox::tool_registry::{CapabilityLifecycleState, ToolRegistry};

#[test]
fn baseline_capability_views_keep_lifecycle_and_usage_separate() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let views = registry.capability_views();

    let read_file = views
        .iter()
        .find(|view| view.tool_id == "read_file")
        .expect("read_file capability exists");
    assert!(read_file.configured);
    assert!(read_file.enabled);
    assert_eq!(read_file.lifecycle_state, CapabilityLifecycleState::Enabled);
    assert_eq!(read_file.startup_mode, "on_demand");
    assert_eq!(read_file.resource_profile, "极低/按需");
    assert_eq!(read_file.use_case, "读取受限工作区文件");

    let edit_markdown = views
        .iter()
        .find(|view| view.tool_id == "edit_markdown")
        .expect("edit_markdown capability exists");
    assert!(edit_markdown.enabled);
    assert_eq!(
        edit_markdown.lifecycle_state,
        CapabilityLifecycleState::Enabled
    );
    assert_eq!(edit_markdown.resource_profile, "极低/按需");
    assert_eq!(edit_markdown.use_case, "审批后修改 Markdown");
}

#[test]
fn capability_table_is_compact_and_does_not_claim_running_tools() {
    let registry = ToolRegistry::baseline().expect("baseline tools register");
    let table = registry.render_capability_table();

    assert!(table.contains("工具/能力"));
    assert!(table.contains("默认状态"));
    assert!(table.contains("运行资源"));
    assert!(table.contains("使用场景"));
    assert!(table.contains("read_file"));
    assert!(table.contains("✅ 启用"));
    assert!(!table.contains("running"));
    assert!(!table.contains("已启动"));
}
