#![cfg(unix)]

use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::{LoopStatus, LoopStepStatus};
use steerbox::event_log::EventLog;
use steerbox::execution_query::ExecutionQuery;
use steerbox::http_transport::{CredentialError, CredentialResolver, ResolvedCredential};
use steerbox::loop_controller::{LoopControllerError, ResolvedModelLoopStopReason};
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{
    BoundedReadOnlyProviderRun, BoundedReadOnlyRunConfig, ForegroundProviderRun, ProviderRunError,
    ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::{
    ProgressEvent, ProgressSink, ProgressStatus, ProgressSubject, StreamReporter,
};
use steerbox::tool_call_store::ToolCallRecordStore;

struct Fixture {
    directory: tempfile::TempDir,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(
            workspace.join("README.md"),
            "# Documentation\n[Guide](guide.markdown)\n",
        )
        .unwrap();
        fs::write(workspace.join("guide.markdown"), "# Guide\n").unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&workspace)
            .status()
            .unwrap()
            .success());
        Self {
            directory,
            workspace,
        }
    }

    fn config(&self, provider: &str, tokens: Option<u64>) -> RuntimeConfig {
        RuntimeConfig::from_json_str(&json!({
            "schema_version":1,"project_ref":"readonly-fixture","data_root":"/temp/steerbox/readonly-fixture",
            "reported_token_limit":tokens,
            "model_profiles":[{"model_id":"fixture-model","provider":provider,"endpoint":"https://fixture.example.test/v1",
                "auth_ref":"secret-ref:synthetic-readonly","capabilities":["text"]}]
        }).to_string()).unwrap()
    }

    fn run(&self, iterations: u32) -> BoundedReadOnlyRunConfig {
        BoundedReadOnlyRunConfig {
            workspace: self.workspace.clone(),
            allowed_paths: vec!["README.md".into(), "guide.markdown".into()],
            max_iterations: iterations,
        }
    }

    fn prepared(
        &self,
        provider: &str,
        iterations: u32,
        tokens: Option<u64>,
    ) -> BoundedReadOnlyProviderRun {
        ForegroundProviderRun::prepare_readonly(
            &self.config(provider, tokens),
            request(),
            self.run(iterations),
        )
        .unwrap()
    }

    fn events_path(&self) -> PathBuf {
        self.directory.path().join("events.jsonl")
    }
    fn events(&self) -> EventLog {
        EventLog::open(self.events_path()).unwrap()
    }
    fn calls(&self) -> ModelCallStore {
        ModelCallStore::open(self.directory.path().join("calls.jsonl")).unwrap()
    }
    fn tools(&self) -> Vec<steerbox::domain::ToolCallRecord> {
        ToolCallRecordStore::open(self.directory.path().join("tool-calls.jsonl"))
            .unwrap()
            .read_page(0, 100)
            .unwrap()
    }
    fn assert_unchanged(&self) {
        assert_eq!(
            fs::read(self.workspace.join("README.md")).unwrap(),
            b"# Documentation\n[Guide](guide.markdown)\n"
        );
        assert_eq!(
            fs::read(self.workspace.join("guide.markdown")).unwrap(),
            b"# Guide\n"
        );
    }
    fn assert_failed(&self) {
        assert!(self
            .events()
            .read_all()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "loop_failed"));
    }

    fn assert_projection(&self, status: LoopStatus, steps: &[(&str, LoopStepStatus)]) {
        let events = self.events().read_all().unwrap();
        assert!(!events.is_empty());
        for event in &events {
            assert_eq!(event.project_id.as_deref(), Some("project"));
            assert_eq!(event.goal_id.as_deref(), Some("goal"));
            assert_eq!(event.loop_run_id.as_deref(), Some("loop"));
            assert_eq!(event.task_run_id.as_deref(), Some("task"));
            if event.event_type == "policy_decision" {
                assert_eq!(event.attempt_id, None);
            } else {
                assert_eq!(event.attempt_id.as_deref(), Some("loop:attempt:1"));
            }
        }

        let state = StateStore::rebuild(&events).unwrap();
        assert_eq!(state.loops().count(), 1);
        assert_eq!(state.steps().count(), steps.len());
        let projected_loop = state.loop_run("loop").unwrap();
        assert_eq!(projected_loop.goal_id, "goal");
        assert_eq!(projected_loop.task_run_id, "task");
        assert_eq!(projected_loop.status, status);
        assert_eq!(
            projected_loop.finished_at.is_some(),
            status != LoopStatus::Running
        );
        assert_eq!(
            projected_loop.current_step_id.as_deref(),
            steps.last().map(|(step_id, _)| *step_id)
        );
        for (index, (step_id, step_status)) in steps.iter().enumerate() {
            let step = state.step("loop", step_id).unwrap();
            assert_eq!(step.loop_run_id, "loop");
            assert_eq!(step.step_id, *step_id);
            assert_eq!(step.step_no as usize, index + 1);
            assert_eq!(step.phase, "model_decision");
            assert_eq!(step.status, *step_status);
            assert!(step.finished_at.is_some());
        }

        let query = ExecutionQuery::from_events(&events).unwrap();
        assert_eq!(query.list().len(), 1);
        let snapshot = query.surface_snapshot("loop").unwrap();
        assert_eq!(snapshot.execution.loop_run_id, "loop");
        assert_eq!(snapshot.execution.project_id.as_deref(), Some("project"));
        assert_eq!(snapshot.execution.goal_id, "goal");
        assert_eq!(snapshot.execution.task_run_id, "task");
        assert_eq!(snapshot.execution.status, status);
        assert_eq!(snapshot.execution.finished_at, projected_loop.finished_at);
        assert_eq!(snapshot.execution.event_count, events.len());
        assert_eq!(snapshot.events.len(), events.len());
        assert_eq!(
            snapshot.captured_through_seq,
            events.last().unwrap().sequence_no
        );
        for (projected, event) in snapshot.events.iter().zip(&events) {
            assert_eq!(projected.loop_run_id, "loop");
            assert_eq!(projected.event_id, event.event_id);
            assert_eq!(projected.event_type, event.event_type);
            assert_eq!(projected.sequence_no, event.sequence_no);
        }

        let calls = self.calls().read_all().unwrap();
        let failures = self.calls().read_failures().unwrap();
        assert_eq!(calls.len() + failures.len(), steps.len());
        for (task_run_id, prompt_pack_id, route_id, model_id) in calls
            .iter()
            .map(|record| {
                (
                    &record.task_run_id,
                    &record.resolved_prompt_pack_id,
                    &record.route_decision_id,
                    &record.model_id,
                )
            })
            .chain(failures.iter().map(|record| {
                (
                    &record.task_run_id,
                    &record.resolved_prompt_pack_id,
                    &record.route_decision_id,
                    &record.model_id,
                )
            }))
        {
            assert_eq!(task_run_id, "task");
            let starts = events
                .iter()
                .filter(|event| {
                    event.event_type == "model_call_started"
                        && event.payload["resolved_prompt_pack_id"].as_str()
                            == Some(prompt_pack_id.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(starts.len(), 1);
            assert_eq!(
                starts[0].payload["route_decision_id"].as_str(),
                Some(route_id.as_str())
            );
            assert_eq!(
                starts[0].payload["model_id"].as_str(),
                Some(model_id.as_str())
            );
            assert!(steps
                .iter()
                .any(|(step_id, _)| starts[0].event_id == format!("{step_id}:model_started")));
        }
        for record in calls.iter().filter(|record| record.error_class.is_none()) {
            assert_eq!(
                events
                    .iter()
                    .filter(|event| {
                        event.event_type == "model_call_completed"
                            && event.payload["model_call_id"].as_str()
                                == Some(record.model_call_id.as_str())
                    })
                    .count(),
                1
            );
        }
        for record in self.tools() {
            assert_eq!(record.project_id, "project");
            assert_eq!(record.goal_id, "goal");
            assert_eq!(record.loop_run_id, "loop");
            assert_eq!(record.task_run_id, "task");
            let owner_step = record
                .step_id
                .strip_suffix(":verification")
                .unwrap_or(&record.step_id);
            assert!(state.step("loop", owner_step).is_some());
            let requested = events
                .iter()
                .find(|event| {
                    event.event_type == "tool_call_requested"
                        && event.event_id == format!("{}:tool_requested", record.step_id)
                })
                .unwrap();
            assert_eq!(
                requested.payload["tool_id"].as_str(),
                Some(record.tool_id.as_str())
            );
            assert_eq!(
                record.tool_call_id,
                format!(
                    "loop:tool:{}:{}:{}",
                    requested.sequence_no.unwrap(),
                    record.step_id,
                    record.tool_id
                )
            );
        }
    }
}

fn request() -> ProviderRunRequest {
    ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "fixture-model".into(),
        objective: "Inspect authorized Markdown links and Git metadata".into(),
        observation: "Caller supplied initial observation".into(),
    }
}

fn action(workspace: &Path) -> Value {
    json!({"decision":"continue_observation","summary":"Inspect and verify","action":{
        "schema_version":1,"action_id":"inspect-1","tool_id":"git_status","parameters":{"scope":"workspace"},
        "path_scope":{"root":workspace,"allowed_paths":["README.md","guide.markdown"]},
        "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}
    }})
}

fn decision(name: &str) -> Value {
    json!({"decision":name,"summary":"Synthetic bounded result"})
}

struct Script {
    responses: Vec<Value>,
    calls: Cell<usize>,
    wires: RefCell<Vec<String>>,
    status: u16,
    hook: Option<Box<dyn Fn()>>,
}

impl Script {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses,
            calls: Cell::new(0),
            wires: RefCell::new(Vec::new()),
            status: 200,
            hook: None,
        }
    }
    fn respond(&self, body: &str, anthropic: bool) -> TransportResponse {
        let index = self.calls.get();
        self.calls.set(index + 1);
        self.wires.borrow_mut().push(body.to_owned());
        if index == 0 {
            if let Some(hook) = &self.hook {
                hook();
            }
        }
        if self.status != 200 {
            return TransportResponse::body(self.status, "RAW_SYNTHETIC_UPSTREAM_SECRET");
        }
        let content = self
            .responses
            .get(index)
            .unwrap_or_else(|| self.responses.last().unwrap())
            .to_string();
        let response = if anthropic {
            json!({"id":format!("response-{index}"),"type":"message","role":"assistant","model":"fixture-model",
                "content":[{"type":"text","text":content}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}})
        } else {
            json!({"id":format!("response-{index}"),"model":"fixture-model","choices":[{"message":{"content":content},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
        };
        TransportResponse::body(200, response.to_string())
    }
}

impl ProviderTransport for &Script {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert!(!streaming);
        Ok(self.respond(body, false))
    }
}
impl AnthropicTransport for &Script {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        assert!(!streaming);
        Ok(self.respond(body, true))
    }
}

fn execute(
    fixture: &Fixture,
    provider: &str,
    script: &Script,
    iterations: u32,
    tokens: Option<u64>,
) -> Result<steerbox::provider_runner::BoundedReadOnlyProviderRunOutcome, ProviderRunError> {
    let prepared = fixture.prepared(provider, iterations, tokens);
    if provider == "anthropic" {
        prepared.run_with_anthropic_transport(
            script,
            fixture.events(),
            fixture.calls(),
            StreamReporter::new(Vec::new()),
        )
    } else {
        prepared.run_with_transport(
            script,
            fixture.events(),
            fixture.calls(),
            StreamReporter::new(Vec::new()),
        )
    }
}

#[test]
fn readonly_provider_loop_exposes_caller_owned_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let run = BoundedReadOnlyRunConfig {
        workspace: directory.path().into(),
        allowed_paths: vec!["README.md".into()],
        max_iterations: 2,
    };
    assert_eq!(run.max_iterations, 2);
    let _: Option<BoundedReadOnlyProviderRun> = None;
    let _ = ForegroundProviderRun::prepare_readonly;
}

