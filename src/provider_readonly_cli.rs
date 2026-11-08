use crate::CliCredentialResolver;
use std::{
    error::Error,
    path::{Component, PathBuf},
};
use steerbox::event_log::EventLog;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_runner::{
    BoundedReadOnlyProviderRun, BoundedReadOnlyProviderRunOutcome, BoundedReadOnlyRunConfig,
    ForegroundProviderRun, ProviderRunError, ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::StreamReporter;

const INVALID: &str = "invalid provider-readonly arguments";

struct Arguments {
    config_path: String,
    request: ProviderRunRequest,
    run: BoundedReadOnlyRunConfig,
    credentials: CliCredentialResolver,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Arguments, &'static str> {
    let config_path = args.next().ok_or(INVALID)?;
    let request = ProviderRunRequest {
        project_id: args.next().ok_or(INVALID)?,
        goal_id: args.next().ok_or(INVALID)?,
        loop_run_id: args.next().ok_or(INVALID)?,
        task_run_id: args.next().ok_or(INVALID)?,
        model_id: args.next().ok_or(INVALID)?,
        objective: args.next().ok_or(INVALID)?,
        observation: args.next().ok_or(INVALID)?,
    };
    let mut workspace = None;
    let mut iterations = None;
    let mut paths = Vec::new();
    let mut credentials = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(INVALID)?;
        match flag.as_str() {
            "--workspace" if workspace.is_none() => {
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(INVALID);
                }
                workspace = Some(path);
            }
            "--max-iterations" if iterations.is_none() => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(INVALID);
                }
                let count = value.parse::<u32>().map_err(|_| INVALID)?;
                if !(1..=16).contains(&count) {
                    return Err(INVALID);
                }
                iterations = Some(count);
            }
            "--path" if paths.len() < 256 => {
                let relative = PathBuf::from(&value);
                let normalized: PathBuf = relative.components().collect();
                if value.is_empty()
                    || value.starts_with("--")
                    || !(value.ends_with(".md") || value.ends_with(".markdown"))
                    || !relative
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                    || normalized.as_os_str() != relative.as_os_str()
                    || paths.contains(&value)
                {
                    return Err(INVALID);
                }
                paths.push(value);
            }
            "--credential-source" if credentials.is_none() && value == "env" => {
                credentials = Some(CliCredentialResolver::Environment);
            }
            _ => return Err(INVALID),
        }
    }
    if paths.is_empty() {
        return Err(INVALID);
    }
    Ok(Arguments {
        config_path,
        request,
        run: BoundedReadOnlyRunConfig {
            workspace: workspace.ok_or(INVALID)?,
            allowed_paths: paths,
            max_iterations: iterations.ok_or(INVALID)?,
        },
        credentials: credentials.unwrap_or(CliCredentialResolver::Unconfigured),
    })
}

pub fn run(args: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    run_with(args, |prepared, credentials, events, calls| {
        prepared.run_with_resolver(
            credentials,
            events,
            calls,
            StreamReporter::new(std::io::stdout()),
        )
    })
}

