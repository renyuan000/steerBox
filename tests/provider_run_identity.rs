use serde_json::json;
use std::{cell::Cell, fs, path::Path, time::Duration};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::{EventEnvelope, HumanSteeringMode};
use steerbox::event_log::EventLog;
use steerbox::http_transport::{
    CredentialError, CredentialResolver, HttpAnthropicTransport, ResolvedCredential,
};
use steerbox::model_call_store::ModelCallStore;
use steerbox::model_contracts::{ModelCallFailureRecord, ModelCallRecord};
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{
    BoundedMarkdownRunConfig, ForegroundProviderRun, ProviderRunError, ProviderRunOutcome,
    ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::StreamReporter;

struct CountingTransport<'a> {
    workspace: &'a Path,
    sends: &'a Cell<usize>,
    markdown: bool,
}

impl CountingTransport<'_> {
    fn response(&self, anthropic: bool) -> TransportResponse {
        self.sends.set(self.sends.get() + 1);
        let mut decision = json!({"decision":"complete_loop","summary":"synthetic result"});
        if self.markdown {
            decision["action"] = json!({
                "schema_version":1,"action_id":"edit-1","tool_id":"edit_markdown",
                "parameters":{"path":"README.md","expected":"before\n","replacement":"after\n"},
                "path_scope":{"root":self.workspace,"allowed_paths":["README.md"]},
                "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}
            });
        }
        let response = if anthropic {
            json!({"id":"synthetic-message","type":"message","role":"assistant","model":"test-model",
                "content":[{"type":"text","text":decision.to_string()}],"stop_reason":"end_turn",
                "usage":{"input_tokens":2,"output_tokens":1}})
        } else {
            json!({"id":"synthetic-response","model":"test-model",
                "choices":[{"message":{"content":decision.to_string()},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":2,"completion_tokens":1}})
        };
        TransportResponse::body(200, response.to_string())
    }
}

impl ProviderTransport for &CountingTransport<'_> {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        Ok(self.response(false))
    }
}

impl AnthropicTransport for &CountingTransport<'_> {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        Ok(self.response(true))
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    sends: Cell<usize>,
    limit: Cell<Option<u64>>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("README.md"), "before\n").unwrap();
        EventLog::open(directory.path().join("events.jsonl")).unwrap();
        Self {
            directory,
            sends: Cell::new(0),
            limit: Cell::new(None),
        }
    }

    fn run(
        &self,
        provider: &str,
        markdown: bool,
        loop_id: &str,
        task_id: &str,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        self.run_with_events(provider, markdown, loop_id, task_id, None)
    }

    fn run_with_events(
        &self,
        provider: &str,
        markdown: bool,
        loop_id: &str,
        task_id: &str,
        events: Option<EventLog>,
    ) -> Result<ProviderRunOutcome, ProviderRunError> {
        let root = self.directory.path();
        let config = RuntimeConfig::from_json_str(&json!({
            "schema_version":1,"project_ref":"identity-fixture","data_root":"/temp/steerbox/identity-fixture",
            "reported_token_limit":self.limit.get(),
            "model_profiles":[{"model_id":"test-model","provider":provider,
                "endpoint":"https://synthetic.example.test/v1","auth_ref":"secret-ref:synthetic","capabilities":["text"]}]
        }).to_string()).unwrap();
        let request = ProviderRunRequest {
            project_id: "project".into(),
            goal_id: "goal".into(),
            loop_run_id: loop_id.into(),
            task_run_id: task_id.into(),
            model_id: "test-model".into(),
            objective: "Review or edit synthetic Markdown".into(),
            observation: "synthetic observation".into(),
        };
        let events = events.unwrap_or_else(|| EventLog::open(root.join("events.jsonl")).unwrap());
        let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
        let transport = CountingTransport {
            workspace: root,
            sends: &self.sends,
            markdown,
        };
        let sink = StreamReporter::new(Vec::new());
        if markdown {
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
            if provider == "anthropic" {
                prepared.run_with_anthropic_transport(&transport, events, calls, sink)
            } else {
                prepared.run_with_transport(&transport, events, calls, sink)
            }
        } else {
            let prepared = ForegroundProviderRun::prepare(&config, request).unwrap();
            if provider == "anthropic" {
                prepared.run_anthropic_with_transport(&transport, events, calls, sink)
            } else {
                prepared.run_with_transport(&transport, events, calls, sink)
            }
        }
    }

    fn snapshot(&self) -> Vec<Option<Vec<u8>>> {
        [
            "events.jsonl",
            "calls.jsonl",
            "calls.failures.jsonl",
            "tool-calls.jsonl",
            "checkpoints.jsonl",
            "traces.jsonl",
            "effects.jsonl",
            "README.md",
        ]
        .iter()
        .map(|name| fs::read(self.directory.path().join(name)).ok())
        .collect()
    }
}

