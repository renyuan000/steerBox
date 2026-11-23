use super::*;
use serde_json::{json, Value};
use std::{cell::Cell, cell::RefCell, rc::Rc, time::Duration};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::LoopStatus;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::stream_reporter::{ProgressStatus, ProgressSubject};

struct Fixture {
    _directory: tempfile::TempDir,
    config: RuntimeConfig,
    workspace: PathBuf,
    anthropic: bool,
}

impl Fixture {
    fn new(anthropic: bool) -> Self {
        fs::create_dir_all("/temp/steerbox").unwrap();
        let directory = tempfile::Builder::new()
            .prefix("markdown-cli-unit-")
            .tempdir_in("/temp/steerbox")
            .unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), "before\n").unwrap();
        fs::write(workspace.join("OTHER.md"), "untouched\n").unwrap();
        let config = RuntimeConfig::from_json_str(
            &json!({
                "schema_version":1,
                "project_ref":directory.path().file_name().unwrap().to_str().unwrap(),
                "data_root":directory.path(),
                "model_profiles":[{"model_id":"fixture-model",
                    "provider":if anthropic { "anthropic" } else { "openai-compatible" },
                    "endpoint":"https://synthetic.example.test/v1",
                    "auth_ref":"env:SYNTHETIC_MARKDOWN_CLI_NOT_CONFIGURED",
                    "capabilities":["text"]}]
            })
            .to_string(),
        )
        .unwrap();
        config.write_json(config.config_path()).unwrap();
        fs::create_dir(config.events_path().parent().unwrap()).unwrap();
        Self {
            _directory: directory,
            config,
            workspace,
            anthropic,
        }
    }

    fn args(&self) -> Vec<String> {
        vec![
            self.config.config_path().display().to_string(),
            "project".into(),
            "goal".into(),
            "loop".into(),
            "task".into(),
            "fixture-model".into(),
            "synthetic hidden objective".into(),
            "synthetic hidden observation".into(),
            "--authorize-markdown".into(),
            "once".into(),
            "--workspace".into(),
            self.workspace.display().to_string(),
            "--path".into(),
            "README.md".into(),
        ]
    }

    fn run(&self, transport: SyntheticTransport) -> Result<String, Box<dyn Error>> {
        run_with(
            self.args().into_iter(),
            |prepared, credentials, events, calls| {
                assert!(matches!(credentials, CliCredentialResolver::Unconfigured));
                execute(prepared, events, calls, transport, self.anthropic)
            },
        )
    }

    fn contents(&self, path: &str) -> String {
        fs::read_to_string(self.workspace.join(path)).unwrap()
    }
}

struct SyntheticTransport {
    action: Value,
    sends: Rc<Cell<usize>>,
}

