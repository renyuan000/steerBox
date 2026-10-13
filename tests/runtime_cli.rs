use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::audit_store::{CheckpointStore, RunSummary, SideEffectLedger, TraceStore};
use steerbox::event_log::EventLog;
use steerbox::model_call_store::ModelCallStore;
use steerbox::model_contracts::ModelCallRecord;
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::user_command_inbox::{CommandKind, CommandStatus, UserCommandInbox};

fn project_ref() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    format!("cli-e2e-{nonce}")
}

fn model_call_record(id: &str) -> ModelCallRecord {
    ModelCallRecord {
        model_call_id: id.into(),
        task_id: "task-cli".into(),
        task_run_id: "task-run-cli".into(),
        agent_role: "implementer".into(),
        provider_id: "fixture-provider".into(),
        endpoint_id: "fixture-endpoint".into(),
        model_id: "fixture-model".into(),
        route_decision_id: format!("route-{id}"),
        resolved_prompt_pack_id: "prompt-cli".into(),
        input_context_hash: "sha256:fixture".into(),
        output_artifact_ref: format!("trace://{id}/output"),
        latency_ms: Some(5),
        input_tokens: Some(10),
        output_tokens: Some(12),
        estimated_cost: Some(0.0),
        streaming: false,
        finish_reason: "stop".into(),
        tool_call_count: 0,
        verification_status: "not_run".into(),
        error_class: None,
        started_at: "2026-10-11T00:00:00Z".into(),
        completed_at: Some("2026-10-11T00:00:01Z".into()),
        created_at: "2026-10-11T00:00:01Z".into(),
    }
}