#[test]
fn both_protocols_execute_verified_action_then_complete_with_shared_audit() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        let script = Script::new(vec![action(&fixture.workspace), decision("complete_loop")]);
        let outcome = execute(&fixture, provider, &script, 2, Some(100)).unwrap();
        assert_eq!(outcome.loop_status, LoopStatus::Completed);
        assert_eq!(outcome.outcome.iterations, 2);
        assert_eq!(outcome.budget().tool_calls_used, 2);
        assert_eq!(outcome.budget().model_calls_used, 2);
        assert_eq!(outcome.budget().steps_used, 2);
        assert_eq!(outcome.budget().attempts_used, 1);
        assert_eq!(outcome.budget().max_tool_calls, 4);
        assert_eq!(
            outcome.outcome.stop_reason,
            ResolvedModelLoopStopReason::CompleteLoop
        );
        let budget = outcome.reported_token_budget().unwrap();
        assert_eq!(budget.records().len(), 2);
        assert_eq!(budget.known_total_tokens(), 10);
        assert_eq!(script.calls.get(), 2);
        let calls = fixture.calls().read_all().unwrap();
        assert_eq!(calls.len(), 2);
        assert_ne!(calls[0].model_call_id, calls[1].model_call_id);
        assert_ne!(
            calls[0].resolved_prompt_pack_id,
            calls[1].resolved_prompt_pack_id
        );
        assert_eq!(calls[0].tool_call_count, 2);
        assert_eq!(calls[1].tool_call_count, 0);
        assert_eq!(calls[0].verification_status, "passed");
        let tools = fixture.tools();
        assert_eq!(
            tools
                .iter()
                .filter(|record| record.status == "completed")
                .count(),
            2
        );
        assert_ne!(tools[0].tool_call_id, tools.last().unwrap().tool_call_id);
        let wires = script.wires.borrow();
        assert_eq!(calls[0].agent_role, "readonly_inspector");
        assert!(wires[0].contains("action_schema"));
        assert!(wires[0].contains("max_iterations"));
        assert!(!wires[0].contains("secret-ref:"));
        assert!(!wires[0].contains("fixture.example.test"));
        assert!(wires[1].contains("verification_output"));
        assert_ne!(wires[0], wires[1]);
        fixture.assert_projection(
            LoopStatus::Completed,
            &[
                ("step-1", LoopStepStatus::Completed),
                ("step-1:loop:2", LoopStepStatus::Completed),
            ],
        );
        fixture.assert_unchanged();
    }
}

