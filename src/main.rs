use serde_json::Value;
use std::env;
use std::error::Error;
use std::process;
use steerbox::audit_store::{
    CheckpointMetadata, CheckpointStore, RunSummary, SideEffectLedger, SideEffectRecord,
    TraceRecord, TraceStore,
};
use steerbox::board_projection::BoardProjection;
use steerbox::domain::{
    EventEnvelope, GoalContract, HumanSteeringMode, LoopBudget, LoopStatus, RiskLevel,
};
use steerbox::event_log::EventLog;
use steerbox::execution_query::ExecutionQuery;
use steerbox::http_transport::{
    CredentialError, CredentialResolver, EnvironmentCredentialResolver, ResolvedCredential,
};
use steerbox::loop_controller::ReadOnlyLoopController;
use steerbox::model_adapter::{LocalFixtureModelAdapter, ModelDecision};
use steerbox::model_call_store::ModelCallStore;
use steerbox::policy_gate::{PolicyDecision, PolicyGate, PolicyRequest};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunRequest};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::StreamReporter;
use steerbox::tool_registry::{SideEffectClass, ToolProfile, ToolRegistry};
use steerbox::user_command_inbox::{UserCommand, UserCommandEntry, UserCommandInbox};
use steerbox::workspace_edit::{edit_markdown, verify_markdown_links};

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("steerbox: {error}");
        eprintln!();
        print_usage();
        process::exit(1);
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match (args.next().as_deref(), args.next().as_deref()) {
        (Some("runtime"), Some("init")) => {
            let project_ref = args.next().ok_or("missing project_ref")?;
            let config = RuntimeConfig::default_for_project(project_ref)?;
            if config.config_path().exists() {
                return Err(format!(
                    "configuration already exists: {}",
                    config.config_path().display()
                )
                .into());
            }
            config.write_json(config.config_path())?;
            let log = EventLog::open(config.events_path())?;
            println!(
                "initialized project={} data_root={}",
                config.project_ref,
                config.data_root.display()
            );
            println!("config={}", config.config_path().display());
            println!("event_log={}", log.path().display());
        }
        (Some("config"), Some("validate")) => {
            let path = args.next().ok_or("missing config path")?;
            let config = RuntimeConfig::load(path)?;
            println!(
                "valid project={} data_root={} model_profiles={}",
                config.project_ref,
                config.data_root.display(),
                config.model_profiles.len()
            );
        }
        (Some("tool"), Some("list")) => {
            let registry = ToolRegistry::baseline()?;
            println!("工具能力\n{}", registry.render_capability_table());
            println!(
                "\n运行说明\n- ✅ 启用表示允许路由选择，不代表自动批准。\n- 按需能力不会因展示而启动；详细调用记录进入 EventLog。"
            );
        }
        (Some("event"), Some("list")) => {
            let path = args.next().ok_or("missing config path")?;
            let offset = args
                .next()
                .map(|value| {
                    value
                        .parse::<usize>()
                        .map_err(|error| format!("invalid offset: {error}"))
                })
                .transpose()?;
            let limit = args
                .next()
                .map(|value| {
                    value
                        .parse::<usize>()
                        .map_err(|error| format!("invalid limit: {error}"))
                })
                .transpose()?;
            if offset.is_some() != limit.is_some() {
                return Err("event list requires both offset and limit".into());
            }
            let config = RuntimeConfig::load(path)?;
            let log = EventLog::open(config.events_path())?;
            let (events, header) = match (offset, limit) {
                (Some(offset), Some(limit)) => (
                    log.read_page(offset, limit)?,
                    format!("offset={offset} limit={limit} "),
                ),
                (None, None) => (log.read_all()?, String::new()),
                _ => unreachable!("offset and limit are checked together"),
            };
            println!("{header}events={}", events.len());
            for event in events {
                println!(
                    "sequence={} type={} aggregate={}:{}",
                    event.sequence_no.unwrap_or_default(),
                    event.event_type,
                    event.aggregate_type,
                    event.aggregate_id
                );
            }
        }
        (Some("event"), Some("append")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let event_type = args.next().ok_or("missing event_type")?;
            let aggregate_type = args.next().ok_or("missing aggregate_type")?;
            let aggregate_id = args.next().ok_or("missing aggregate_id")?;
            let event_id = args.next().ok_or("missing event_id")?;
            let correlation_id = args.next().ok_or("missing correlation_id")?;
            let payload_text = args.next().unwrap_or_else(|| "{}".into());
            let payload = serde_json::from_str::<Value>(&payload_text)
                .map_err(|error| format!("invalid payload JSON: {error}"))?;
            let config = RuntimeConfig::load(config_path)?;
            let mut log = EventLog::open(config.events_path())?;
            let event = EventEnvelope::new(
                aggregate_type,
                event_id,
                event_type,
                aggregate_id,
                correlation_id,
                payload,
            )?;
            let sequence = log.append(event)?;
            println!("appended sequence={sequence}");
        }
        (Some("execution"), Some("list")) => {
            let path = args.next().ok_or("missing config path")?;
            let config = RuntimeConfig::load(path)?;
            let log = EventLog::open(config.events_path())?;
            let query = ExecutionQuery::from_event_log(&log)?;
            println!("{}", serde_json::to_string_pretty(&query.list())?);
        }
        (Some("execution"), Some("show")) => {
            let path = args.next().ok_or("missing config path")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let config = RuntimeConfig::load(path)?;
            let log = EventLog::open(config.events_path())?;
            let query = ExecutionQuery::from_event_log(&log)?;
            let snapshot = query
                .surface_snapshot(&loop_run_id)
                .ok_or_else(|| format!("execution not found: {loop_run_id}"))?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        (Some("execution"), Some("page")) => {
            let path = args.next().ok_or("missing config path")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let offset = args
                .next()
                .ok_or("missing offset")?
                .parse::<usize>()
                .map_err(|error| format!("invalid offset: {error}"))?;
            let limit = args
                .next()
                .ok_or("missing limit")?
                .parse::<usize>()
                .map_err(|error| format!("invalid limit: {error}"))?;
            // Pagination addresses this detached observation, not a live cursor or stream.
            let config = RuntimeConfig::load(path)?;
            let log = EventLog::open(config.events_path())?;
            let query = ExecutionQuery::from_event_log(&log)?;
            let page = query
                .events_page(&loop_run_id, offset, limit)
                .ok_or_else(|| format!("execution not found: {loop_run_id}"))?;
            println!("{}", serde_json::to_string_pretty(&page)?);
        }
        (Some("model-call"), Some("page")) => {
            let path = args.next().ok_or("missing config path")?;
            let offset = args
                .next()
                .ok_or("missing offset")?
                .parse::<usize>()
                .map_err(|error| format!("invalid offset: {error}"))?;
            let limit = args
                .next()
                .ok_or("missing limit")?
                .parse::<usize>()
                .map_err(|error| format!("invalid limit: {error}"))?;
            let config = RuntimeConfig::load(path)?;
            let store = ModelCallStore::open(config.model_calls_path())?;
            let model_calls = store.read_page(offset, limit)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "offset": offset,
                    "limit": limit,
                    "count": model_calls.len(),
                    "model_calls": model_calls,
                }))?
            );
        }
        (Some("command"), Some("submit")) => {
            let path = args.next().ok_or("missing config path")?;
            let command_json = args.next().ok_or("missing command JSON")?;
            let config = RuntimeConfig::load(path)?;
            let command = serde_json::from_str::<UserCommand>(&command_json)
                .map_err(|error| format!("invalid command JSON: {error}"))?;
            let inbox = UserCommandInbox::open(config.commands_path());
            let entry = inbox.submit(command)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&command_metadata(&entry))?
            );
        }
        (Some("command"), Some("show")) => {
            let path = args.next().ok_or("missing config path")?;
            let command_id = args.next().ok_or("missing command_id")?;
            let config = RuntimeConfig::load(path)?;
            let inbox = UserCommandInbox::open(config.commands_path());
            let entry = inbox.latest(&command_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&command_metadata(&entry))?
            );
        }
        (Some("board"), Some("list")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let config = RuntimeConfig::load(config_path)?;
            let events = EventLog::open(config.events_path())?.read_all()?;
            let projection = BoardProjection::rebuild(&events)?;
            println!("{}", serde_json::to_string_pretty(&projection)?);
        }
        (Some("loop"), Some("readonly")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let project_id = args.next().ok_or("missing project_id")?;
            let goal_id = args.next().ok_or("missing goal_id")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let task_run_id = args.next().ok_or("missing task_run_id")?;
            let step_id = args.next().ok_or("missing step_id")?;
            let objective = args.next().ok_or("missing objective")?;
            let observation = args.next().ok_or("missing observation")?;
            let config = RuntimeConfig::load(config_path)?;
            let log = EventLog::open(config.events_path())?;
            let event_project_id = project_id.clone();
            let goal = GoalContract::new(
                goal_id,
                project_id,
                objective,
                vec![".".into()],
                Vec::new(),
                vec!["manual_review".into()],
            );
            let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
            let mut controller = ReadOnlyLoopController::start_with_progress_sink(
                log,
                goal,
                loop_run_id,
                task_run_id,
                budget,
                StreamReporter::new(std::io::stdout()),
            )?;
            let step = controller.run_observation(step_id, observation)?;
            let run = controller.complete()?;
            let events = EventLog::open(config.events_path())?.read_all()?;
            let state = StateStore::rebuild(&events)?;
            state.persist(config.state_path())?;
            let traces = TraceStore::open(config.traces_path());
            traces.append(&TraceRecord {
                trace_id: format!("{}:step:{}", run.loop_run_id, step.step_id),
                goal_id: Some(run.goal_id.clone()),
                loop_run_id: Some(run.loop_run_id.clone()),
                step_id: Some(step.step_id.clone()),
                kind: "step_summary".into(),
                summary: step.summary.clone().unwrap_or_default(),
                occurred_at: step
                    .finished_at
                    .clone()
                    .unwrap_or_else(|| run.started_at.clone()),
            })?;
            traces.append(&TraceRecord {
                trace_id: format!("{}:completed", run.loop_run_id),
                goal_id: Some(run.goal_id.clone()),
                loop_run_id: Some(run.loop_run_id.clone()),
                step_id: None,
                kind: "loop_summary".into(),
                summary: "readonly loop completed".into(),
                occurred_at: run
                    .finished_at
                    .clone()
                    .unwrap_or_else(|| run.started_at.clone()),
            })?;
            let checkpoint_id = format!("{}:checkpoint:completed", run.loop_run_id);
            CheckpointStore::open(config.checkpoints_path()).append(&CheckpointMetadata {
                checkpoint_id: checkpoint_id.clone(),
                goal_id: run.goal_id.clone(),
                loop_run_id: run.loop_run_id.clone(),
                step_id: Some(step.step_id.clone()),
                files: Vec::new(),
                diff_boundary: "no_workspace_write_readonly_loop".into(),
                recoverability: "event_and_state_replayable".into(),
                side_effect_refs: Vec::new(),
                created_at: run
                    .finished_at
                    .clone()
                    .unwrap_or_else(|| run.started_at.clone()),
            })?;
            let mut checkpoint_log = EventLog::open(config.events_path())?;
            let mut checkpoint_event = EventEnvelope::new(
                "checkpoint",
                checkpoint_id,
                "checkpoint_created",
                &run.loop_run_id,
                &run.loop_run_id,
                serde_json::json!({
                    "step_id": step.step_id,
                    "diff_boundary": "no_workspace_write_readonly_loop",
                    "recoverability": "event_and_state_replayable"
                }),
            )?;
            checkpoint_event.project_id = Some(event_project_id);
            checkpoint_event.goal_id = Some(run.goal_id.clone());
            checkpoint_event.loop_run_id = Some(run.loop_run_id.clone());
            checkpoint_log.append(checkpoint_event)?;
            println!(
                "loop={} status={:?} step={} step_status={:?}",
                run.loop_run_id, run.status, step.step_id, step.status
            );
        }
        (Some("loop"), Some("fixture")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let project_id = args.next().ok_or("missing project_id")?;
            let goal_id = args.next().ok_or("missing goal_id")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let task_run_id = args.next().ok_or("missing task_run_id")?;
            let model_id = args.next().ok_or("missing model_id")?;
            let decision = parse_model_decision(args.next().as_deref().unwrap_or("complete_loop"))?;
            let observation = args.next().ok_or("missing observation")?;
            let summary = args
                .next()
                .unwrap_or_else(|| "local fixture response".into());
            let config = RuntimeConfig::load(config_path)?;
            let log = EventLog::open(config.events_path())?;
            let trace_goal_id = goal_id.clone();
            let trace_loop_run_id = loop_run_id.clone();
            let goal = GoalContract::new(
                goal_id,
                project_id.clone(),
                "fixture model observation",
                vec![".".into()],
                Vec::new(),
                vec!["manual_review".into()],
            );
            let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
            let mut controller = ReadOnlyLoopController::start_with_progress_sink(
                log,
                goal,
                loop_run_id,
                task_run_id,
                budget,
                StreamReporter::new(std::io::stdout()),
            )?;
            let adapter = LocalFixtureModelAdapter::new(decision, summary);
            let actual =
                controller.run_model_step(&adapter, model_id.clone(), "step-1", observation)?;
            let events = EventLog::open(config.events_path())?.read_all()?;
            let state = StateStore::rebuild(&events)?;
            state.persist(config.state_path())?;
            TraceStore::open(config.traces_path()).append(&TraceRecord {
                trace_id: format!("{}:model:step-1", trace_loop_run_id),
                goal_id: Some(trace_goal_id.clone()),
                loop_run_id: Some(trace_loop_run_id.clone()),
                step_id: Some("step-1".into()),
                kind: "model_decision".into(),
                summary: format!("fixture decision: {actual:?}"),
                occurred_at: chrono::Utc::now().to_rfc3339(),
            })?;
            let checkpoint_id = format!("{}:checkpoint:fixture", trace_loop_run_id);
            CheckpointStore::open(config.checkpoints_path()).append(&CheckpointMetadata {
                checkpoint_id: checkpoint_id.clone(),
                goal_id: trace_goal_id.clone(),
                loop_run_id: trace_loop_run_id.clone(),
                step_id: Some("step-1".into()),
                files: Vec::new(),
                diff_boundary: "no_workspace_write_fixture_loop".into(),
                recoverability: "event_and_state_replayable".into(),
                side_effect_refs: Vec::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
            let mut checkpoint_log = EventLog::open(config.events_path())?;
            let mut checkpoint_event = EventEnvelope::new(
                "checkpoint",
                checkpoint_id,
                "checkpoint_created",
                &trace_loop_run_id,
                &trace_loop_run_id,
                serde_json::json!({
                    "step_id": "step-1",
                    "diff_boundary": "no_workspace_write_fixture_loop",
                    "recoverability": "event_and_state_replayable"
                }),
            )?;
            checkpoint_event.project_id = Some(project_id);
            checkpoint_event.goal_id = Some(trace_goal_id);
            checkpoint_event.loop_run_id = Some(trace_loop_run_id);
            checkpoint_log.append(checkpoint_event)?;
            println!(
                "decision={actual:?} loop_status={:?}",
                controller_status(&controller)
            );
        }
        (Some("loop"), Some("provider")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let request = ProviderRunRequest {
                project_id: args.next().ok_or("missing project_id")?,
                goal_id: args.next().ok_or("missing goal_id")?,
                loop_run_id: args.next().ok_or("missing loop_run_id")?,
                task_run_id: args.next().ok_or("missing task_run_id")?,
                model_id: args.next().ok_or("missing model_id")?,
                objective: args.next().ok_or("missing objective")?,
                observation: args.next().ok_or("missing observation")?,
            };
            let credential_source = match args.next() {
                None => CliCredentialResolver::Unconfigured,
                Some(flag) if flag == "--credential-source" => match args.next().as_deref() {
                    Some("env") => CliCredentialResolver::Environment,
                    Some(_) => {
                        return Err(
                            "credential source must be `env`; inline credentials are not accepted"
                                .into(),
                        )
                    }
                    None => return Err("missing credential source after --credential-source".into()),
                },
                Some(_) => {
                    return Err(
                        "unexpected provider argument; use --credential-source env; inline credentials are not accepted".into(),
                    )
                }
            };
            if args.next().is_some() {
                return Err("unexpected provider argument after credential source".into());
            }
            let config = RuntimeConfig::load(config_path)?;
            let prepared = ForegroundProviderRun::prepare(&config, request)?;
            let result = prepared.run_with_resolver(
                credential_source,
                EventLog::open(config.events_path())?,
                ModelCallStore::open(config.model_calls_path())?,
                StreamReporter::new(std::io::stdout()),
            );
            let events = EventLog::open(config.events_path())?.read_all()?;
            StateStore::rebuild(&events)?.persist(config.state_path())?;
            let outcome = result?;
            println!(
                "decision={:?} loop_status={:?}",
                outcome.decision, outcome.loop_status
            );
        }
        (Some("dev"), Some("markdown")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let project_id = args.next().ok_or("missing project_id")?;
            let goal_id = args.next().ok_or("missing goal_id")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let task_run_id = args.next().ok_or("missing task_run_id")?;
            let step_id = args.next().ok_or("missing step_id")?;
            let relative_path = args.next().ok_or("missing relative markdown path")?;
            let expected = args.next().ok_or("missing expected content")?;
            let replacement = args.next().ok_or("missing replacement content")?;
            let interaction_mode = args
                .next()
                .ok_or("missing interaction mode (auto|ask|approved)")?;
            let (human_steering_mode, approved) = match interaction_mode.as_str() {
                "auto" => (HumanSteeringMode::AutonomousLoop, true),
                "approved" => (HumanSteeringMode::HumanSteerableLoop, true),
                "ask" => {
                    return Err("ask mode requires an interactive approval resolver; use auto for bounded Markdown edits or approved for an explicit approval marker".into());
                }
                _ => {
                    return Err(
                        "dev markdown interaction mode must be one of: auto, ask, approved".into(),
                    );
                }
            };
            let config = RuntimeConfig::load(config_path)?;
            let mut log = EventLog::open(config.events_path())?;
            let mut goal = GoalContract::new(
                goal_id.clone(),
                project_id.clone(),
                "update a Markdown document and verify links",
                vec![".".into()],
                vec![".steerbox".into()],
                vec!["markdown_link_check".into()],
            );
            goal.allowed_tools = vec!["write_file".into()];
            append_runtime_event(
                &mut log,
                "loop_run",
                &loop_run_id,
                "loop_started",
                &project_id,
                &goal_id,
                &loop_run_id,
                None,
                serde_json::json!({"status":"running","task_run_id":task_run_id.clone()}),
            )?;
            append_runtime_event(
                &mut log,
                "loop_step",
                &step_id,
                "step_started",
                &project_id,
                &goal_id,
                &loop_run_id,
                Some(&step_id),
                serde_json::json!({"phase":"act","step_no":1}),
            )?;

            let mut registry = ToolRegistry::new();
            registry.register(ToolProfile {
                tool_id: "write_file".into(),
                name: "Markdown file write".into(),
                description: "approved local Markdown edit".into(),
                input_schema: serde_json::json!({"path":"string","content":"string"}),
                output_schema: serde_json::json!({"changed":"boolean"}),
                side_effect_class: SideEffectClass::WorkspaceWrite,
                risk_level: if human_steering_mode == HumanSteeringMode::AutonomousLoop {
                    RiskLevel::Medium
                } else {
                    RiskLevel::High
                },
                allowed_in_modes: vec![human_steering_mode],
                recipe_ref: Some(
                    if human_steering_mode == HumanSteeringMode::AutonomousLoop {
                        "markdown-edit-workspace"
                    } else {
                        "file-write-verification"
                    }
                    .into(),
                ),
                version: 1,
            })?;
            let request = PolicyRequest {
                policy_event_id: format!("{step_id}:policy"),
                tool_id: "write_file".into(),
                project_id: project_id.clone(),
                goal_id: goal_id.clone(),
                task_run_id: task_run_id.clone(),
                loop_run_id: loop_run_id.clone(),
                step_id: step_id.clone(),
                correlation_id: loop_run_id.clone(),
                human_steering_mode,
            };
            let policy = PolicyGate::new();
            let decision = policy.evaluate(&registry, &goal, &request);
            policy.record_decision(&mut log, &request, &decision)?;
            let expected_decision = if human_steering_mode == HumanSteeringMode::AutonomousLoop {
                PolicyDecision::Allow
            } else {
                PolicyDecision::RequireHumanApproval
            };
            if decision.decision != expected_decision {
                return Err(format!("write policy denied: {:?}", decision.decision).into());
            }

            let checkpoint_id = format!("{step_id}:checkpoint:before-write");
            CheckpointStore::open(config.checkpoints_path()).append(&CheckpointMetadata {
                checkpoint_id: checkpoint_id.clone(),
                goal_id: goal_id.clone(),
                loop_run_id: loop_run_id.clone(),
                step_id: Some(step_id.clone()),
                files: vec![relative_path.clone()],
                diff_boundary: format!("{relative_path}:before-write"),
                recoverability: "original-content-captured-by-expected-input".into(),
                side_effect_refs: vec![format!("{step_id}:side-effect")],
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
            append_runtime_event(
                &mut log,
                "checkpoint",
                &checkpoint_id,
                "checkpoint_created",
                &project_id,
                &goal_id,
                &loop_run_id,
                Some(&step_id),
                serde_json::json!({"files":[relative_path],"recoverability":"original-content-captured-by-expected-input"}),
            )?;

            let edit = edit_markdown(
                &config.data_root,
                &goal,
                &relative_path,
                &expected,
                &replacement,
                approved,
            )?;
            let side_effect_id = format!("{step_id}:side-effect");
            SideEffectLedger::open(config.side_effects_path()).append(&SideEffectRecord {
                side_effect_id: side_effect_id.clone(),
                goal_id: goal_id.clone(),
                loop_run_id: loop_run_id.clone(),
                tool_id: "write_file".into(),
                class: "reversible_local_change".into(),
                status: "completed".into(),
                summary: format!("{} changed={}", edit.relative_path, edit.changed),
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
            append_runtime_event(
                &mut log,
                "workspace",
                &relative_path,
                "file_change_recorded",
                &project_id,
                &goal_id,
                &loop_run_id,
                Some(&step_id),
                serde_json::json!({"path":edit.relative_path,"changed":edit.changed,"bytes_before":edit.bytes_before,"bytes_after":edit.bytes_after}),
            )?;
            let verification = verify_markdown_links(&config.data_root, &relative_path)?;
            append_runtime_event(
                &mut log,
                "verification",
                &relative_path,
                "verification_completed",
                &project_id,
                &goal_id,
                &loop_run_id,
                Some(&step_id),
                serde_json::json!({"check_type":"markdown_links","status":if verification.passed {"passed"} else {"failed"},"missing_links":verification.missing_links}),
            )?;
            let verification_status = if verification.passed {
                "passed"
            } else {
                "failed"
            };
            append_runtime_event(
                &mut log,
                "loop_step",
                &step_id,
                if verification.passed {
                    "step_completed"
                } else {
                    "step_failed"
                },
                &project_id,
                &goal_id,
                &loop_run_id,
                Some(&step_id),
                serde_json::json!({"phase":"verify","verification":verification_status}),
            )?;
            let final_status = if verification.passed {
                "completed"
            } else {
                "failed"
            };
            append_runtime_event(
                &mut log,
                "loop_run",
                &loop_run_id,
                if verification.passed {
                    "loop_completed"
                } else {
                    "loop_failed"
                },
                &project_id,
                &goal_id,
                &loop_run_id,
                None,
                serde_json::json!({"status":final_status,"verification":verification_status}),
            )?;
            let events = EventLog::open(config.events_path())?.read_all()?;
            StateStore::rebuild(&events)?.persist(config.state_path())?;
            let run_events: Vec<_> = events
                .iter()
                .filter(|event| event.loop_run_id.as_deref() == Some(loop_run_id.as_str()))
                .collect();
            let trace_id = format!("{step_id}:verification");
            TraceStore::open(config.traces_path()).append(&TraceRecord {
                trace_id: trace_id.clone(),
                goal_id: Some(goal_id.clone()),
                loop_run_id: Some(loop_run_id.clone()),
                step_id: Some(step_id.clone()),
                kind: "verification".into(),
                summary: format!(
                    "markdown links {}",
                    if verification.passed {
                        "passed"
                    } else {
                        "failed"
                    }
                ),
                occurred_at: chrono::Utc::now().to_rfc3339(),
            })?;
            RunSummary {
                schema_version: 1,
                project_id: project_id.clone(),
                goal_id: goal_id.clone(),
                loop_run_id: loop_run_id.clone(),
                task_run_id: task_run_id.clone(),
                final_status: final_status.into(),
                step_ids: vec![step_id.clone()],
                file_changes: if edit.changed {
                    vec![relative_path.clone()]
                } else {
                    Vec::new()
                },
                verification_status: verification_status.into(),
                event_ids: run_events
                    .iter()
                    .map(|event| event.event_id.clone())
                    .collect(),
                verification_event_ids: run_events
                    .iter()
                    .filter(|event| event.event_type == "verification_completed")
                    .map(|event| event.event_id.clone())
                    .collect(),
                trace_ids: vec![trace_id],
                checkpoint_ids: vec![checkpoint_id],
                side_effect_ids: vec![side_effect_id],
                event_count: run_events.len(),
                generated_at: chrono::Utc::now().to_rfc3339(),
            }
            .write_json(config.run_summary_path(&loop_run_id))?;
            println!(
                "edited={} verification={}",
                relative_path,
                if verification.passed {
                    "passed"
                } else {
                    "failed"
                }
            );
            if !verification.passed {
                return Err("markdown verification failed".into());
            }
        }
        _ => {
            return Err("unsupported command".into());
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  steerbox runtime init <project_ref>");
    eprintln!("  steerbox config validate <config.json>");
    eprintln!("  steerbox tool list");
    eprintln!("  steerbox event list <config.json> [offset limit]");
    eprintln!("  steerbox event append <config.json> <event_type> <aggregate_type> <aggregate_id> <event_id> <correlation_id> [payload_json]");
    eprintln!("  steerbox execution list <config.json>");
    eprintln!("  steerbox execution show <config.json> <loop_run_id>");
    eprintln!("  steerbox execution page <config.json> <loop_run_id> <offset> <limit>");
    eprintln!("  steerbox model-call page <config.json> <offset> <limit>");
    eprintln!("  steerbox command submit <config.json> <command-json>");
    eprintln!("  steerbox command show <config.json> <command_id>");
    eprintln!("  steerbox board list <config.json>");
    eprintln!("  steerbox loop readonly <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <step_id> <objective> <observation>");
    eprintln!("  steerbox loop fixture <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <model_id> [continue_observation|complete_loop|request_human] <observation> [summary]");
    eprintln!("  steerbox loop provider <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <model_id> <objective> <observation> [--credential-source env]");
    eprintln!("    Default credential source is fail-closed; `--credential-source env` resolves only opaque env:NAME references.");
    eprintln!("  steerbox dev markdown <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <step_id> <path> <expected> <replacement> <auto|ask|approved>");
}

#[allow(clippy::too_many_arguments)]
fn append_runtime_event(
    log: &mut EventLog,
    aggregate_type: &str,
    aggregate_id: &str,
    event_type: &str,
    project_id: &str,
    goal_id: &str,
    loop_run_id: &str,
    step_id: Option<&str>,
    payload: Value,
) -> Result<u64, Box<dyn Error>> {
    let mut event = EventEnvelope::new(
        aggregate_type,
        format!("{aggregate_id}:{event_type}"),
        event_type,
        aggregate_id,
        loop_run_id,
        payload,
    )?;
    event.project_id = Some(project_id.into());
    event.goal_id = Some(goal_id.into());
    event.loop_run_id = Some(loop_run_id.into());
    event.step_id = step_id.map(str::to_owned);
    Ok(log.append(event)?)
}

fn command_metadata(entry: &UserCommandEntry) -> Value {
    let command = &entry.command;
    serde_json::json!({
        "command_id": command.command_id,
        "actor_ref": command.actor_ref,
        "project_id": command.project_id,
        "goal_id": command.goal_id,
        "task_run_id": command.task_run_id,
        "loop_run_id": command.loop_run_id,
        "command_kind": command.command_kind,
        "correlation_id": command.correlation_id,
        "expected_object_version": command.expected_object_version,
        "requested_at": command.requested_at,
        "status": entry.status,
        "updated_at": entry.updated_at,
        "payload_omitted": true,
    })
}

fn parse_model_decision(value: &str) -> Result<ModelDecision, Box<dyn Error>> {
    match value {
        "continue_observation" => Ok(ModelDecision::ContinueObservation),
        "complete_loop" => Ok(ModelDecision::CompleteLoop),
        "request_human" => Ok(ModelDecision::RequestHuman),
        _ => Err(format!("unsupported fixture decision: {value}").into()),
    }
}

fn controller_status(controller: &ReadOnlyLoopController) -> LoopStatus {
    controller.status()
}

/// No implicit environment, config, or disk secret lookup. A trusted embedding
/// supplies its resolver through ForegroundProviderRun::run_with_resolver.
struct UnconfiguredCredentialResolver;

impl CredentialResolver for UnconfiguredCredentialResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

enum CliCredentialResolver {
    Unconfigured,
    Environment,
}

impl CredentialResolver for CliCredentialResolver {
    fn resolve(&self, auth_ref: &str) -> Result<ResolvedCredential, CredentialError> {
        match self {
            Self::Unconfigured => UnconfiguredCredentialResolver.resolve(auth_ref),
            Self::Environment => EnvironmentCredentialResolver.resolve(auth_ref),
        }
    }
}