#[test]
fn command_cli_submits_and_shows_metadata_without_payload() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let command_json = r#"{"command_id":"command-cli-1","actor_ref":"user-cli","project_id":"project-cli","goal_id":"goal-cli","task_run_id":"task-run-cli","loop_run_id":"loop-cli","command_kind":"steer","correlation_id":"corr-cli-1","expected_object_version":null,"requested_at":"2026-10-11T00:00:00Z","payload":{"instruction":"pause after current safe boundary"}}"#;
    let submit = Command::new(binary)
        .args([
            "command",
            "submit",
            &config.config_path().display().to_string(),
            command_json,
        ])
        .output()
        .expect("command submit starts");
    assert!(
        submit.status.success(),
        "submit stderr: {}",
        String::from_utf8_lossy(&submit.stderr)
    );
    let submit_json: serde_json::Value =
        serde_json::from_slice(&submit.stdout).expect("submit metadata is JSON");
    assert_eq!(submit_json["command_id"], "command-cli-1");
    assert_eq!(submit_json["status"], "pending");
    assert_eq!(submit_json["payload_omitted"], true);
    assert!(submit_json.get("payload").is_none());
    assert!(!String::from_utf8_lossy(&submit.stdout).contains("pause after current safe boundary"));

    let show = Command::new(binary)
        .args([
            "command",
            "show",
            &config.config_path().display().to_string(),
            "command-cli-1",
        ])
        .output()
        .expect("command show starts");
    assert!(
        show.status.success(),
        "show stderr: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_json: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show metadata is JSON");
    assert_eq!(show_json["command_id"], "command-cli-1");
    assert_eq!(show_json["status"], "pending");
    assert!(show_json.get("payload").is_none());

    let entries = UserCommandInbox::open(config.commands_path())
        .read_all()
        .expect("command inbox reopens");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, CommandStatus::Pending);
    assert_eq!(entries[0].command.command_kind, CommandKind::Steer);
    assert_eq!(
        entries[0].command.payload["instruction"],
        "pause after current safe boundary"
    );

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn command_cli_rejects_unknown_command_fields() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(init.status.success());

    let command_json = r#"{"command_id":"command-cli-unknown","actor_ref":"user-cli","project_id":"project-cli","goal_id":"goal-cli","task_run_id":"task-run-cli","loop_run_id":"loop-cli","command_kind":"steer","correlation_id":"corr-cli-unknown","requested_at":"2026-10-11T00:00:00Z","payload":{},"unexpected":"reject me"}"#;
    let submit = Command::new(binary)
        .args([
            "command",
            "submit",
            &config.config_path().display().to_string(),
            command_json,
        ])
        .output()
        .expect("command submit starts");
    assert!(!submit.status.success());
    assert!(String::from_utf8_lossy(&submit.stderr).contains("unknown field"));
    assert!(!config.commands_path().exists());

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn readonly_cli_reopens_event_state_and_trace_artifacts() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let loop_run = Command::new(binary)
        .args([
            "loop",
            "readonly",
            &config.config_path().display().to_string(),
            "project-cli",
            "goal-cli",
            "loop-cli",
            "task-cli",
            "step-cli",
            "inspect repository",
            "repository inspected",
        ])
        .output()
        .expect("readonly loop starts");
    assert!(
        loop_run.status.success(),
        "loop stderr: {}",
        String::from_utf8_lossy(&loop_run.stderr)
    );
    let loop_stdout = String::from_utf8_lossy(&loop_run.stdout);
    assert!(loop_stdout.contains("loop loop-cli started"));
    assert!(loop_stdout.contains("loop loop-cli completed"));

    let events = EventLog::open(config.events_path())
        .expect("event log opens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
    let state = StateStore::load(config.state_path()).expect("state reopens");
    assert_eq!(
        state
            .loop_run("loop-cli")
            .expect("loop state exists")
            .status,
        steerbox::domain::LoopStatus::Completed
    );
    let traces = TraceStore::open(config.traces_path())
        .read_all()
        .expect("traces read");
    assert!(!traces.is_empty());
    let checkpoints = CheckpointStore::open(config.checkpoints_path())
        .read_all()
        .expect("checkpoints read");
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].loop_run_id, "loop-cli");

    let history = Command::new(binary)
        .args([
            "execution",
            "list",
            &config.config_path().display().to_string(),
        ])
        .output()
        .expect("execution list starts");
    assert!(
        history.status.success(),
        "history stderr: {}",
        String::from_utf8_lossy(&history.stderr)
    );
    let history_text = String::from_utf8_lossy(&history.stdout);
    assert!(history_text.contains("loop-cli"));
    assert!(history_text.contains("completed"));

    let execution = Command::new(binary)
        .args([
            "execution",
            "show",
            &config.config_path().display().to_string(),
            "loop-cli",
        ])
        .output()
        .expect("execution show starts");
    assert!(
        execution.status.success(),
        "execution stderr: {}",
        String::from_utf8_lossy(&execution.stderr)
    );
    let execution_text = String::from_utf8_lossy(&execution.stdout);
    assert!(execution_text.contains("loop_started"));
    assert!(execution_text.contains("loop_completed"));
    assert!(!execution_text.contains("repository inspected"));

    let page = Command::new(binary)
        .args([
            "execution",
            "page",
            &config.config_path().display().to_string(),
            "loop-cli",
            "1",
            "2",
        ])
        .output()
        .expect("execution page starts");
    assert!(
        page.status.success(),
        "page stderr: {}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page_json: serde_json::Value =
        serde_json::from_slice(&page.stdout).expect("page output is JSON");
    assert_eq!(page_json["loop_run_id"], "loop-cli");
    assert_eq!(page_json["offset"], 1);
    assert_eq!(page_json["limit"], 2);
    assert_eq!(page_json["total"], 5);
    assert_eq!(page_json["has_more"], true);
    assert_eq!(
        page_json["events"].as_array().expect("page events").len(),
        2
    );
    assert!(!String::from_utf8_lossy(&page.stdout).contains("repository inspected"));
    assert!(page_json["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event.get("payload").is_none()));

    // These offsets address one detached snapshot, not a live cursor or streaming read.
    for (offset, limit, expected_len) in [("0", "0", 0), ("99", "2", 0), ("4", "2", 1)] {
        let output = Command::new(binary)
            .args([
                "execution",
                "page",
                &config.config_path().display().to_string(),
                "loop-cli",
                offset,
                limit,
            ])
            .output()
            .expect("snapshot page starts");
        assert!(output.status.success());
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("page JSON");
        assert_eq!(json["has_more"], false);
        assert_eq!(json["total"], 5);
        assert_eq!(json["events"].as_array().unwrap().len(), expected_len);
        assert!(json["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event.get("payload").is_none()));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("repository inspected"));
    }

    for (offset, limit, error_field) in [
        ("not-a-number", "2", "offset"),
        ("-1", "2", "offset"),
        ("0", "not-a-number", "limit"),
        ("0", "-1", "limit"),
    ] {
        let output = Command::new(binary)
            .args([
                "execution",
                "page",
                &config.config_path().display().to_string(),
                "loop-cli",
                offset,
                limit,
            ])
            .output()
            .expect("invalid page starts");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("invalid {error_field}")));
    }

    let missing_page = Command::new(binary)
        .args([
            "execution",
            "page",
            &config.config_path().display().to_string(),
            "missing-loop",
            "0",
            "10",
        ])
        .output()
        .expect("missing execution page starts");
    assert!(!missing_page.status.success());
    assert!(missing_page.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&missing_page.stderr).contains("execution not found: missing-loop")
    );

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn fixture_cli_persists_state_trace_and_checkpoint() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let loop_run = Command::new(binary)
        .args([
            "loop",
            "fixture",
            &config.config_path().display().to_string(),
            "project-fixture",
            "goal-fixture",
            "loop-fixture",
            "task-fixture",
            "local-fixture",
            "complete_loop",
            "fixture observation",
            "fixture complete",
        ])
        .output()
        .expect("fixture loop starts");
    assert!(
        loop_run.status.success(),
        "loop stderr: {}",
        String::from_utf8_lossy(&loop_run.stderr)
    );

    let state = StateStore::load(config.state_path()).expect("state reopens");
    assert_eq!(
        state
            .loop_run("loop-fixture")
            .expect("loop state exists")
            .status,
        steerbox::domain::LoopStatus::Completed
    );
    assert_eq!(
        TraceStore::open(config.traces_path())
            .read_all()
            .expect("traces read")
            .len(),
        1
    );
    assert_eq!(
        CheckpointStore::open(config.checkpoints_path())
            .read_all()
            .expect("checkpoints read")
            .len(),
        1
    );

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

fn fixture_decision_output(
    decision: &str,
) -> (
    String,
    Vec<steerbox::domain::EventEnvelope>,
    steerbox::domain::LoopStatus,
) {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let binary = env!("CARGO_BIN_EXE_steerbox");
    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(init.status.success());
    let run = Command::new(binary)
        .args([
            "loop",
            "fixture",
            &config.config_path().display().to_string(),
            "project-fixture",
            "goal-fixture",
            "loop-fixture-decision",
            "task-fixture",
            "local-fixture",
            decision,
            "fake observation",
            "fake summary",
        ])
        .output()
        .expect("fixture starts");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let events = EventLog::open(config.events_path())
        .expect("event log opens")
        .read_all()
        .expect("events read");
    assert_eq!(
        events
            .iter()
            .find(|event| event.event_type == "model_call_completed")
            .expect("model completion event")
            .payload["decision"],
        decision,
    );
    let status = StateStore::load(config.state_path())
        .expect("state loads")
        .loop_run("loop-fixture-decision")
        .expect("loop exists")
        .status;
    let stdout = String::from_utf8(run.stdout).expect("UTF-8 output");
    assert!(stdout.contains("model local-fixture started"));
    assert!(stdout.contains("model local-fixture completed"));
    fs::remove_dir_all(&config.data_root).expect("fixture data removed");
    (stdout, events, status)
}

#[test]
fn fixture_cli_complete_reports_the_audited_terminal_state() {
    let (stdout, events, status) = fixture_decision_output("complete_loop");
    assert_eq!(status, steerbox::domain::LoopStatus::Completed);
    assert_eq!(
        stdout
            .matches("loop loop-fixture-decision completed")
            .count(),
        1
    );
    assert!(events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "human_intervention_required"));
}

#[test]
fn fixture_cli_continue_does_not_report_a_false_terminal_state() {
    let (stdout, events, status) = fixture_decision_output("continue_observation");
    assert_eq!(status, steerbox::domain::LoopStatus::Running);
    assert!(!stdout.contains("loop loop-fixture-decision completed"));
    assert!(stdout.contains("continue observation; loop remains running"));
    assert!(stdout.contains("loop_status=Running"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "model_call_completed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "human_intervention_required"));
}

#[test]
fn fixture_cli_request_human_reports_intervention_without_claiming_pause_or_completion() {
    let (stdout, events, status) = fixture_decision_output("request_human");
    assert_eq!(status, steerbox::domain::LoopStatus::Running);
    assert!(!stdout.contains("loop loop-fixture-decision completed"));
    assert!(!stdout.contains("loop loop-fixture-decision paused"));
    assert!(stdout.contains("human intervention required"));
    assert!(stdout.contains("loop_status=Running"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "human_intervention_required"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed" || event.event_type == "loop_paused"));
}

#[test]
fn dev_markdown_cli_explains_that_ask_mode_needs_a_resolver() {
    let binary = env!("CARGO_BIN_EXE_steerbox");
    let run = Command::new(binary)
        .args([
            "dev",
            "markdown",
            "/temp/steerbox/missing-config.json",
            "project-ask",
            "goal-ask",
            "loop-ask",
            "task-ask",
            "step-ask",
            "README.md",
            "before",
            "after",
            "ask",
        ])
        .output()
        .expect("ask mode starts");

    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("interactive approval resolver"));
}

#[test]
fn dev_markdown_cli_runs_guarded_edit_and_persists_verification() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    fs::write(
        config.data_root.join("README.md"),
        "See [guide](guide.md).\n",
    )
    .expect("readme writes");
    fs::write(config.data_root.join("guide.md"), "guide\n").expect("guide writes");

    let run = Command::new(binary)
        .args([
            "dev",
            "markdown",
            &config.config_path().display().to_string(),
            "project-dev",
            "goal-dev",
            "loop-dev",
            "task-dev",
            "step-dev",
            "README.md",
            "See [guide](guide.md).\n",
            "See [guide](guide.md) and [notes](notes.md).\n",
            "approved",
        ])
        .output()
        .expect("dev loop starts");
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("verification failed"));
    assert_eq!(
        fs::read_to_string(config.data_root.join("README.md")).expect("edited readme reads"),
        "See [guide](guide.md) and [notes](notes.md).\n"
    );

    let events = EventLog::open(config.events_path())
        .expect("event log opens")
        .read_all()
        .expect("events read");
    assert!(events
        .iter()
        .any(|event| event.event_type == "checkpoint_created"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "file_change_recorded"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "verification_completed"));
    assert_eq!(
        StateStore::load(config.state_path())
            .expect("state reopens")
            .loop_run("loop-dev")
            .expect("loop exists")
            .status,
        steerbox::domain::LoopStatus::Failed
    );
    assert_eq!(
        CheckpointStore::open(config.checkpoints_path())
            .read_all()
            .expect("checkpoints read")
            .len(),
        1
    );
    assert_eq!(
        SideEffectLedger::open(config.side_effects_path())
            .read_all()
            .expect("side effects read")
            .len(),
        1
    );
    let summary = RunSummary::read_json(config.run_summary_path("loop-dev"))
        .expect("failed run summary reopens");
    assert_eq!(summary.final_status, "failed");
    assert_eq!(summary.verification_status, "failed");
    assert_eq!(summary.step_ids, vec!["step-dev"]);
    assert_eq!(summary.file_changes, vec!["README.md"]);
    assert_eq!(
        summary.checkpoint_ids,
        vec!["step-dev:checkpoint:before-write"]
    );
    assert_eq!(summary.side_effect_ids, vec!["step-dev:side-effect"]);

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn dev_markdown_cli_completes_and_writes_run_summary_when_verification_passes() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(
        init.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    fs::write(
        config.data_root.join("README.md"),
        "See [guide](guide.md).\n",
    )
    .expect("readme writes");
    fs::write(config.data_root.join("guide.md"), "guide\n").expect("guide writes");

    let run = Command::new(binary)
        .args([
            "dev",
            "markdown",
            &config.config_path().display().to_string(),
            "project-dev-pass",
            "goal-dev-pass",
            "loop-dev-pass",
            "task-dev-pass",
            "step-dev-pass",
            "README.md",
            "See [guide](guide.md).\n",
            "See [guide](guide.md) and [guide2](guide.md).\n",
            "auto",
        ])
        .output()
        .expect("dev loop starts");
    assert!(
        run.status.success(),
        "dev stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let summary = RunSummary::read_json(config.run_summary_path("loop-dev-pass"))
        .expect("successful run summary reopens");
    assert_eq!(summary.final_status, "completed");
    assert_eq!(summary.verification_status, "passed");
    assert_eq!(summary.step_ids, vec!["step-dev-pass"]);
    assert_eq!(summary.file_changes, vec!["README.md"]);
    assert_eq!(summary.event_count, 8);
    assert_eq!(summary.event_ids.len(), summary.event_count);
    assert_eq!(
        summary.verification_event_ids,
        vec!["README.md:verification_completed"]
    );
    assert_eq!(summary.trace_ids, vec!["step-dev-pass:verification"]);

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn board_list_cli_rebuilds_read_only_projection_from_events() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");
    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(init.status.success());

    let mut event = steerbox::domain::EventEnvelope::new(
        "task",
        "task-created-cli",
        "task_created",
        "task-cli",
        "corr-cli",
        serde_json::json!({"title":"CLI board task"}),
    )
    .expect("task event builds");
    event.project_id = Some("project-cli".into());
    event.goal_id = Some("goal-cli".into());
    event.task_id = Some("task-cli".into());
    let mut log = EventLog::open(config.events_path()).expect("event log opens");
    log.append(event).expect("task event appends");

    let board = Command::new(binary)
        .args(["board", "list", &config.config_path().display().to_string()])
        .output()
        .expect("board list starts");
    assert!(
        board.status.success(),
        "board stderr: {}",
        String::from_utf8_lossy(&board.stderr)
    );
    let output = String::from_utf8_lossy(&board.stdout);
    assert!(output.contains("task-cli"));
    assert!(output.contains("CLI board task"));
    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}

#[test]
fn model_call_page_cli_reads_bounded_audit_records() {
    let project_ref = project_ref();
    let config = RuntimeConfig::default_for_project(project_ref.clone()).expect("config builds");
    let _ = fs::remove_dir_all(&config.data_root);
    let binary = env!("CARGO_BIN_EXE_steerbox");

    let init = Command::new(binary)
        .args(["runtime", "init", &project_ref])
        .output()
        .expect("runtime init starts");
    assert!(init.status.success());
    assert!(!config.model_calls_path().exists());

    let missing = Command::new(binary)
        .args([
            "model-call",
            "page",
            &config.config_path().display().to_string(),
            "0",
            "10",
        ])
        .output()
        .expect("missing model-call page starts");
    assert!(
        missing.status.success(),
        "missing page stderr: {}",
        String::from_utf8_lossy(&missing.stderr)
    );
    let missing_json: serde_json::Value =
        serde_json::from_slice(&missing.stdout).expect("missing page JSON");
    assert_eq!(missing_json["offset"], 0);
    assert_eq!(missing_json["limit"], 10);
    assert_eq!(missing_json["count"], 0);
    assert!(missing_json["model_calls"].as_array().unwrap().is_empty());

    let store = ModelCallStore::open(config.model_calls_path()).expect("model call store opens");
    store
        .append(&model_call_record("call-1"))
        .expect("first model call appends");
    store
        .append(&model_call_record("call-2"))
        .expect("second model call appends");

    for (offset, limit) in [(0, 0), (99, 2)] {
        let output = Command::new(binary)
            .args([
                "model-call",
                "page",
                &config.config_path().display().to_string(),
                &offset.to_string(),
                &limit.to_string(),
            ])
            .output()
            .expect("empty model-call page starts");
        assert!(output.status.success());
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("page JSON");
        assert_eq!(json["offset"], offset);
        assert_eq!(json["limit"], limit);
        assert_eq!(json["count"], 0);
        assert!(json["model_calls"].as_array().unwrap().is_empty());
    }

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(config.model_calls_path())
        .expect("model call log reopens");
    writeln!(file, "not-json").expect("malformed trailing line writes");
    drop(file);

    let bounded = Command::new(binary)
        .args([
            "model-call",
            "page",
            &config.config_path().display().to_string(),
            "1",
            "1",
        ])
        .output()
        .expect("bounded model-call page starts");
    assert!(
        bounded.status.success(),
        "bounded page stderr: {}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    let bounded_json: serde_json::Value =
        serde_json::from_slice(&bounded.stdout).expect("bounded page JSON");
    assert_eq!(bounded_json["count"], 1);
    assert_eq!(bounded_json["model_calls"][0]["model_call_id"], "call-2");

    let _ = fs::remove_dir_all(PathBuf::from("/temp/steerbox").join(project_ref));
}