#[test]
fn explicit_human_and_no_action_stops_do_not_spin_or_pretend_complete() {
    for provider in ["openai-compatible", "anthropic"] {
        for (name, reason) in [
            ("request_human", ResolvedModelLoopStopReason::RequestHuman),
            (
                "continue_observation",
                ResolvedModelLoopStopReason::ContinueObservationWithoutAction,
            ),
        ] {
            let fixture = Fixture::new();
            let script = Script::new(vec![decision(name)]);
            let outcome = execute(&fixture, provider, &script, 3, None).unwrap();
            assert_eq!(outcome.outcome.stop_reason, reason);
            assert_eq!(outcome.outcome.iterations, 1);
            assert_eq!(outcome.loop_status, LoopStatus::Running);
            assert_eq!(script.calls.get(), 1);
            assert!(fixture.tools().is_empty());
            fixture.assert_projection(
                LoopStatus::Running,
                &[("step-1", LoopStepStatus::Completed)],
            );
            fixture.assert_unchanged();
        }
    }
}

#[test]
fn invalid_action_and_verifier_matrix_preserves_response_evidence_without_tools() {
    for provider in ["openai-compatible", "anthropic"] {
        for case in 0..17 {
            let fixture = Fixture::new();
            let mut response = action(&fixture.workspace);
            match case {
                0 => response["action"]["tool_id"] = json!("read_file"),
                1 => response["action"]["tool_id"] = json!("git_diff"),
                2 => response["action"]["tool_id"] = json!("edit_markdown"),
                3 => response["action"]["tool_id"] = json!("bash"),
                4 => response["action"]["verification"]["verifier_tool_id"] = json!("read_file"),
                5 => response["action"]["verification"]["verifier_tool_id"] = json!("git_diff"),
                6 => response["action"]["path_scope"]["root"] = json!("/tmp"),
                7 => {
                    response["action"]["path_scope"]["allowed_paths"] =
                        json!(["README.md", "other.md"])
                }
                8 => response["action"]["parameters"] = json!({"scope":"workspace","extra":true}),
                9 => response["action"]["parameters"] = json!({"scope":"subtree"}),
                10 => {
                    response["action"]["verification"]["parameters"] =
                        json!({"path":"README.md","extra":true})
                }
                11 => response["action"]["verification"]["parameters"] = json!({"path":"other.md"}),
                12 => response["action"]["path_scope"]["allowed_paths"] = json!(["guide.markdown"]),
                13 => {
                    response["action"]["verification"] = json!({"verifier_tool_id":"git_status","parameters":{"scope":"workspace","path":"README.md"}})
                }
                14 => {
                    response["action"]["verification"]["parameters"] =
                        json!({"path":"../README.md"})
                }
                15 => response["action"]["tool_id"] = json!("rg_search"),
                16 => {
                    response["action"]["tool_id"] = json!("markdown_link_check");
                    response["action"]["parameters"] = json!({"path":"other.md"});
                }
                _ => unreachable!(),
            }
            let script = Script::new(vec![response, decision("complete_loop")]);
            assert!(
                execute(&fixture, provider, &script, 3, Some(100)).is_err(),
                "{provider} case {case}"
            );
            assert_eq!(script.calls.get(), 1, "{provider} case {case}");
            assert!(fixture.tools().is_empty());
            let failures = fixture.calls().read_failures().unwrap();
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].input_tokens, Some(3));
            assert_eq!(failures[0].output_tokens, Some(2));
            fixture.assert_failed();
            fixture.assert_projection(LoopStatus::Failed, &[("step-1", LoopStepStatus::Failed)]);
            fixture.assert_unchanged();
        }
    }
}