fn seed_event(fixture: &Fixture, kind: &str, loop_id: Option<&str>, task_id: Option<&str>) {
    let mut event = EventEnvelope::new(
        kind,
        "synthetic-event",
        "loop_started",
        "identity",
        "synthetic",
        json!({"status":"running"}),
    )
    .unwrap();
    event.loop_run_id = loop_id.map(Into::into);
    event.task_run_id = task_id.map(Into::into);
    EventLog::open(fixture.directory.path().join("events.jsonl"))
        .unwrap()
        .append(event)
        .unwrap();
}

fn seed_audit(fixture: &Fixture, failure: bool, call_id: &str, task_id: &str) {
    let store = ModelCallStore::open(fixture.directory.path().join("calls.jsonl")).unwrap();
    let mut record = json!({
        "model_call_id":call_id,"task_id":"task","task_run_id":task_id,"agent_role":"observer",
        "provider_id":"synthetic","endpoint_id":"endpoint","model_id":"test-model",
        "route_decision_id":"route","resolved_prompt_pack_id":"prompt",
        "started_at":"2026-11-02T00:00:00Z","input_context_hash":"sha256:synthetic",
        "output_artifact_ref":format!("trace://{call_id}/output"),"input_tokens":2,"output_tokens":1,"estimated_cost":null,
        "finish_reason":"stop"
    });
    if failure {
        record["schema_version"] = json!(1);
        record["ended_at"] = json!("2026-11-02T00:00:01Z");
        record["failure_stage"] = json!("provider_request");
        record["failure_class"] = json!("timeout");
        store
            .append_failure(&serde_json::from_value::<ModelCallFailureRecord>(record).unwrap())
            .unwrap();
    } else {
        record["latency_ms"] = json!(1);
        record["streaming"] = json!(false);
        record["tool_call_count"] = json!(0);
        record["verification_status"] = json!("not_run");
        record["error_class"] = json!(null);
        record["completed_at"] = json!("2026-11-02T00:00:01Z");
        record["created_at"] = json!("2026-11-02T00:00:01Z");
        store
            .append(&serde_json::from_value::<ModelCallRecord>(record).unwrap())
            .unwrap();
    }
}

#[test]
fn existing_loop_or_task_references_reject_all_four_runners_without_mutation() {
    for provider in ["openai-compatible", "anthropic"] {
        for markdown in [false, true] {
            for history in 0..9 {
                let fixture = Fixture::new();
                match history {
                    0 => seed_event(&fixture, "loop_run", Some("loop-1"), Some("different-task")),
                    1 => seed_event(&fixture, "loop_run", Some("different-loop"), Some("task-1")),
                    2 => seed_event(&fixture, "loop_run", Some("loop-1"), Some("task-1")),
                    3 | 4 => {
                        let kind = if history == 3 { "loop_run" } else { "task_run" };
                        let identity = if history == 3 { "loop-1" } else { "task-1" };
                        let event = EventEnvelope::new(
                            kind,
                            "synthetic-event",
                            "unknown_future_event",
                            identity,
                            "synthetic",
                            json!({}),
                        )
                        .unwrap();
                        EventLog::open(fixture.directory.path().join("events.jsonl"))
                            .unwrap()
                            .append(event)
                            .unwrap();
                    }
                    5 => seed_audit(&fixture, false, "unrelated-call", "task-1"),
                    6 => seed_audit(&fixture, false, "call-loop-1-step-1", "different-task"),
                    7 => seed_audit(&fixture, true, "unrelated-call", "task-1"),
                    8 => seed_audit(&fixture, true, "call-loop-1-step-1", "different-task"),
                    _ => unreachable!(),
                }
                let snapshot = fixture.snapshot();
                let result = fixture.run(provider, markdown, "loop-1", "task-1");
                assert_eq!(
                    fixture.sends.get(),
                    0,
                    "{provider}, markdown={markdown}, history={history}: {result:?}"
                );
                assert!(matches!(
                    result,
                    Err(ProviderRunError::DuplicateRunIdentity)
                ));
                assert_eq!(fixture.snapshot(), snapshot);
            }
        }
    }
}

