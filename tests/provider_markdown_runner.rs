use serde_json::json;
use std::{cell::Cell, fs, path::Path, time::Duration};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::{HumanSteeringMode, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{
    BoundedMarkdownRunConfig, ForegroundProviderRun, ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::StreamReporter;

struct AnthropicMarkdownTransport<'a> {
    workspace: &'a Path,
    replacement: &'a str,
}

impl AnthropicTransport for &AnthropicMarkdownTransport<'_> {
    fn send(
        &self,
        _: &str,
        headers: &AnthropicHeaders,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        assert_eq!(
            headers.api_key_ref.as_deref(),
            Some("secret-ref:anthropic-markdown")
        );
        assert!(!streaming);
        assert!(!body.contains("secret-ref:anthropic-markdown"));
        assert!(body.contains("edit_markdown"));
        let response = json!({
            "id": "anthropic-markdown-call",
            "type": "message",
            "role": "assistant",
            "model": "claude-fixture",
            "content": [{"type":"text","text":json!({
                "decision":"complete_loop",
                "summary":"bounded Anthropic edit",
                "action":{"schema_version":1,"action_id":"anthropic-edit-1","tool_id":"edit_markdown",
                    "parameters":{"path":"README.md","expected":"before\n","replacement":self.replacement},
                    "path_scope":{"root":self.workspace,"allowed_paths":["README.md"]},
                    "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}}
            }).to_string()}],
            "stop_reason":"end_turn",
            "stop_sequence":null,
            "usage":{"input_tokens":3,"output_tokens":2}
        });
        Ok(TransportResponse::body(200, response.to_string()))
    }
}

struct FakeTransport<'a> {
    workspace: &'a Path,
    replacement: &'a str,
    action_path: &'a str,
    decision: &'a str,
    calls: Cell<usize>,
}

impl ProviderTransport for &FakeTransport<'_> {
    fn send(
        &self,
        _: &str,
        auth_ref: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(auth_ref, Some("secret-ref:markdown-fixture"));
        assert!(!streaming);
        assert!(!body.contains("secret-ref:markdown-fixture"));
        let wire: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(wire["messages"][0]["role"], "system");
        assert_eq!(wire["messages"][1]["role"], "developer");
        assert_eq!(wire["messages"][2]["role"], "user");
        assert!(body.contains("edit_markdown"));
        assert!(body.contains("markdown_link_check"));
        let mut envelope = json!({
            "id":"fixture-markdown-call", "model":"test-model",
            "choices":[{"message":{"content":json!({
                "decision":self.decision, "summary":"bounded edit proposed",
                "action":{"schema_version":1,"action_id":"edit-1","tool_id":"edit_markdown",
                    "parameters":{"path":self.action_path,"expected":"before\n","replacement":self.replacement},
                    "path_scope":{"root":self.workspace,"allowed_paths":[self.action_path]},
                    "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":self.action_path}}}
            }).to_string()},"finish_reason":"stop"}]
        });
        if self.action_path.is_empty() {
            envelope["choices"][0]["message"]["content"] = json!({
                "decision": self.decision,
                "summary": "no action returned",
            })
            .to_string()
            .into();
        }
        Ok(TransportResponse::body(200, envelope.to_string()))
    }
}

fn run(
    root: &Path,
    replacement: &str,
    approved: bool,
) -> (
    Result<
        steerbox::provider_runner::ProviderRunOutcome,
        steerbox::provider_runner::ProviderRunError,
    >,
    ModelCallStore,
) {
    run_with_path(root, replacement, approved, "README.md")
}

fn run_with_path(
    root: &Path,
    replacement: &str,
    approved: bool,
    action_path: &str,
) -> (
    Result<
        steerbox::provider_runner::ProviderRunOutcome,
        steerbox::provider_runner::ProviderRunError,
    >,
    ModelCallStore,
) {
    run_with_decision(root, replacement, approved, action_path, "complete_loop")
}

fn run_with_decision(
    root: &Path,
    replacement: &str,
    approved: bool,
    action_path: &str,
    decision: &str,
) -> (
    Result<
        steerbox::provider_runner::ProviderRunOutcome,
        steerbox::provider_runner::ProviderRunError,
    >,
    ModelCallStore,
) {
    fs::write(root.join("README.md"), "before\n").unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture","capabilities":["text"]}]
    }).to_string()).unwrap();
    let request = ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "test-model".into(),
        objective: "Update README.md".into(),
        observation: "before\n".into(),
    };
    let run = BoundedMarkdownRunConfig {
        workspace: root.into(),
        allowed_paths: vec!["README.md".into()],
        checkpoint_path: root.join("checkpoints.jsonl"),
        trace_path: root.join("traces.jsonl"),
        side_effect_path: root.join("effects.jsonl"),
        approved,
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let transport = FakeTransport {
        workspace: root,
        replacement,
        action_path,
        decision,
        calls: Cell::new(0),
    };
    let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
    let result = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        );
    assert_eq!(transport.calls.get(), 1);
    (result, calls)
}