#[test]
fn invalid_caller_authority_is_rejected_during_prepare() {
    for case in 0..15 {
        let fixture = Fixture::new();
        let mut run = fixture.run(2);
        match case {
            0 => run.workspace = "/".into(),
            1 => run.workspace = "relative".into(),
            2 => run.workspace = fixture.workspace.join("../workspace"),
            3 => run.allowed_paths.clear(),
            4 => run.allowed_paths.push("README.md".into()),
            5 => run.allowed_paths = vec!["../README.md".into()],
            6 => run.allowed_paths = vec!["missing.md".into()],
            7 => {
                fs::write(fixture.workspace.join("secret.txt"), "not authorized").unwrap();
                run.allowed_paths = vec!["secret.txt".into()];
            }
            8 => {
                run.allowed_paths = vec![fixture
                    .workspace
                    .join("README.md")
                    .to_string_lossy()
                    .into_owned()]
            }
            9 => run.max_iterations = 0,
            10 => run.max_iterations = 17,
            11 => run.allowed_paths = vec!["./README.md".into()],
            12 => {
                fs::write(fixture.workspace.join("bad\\path.md"), "not authorized").unwrap();
                run.allowed_paths = vec!["bad\\path.md".into()];
            }
            13 => {
                fs::write(fixture.workspace.join("bad\npath.md"), "not authorized").unwrap();
                run.allowed_paths = vec!["bad\npath.md".into()];
            }
            14 => {
                fs::create_dir(fixture.workspace.join("directory.md")).unwrap();
                run.allowed_paths = vec!["directory.md".into()];
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                ForegroundProviderRun::prepare_readonly(
                    &fixture.config("openai-compatible", None),
                    request(),
                    run
                ),
                Err(ProviderRunError::InvalidRequest)
            ),
            "case {case}"
        );
    }
}