#[test]
fn different_identities_and_non_identity_aggregate_names_do_not_block_a_fresh_run() {
    for provider in ["openai-compatible", "anthropic"] {
        for markdown in [false, true] {
            let fixture = Fixture::new();
            seed_event(&fixture, "loop_run", Some("old-loop"), Some("old-task"));
            let event = EventEnvelope::new(
                "goal",
                "goal-event",
                "unknown_future_event",
                "loop-1",
                "synthetic",
                json!({}),
            )
            .unwrap();
            EventLog::open(fixture.directory.path().join("events.jsonl"))
                .unwrap()
                .append(event)
                .unwrap();
            seed_audit(&fixture, false, "call-loop-1-step-10", "old-task");
            seed_audit(&fixture, true, "call-old-loop-step-1", "old-task");
            fixture.run(provider, markdown, "loop-1", "task-1").unwrap();
            assert_eq!(fixture.sends.get(), 1);
        }
    }
}

#[test]
fn corrupt_or_unreadable_history_fails_closed_without_reflecting_bad_lines() {
    const MARKER: &str = "UNTRUSTED_HISTORY_MARKER";
    for provider in ["openai-compatible", "anthropic"] {
        for markdown in [false, true] {
            for name in ["events.jsonl", "calls.jsonl", "calls.failures.jsonl"] {
                let fixture = Fixture::new();
                if name == "events.jsonl" {
                    seed_event(&fixture, "loop_run", Some("old-loop"), Some("old-task"));
                } else {
                    seed_audit(
                        &fixture,
                        name == "calls.failures.jsonl",
                        "old-call",
                        "old-task",
                    );
                }
                let events = EventLog::open(fixture.directory.path().join("events.jsonl")).unwrap();
                let prefix = fs::read_to_string(fixture.directory.path().join(name)).unwrap();
                fs::write(
                    fixture.directory.path().join(name),
                    format!("{prefix}{{\"schema_version\":\"{MARKER}\"}}\n"),
                )
                .unwrap();
                let snapshot = fixture.snapshot();
                let result =
                    fixture.run_with_events(provider, markdown, "loop-1", "task-1", Some(events));
                assert_eq!(
                    fixture.sends.get(),
                    0,
                    "{provider}, {markdown}, {name}: {result:?}"
                );
                let error = result.unwrap_err();
                assert!(matches!(error, ProviderRunError::RunHistoryUnavailable));
                assert_eq!(
                    error.to_string(),
                    "provider run history could not be checked safely"
                );
                assert!(std::error::Error::source(&error).is_none());
                assert!(!error.to_string().contains(MARKER));
                assert!(!format!("{error:?}").contains(MARKER));
                assert_eq!(fixture.snapshot(), snapshot);
            }
        }
    }
}

struct CountingResolver(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl CredentialResolver for CountingResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }
}

#[test]
fn duplicate_history_is_rejected_before_either_http_credential_resolver() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        seed_event(&fixture, "loop_run", Some("loop-1"), None);
        let root = fixture.directory.path();
        let config = RuntimeConfig::from_json_str(&json!({
            "schema_version":1,"project_ref":"resolver-fixture","data_root":"/temp/steerbox/resolver-fixture",
            "model_profiles":[{"model_id":"test-model","provider":provider,"endpoint":"https://synthetic.example.test/v1",
                "auth_ref":"secret-ref:synthetic","capabilities":["text"]}]
        }).to_string()).unwrap();
        let request = ProviderRunRequest {
            project_id: "project".into(),
            goal_id: "goal".into(),
            loop_run_id: "loop-1".into(),
            task_run_id: "task-1".into(),
            model_id: "test-model".into(),
            objective: "Review fixture".into(),
            observation: "synthetic".into(),
        };
        let prepared = ForegroundProviderRun::prepare(&config, request).unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolver = CountingResolver(count.clone());
        let events = EventLog::open(root.join("events.jsonl")).unwrap();
        let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
        let snapshot = fixture.snapshot();
        let result = if provider == "anthropic" {
            prepared.run_anthropic_with_transport(
                HttpAnthropicTransport::new(resolver).unwrap(),
                events,
                calls,
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_resolver(resolver, events, calls, StreamReporter::new(Vec::new()))
        };
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{provider}: {result:?}"
        );
        assert!(matches!(
            result,
            Err(ProviderRunError::DuplicateRunIdentity)
        ));
        assert_eq!(fixture.snapshot(), snapshot);
    }
}

