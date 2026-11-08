#![cfg(unix)]

use std::{fs, path::PathBuf, process::Command};
use steerbox::runtime_config::RuntimeConfig;

fn arguments() -> Vec<String> {
    [
        "loop",
        "provider-readonly",
        "/missing/config.json",
        "project",
        "goal",
        "loop",
        "task",
        "fixture-model",
        "Inspect",
        "Observe",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn readonly_cli_parses_all_flags_before_loading_config_without_echoing_inputs() {
    for flags in [
        vec!["--untrusted-sensitive-value"],
        vec!["--workspace"],
        vec!["--workspace", "/tmp", "--workspace", "/tmp"],
        vec![
            "--workspace",
            "/tmp",
            "--max-iterations",
            "0",
            "--path",
            "README.md",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--max-iterations",
            "17",
            "--path",
            "README.md",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--max-iterations",
            "4294967296",
            "--path",
            "README.md",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--max-iterations",
            "1",
            "--path",
            "README.md",
            "--credential-source",
            "untrusted-sensitive-value",
        ],
    ] {
        let mut args = arguments();
        args.extend(flags.into_iter().map(str::to_owned));
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with("steerbox: invalid provider-readonly arguments"),
            "{stderr}"
        );
        assert!(!stderr.contains("untrusted-sensitive-value"));
        assert!(!stderr.contains("configuration file read failed"));
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    config: RuntimeConfig,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        fs::create_dir_all("/temp/steerbox").unwrap();
        let directory = tempfile::Builder::new()
            .prefix("readonly-cli-")
            .tempdir_in("/temp/steerbox")
            .unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), "# Synthetic fixture\n").unwrap();
        let config = RuntimeConfig::from_json_str(&serde_json::json!({
            "schema_version":1,"project_ref":directory.path().file_name().unwrap().to_str().unwrap(),"data_root":directory.path(),
            "model_profiles":[{"model_id":"fixture-model","provider":"fixture","endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:synthetic-cli","capabilities":["text"]}]
        }).to_string()).unwrap();
        config.write_json(config.config_path()).unwrap();
        Self {
            _directory: directory,
            config,
            workspace,
        }
    }

    fn invoke(&self) -> std::process::Output {
        let mut args = arguments();
        args[2] = self.config.config_path().display().to_string();
        args.extend([
            "--workspace".into(),
            self.workspace.display().to_string(),
            "--max-iterations".into(),
            "1".into(),
            "--path".into(),
            "README.md".into(),
        ]);
        Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn readonly_cli_requires_initialized_store_parent_without_creating_it() {
    let fixture = Fixture::new();
    let parent = fixture.config.events_path().parent().unwrap().to_owned();
    assert!(!parent.exists());
    let output = fixture.invoke();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("foreground provider request or profile is invalid"));
    assert!(!parent.exists());
}

#[test]
fn readonly_cli_rejects_each_directory_destination_before_creating_events() {
    for filename in [
        "events.jsonl",
        "model-calls.jsonl",
        "model-calls.failures.jsonl",
        "model-calls.jsonl.lock",
        "model-calls.failures.jsonl.lock",
        "tool-calls.jsonl",
        "tool-calls.jsonl.lock",
        "state.json",
    ] {
        let fixture = Fixture::new();
        let parent = fixture.config.events_path().parent().unwrap().to_owned();
        fs::create_dir(&parent).unwrap();
        fs::create_dir(parent.join(filename)).unwrap();
        let before = fs::read_dir(&parent).unwrap().count();
        let output = fixture.invoke();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("foreground provider request or profile is invalid"),
            "{filename}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_dir(&parent).unwrap().count(), before, "{filename}");
    }
}

#[test]
fn readonly_cli_rejects_all_symlink_and_hardlink_destinations_without_writes() {
    use std::os::unix::fs::symlink;
    for filename in [
        "events.jsonl",
        "model-calls.jsonl",
        "model-calls.failures.jsonl",
        "model-calls.jsonl.lock",
        "model-calls.failures.jsonl.lock",
        "tool-calls.jsonl",
        "tool-calls.jsonl.lock",
        "state.json",
    ] {
        for hardlink in [false, true] {
            let fixture = Fixture::new();
            let parent = fixture.config.events_path().parent().unwrap().to_owned();
            fs::create_dir(&parent).unwrap();
            let protected = fixture.config.data_root.join("protected.md");
            fs::write(&protected, "# Synthetic fixture\n").unwrap();
            if hardlink {
                fs::hard_link(&protected, parent.join(filename)).unwrap();
            } else {
                symlink(&protected, parent.join(filename)).unwrap();
            }
            let output = fixture.invoke();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("foreground provider request or profile is invalid"));
            assert_eq!(fs::read_dir(&parent).unwrap().count(), 1, "{filename}");
            assert_eq!(
                fs::read_to_string(protected).unwrap(),
                "# Synthetic fixture\n"
            );
        }
    }
}