impl SyntheticTransport {
    fn new(workspace: &Path, replacement: &str) -> Self {
        Self {
            action: json!({"decision":"complete_loop","summary":"untrusted model summary",
                "action":{"schema_version":1,"action_id":"edit-1","tool_id":"edit_markdown",
                    "parameters":{"path":"README.md","expected":"before\n","replacement":replacement},
                    "path_scope":{"root":workspace,"allowed_paths":["README.md"]},
                    "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}}}),
            sends: Rc::new(Cell::new(0)),
        }
    }

    fn response(&self, anthropic: bool) -> TransportResponse {
        self.sends.set(self.sends.get() + 1);
        let content = self.action.to_string();
        let wire = if anthropic {
            json!({"id":"synthetic-response","type":"message","role":"assistant",
                "model":"fixture-model","content":[{"type":"text","text":content}],
                "stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}})
        } else {
            json!({"id":"synthetic-response","model":"fixture-model",
                "choices":[{"message":{"content":content},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
        };
        TransportResponse::body(200, wire.to_string())
    }
}

impl ProviderTransport for SyntheticTransport {
    fn send(
        &self,
        endpoint: &str,
        _: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert_eq!(
            endpoint,
            "https://synthetic.example.test/v1/chat/completions"
        );
        assert!(!streaming);
        assert!(!body.contains("SYNTHETIC_MARKDOWN_CLI_NOT_CONFIGURED"));
        Ok(self.response(false))
    }
}

impl AnthropicTransport for SyntheticTransport {
    fn send(
        &self,
        endpoint: &str,
        _: &AnthropicHeaders,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        assert_eq!(endpoint, "https://synthetic.example.test/v1/messages");
        assert!(!streaming);
        assert!(!body.contains("SYNTHETIC_MARKDOWN_CLI_NOT_CONFIGURED"));
        Ok(self.response(true))
    }
}

fn execute(
    prepared: BoundedMarkdownProviderRun,
    events: EventLog,
    calls: ModelCallStore,
    transport: SyntheticTransport,
    anthropic: bool,
) -> Result<ProviderRunOutcome, ProviderRunError> {
    let sink = MetadataProgress(StreamReporter::new(Vec::new()));
    if anthropic {
        prepared.run_with_anthropic_transport(transport, events, calls, sink)
    } else {
        prepared.run_with_transport(transport, events, calls, sink)
    }
}

#[test]
fn markdown_cli_both_protocols_edit_once_verify_and_project_correlated_state() {
    for anthropic in [false, true] {
        let fixture = Fixture::new(anthropic);
        let transport = SyntheticTransport::new(&fixture.workspace, "after\n");
        let sends = transport.sends.clone();
        let summary = fixture.run(transport).unwrap();
        assert_eq!(
            summary,
            "decision=CompleteLoop loop_status=Completed scope=markdown"
        );
        assert_eq!(sends.get(), 1);
        assert_eq!(fixture.contents("README.md"), "after\n");
        assert_eq!(fixture.contents("OTHER.md"), "untouched\n");
        let state = StateStore::load(fixture.config.state_path()).unwrap();
        let run = state.loop_run("loop").unwrap();
        assert_eq!(run.status, LoopStatus::Completed);
        assert_eq!(run.task_run_id, "task");
        assert_eq!(run.goal_id, "goal");
        let events = EventLog::open(fixture.config.events_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "loop_completed"));
        for event in &events {
            assert_eq!(event.project_id.as_deref(), Some("project"));
            assert_eq!(event.goal_id.as_deref(), Some("goal"));
            assert_eq!(event.loop_run_id.as_deref(), Some("loop"));
            assert_eq!(event.task_run_id.as_deref(), Some("task"));
        }
        let calls = ModelCallStore::open(fixture.config.model_calls_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].verification_status, "passed");
        assert_eq!(calls[0].task_run_id, "task");
        assert!(fixture.config.checkpoints_path().is_file());
        assert!(fixture.config.side_effects_path().is_file());
    }
}

#[test]
fn markdown_cli_tail_verification_failure_does_not_complete_or_rollback() {
    for anthropic in [false, true] {
        let fixture = Fixture::new(anthropic);
        let replacement = format!("{}\n[missing](missing.md)\n", "x".repeat(17 * 1024));
        let transport = SyntheticTransport::new(&fixture.workspace, &replacement);
        let sends = transport.sends.clone();
        assert_eq!(
            fixture.run(transport).unwrap_err().to_string(),
            EXECUTION_FAILED
        );
        assert_eq!(sends.get(), 1);
        assert_eq!(fixture.contents("README.md"), replacement);
        let state = StateStore::load(fixture.config.state_path()).unwrap();
        assert_eq!(state.loop_run("loop").unwrap().status, LoopStatus::Failed);
        let events = EventLog::open(fixture.config.events_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert!(!events
            .iter()
            .any(|event| event.event_type == "loop_completed"));
        let calls = ModelCallStore::open(fixture.config.model_calls_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].verification_status, "failed");
    }
}

#[test]
fn markdown_cli_reused_identity_never_sends_a_second_model_request() {
    for anthropic in [false, true] {
        let fixture = Fixture::new(anthropic);
        fixture
            .run(SyntheticTransport::new(&fixture.workspace, "after\n"))
            .unwrap();
        let history = fs::read(fixture.config.events_path()).unwrap();
        let transport = SyntheticTransport::new(&fixture.workspace, "not permitted\n");
        let sends = transport.sends.clone();
        assert_eq!(
            fixture.run(transport).unwrap_err().to_string(),
            EXECUTION_FAILED
        );
        assert_eq!(sends.get(), 0);
        assert_eq!(fixture.contents("README.md"), "after\n");
        assert_eq!(fs::read(fixture.config.events_path()).unwrap(), history);
    }
}

#[test]
fn markdown_cli_corrupt_event_or_model_history_never_sends() {
    for anthropic in [false, true] {
        for model_history in [false, true] {
            let fixture = Fixture::new(anthropic);
            let path = if model_history {
                fixture.config.model_calls_path()
            } else {
                fixture.config.events_path()
            };
            fs::write(&path, "untrusted-invalid-history\n").unwrap();
            let transport = SyntheticTransport::new(&fixture.workspace, "after\n");
            let sends = transport.sends.clone();
            assert_eq!(
                fixture.run(transport).unwrap_err().to_string(),
                EXECUTION_FAILED
            );
            assert_eq!(sends.get(), 0);
            assert_eq!(fixture.contents("README.md"), "before\n");
            assert_eq!(
                fs::read_to_string(path).unwrap(),
                "untrusted-invalid-history\n"
            );
        }
    }
}

#[test]
fn markdown_cli_rejects_outside_scope_wrong_verifier_and_nonterminal_actions() {
    for anthropic in [false, true] {
        for violation in 0..4 {
            let fixture = Fixture::new(anthropic);
            let mut transport = SyntheticTransport::new(&fixture.workspace, "after\n");
            match violation {
                0 => {
                    transport.action["action"]["parameters"]["path"] = json!("OTHER.md");
                    transport.action["action"]["path_scope"]["allowed_paths"] = json!(["OTHER.md"]);
                    transport.action["action"]["verification"]["parameters"]["path"] =
                        json!("OTHER.md");
                }
                1 => {
                    transport.action["action"]["verification"]["parameters"]["path"] =
                        json!("OTHER.md")
                }
                2 => transport.action["decision"] = json!("continue_observation"),
                _ => {
                    transport.action.as_object_mut().unwrap().remove("action");
                }
            }
            let sends = transport.sends.clone();
            assert_eq!(
                fixture.run(transport).unwrap_err().to_string(),
                EXECUTION_FAILED
            );
            assert_eq!(sends.get(), 1);
            assert_eq!(fixture.contents("README.md"), "before\n");
            assert_eq!(fixture.contents("OTHER.md"), "untouched\n");
        }
    }
}

#[test]
fn markdown_cli_request_human_with_or_without_action_never_mutates_or_completes() {
    for anthropic in [false, true] {
        for include_action in [false, true] {
            let fixture = Fixture::new(anthropic);
            let mut transport = SyntheticTransport::new(&fixture.workspace, "after\n");
            transport.action["decision"] = json!("request_human");
            if !include_action {
                transport.action.as_object_mut().unwrap().remove("action");
            }
            let sends = transport.sends.clone();
            assert_eq!(
                fixture.run(transport).unwrap_err().to_string(),
                EXECUTION_FAILED
            );
            assert_eq!(sends.get(), 1);
            assert_eq!(fixture.contents("README.md"), "before\n");
            assert_eq!(fixture.contents("OTHER.md"), "untouched\n");
            assert!(!fixture.config.checkpoints_path().exists());
            assert!(!fixture.config.side_effects_path().exists());
            assert!(!fixture.config.tool_calls_path().exists());
            let state = StateStore::load(fixture.config.state_path()).unwrap();
            assert_eq!(state.loop_run("loop").unwrap().status, LoopStatus::Failed);
            let events = EventLog::open(fixture.config.events_path())
                .unwrap()
                .read_all()
                .unwrap();
            assert!(!events
                .iter()
                .any(|event| event.event_type == "loop_completed"));
            let calls = ModelCallStore::open(fixture.config.model_calls_path()).unwrap();
            assert!(calls.read_all().unwrap().is_empty());
            assert_eq!(calls.read_failures().unwrap().len(), 1);
        }
    }
}

#[test]
fn markdown_cli_real_verification_and_projection_failures_preserve_edit_and_execution_error() {
    for anthropic in [false, true] {
        let fixture = Fixture::new(anthropic);
        let replacement = "[missing](missing.md)\n";
        let transport = SyntheticTransport::new(&fixture.workspace, replacement);
        let sends = transport.sends.clone();
        let error = run_with(fixture.args().into_iter(), |prepared, _, events, calls| {
            let result = execute(prepared, events, calls, transport, anthropic);
            assert!(result.is_err());
            assert_eq!(fixture.contents("README.md"), replacement);
            fs::create_dir(fixture.config.state_path()).unwrap();
            result
        })
        .unwrap_err();
        assert_eq!(error.to_string(), EXECUTION_FAILED);
        assert_eq!(sends.get(), 1);
        assert_eq!(fixture.contents("README.md"), replacement);
        assert!(fixture.config.state_path().is_dir());
        assert_eq!(
            fs::read_dir(fixture.config.state_path()).unwrap().count(),
            0
        );
        let calls = ModelCallStore::open(fixture.config.model_calls_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].verification_status, "failed");
        let events = EventLog::open(fixture.config.events_path())
            .unwrap()
            .read_all()
            .unwrap();
        assert!(events.iter().any(|event| event.event_type == "loop_failed"));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "loop_completed"));
    }
}

#[test]
fn markdown_cli_post_execution_path_replacement_blocks_projection_without_rollback() {
    use std::os::unix::fs::symlink;

    for anthropic in [false, true] {
        for replace_runtime in [false, true] {
            let fixture = Fixture::new(anthropic);
            let runtime = fixture.config.events_path().parent().unwrap().to_path_buf();
            let moved = fixture._directory.path().join("moved-original");
            let decoy = fixture._directory.path().join("replacement-runtime");
            let transport = SyntheticTransport::new(&fixture.workspace, "after\n");
            let sends = transport.sends.clone();
            let error = run_with(fixture.args().into_iter(), |prepared, _, events, calls| {
                let result = execute(prepared, events, calls, transport, anthropic);
                assert!(result.is_ok());
                assert_eq!(fixture.contents("README.md"), "after\n");
                assert!(!fixture.config.state_path().exists());
                if replace_runtime {
                    fs::rename(&runtime, &moved).unwrap();
                    fs::create_dir(&decoy).unwrap();
                    fs::write(decoy.join("state.json"), "untrusted sentinel\n").unwrap();
                    symlink(&decoy, &runtime).unwrap();
                } else {
                    fs::rename(&fixture.workspace, &moved).unwrap();
                    fs::create_dir(&fixture.workspace).unwrap();
                    fs::write(
                        fixture.workspace.join("README.md"),
                        "replacement workspace\n",
                    )
                    .unwrap();
                }
                result
            })
            .unwrap_err();
            assert_eq!(error.to_string(), PROJECTION_FAILED);
            assert_eq!(sends.get(), 1);
            if replace_runtime {
                assert_eq!(fixture.contents("README.md"), "after\n");
                assert_eq!(
                    fs::read_to_string(decoy.join("state.json")).unwrap(),
                    "untrusted sentinel\n"
                );
                assert_eq!(fs::read_dir(&decoy).unwrap().count(), 1);
                assert!(!moved
                    .join(fixture.config.state_path().file_name().unwrap())
                    .exists());
            } else {
                assert_eq!(
                    fs::read_to_string(moved.join("README.md")).unwrap(),
                    "after\n"
                );
                assert_eq!(fixture.contents("README.md"), "replacement workspace\n");
                assert!(!fixture.config.state_path().exists());
            }
        }
    }
}

#[test]
fn markdown_cli_projection_refusal_after_success_is_not_reported_as_success() {
    let fixture = Fixture::new(false);
    let error = run_with(fixture.args().into_iter(), |prepared, _, events, calls| {
        let result = execute(
            prepared,
            events,
            calls,
            SyntheticTransport::new(&fixture.workspace, "after\n"),
            false,
        );
        assert!(result.is_ok());
        fs::create_dir(fixture.config.state_path()).unwrap();
        result
    })
    .unwrap_err();
    assert_eq!(error.to_string(), PROJECTION_FAILED);
    assert_eq!(fixture.contents("README.md"), "after\n");
    assert!(fixture.config.state_path().is_dir());
    assert_eq!(
        fs::read_dir(fixture.config.state_path()).unwrap().count(),
        0
    );
}

#[test]
fn markdown_cli_preserves_execution_classification_when_projection_also_fails() {
    let fixture = Fixture::new(false);
    let error = run_with(fixture.args().into_iter(), |_, _, _, _| {
        fs::create_dir(fixture.config.state_path()).unwrap();
        Err(ProviderRunError::TransportBuild)
    })
    .unwrap_err();
    assert_eq!(error.to_string(), EXECUTION_FAILED);
    assert!(fixture.config.state_path().is_dir());
    assert_eq!(fixture.contents("README.md"), "before\n");
}

#[test]
fn markdown_cli_selects_credentials_without_resolving_environment() {
    let fixture = Fixture::new(false);
    let defaults = parse(fixture.args().into_iter(), true).unwrap();
    assert!(matches!(
        defaults.credentials,
        CliCredentialResolver::Unconfigured
    ));
    let mut args = fixture.args();
    args.extend(["--credential-source".into(), "env".into()]);
    let parsed = parse(args.clone().into_iter(), true).unwrap();
    assert!(matches!(
        parsed.credentials,
        CliCredentialResolver::Environment
    ));
    let visited = Cell::new(false);
    let error = run_with(args.into_iter(), |_, credentials, _, _| {
        assert!(matches!(credentials, CliCredentialResolver::Environment));
        visited.set(true);
        Err(ProviderRunError::TransportBuild)
    })
    .unwrap_err();
    assert!(visited.get());
    assert_eq!(error.to_string(), EXECUTION_FAILED);
}

#[test]
fn markdown_cli_parser_accepts_interleaved_flags_and_rejects_scope_escalation() {
    let fixture = Fixture::new(false);
    let mut positional = fixture.args();
    positional.truncate(8);
    let workspace = fixture.workspace.to_str().unwrap();
    let mut valid = positional.clone();
    valid.extend(
        [
            "--path",
            "README.md",
            "--credential-source",
            "env",
            "--authorize-markdown",
            "once",
            "--path",
            "OTHER.md",
            "--workspace",
            workspace,
        ]
        .map(String::from),
    );
    let parsed = parse(valid.into_iter(), true).unwrap();
    assert_eq!(parsed.paths, ["README.md", "OTHER.md"]);
    assert_eq!(parsed.workspace, fixture.workspace);
    assert!(matches!(
        parsed.credentials,
        CliCredentialResolver::Environment
    ));
    for suffix in [
        vec!["--credential-source", "env"],
        vec!["--authorize-markdown", "once"],
        vec!["--approved", "true"],
        vec!["--mode", "AutonomousLoop"],
        vec!["--max-iterations", "2"],
        vec!["--path", "README.md"],
        vec!["--workspace", workspace],
        vec!["--credential-source", "file"],
        vec!["--path", "../README.md"],
        vec!["--path", "/README.md"],
        vec!["--path", "a\\b.md"],
        vec!["--path", "a\nb.md"],
        vec!["--path", "a//b.md"],
        vec!["--path", "a.txt"],
        vec!["--path"],
    ] {
        let mut args = fixture.args();
        if suffix == ["--credential-source", "env"] {
            args.extend(["--credential-source".into(), "env".into()]);
        }
        args.extend(suffix.into_iter().map(String::from));
        assert!(parse(args.into_iter(), true).is_err());
    }
    for suffix in [vec![], vec!["--authorize-markdown", "always"]] {
        let mut args = positional.clone();
        args.extend(["--workspace", workspace, "--path", "README.md"].map(String::from));
        args.extend(suffix.into_iter().map(String::from));
        assert!(parse(args.into_iter(), true).is_err());
    }
    assert!(parse(fixture.args().into_iter(), false).is_err());
    let mut preflight = positional;
    preflight.extend(
        [
            "--workspace",
            workspace,
            "--path",
            "README.md",
            "--credential-source",
            "env",
        ]
        .map(String::from),
    );
    assert!(parse(preflight.into_iter(), false).is_err());
}

#[derive(Clone)]
struct RecordingSink(Rc<RefCell<Vec<ProgressEvent>>>);

impl ProgressSink for RecordingSink {
    fn report(&mut self, event: ProgressEvent) -> std::io::Result<()> {
        self.0.borrow_mut().push(event);
        Ok(())
    }
}

#[test]
fn markdown_cli_metadata_progress_discards_untrusted_identity_summary_and_counts() {
    let captured = Rc::new(RefCell::new(Vec::new()));
    let mut sink = MetadataProgress(RecordingSink(captured.clone()));
    sink.report(ProgressEvent {
        subject: ProgressSubject::Model,
        subject_id: "malicious-id\nendpoint-and-secret-marker".into(),
        status: ProgressStatus::Failed,
        summary: Some("malicious tool payload and model response".into()),
        progress: Some((123, 456)),
    })
    .unwrap();
    let events = captured.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, ProgressSubject::Model);
    assert_eq!(events[0].status, ProgressStatus::Failed);
    assert_eq!(events[0].subject_id, "bounded-markdown");
    assert_eq!(events[0].summary, None);
    assert_eq!(events[0].progress, None);
}
