use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use steerbox::audit_store::{CheckpointStore, RunSummary, SideEffectLedger, TraceStore};
use steerbox::event_log::EventLog;
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;

fn project_ref() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    format!("cli-e2e-{nonce}")
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
            "approved",
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