#[test]
fn readonly_cli_default_credentials_fail_closed_and_persist_failed_projection() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.config.events_path().parent().unwrap()).unwrap();
    let output = fixture.invoke();
    assert!(!output.status.success());
    let events = steerbox::event_log::EventLog::open(fixture.config.events_path())
        .unwrap()
        .read_all()
        .unwrap();
    assert!(events.iter().any(|event| event.event_type == "loop_failed"));
    let state = steerbox::state_store::StateStore::load(fixture.config.state_path()).unwrap();
    assert_eq!(
        state.loop_run("loop").unwrap().status,
        steerbox::domain::LoopStatus::Failed
    );
    assert!(!fixture.config.tool_calls_path().exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("decision=CompleteLoop"));
}

#[test]
fn readonly_cli_rejects_runtime_store_aliases_and_workspace_parent_without_writes() {
    for target in [
        "model-calls.jsonl",
        "model-calls.failures.jsonl",
        "model-calls.jsonl.lock",
        "model-calls.failures.jsonl.lock",
        "tool-calls.jsonl",
        "tool-calls.jsonl.lock",
        "state.json",
    ] {
        let fixture = Fixture::new();
        let parent = fixture.config.events_path().parent().unwrap().to_owned();
        fs::create_dir(&parent).unwrap();
        fs::write(fixture.config.events_path(), "synthetic-not-json\n").unwrap();
        fs::hard_link(fixture.config.events_path(), parent.join(target)).unwrap();
        let output = fixture.invoke();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("foreground provider request or profile is invalid"),
            "{target}"
        );
        assert_eq!(fs::read_dir(parent).unwrap().count(), 2);
        assert_eq!(
            fs::read_to_string(fixture.config.events_path()).unwrap(),
            "synthetic-not-json\n"
        );
    }
    let fixture = Fixture::new();
    std::os::unix::fs::symlink(
        &fixture.workspace,
        fixture.config.events_path().parent().unwrap(),
    )
    .unwrap();
    let output = fixture.invoke();
    assert!(!output.status.success());
    assert_eq!(fs::read_dir(&fixture.workspace).unwrap().count(), 1);
}

#[test]
fn model_call_storage_names_are_pure_and_match_open_store_names() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("absent").join("model-calls.jsonl");
    let paths = steerbox::model_call_store::ModelCallStore::storage_paths_for(&path);
    assert!(!path.parent().unwrap().exists());
    assert_eq!(paths[1], path.with_extension("failures.jsonl"));
    let store = steerbox::model_call_store::ModelCallStore::open(path).unwrap();
    assert_eq!(paths, store.storage_paths());
}

#[test]
fn readonly_cli_explicit_env_source_fails_closed_for_removed_synthetic_variable() {
    let mut fixture = Fixture::new();
    fixture.config.model_profiles[0].auth_ref =
        Some("env:STEERBOX_SYNTHETIC_READONLY_CLI_MISSING_CREDENTIAL".into());
    fixture
        .config
        .write_json(fixture.config.config_path())
        .unwrap();
    fs::create_dir(fixture.config.events_path().parent().unwrap()).unwrap();
    let mut args = arguments();
    args[2] = fixture.config.config_path().display().to_string();
    args.extend([
        "--workspace".into(),
        fixture.workspace.display().to_string(),
        "--max-iterations".into(),
        "1".into(),
        "--path".into(),
        "README.md".into(),
        "--credential-source".into(),
        "env".into(),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
        .args(args)
        .env_remove("STEERBOX_SYNTHETIC_READONLY_CLI_MISSING_CREDENTIAL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let calls = steerbox::model_call_store::ModelCallStore::open(fixture.config.model_calls_path())
        .unwrap();
    assert!(calls.read_all().unwrap().is_empty());
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    assert!(!fixture.config.tool_calls_path().exists());
    let state = steerbox::state_store::StateStore::load(fixture.config.state_path()).unwrap();
    assert_eq!(
        state.loop_run("loop").unwrap().status,
        steerbox::domain::LoopStatus::Failed
    );
}
