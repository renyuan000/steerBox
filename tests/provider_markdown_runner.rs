use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
    time::Duration,
};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::{HumanSteeringMode, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::LoopControllerError;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{
    BoundedMarkdownRunConfig, ForegroundProviderRun, ProviderRunError, ProviderRunRequest,
};
use steerbox::reported_token_budget::ReportedTokenBudgetError;
use steerbox::runtime_config::{ConfigError, RuntimeConfig};
use steerbox::stream_reporter::StreamReporter;
use steerbox::tool_call_store::ToolCallRecordStore;

struct AnthropicMarkdownTransport<'a> {
    workspace: &'a Path,
    replacement: &'a str,
}

struct CountingBlockedTransport(Cell<usize>);

impl ProviderTransport for &CountingBlockedTransport {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.0.set(self.0.get() + 1);
        Ok(TransportResponse::body(500, "synthetic blocked response"))
    }
}

impl AnthropicTransport for &CountingBlockedTransport {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        self.0.set(self.0.get() + 1);
        Ok(TransportResponse::body(500, "synthetic blocked response"))
    }
}

#[test]
fn zero_reported_token_limit_stops_both_markdown_transports_without_edit_or_invocation_records() {
    for provider in ["openai-compatible", "anthropic"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let original = b"before\r\n\0unchanged bytes\n";
        fs::write(root.join("README.md"), original).unwrap();
        let mut config = RuntimeConfig::from_json_str(&json!({
            "schema_version":1,"project_ref":"zero-markdown","data_root":"/temp/steerbox/zero-markdown",
            "reported_token_limit":0,
            "model_profiles":[{"model_id":"test-model","provider":provider,
                "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:synthetic","capabilities":["text"]}]
        }).to_string()).unwrap();
        let request = ProviderRunRequest {
            project_id: "project".into(),
            goal_id: "goal".into(),
            loop_run_id: "loop".into(),
            task_run_id: "task".into(),
            model_id: "test-model".into(),
            objective: "Update README.md".into(),
            observation: "synthetic observation".into(),
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
        let prepared = ForegroundProviderRun::prepare_markdown(&config, request, run).unwrap();
        config.reported_token_limit = None;
        let transport = CountingBlockedTransport(Cell::new(0));
        let events = EventLog::open(root.join("events.jsonl")).unwrap();
        let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_with_anthropic_transport(
                &transport,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_transport(
                &transport,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        };
        assert_eq!(transport.0.get(), 0, "{provider}");
        assert!(
            matches!(
                result,
                Err(ProviderRunError::Controller(
                    LoopControllerError::ReportedTokenBudget(ReportedTokenBudgetError::Exhausted)
                ))
            ),
            "{result:?}"
        );
        assert_eq!(fs::read(root.join("README.md")).unwrap(), original);
        assert!(calls.read_all().unwrap().is_empty());
        assert!(calls.read_failures().unwrap().is_empty());
        assert!(tool_call_records(root).is_empty());
    }
}

#[test]
fn markdown_outcomes_snapshot_frozen_known_usage_or_disabled_budget_for_both_protocols() {
    for provider in ["openai-compatible", "anthropic"] {
        for limit in [None, Some(1)] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            fs::write(root.join("README.md"), "before\n").unwrap();
            let (model_id, auth_ref) = if provider == "anthropic" {
                ("claude-fixture", "secret-ref:anthropic-markdown")
            } else {
                ("test-model", "secret-ref:markdown-fixture")
            };
            let mut config = RuntimeConfig::from_json_str(&json!({
                "schema_version":1,"project_ref":"usage-markdown","data_root":"/temp/steerbox/usage-markdown",
                "reported_token_limit":limit,
                "model_profiles":[{"model_id":model_id,"provider":provider,
                    "endpoint":"https://fixture.example.test/v1","auth_ref":auth_ref,"capabilities":["text"]}]
            }).to_string()).unwrap();
            let request = ProviderRunRequest {
                project_id: "project".into(),
                goal_id: "goal".into(),
                loop_run_id: "loop".into(),
                task_run_id: "task".into(),
                model_id: model_id.into(),
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
            let prepared = ForegroundProviderRun::prepare_markdown(&config, request, run).unwrap();
            config.reported_token_limit = Some(0);
            let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
            let events = EventLog::open(root.join("events.jsonl")).unwrap();
            let openai = FakeTransport {
                workspace: root,
                replacement: "after\n",
                action_path: "README.md",
                decision: "complete_loop",
                calls: Cell::new(0),
            };
            let anthropic = AnthropicMarkdownTransport {
                workspace: root,
                replacement: "after\n",
            };
            let outcome = if provider == "anthropic" {
                prepared.run_with_anthropic_transport(
                    &anthropic,
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            } else {
                prepared.run_with_transport(
                    &openai,
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            }
            .unwrap();
            assert_eq!(outcome.loop_status, LoopStatus::Completed);
            assert_eq!(fs::read(root.join("README.md")).unwrap(), b"after\n");
            let records = calls.read_all().unwrap();
            assert_eq!(records.len(), 1);
            if let Some(limit) = limit {
                let budget = outcome.reported_token_budget().unwrap();
                let (input, output) = if provider == "anthropic" {
                    (3, 2)
                } else {
                    (0, 2)
                };
                assert_eq!(budget.limit(), limit);
                assert_eq!(budget.known_input_tokens(), input);
                assert_eq!(budget.known_output_tokens(), output);
                assert_eq!(budget.known_total_tokens(), input + output);
                assert_eq!(budget.blocked(), None);
                assert_eq!(budget.records().len(), 1);
                assert_eq!(
                    budget.records()[0].model_call_id(),
                    records[0].model_call_id
                );
            } else {
                assert_eq!(outcome.reported_token_budget(), None);
            }
        }
    }
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

struct EffectCaptureTransport<'a> {
    inner: FakeTransport<'a>,
    body: RefCell<String>,
}

impl ProviderTransport for &EffectCaptureTransport<'_> {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        *self.body.borrow_mut() = body.into();
        (&self.inner).send(endpoint, auth_ref, body, timeout, streaming)
    }
}

#[test]
fn configured_effect_binds_markdown_wire_and_exact_audit_hash() {
    run_markdown_effect_fixture(true, false);
    run_markdown_effect_fixture(false, false);
}

#[test]
fn invalid_mutated_markdown_effect_is_rejected_before_runtime_or_transport() {
    run_markdown_effect_fixture(true, true);
}

fn run_markdown_effect_fixture(configured: bool, invalid: bool) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("README.md"), "before\n").unwrap();
    let mut encoded = json!({
        "schema_version":1,"project_ref":"markdown-fixture","data_root":"/temp/steerbox/markdown-fixture",
        "model_profiles":[{"model_id":"test-model","provider":"openai-compatible",
            "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:markdown-fixture",
            "capabilities":["text"],"effect":{"selected_level":"strong","mapping":{"strong":"high"}}}]
    });
    if !configured {
        encoded["model_profiles"][0]
            .as_object_mut()
            .unwrap()
            .remove("effect");
    }
    let mut config = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap();
    if invalid {
        let mut profile = encoded["model_profiles"][0].clone();
        profile["effect"]["selected_level"] = json!("fast");
        config.model_profiles[0] = serde_json::from_value(profile).unwrap();
    }
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
    let transport = EffectCaptureTransport {
        inner: FakeTransport {
            workspace: root,
            replacement: "after\n",
            action_path: "README.md",
            decision: "complete_loop",
            calls: Cell::new(0),
        },
        body: RefCell::new(String::new()),
    };
    if invalid {
        let error = ForegroundProviderRun::prepare_markdown(&config, request, run)
            .err()
            .unwrap();
        assert!(matches!(
            error,
            steerbox::provider_runner::ProviderRunError::Config(
                ConfigError::InvalidModelEffect { .. }
            )
        ));
        assert_eq!(transport.inner.calls.get(), 0);
        assert!(transport.body.borrow().is_empty());
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "before\n"
        );
        assert_eq!(root.read_dir().unwrap().count(), 1);
        return;
    }
    let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
    let outcome = ForegroundProviderRun::prepare_markdown(&config, request, run)
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(outcome.loop_status, LoopStatus::Completed);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "after\n"
    );
    let body = transport.body.borrow();
    let wire: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(wire["model"], "test-model");
    if configured {
        assert_eq!(wire["reasoning_effort"], "high");
    } else {
        assert!(wire.get("reasoning_effort").is_none());
    }
    assert_eq!(
        calls.read_all().unwrap()[0].input_context_hash,
        format!("sha256:{:x}", Sha256::digest(body.as_bytes()))
    );
    assert_eq!(transport.inner.calls.get(), 1);
    assert!(!body.contains("secret-ref:"));
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
            }).to_string()},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":0,"completion_tokens":2}
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
    let transport = EffectCaptureTransport {
        inner: FakeTransport {
            workspace: root,
            replacement,
            action_path,
            decision,
            calls: Cell::new(0),
        },
        body: RefCell::new(String::new()),
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
    assert_eq!(transport.inner.calls.get(), 1);
    for failure in calls.read_failures().unwrap() {
        assert_eq!(
            failure.input_context_hash,
            Some(format!(
                "sha256:{:x}",
                Sha256::digest(transport.body.borrow().as_bytes())
            ))
        );
        assert_eq!(failure.input_tokens, Some(0));
        assert_eq!(failure.output_tokens, Some(2));
        assert_eq!(failure.finish_reason.as_deref(), Some("stop"));
        assert_eq!(failure.output_artifact_ref, None);
        assert_eq!(failure.estimated_cost, None);
    }
    (result, calls)
}