#[test]
fn markdown_provider_runs_policy_checkpoint_edit_verification_and_metadata_audit() {
    let directory = tempfile::tempdir().unwrap();
    let (result, calls) = run(directory.path(), "after\n", true);
    assert_eq!(result.unwrap().loop_status, LoopStatus::Completed);
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "after\n"
    );
    let records = calls.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].verification_status, "passed");
    assert!(records[0].input_context_hash.starts_with("sha256:"));
    let audit = format!(
        "{} {} {}",
        fs::read_to_string(directory.path().join("events.jsonl")).unwrap(),
        fs::read_to_string(directory.path().join("checkpoints.jsonl")).unwrap(),
        fs::read_to_string(directory.path().join("effects.jsonl")).unwrap()
    );
    assert!(audit.contains("model_action_verified"));
    assert!(audit.contains("before-write"));
    assert!(!audit.contains("secret-ref:markdown-fixture"));
    assert!(!audit.contains("after\\n"));
}

#[test]
fn anthropic_markdown_provider_runs_through_protocol_specific_adapter() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("README.md"), "before\n").unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,
        "project_ref":"anthropic-markdown-fixture",
        "data_root":"/temp/steerbox/anthropic-markdown-fixture",
        "model_profiles":[{"model_id":"claude-fixture","provider":"anthropic",
            "endpoint":"https://api.anthropic.example","auth_ref":"secret-ref:anthropic-markdown","capabilities":["text"]}]
    }).to_string()).unwrap();
    let request = ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "claude-fixture".into(),
        objective: "Update README.md".into(),
        observation: "before\n".into(),
    };
    let run = BoundedMarkdownRunConfig {
        workspace: directory.path().into(),
        allowed_paths: vec!["README.md".into()],
        checkpoint_path: directory.path().join("checkpoints.jsonl"),
        trace_path: directory.path().join("traces.jsonl"),
        side_effect_path: directory.path().join("effects.jsonl"),
        approved: true,
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let transport = AnthropicMarkdownTransport {
        workspace: directory.path(),
        replacement: "after\n",
    };
    let result = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_anthropic_transport(
            &transport,
            EventLog::open(directory.path().join("events.jsonl")).unwrap(),
            ModelCallStore::open(directory.path().join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(result.loop_status, LoopStatus::Completed);
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "after\n"
    );
}

#[test]
fn failed_markdown_link_check_does_not_claim_completion_or_automatic_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let replacement = "[missing](missing.md)\n";
    let (result, calls) = run(directory.path(), replacement, true);
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        replacement
    );
    let records = calls.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].verification_status, "failed");
    let events = fs::read_to_string(directory.path().join("events.jsonl")).unwrap();
    assert!(!events.contains("\"event_type\":\"loop_completed\""));
    assert!(events.contains("model_action_failed"));
    assert!(events.contains("loop_failed"));
}

#[test]
fn unapproved_markdown_edit_is_denied_without_changing_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let (result, _) = run(directory.path(), "after\n", false);
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "before\n"
    );
}

#[test]
fn model_scope_does_not_grant_access_to_another_markdown_file() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("OTHER.md"), "before\n").unwrap();
    let (result, calls) = run_with_path(directory.path(), "after\n", true, "OTHER.md");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("OTHER.md")).unwrap(),
        "before\n"
    );
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    assert!(calls.read_all().unwrap().is_empty());
}

#[test]
fn one_shot_markdown_runner_closes_continue_decision_as_failed() {
    let directory = tempfile::tempdir().unwrap();
    let (result, _) = run_with_decision(
        directory.path(),
        "after\n",
        true,
        "README.md",
        "continue_observation",
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "before\n"
    );
    let events = fs::read_to_string(directory.path().join("events.jsonl")).unwrap();
    assert!(events.contains("loop_failed"));
    assert!(!events.contains("\"event_type\":\"loop_completed\""));
}

#[test]
fn no_action_response_is_rejected_without_workspace_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let (result, _) = run_with_decision(directory.path(), "after\n", true, "", "complete_loop");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "before\n"
    );
}