#[test]
fn repeated_identity_is_rejected_before_send_and_without_business_log_changes_in_all_runners() {
    for provider in ["openai-compatible", "anthropic"] {
        for markdown in [false, true] {
            let fixture = Fixture::new();
            fixture.run(provider, markdown, "loop-1", "task-1").unwrap();
            assert_eq!(fixture.sends.get(), 1);
            let snapshot = fixture.snapshot();
            let result = fixture.run(provider, markdown, "loop-1", "task-1");
            assert_eq!(
                fixture.sends.get(),
                1,
                "{provider}, markdown={markdown}: {result:?}"
            );
            let error = result.unwrap_err();
            assert!(matches!(error, ProviderRunError::DuplicateRunIdentity));
            assert!(std::error::Error::source(&error).is_none());
            assert_eq!(fixture.snapshot(), snapshot);
        }
    }
}

#[test]
fn invalid_model_contracts_and_unreadable_audit_paths_fail_closed() {
    for provider in ["openai-compatible", "anthropic"] {
        for failure in [false, true] {
            for markdown in [false, true] {
                let fixture = Fixture::new();
                seed_audit(&fixture, failure, "unrelated-call", "old-task");
                let name = if failure {
                    "calls.failures.jsonl"
                } else {
                    "calls.jsonl"
                };
                let path = fixture.directory.path().join(name);
                let prefix = fs::read_to_string(&path).unwrap();
                let mut record: serde_json::Value = serde_json::from_str(&prefix).unwrap();
                record["task_run_id"] = json!("");
                fs::write(path, format!("{prefix}{record}\n")).unwrap();
                let snapshot = fixture.snapshot();
                let result = fixture.run(provider, markdown, "loop-1", "task-1");
                assert!(
                    matches!(result, Err(ProviderRunError::RunHistoryUnavailable)),
                    "{result:?}"
                );
                assert_eq!(fixture.sends.get(), 0);
                assert_eq!(fixture.snapshot(), snapshot);
            }
        }
        for name in ["calls.jsonl", "calls.failures.jsonl", "calls.jsonl.lock"] {
            let fixture = Fixture::new();
            fs::create_dir(fixture.directory.path().join(name)).unwrap();
            let snapshot = fixture.snapshot();
            let result = fixture.run(provider, false, "loop-1", "task-1");
            assert!(
                matches!(result, Err(ProviderRunError::RunHistoryUnavailable)),
                "{result:?}"
            );
            assert_eq!(fixture.sends.get(), 0);
            assert_eq!(fixture.snapshot(), snapshot);
            assert!(fixture.directory.path().join(name).is_dir());
        }
    }
}

#[test]
fn identity_guard_precedes_disabled_zero_and_positive_token_budget() {
    for limit in [None, Some(0), Some(10)] {
        let fixture = Fixture::new();
        fixture.limit.set(limit);
        seed_event(&fixture, "loop_run", Some("loop-1"), None);
        let snapshot = fixture.snapshot();
        let result = fixture.run("openai-compatible", false, "loop-1", "task-1");
        assert!(
            matches!(result, Err(ProviderRunError::DuplicateRunIdentity)),
            "{result:?}"
        );
        assert_eq!(fixture.sends.get(), 0);
        assert_eq!(fixture.snapshot(), snapshot);
    }
}

#[test]
fn distinct_foreground_runs_share_history_and_rebuild_loop_scoped_steps() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        fixture.run(provider, false, "loop-1", "task-1").unwrap();
        fixture.run(provider, false, "loop-2", "task-2").unwrap();
        assert_eq!(fixture.sends.get(), 2);
        let events = EventLog::open(fixture.directory.path().join("events.jsonl"))
            .unwrap()
            .read_all()
            .unwrap();
        let state = steerbox::state_store::StateStore::rebuild(&events).unwrap();
        assert_eq!(state.steps().count(), 2);
        let query = steerbox::execution_query::ExecutionQuery::from_events(&events).unwrap();
        assert_eq!(query.list().len(), 2);
        for (loop_id, task_id) in [("loop-1", "task-1"), ("loop-2", "task-2")] {
            let step = state.step(loop_id, "step-1").unwrap();
            assert_eq!(step.loop_run_id, loop_id);
            assert_eq!(step.status, steerbox::domain::LoopStepStatus::Completed);
            let snapshot = query.surface_snapshot(loop_id).unwrap();
            assert_eq!(snapshot.execution.loop_run_id, loop_id);
            assert_eq!(snapshot.execution.task_run_id, task_id);
            assert_eq!(
                snapshot.execution.status,
                steerbox::domain::LoopStatus::Completed
            );
            assert!(snapshot
                .events
                .iter()
                .all(|event| event.loop_run_id == loop_id));
            let page = query.events_page(loop_id, 0, 2).unwrap();
            assert_eq!(page.loop_run_id, loop_id);
            assert_eq!(page.total, snapshot.events.len());
            assert_eq!(page.events, snapshot.events[..2]);
        }
    }
}