#[test]
fn zero_and_cumulative_reported_tokens_iteration_ceiling_and_repeat_guard_are_task_local() {
    for provider in ["openai-compatible", "anthropic"] {
        for (iterations, tokens, expected_sends, expected_tools) in [
            (3, Some(0), 0, 0),
            (3, Some(5), 1, 2),
            (1, None, 1, 2),
            (4, None, 2, 4),
        ] {
            let fixture = Fixture::new();
            let script = Script::new(vec![action(&fixture.workspace)]);
            let result = execute(&fixture, provider, &script, iterations, tokens);
            assert!(result.is_err(), "{provider} {iterations} {tokens:?}");
            assert_eq!(script.calls.get(), expected_sends);
            assert_eq!(
                fixture
                    .tools()
                    .iter()
                    .filter(|record| record.status == "completed")
                    .count(),
                expected_tools
            );
            assert_eq!(fixture.calls().read_all().unwrap().len(), expected_sends);
            fixture.assert_failed();
            fixture.assert_unchanged();
        }
    }
}

#[test]
fn verifier_semantic_failure_stops_before_second_send_and_records_failed_verification() {
    for provider in ["openai-compatible", "anthropic"] {
        for missing_links in [true, false] {
            let fixture = Fixture::new();
            let mut response = action(&fixture.workspace);
            if missing_links {
                fs::write(
                    fixture.workspace.join("README.md"),
                    "[Missing](absent.md)\n",
                )
                .unwrap();
            } else {
                fs::rename(
                    fixture.workspace.join(".git"),
                    fixture.directory.path().join("saved-git"),
                )
                .unwrap();
                response["action"]["tool_id"] = json!("markdown_link_check");
                response["action"]["parameters"] = json!({"path":"guide.markdown"});
                response["action"]["verification"] =
                    json!({"verifier_tool_id":"git_status","parameters":{"scope":"workspace"}});
            }
            let script = Script::new(vec![response, decision("complete_loop")]);
            let result = execute(&fixture, provider, &script, 2, None);
            assert!(
                result.is_err(),
                "{provider} missing={missing_links}: {result:?}"
            );
            assert_eq!(script.calls.get(), 1);
            let calls = fixture.calls().read_all().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].verification_status, "failed");
            assert_eq!(calls[0].tool_call_count, 2);
            assert!(!fixture
                .events()
                .read_all()
                .unwrap()
                .iter()
                .any(|event| event.event_type == "model_action_verified"));
            fixture.assert_failed();
            fixture.assert_projection(LoopStatus::Failed, &[("step-1", LoopStepStatus::Failed)]);
        }
    }
}

struct StopFailure;
impl ProgressSink for StopFailure {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if event.subject == ProgressSubject::Loop
            && event.status == ProgressStatus::Progress
            && event
                .summary
                .as_deref()
                .is_some_and(|summary| summary.starts_with("bounded resolved loop stopped:"))
        {
            return Err(std::io::Error::other("synthetic stop sink failure"));
        }
        Ok(())
    }
}

#[test]
fn stop_progress_failure_propagates_and_closes_human_and_no_action_runs() {
    for name in ["request_human", "continue_observation"] {
        let fixture = Fixture::new();
        let script = Script::new(vec![decision(name)]);
        let result = fixture
            .prepared("openai-compatible", 3, None)
            .run_with_transport(&script, fixture.events(), fixture.calls(), StopFailure);
        assert!(
            matches!(
                result,
                Err(ProviderRunError::Controller(
                    LoopControllerError::ProgressReport(_)
                ))
            ),
            "{result:?}"
        );
        assert_eq!(script.calls.get(), 1);
        fixture.assert_failed();
    }
}

struct UnavailableResolver(Arc<AtomicUsize>);
impl CredentialResolver for UnavailableResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }
}

#[test]
fn http_dispatch_checks_zero_tokens_history_and_paths_before_credential_resolution() {
    for provider in ["openai-compatible", "anthropic"] {
        for case in 0..5 {
            let fixture = Fixture::new();
            let resolutions = Arc::new(AtomicUsize::new(0));
            let limit = if case == 0 { Some(0) } else { None };
            if case == 1 {
                let script = Script::new(vec![decision("complete_loop")]);
                execute(&fixture, provider, &script, 2, None).unwrap();
            }
            let prepared = fixture.prepared(provider, 2, limit);
            let events = fixture.events();
            let calls = if case == 2 {
                ModelCallStore::open(fixture.workspace.join("forbidden-runtime.jsonl")).unwrap()
            } else {
                fixture.calls()
            };
            if case == 3 {
                fs::write(
                    fixture.directory.path().join("calls.jsonl"),
                    "not valid model history\n",
                )
                .unwrap();
            }
            let result = prepared.run_with_resolver(
                UnavailableResolver(resolutions.clone()),
                events,
                calls,
                StreamReporter::new(Vec::new()),
            );
            assert!(result.is_err(), "{provider} case {case}");
            assert_eq!(
                resolutions.load(Ordering::SeqCst),
                usize::from(case == 4),
                "{provider} case {case}"
            );
            fixture.assert_unchanged();
        }
    }
}