fn run_with(
    args: impl Iterator<Item = String>,
    execute: impl FnOnce(
        BoundedReadOnlyProviderRun,
        CliCredentialResolver,
        EventLog,
        ModelCallStore,
    ) -> Result<BoundedReadOnlyProviderRunOutcome, ProviderRunError>,
) -> Result<String, Box<dyn Error>> {
    let args = parse(args)?;
    let config = RuntimeConfig::load(args.config_path)?;
    let prepared = ForegroundProviderRun::prepare_readonly(&config, args.request, args.run)?;
    let paths = prepared.validate_runtime_paths(
        &config.events_path(),
        &config.model_calls_path(),
        Some(&config.state_path()),
    )?;
    let events = EventLog::open(config.events_path())?;
    let calls = ModelCallStore::open(config.model_calls_path())?;
    let result = execute(prepared, args.credentials, events, calls);
    let projection = (|| -> Result<(), Box<dyn Error>> {
        paths.revalidate()?;
        let events = EventLog::open(config.events_path())?.read_all()?;
        let state = StateStore::rebuild(&events)?;
        paths.revalidate()?;
        state.persist(config.state_path())?;
        Ok(())
    })();
    let outcome = match (result, projection) {
        (Err(original), _) => return Err(original.into()),
        (Ok(_), Err(projection)) => return Err(projection),
        (Ok(outcome), Ok(())) => outcome,
    };
    Ok(format!(
        "decision={:?} stop_reason={:?} iterations={} loop_status={:?}",
        outcome.outcome.decision,
        outcome.outcome.stop_reason,
        outcome.outcome.iterations,
        outcome.loop_status
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::{cell::RefCell, collections::VecDeque, fs, time::Duration};
    use steerbox::anthropic_adapter::{
        AnthropicHeaders, AnthropicTransport, AnthropicTransportError,
    };
    use steerbox::domain::LoopStatus;
    use steerbox::execution_query::ExecutionQuery;
    use steerbox::http_transport::CredentialResolver;
    use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};

    struct Fixture {
        _directory: tempfile::TempDir,
        config: RuntimeConfig,
        workspace: PathBuf,
    }

    impl Fixture {
        fn new(provider: &str) -> Self {
            fs::create_dir_all("/temp/steerbox").unwrap();
            let directory = tempfile::Builder::new()
                .prefix("readonly-cli-unit-")
                .tempdir_in("/temp/steerbox")
                .unwrap();
            let workspace = directory.path().join("workspace");
            fs::create_dir(&workspace).unwrap();
            fs::write(workspace.join("README.md"), "# Fixture\n").unwrap();
            let config = RuntimeConfig::from_json_str(&json!({
                "schema_version":1,"project_ref":directory.path().file_name().unwrap().to_str().unwrap(),"data_root":directory.path(),
                "model_profiles":[{"model_id":"fixture-model","provider":provider,"endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:synthetic-cli","capabilities":["text"]}]
            }).to_string()).unwrap();
            config.write_json(config.config_path()).unwrap();
            fs::create_dir(config.events_path().parent().unwrap()).unwrap();
            Self {
                _directory: directory,
                config,
                workspace,
            }
        }

        fn args(&self) -> std::vec::IntoIter<String> {
            vec![
                self.config.config_path().display().to_string(),
                "project".into(),
                "goal".into(),
                "loop".into(),
                "task".into(),
                "fixture-model".into(),
                "Inspect".into(),
                "Observe".into(),
                "--workspace".into(),
                self.workspace.display().to_string(),
                "--max-iterations".into(),
                "2".into(),
                "--path".into(),
                "README.md".into(),
            ]
            .into_iter()
        }
    }

    struct SyntheticTransport {
        responses: RefCell<VecDeque<Value>>,
    }

    impl SyntheticTransport {
        fn new(decision: &str) -> Self {
            Self {
                responses: RefCell::new(VecDeque::from([
                    json!({"decision":decision,"summary":"Synthetic bounded CLI result"}),
                ])),
            }
        }

        fn action(workspace: &std::path::Path) -> Self {
            Self {
                responses: RefCell::new(VecDeque::from([
                    json!({"decision":"continue_observation","summary":"Inspect and verify","action":{
                        "schema_version":1,"action_id":"inspect-1","tool_id":"git_status","parameters":{"scope":"workspace"},
                        "path_scope":{"root":workspace,"allowed_paths":["README.md"]},
                        "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}
                    }}),
                    json!({"decision":"complete_loop","summary":"Synthetic completed inspection"}),
                ])),
            }
        }

        fn response(&self, anthropic: bool) -> TransportResponse {
            let content = self
                .responses
                .borrow_mut()
                .pop_front()
                .expect("bounded synthetic response")
                .to_string();
            let wire: Value = if anthropic {
                json!({"id":"synthetic-response","type":"message","role":"assistant","model":"fixture-model","content":[{"type":"text","text":content}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}})
            } else {
                json!({"id":"synthetic-response","model":"fixture-model","choices":[{"message":{"content":content},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
            };
            TransportResponse::body(200, wire.to_string())
        }
    }

    impl ProviderTransport for SyntheticTransport {
        fn send(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
            _: Duration,
            streaming: bool,
        ) -> Result<TransportResponse, TransportError> {
            assert!(!streaming);
            Ok(self.response(false))
        }
    }

    impl AnthropicTransport for SyntheticTransport {
        fn send(
            &self,
            _: &str,
            _: &AnthropicHeaders,
            _: &str,
            _: Duration,
            streaming: bool,
        ) -> Result<TransportResponse, AnthropicTransportError> {
            assert!(!streaming);
            Ok(self.response(true))
        }
    }

    #[test]
    fn readonly_cli_parser_rejects_invalid_scope_and_all_flag_boundary_cases() {
        let positional = [
            "missing-config",
            "project",
            "goal",
            "loop",
            "task",
            "fixture-model",
            "Inspect",
            "Observe",
        ];
        let valid = [
            "--workspace",
            "/tmp",
            "--max-iterations",
            "16",
            "--path",
            "README.md",
        ];
        for invalid in [
            vec![],
            vec!["--workspace", "/tmp", "--path", "README.md"],
            vec!["--max-iterations", "1", "--path", "README.md"],
            vec!["--workspace", "/tmp", "--max-iterations", "1"],
            vec![
                "--workspace",
                "relative",
                "--max-iterations",
                "1",
                "--path",
                "README.md",
            ],
            vec![
                "--workspace",
                "/tmp",
                "--max-iterations",
                "+1",
                "--path",
                "README.md",
            ],
            vec![
                "--workspace",
                "/tmp",
                "--max-iterations",
                "1",
                "--path",
                "../README.md",
            ],
            vec![
                "--workspace",
                "/tmp",
                "--max-iterations",
                "1",
                "--path",
                "/README.md",
            ],
            vec![
                "--workspace",
                "/tmp",
                "--max-iterations",
                "1",
                "--path",
                "README.txt",
            ],
            vec![
                "--workspace",
                "/tmp",
                "--max-iterations",
                "1",
                "--path",
                "./README.md",
            ],
        ] {
            assert!(parse(positional.into_iter().chain(invalid).map(str::to_owned)).is_err());
        }
        for extra in [
            vec!["--max-iterations", "2"],
            vec!["--credential-source", "env", "--credential-source", "env"],
            vec!["--path", "README.md"],
            vec!["--credential-source", "env", "trailing-input"],
        ] {
            assert!(parse(
                positional
                    .into_iter()
                    .chain(valid)
                    .chain(extra)
                    .map(str::to_owned)
            )
            .is_err());
        }
        let mut flags: Vec<String> = positional
            .into_iter()
            .chain(["--workspace", "/tmp", "--max-iterations", "1"])
            .map(str::to_owned)
            .collect();
        for index in 0..256 {
            flags.extend(["--path".into(), format!("file-{index}.md")]);
        }
        assert_eq!(
            parse(flags.clone().into_iter())
                .unwrap()
                .run
                .allowed_paths
                .len(),
            256
        );
        flags.extend(["--path".into(), "overflow.md".into()]);
        assert!(parse(flags.into_iter()).is_err());
        let parsed = parse(
            positional
                .into_iter()
                .chain(valid)
                .chain(["--credential-source", "env"])
                .map(str::to_owned),
        )
        .unwrap();
        assert!(matches!(
            parsed.credentials,
            CliCredentialResolver::Environment
        ));
    }

    #[test]
    fn readonly_cli_both_protocols_preserve_actual_stops_and_correlated_projection() {
        for provider in ["fixture", "anthropic"] {
            for (decision, expected, status) in [
                ("complete_loop", "CompleteLoop", LoopStatus::Completed),
                ("request_human", "RequestHuman", LoopStatus::Running),
                (
                    "continue_observation",
                    "ContinueObservationWithoutAction",
                    LoopStatus::Running,
                ),
            ] {
                let fixture = Fixture::new(provider);
                let summary = run_with(fixture.args(), |prepared, credentials, events, calls| {
                    assert!(credentials.resolve("secret-ref:synthetic-cli").is_err());
                    let transport = SyntheticTransport::new(decision);
                    if provider == "anthropic" {
                        prepared.run_with_anthropic_transport(
                            transport,
                            events,
                            calls,
                            StreamReporter::new(Vec::new()),
                        )
                    } else {
                        prepared.run_with_transport(
                            transport,
                            events,
                            calls,
                            StreamReporter::new(Vec::new()),
                        )
                    }
                })
                .unwrap();
                assert!(
                    summary.contains(&format!("stop_reason={expected}")),
                    "{summary}"
                );
                assert!(summary.contains("iterations=1"));
                assert!(summary.contains(&format!("loop_status={status:?}")));
                let state = StateStore::load(fixture.config.state_path()).unwrap();
                let loop_run = state.loop_run("loop").unwrap();
                assert_eq!(loop_run.task_run_id, "task");
                assert_eq!(loop_run.goal_id, "goal");
                assert_eq!(loop_run.status, status);
                let events = EventLog::open(fixture.config.events_path())
                    .unwrap()
                    .read_all()
                    .unwrap();
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
                assert_eq!(calls[0].task_run_id, "task");
                assert_eq!(calls[0].model_id, "fixture-model");
                assert!(events
                    .iter()
                    .any(|event| event.event_type == "model_call_completed"
                        && event.payload["model_call_id"] == calls[0].model_call_id));
                let query = ExecutionQuery::from_events(&events).unwrap();
                assert_eq!(query.list().len(), 1);
                assert_eq!(
                    fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
                    "# Fixture\n"
                );
            }
        }
    }

    #[test]
    fn readonly_cli_preserves_execution_error_when_projection_recheck_also_fails() {
        let fixture = Fixture::new("fixture");
        let error = run_with(fixture.args(), |_, _, _, _| {
            fs::create_dir(fixture.config.state_path()).unwrap();
            Err(ProviderRunError::TransportBuild)
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            ProviderRunError::TransportBuild.to_string()
        );
        assert!(fixture.config.state_path().is_dir());
        assert!(fs::read_dir(fixture.config.state_path())
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn readonly_cli_rechecks_projection_after_success_before_persisting() {
        let fixture = Fixture::new("fixture");
        let error = run_with(fixture.args(), |prepared, _, events, calls| {
            let result = prepared.run_with_transport(
                SyntheticTransport::new("complete_loop"),
                events,
                calls,
                StreamReporter::new(Vec::new()),
            );
            std::os::unix::fs::symlink(
                fixture.workspace.join("README.md"),
                fixture.config.state_path(),
            )
            .unwrap();
            result
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            ProviderRunError::InvalidRequest.to_string()
        );
        assert_eq!(
            fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
            "# Fixture\n"
        );
    }

    #[test]
    fn readonly_cli_two_protocol_action_verification_and_completion_share_identity_and_budget() {
        for provider in ["fixture", "anthropic"] {
            let fixture = Fixture::new(provider);
            assert!(std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&fixture.workspace)
                .status()
                .unwrap()
                .success());
            let summary = run_with(fixture.args(), |prepared, _, events, calls| {
                let transport = SyntheticTransport::action(&fixture.workspace);
                let outcome = if provider == "anthropic" {
                    prepared.run_with_anthropic_transport(
                        transport,
                        events,
                        calls,
                        StreamReporter::new(Vec::new()),
                    )
                } else {
                    prepared.run_with_transport(
                        transport,
                        events,
                        calls,
                        StreamReporter::new(Vec::new()),
                    )
                }?;
                assert_eq!(outcome.budget().model_calls_used, 2);
                assert_eq!(outcome.budget().tool_calls_used, 2);
                Ok(outcome)
            })
            .unwrap();
            assert_eq!(
                summary,
                "decision=CompleteLoop stop_reason=CompleteLoop iterations=2 loop_status=Completed"
            );
            let state = StateStore::load(fixture.config.state_path()).unwrap();
            assert_eq!(state.loop_run("loop").unwrap().task_run_id, "task");
            assert_eq!(state.steps().count(), 2);
            let events = EventLog::open(fixture.config.events_path())
                .unwrap()
                .read_all()
                .unwrap();
            let query = ExecutionQuery::from_events(&events).unwrap();
            assert_eq!(query.list().len(), 1);
            let calls = ModelCallStore::open(fixture.config.model_calls_path())
                .unwrap()
                .read_all()
                .unwrap();
            assert_eq!(calls.len(), 2);
            assert_ne!(
                calls[0].resolved_prompt_pack_id,
                calls[1].resolved_prompt_pack_id
            );
            assert_ne!(calls[0].model_call_id, calls[1].model_call_id);
            for call in &calls {
                assert_eq!(call.task_run_id, "task");
                assert_eq!(call.model_id, "fixture-model");
                assert!(events
                    .iter()
                    .any(|event| event.event_type == "model_call_completed"
                        && event.payload["model_call_id"] == call.model_call_id));
            }
            let tools = steerbox::tool_call_store::ToolCallRecordStore::open(
                fixture.config.tool_calls_path(),
            )
            .unwrap()
            .read_page(0, 10)
            .unwrap();
            assert_eq!(tools.len(), 4);
            assert_eq!(
                tools
                    .iter()
                    .filter(|tool| tool.status == "completed")
                    .count(),
                2
            );
            for tool in tools {
                assert_eq!(tool.project_id, "project");
                assert_eq!(tool.goal_id, "goal");
                assert_eq!(tool.loop_run_id, "loop");
                assert_eq!(tool.task_run_id, "task");
                let owner_step = tool
                    .step_id
                    .strip_suffix(":verification")
                    .unwrap_or(&tool.step_id);
                assert!(state.step("loop", owner_step).is_some());
                assert!(events
                    .iter()
                    .any(|event| event.step_id.as_deref() == Some(tool.step_id.as_str())));
            }
            assert_eq!(
                fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
                "# Fixture\n"
            );
        }
    }
}