#[test]
fn audit_store_collisions_are_rejected_before_provider_invocation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture","capabilities":["text"]}]
    }).to_string()).unwrap();
    for collision in ["checkpoints.jsonl", "README.md", "events.jsonl"] {
        let run = BoundedMarkdownRunConfig {
            workspace: root.into(),
            allowed_paths: vec!["README.md".into()],
            checkpoint_path: root.join("checkpoints.jsonl"),
            trace_path: root.join(collision),
            side_effect_path: root.join("effects.jsonl"),
            approved: true,
            human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
        };
        let request = ProviderRunRequest {
            project_id: "project".into(),
            goal_id: "goal".into(),
            loop_run_id: "loop".into(),
            task_run_id: "task".into(),
            model_id: "test-model".into(),
            objective: "Update README.md".into(),
            observation: "before\n".into(),
        };
        let transport = FakeTransport {
            workspace: root,
            replacement: "after\n",
            action_path: "README.md",
            decision: "complete_loop",
            calls: Cell::new(0),
        };
        let result =
            ForegroundProviderRun::prepare_markdown(&config, request, run).and_then(|prepared| {
                prepared.run_with_transport(
                    &transport,
                    EventLog::open(root.join("events.jsonl")).unwrap(),
                    ModelCallStore::open(root.join("calls.jsonl")).unwrap(),
                    StreamReporter::new(Vec::new()),
                )
            });
        assert!(result.is_err());
        assert_eq!(transport.calls.get(), 0);
    }
}

#[test]
fn model_call_store_cannot_alias_workspace_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("README.md"), "before\n").unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture","capabilities":["text"]}]
    }).to_string()).unwrap();
    let request = ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "test-model".into(),
        objective: "Update README.md".into(),
        observation: "before\n".into(),
    };
    let run = BoundedMarkdownRunConfig {
        workspace: root.into(),
        allowed_paths: vec!["README.md".into()],
        checkpoint_path: root.join("checkpoints.jsonl"),
        trace_path: root.join("traces.jsonl"),
        side_effect_path: root.join("effects.jsonl"),
        approved: true,
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let transport = FakeTransport {
        workspace: root,
        replacement: "after\n",
        action_path: "README.md",
        decision: "complete_loop",
        calls: Cell::new(0),
    };
    let model_calls = ModelCallStore::open(root.join("README.md")).unwrap();
    let result = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            model_calls,
            StreamReporter::new(Vec::new()),
        );
    assert!(result.is_err());
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "before\n"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_audit_store_is_rejected_before_provider_invocation() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("README.md"), "before\n").unwrap();
    symlink(root.join("README.md"), root.join("traces.jsonl")).unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture","capabilities":["text"]}]
    }).to_string()).unwrap();
    let request = ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "test-model".into(),
        objective: "Update README.md".into(),
        observation: "before\n".into(),
    };
    let run = BoundedMarkdownRunConfig {
        workspace: root.into(),
        allowed_paths: vec!["README.md".into()],
        checkpoint_path: root.join("checkpoints.jsonl"),
        trace_path: root.join("traces.jsonl"),
        side_effect_path: root.join("effects.jsonl"),
        approved: true,
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let transport = FakeTransport {
        workspace: root,
        replacement: "after\n",
        action_path: "README.md",
        decision: "complete_loop",
        calls: Cell::new(0),
    };
    let result = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            ModelCallStore::open(root.join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        );
    assert!(result.is_err());
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "before\n"
    );
}

#[cfg(unix)]
#[test]
fn hardlinked_audit_store_is_rejected_before_provider_invocation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("README.md"), "before\n").unwrap();
    fs::hard_link(root.join("README.md"), root.join("traces.jsonl")).unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture","capabilities":["text"]}]
    }).to_string()).unwrap();
    let request = ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "test-model".into(),
        objective: "Update README.md".into(),
        observation: "before\n".into(),
    };
    let run = BoundedMarkdownRunConfig {
        workspace: root.into(),
        allowed_paths: vec!["README.md".into()],
        checkpoint_path: root.join("checkpoints.jsonl"),
        trace_path: root.join("traces.jsonl"),
        side_effect_path: root.join("effects.jsonl"),
        approved: true,
        human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
    };
    let transport = FakeTransport {
        workspace: root,
        replacement: "after\n",
        action_path: "README.md",
        decision: "complete_loop",
        calls: Cell::new(0),
    };
    let result = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            ModelCallStore::open(root.join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        );
    assert!(result.is_err());
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "before\n"
    );
}