#[test]
fn wrong_protocol_and_duplicate_identity_reject_injected_transport_without_send() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        let script = Script::new(vec![decision("complete_loop")]);
        let prepared = fixture.prepared(provider, 2, None);
        let wrong_protocol = if provider == "anthropic" {
            prepared.run_with_transport(
                &script,
                fixture.events(),
                fixture.calls(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_anthropic_transport(
                &script,
                fixture.events(),
                fixture.calls(),
                StreamReporter::new(Vec::new()),
            )
        };
        assert!(matches!(
            wrong_protocol,
            Err(ProviderRunError::UnsupportedProtocol)
        ));
        assert_eq!(script.calls.get(), 0);
        execute(&fixture, provider, &script, 2, None).unwrap();
        let before = fs::read(fixture.events_path()).unwrap();
        let duplicate = execute(&fixture, provider, &script, 2, None);
        assert!(matches!(
            duplicate,
            Err(ProviderRunError::DuplicateRunIdentity)
        ));
        assert_eq!(script.calls.get(), 1);
        assert_eq!(fs::read(fixture.events_path()).unwrap(), before);
    }
}

#[test]
fn all_runtime_stores_and_locks_reject_workspace_placement_and_aliases() {
    for case in 0..8 {
        let fixture = Fixture::new();
        let prepared = fixture.prepared("openai-compatible", 2, None);
        let script = Script::new(vec![decision("complete_loop")]);
        let mut events_path = fixture.events_path();
        let mut calls_path = fixture.directory.path().join("calls.jsonl");
        match case {
            0 => events_path = fixture.workspace.join("runtime.jsonl"),
            1 => calls_path = fixture.workspace.join("calls.jsonl"),
            2 => calls_path = events_path.clone(),
            3 => calls_path = fixture.directory.path().join("tool-calls.jsonl"),
            4 => calls_path = fixture.directory.path().join("tool-calls.jsonl.lock"),
            5 => events_path = fixture.directory.path().join("calls.failures.jsonl"),
            6 => events_path = fixture.directory.path().join("calls.jsonl.lock"),
            7 => events_path = fixture.directory.path().join("calls.failures.jsonl.lock"),
            _ => unreachable!(),
        }
        let events = EventLog::open(&events_path).unwrap();
        let calls = ModelCallStore::open(&calls_path).unwrap();
        let before = fs::read(&events_path).unwrap();
        let result =
            prepared.run_with_transport(&script, events, calls, StreamReporter::new(Vec::new()));
        assert!(
            matches!(result, Err(ProviderRunError::InvalidRequest)),
            "case {case}: {result:?}"
        );
        assert_eq!(script.calls.get(), 0);
        assert_eq!(fs::read(events_path).unwrap(), before);
        fixture.assert_unchanged();
    }
}

#[cfg(unix)]
#[test]
fn runtime_symlinks_hardlinks_directories_and_authorized_file_aliases_fail_preflight() {
    use std::os::unix::fs::symlink;
    for alias in [
        "events.jsonl",
        "calls.jsonl",
        "calls.failures.jsonl",
        "calls.jsonl.lock",
        "calls.failures.jsonl.lock",
        "tool-calls.jsonl",
        "tool-calls.jsonl.lock",
    ] {
        for mode in ["symlink", "hardlink"] {
            let fixture = Fixture::new();
            let prepared = fixture.prepared("openai-compatible", 2, None);
            let events = fixture.events();
            let calls = fixture.calls();
            let path = fixture.directory.path().join(alias);
            if path.exists() {
                fs::rename(&path, fixture.directory.path().join("saved-events.jsonl")).unwrap();
            }
            let target = fixture.workspace.join("README.md");
            if mode == "symlink" {
                symlink(&target, &path).unwrap();
            } else {
                fs::hard_link(&target, &path).unwrap();
            }
            let script = Script::new(vec![decision("complete_loop")]);
            let result = prepared.run_with_transport(
                &script,
                events,
                calls,
                StreamReporter::new(Vec::new()),
            );
            assert!(
                matches!(result, Err(ProviderRunError::InvalidRequest)),
                "{alias} {mode}: {result:?}"
            );
            assert_eq!(script.calls.get(), 0);
            fixture.assert_unchanged();
        }
    }
    for alias in [
        "calls.jsonl",
        "calls.failures.jsonl",
        "calls.jsonl.lock",
        "calls.failures.jsonl.lock",
        "tool-calls.jsonl",
        "tool-calls.jsonl.lock",
    ] {
        let fixture = Fixture::new();
        let prepared = fixture.prepared("openai-compatible", 2, None);
        fs::create_dir(fixture.directory.path().join(alias)).unwrap();
        let script = Script::new(vec![decision("complete_loop")]);
        let result = prepared.run_with_transport(
            &script,
            fixture.events(),
            fixture.calls(),
            StreamReporter::new(Vec::new()),
        );
        assert!(
            matches!(result, Err(ProviderRunError::InvalidRequest)),
            "directory {alias}: {result:?}"
        );
        assert_eq!(script.calls.get(), 0);
    }
}