fn tool_call_records(root: &Path) -> Vec<steerbox::domain::ToolCallRecord> {
    ToolCallRecordStore::open(root.join("tool-calls.jsonl"))
        .unwrap()
        .read_page(0, 16)
        .unwrap()
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

    let tool_calls = tool_call_records(directory.path());
    assert_eq!(tool_calls.len(), 4);
    assert_eq!(tool_calls[0].tool_id, "edit_markdown");
    assert_eq!(tool_calls[0].status, "started");
    assert_eq!(tool_calls[1].tool_id, "edit_markdown");
    assert_eq!(tool_calls[1].status, "completed");
    assert_eq!(tool_calls[2].tool_id, "markdown_link_check");
    assert_eq!(tool_calls[2].status, "started");
    assert_eq!(tool_calls[3].tool_id, "markdown_link_check");
    assert_eq!(tool_calls[3].status, "completed");
    assert_eq!(tool_calls[0].tool_call_id, tool_calls[1].tool_call_id);
    assert_eq!(tool_calls[2].tool_call_id, tool_calls[3].tool_call_id);
    assert_ne!(tool_calls[0].tool_call_id, tool_calls[2].tool_call_id);
    let tool_metadata = serde_json::to_string(&tool_calls).unwrap();
    assert!(!tool_metadata.contains("secret-ref:markdown-fixture"));
    assert!(!tool_metadata.contains("after\\n"));
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

    let tool_calls = tool_call_records(directory.path());
    assert_eq!(tool_calls.len(), 4);
    assert_eq!(tool_calls[0].tool_id, "edit_markdown");
    assert_eq!(tool_calls[0].status, "started");
    assert_eq!(tool_calls[1].tool_id, "edit_markdown");
    assert_eq!(tool_calls[1].status, "completed");
    assert_eq!(tool_calls[2].tool_id, "markdown_link_check");
    assert_eq!(tool_calls[2].status, "started");
    assert_eq!(tool_calls[3].tool_id, "markdown_link_check");
    // The verifier tool completed successfully; its result reported missing
    // links, so the parent workspace action is marked failed separately.
    assert_eq!(tool_calls[3].status, "completed");
    assert_eq!(tool_calls[0].tool_call_id, tool_calls[1].tool_call_id);
    assert_ne!(tool_calls[0].tool_call_id, tool_calls[2].tool_call_id);
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
    assert!(tool_call_records(directory.path()).is_empty());
}

#[test]
fn one_shot_markdown_runner_closes_continue_decision_as_failed() {
    let directory = tempfile::tempdir().unwrap();
    let (result, calls) = run_with_decision(
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
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    assert!(tool_call_records(directory.path()).is_empty());
}

#[test]
fn no_action_response_is_rejected_without_workspace_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let (result, calls) = run_with_decision(directory.path(), "after\n", true, "", "complete_loop");
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("README.md")).unwrap(),
        "before\n"
    );
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    assert!(tool_call_records(directory.path()).is_empty());
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