#[cfg(unix)]
#[test]
fn symlink_hardlink_and_replaced_workspace_authority_are_rechecked_before_send_and_after_response()
{
    use std::os::unix::fs::symlink;
    for case in 0..6 {
        let fixture = Fixture::new();
        let prepared = fixture.prepared("openai-compatible", 2, None);
        let mut run = fixture.run(2);
        match case {
            0 => {
                symlink(
                    &fixture.workspace,
                    fixture.directory.path().join("workspace-alias"),
                )
                .unwrap();
                run.workspace = fixture.directory.path().join("workspace-alias");
            }
            1 => {
                symlink(
                    fixture.workspace.join("README.md"),
                    fixture.workspace.join("alias.md"),
                )
                .unwrap();
                run.allowed_paths.push("alias.md".into());
            }
            2 => {
                fs::hard_link(
                    fixture.workspace.join("README.md"),
                    fixture.directory.path().join("alias.md"),
                )
                .unwrap();
            }
            3 => {
                fs::rename(
                    &fixture.workspace,
                    fixture.directory.path().join("old-workspace"),
                )
                .unwrap();
                fs::create_dir(&fixture.workspace).unwrap();
                fs::write(fixture.workspace.join("README.md"), "replacement").unwrap();
                fs::write(fixture.workspace.join("guide.markdown"), "replacement").unwrap();
            }
            4 => {
                fs::rename(
                    fixture.workspace.join("README.md"),
                    fixture.directory.path().join("saved-readme.md"),
                )
                .unwrap();
                symlink(
                    fixture.directory.path().join("saved-readme.md"),
                    fixture.workspace.join("README.md"),
                )
                .unwrap();
            }
            5 => {
                fs::rename(
                    fixture.workspace.join("README.md"),
                    fixture.directory.path().join("saved-readme.md"),
                )
                .unwrap();
                fs::create_dir(fixture.workspace.join("README.md")).unwrap();
            }
            _ => unreachable!(),
        }
        if case < 3 {
            assert!(
                matches!(
                    ForegroundProviderRun::prepare_readonly(
                        &fixture.config("openai-compatible", None),
                        request(),
                        run
                    ),
                    Err(ProviderRunError::InvalidRequest)
                ),
                "prepare case {case}"
            );
        }
        if case >= 2 {
            let script = Script::new(vec![decision("complete_loop")]);
            assert!(
                matches!(
                    prepared.run_with_transport(
                        &script,
                        fixture.events(),
                        fixture.calls(),
                        StreamReporter::new(Vec::new())
                    ),
                    Err(ProviderRunError::InvalidRequest)
                ),
                "run case {case}"
            );
            assert_eq!(script.calls.get(), 0);
        }
    }
    for inside_workspace in [false, true] {
        let fixture = Fixture::new();
        let workspace = fixture.workspace.clone();
        let saved = fixture.directory.path().join("saved-readme.md");
        let target = if inside_workspace {
            fixture.workspace.join("unauthorized.md")
        } else {
            saved.clone()
        };
        if inside_workspace {
            fs::write(&target, "unauthorized synthetic document body").unwrap();
        }
        let mut script = Script::new(vec![action(&fixture.workspace)]);
        script.hook = Some(Box::new(move || {
            fs::rename(workspace.join("README.md"), &saved).unwrap();
            symlink(&target, workspace.join("README.md")).unwrap();
        }));
        assert!(execute(&fixture, "openai-compatible", &script, 2, None).is_err());
        assert_eq!(script.calls.get(), 1);
        assert!(fixture.tools().is_empty());
        let failures = fixture.calls().read_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].input_tokens, Some(3));
        assert_eq!(failures[0].output_tokens, Some(2));
        fixture.assert_failed();
    }
}

#[test]
fn provider_parse_and_audit_failures_close_active_runs_without_raw_upstream_material() {
    for provider in ["openai-compatible", "anthropic"] {
        for case in 0..3 {
            let fixture = Fixture::new();
            let mut script = Script::new(vec![json!("RAW_SYNTHETIC_UPSTREAM_SECRET")]);
            if case == 0 {
                script.status = 503;
            }
            if case == 2 {
                let path = fixture.directory.path().join("calls.jsonl");
                script.responses = vec![decision("complete_loop")];
                script.hook = Some(Box::new(move || {
                    fs::create_dir(&path).unwrap();
                }));
            }
            let result = execute(&fixture, provider, &script, 2, None);
            if case == 2 {
                assert!(
                    matches!(
                        result,
                        Err(ProviderRunError::Controller(
                            LoopControllerError::ModelCallStore(_)
                        ))
                    ),
                    "{result:?}"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(ProviderRunError::Controller(
                            LoopControllerError::ModelAdapter(_)
                        ))
                    ),
                    "{result:?}"
                );
            }
            assert_eq!(script.calls.get(), 1);
            assert!(fixture.tools().is_empty());
            let events = fs::read_to_string(fixture.events_path()).unwrap();
            assert!(!events.contains("RAW_SYNTHETIC_UPSTREAM_SECRET"));
            fixture.assert_failed();
            fixture.assert_unchanged();
        }
    }
}

struct SelectedProgressFailure {
    subject: ProgressSubject,
    status: ProgressStatus,
}
impl ProgressSink for SelectedProgressFailure {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        if event.subject == self.subject && event.status == self.status {
            return Err(std::io::Error::other("synthetic lifecycle sink failure"));
        }
        Ok(())
    }
}

#[test]
fn postconstruction_progress_failure_closes_running_state_but_does_not_overwrite_completed() {
    for (subject, status, expected_sends, terminal_completed) in [
        (ProgressSubject::Model, ProgressStatus::Started, 0, false),
        (ProgressSubject::Tool, ProgressStatus::Completed, 1, false),
        (ProgressSubject::Loop, ProgressStatus::Completed, 1, true),
    ] {
        let fixture = Fixture::new();
        let response = if subject == ProgressSubject::Tool {
            action(&fixture.workspace)
        } else {
            decision("complete_loop")
        };
        let script = Script::new(vec![response]);
        let result = fixture
            .prepared("openai-compatible", 2, None)
            .run_with_transport(
                &script,
                fixture.events(),
                fixture.calls(),
                SelectedProgressFailure { subject, status },
            );
        assert!(
            matches!(
                result,
                Err(ProviderRunError::Controller(
                    LoopControllerError::ProgressReport(_)
                ))
            ),
            "{result:?}"
        );
        assert_eq!(script.calls.get(), expected_sends);
        let events = fixture.events().read_all().unwrap();
        assert_eq!(
            events
                .iter()
                .any(|event| event.event_type == "loop_completed"),
            terminal_completed
        );
        assert_eq!(
            events.iter().any(|event| event.event_type == "loop_failed"),
            !terminal_completed
        );
        fixture.assert_unchanged();
    }
}

#[test]
fn terminal_audit_failure_takes_precedence_over_original_provider_error() {
    let fixture = Fixture::new();
    let path = fixture.events_path();
    let saved = fixture.directory.path().join("saved-events.jsonl");
    let mut script = Script::new(vec![decision("complete_loop")]);
    script.status = 503;
    script.hook = Some(Box::new(move || {
        fs::rename(&path, &saved).unwrap();
        fs::create_dir(&path).unwrap();
    }));
    let result = execute(&fixture, "openai-compatible", &script, 2, None);
    assert!(
        matches!(
            result,
            Err(ProviderRunError::Controller(LoopControllerError::EventLog(
                _
            )))
        ),
        "{result:?}"
    );
    assert_eq!(script.calls.get(), 1);
    fixture.assert_unchanged();
}

#[test]
fn frozen_token_configuration_and_redacted_followup_survive_both_protocols() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        fs::write(
            fixture.workspace.join("password=SYNTHETIC_TEST_VALUE.txt"),
            "never read this body",
        )
        .unwrap();
        let mut config = fixture.config(provider, Some(100));
        let prepared =
            ForegroundProviderRun::prepare_readonly(&config, request(), fixture.run(2)).unwrap();
        config.reported_token_limit = Some(0);
        let script = Script::new(vec![action(&fixture.workspace), decision("complete_loop")]);
        let result = if provider == "anthropic" {
            prepared.run_with_anthropic_transport(
                &script,
                fixture.events(),
                fixture.calls(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_transport(
                &script,
                fixture.events(),
                fixture.calls(),
                StreamReporter::new(Vec::new()),
            )
        }
        .unwrap();
        assert_eq!(result.reported_token_budget().unwrap().limit(), 100);
        assert_eq!(result.reported_token_budget().unwrap().records().len(), 2);
        let wires = script.wires.borrow();
        assert!(!wires[1].contains("SYNTHETIC_TEST_VALUE"));
        assert!(!wires[1].contains("never read this body"));
        assert!(wires[1].contains("REDACTED"));
        fixture.assert_unchanged();
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_workspace_is_rejected_without_json_serialization_panic() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let invalid = fixture
        .directory
        .path()
        .join(std::ffi::OsString::from_vec(b"workspace-\xff".to_vec()));
    fs::rename(&fixture.workspace, &invalid).unwrap();
    let mut run = fixture.run(2);
    run.workspace = invalid;
    let result = ForegroundProviderRun::prepare_readonly(
        &fixture.config("openai-compatible", None),
        request(),
        run,
    );
    assert!(matches!(result, Err(ProviderRunError::InvalidRequest)));
}

#[test]
fn allowed_scope_count_has_the_same_boundary_as_model_action_contracts() {
    let fixture = Fixture::new();
    let mut run = fixture.run(16);
    run.allowed_paths.clear();
    for index in 0..257 {
        let path = format!("document-{index}.md");
        fs::write(fixture.workspace.join(&path), "# Bounded document\n").unwrap();
        run.allowed_paths.push(path);
    }
    assert!(matches!(
        ForegroundProviderRun::prepare_readonly(
            &fixture.config("openai-compatible", None),
            request(),
            run.clone()
        ),
        Err(ProviderRunError::InvalidRequest)
    ));
    run.allowed_paths.pop();
    assert!(ForegroundProviderRun::prepare_readonly(
        &fixture.config("openai-compatible", None),
        request(),
        run
    )
    .is_ok());
}
