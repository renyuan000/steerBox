#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    net::TcpListener,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use steerbox::runtime_config::RuntimeConfig;

const SUMMARY: &str = "preflight=passed scope=readonly credentials=not_checked transport=not_checked history=not_checked\n";
const DESTINATIONS: [&str; 8] = [
    "events.jsonl",
    "model-calls.jsonl",
    "model-calls.failures.jsonl",
    "model-calls.jsonl.lock",
    "model-calls.failures.jsonl.lock",
    "tool-calls.jsonl",
    "tool-calls.jsonl.lock",
    "state.json",
];

struct Fixture {
    directory: tempfile::TempDir,
    config: RuntimeConfig,
    workspace: PathBuf,
}

impl Fixture {
    fn new(provider: &str, initialized: bool) -> Self {
        fs::create_dir_all("/temp/steerbox").unwrap();
        let directory = tempfile::Builder::new()
            .prefix("readonly-preflight-")
            .tempdir_in("/temp/steerbox")
            .unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(
            workspace.join("README.md"),
            "# Synthetic\n[missing](missing.md)\n",
        )
        .unwrap();
        let protocol = if provider == "anthropic" {
            "anthropic-messages"
        } else {
            "openai-compatible-chat"
        };
        let config = RuntimeConfig::from_json_str(
            &serde_json::json!({
                "schema_version": 1,
                "project_ref": directory.path().file_name().unwrap().to_str().unwrap(),
                "data_root": directory.path(),
                "model_profiles": [{
                    "model_id": "synthetic-model", "provider": provider,
                    "endpoint_protocol": protocol,
                    "endpoint": "https://fixture.example.test/v1",
                    "auth_ref": "env:STEERBOX_PREFLIGHT_SYNTHETIC_CREDENTIAL",
                    "capabilities": ["text"]
                }]
            })
            .to_string(),
        )
        .unwrap();
        config.write_json(config.config_path()).unwrap();
        if initialized {
            fs::create_dir(config.events_path().parent().unwrap()).unwrap();
        }
        Self {
            directory,
            config,
            workspace,
        }
    }

    fn args(&self) -> Vec<String> {
        vec![
            "loop".into(),
            "preflight-readonly".into(),
            self.config.config_path().display().to_string(),
            "project".into(),
            "goal".into(),
            "preflight-loop".into(),
            "preflight-task".into(),
            "synthetic-model".into(),
            "Inspect".into(),
            "Observe".into(),
            "--workspace".into(),
            self.workspace.display().to_string(),
            "--max-iterations".into(),
            "2".into(),
            "--path".into(),
            "README.md".into(),
        ]
    }

    fn invoke(&self, args: Vec<String>) -> Output {
        Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .env_clear()
            .env(
                "STEERBOX_PREFLIGHT_SYNTHETIC_CREDENTIAL",
                "synthetic\ninvalid",
            )
            .output()
            .unwrap()
    }

    fn check_unchanged(&self, args: Vec<String>, success: bool) -> Output {
        let before = snapshot(self.directory.path());
        let output = self.invoke(args);
        assert_eq!(
            output.status.success(),
            success,
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(snapshot(self.directory.path()), before);
        if success {
            assert_eq!(String::from_utf8_lossy(&output.stdout), SUMMARY);
            assert!(output.stderr.is_empty());
        } else {
            assert!(!String::from_utf8_lossy(&output.stdout).contains("preflight=passed"));
        }
        output
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let value = if metadata.file_type().is_symlink() {
            format!("link:{}", fs::read_link(path).unwrap().display()).into_bytes()
        } else if metadata.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                visit(root, &child.unwrap().path(), entries);
            }
            b"directory".to_vec()
        } else {
            let mut value = b"file:".to_vec();
            value.extend(fs::read(path).unwrap());
            value
        };
        entries.insert(path.strip_prefix(root).unwrap().to_owned(), value);
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[test]
fn preflight_accepts_both_protocols_without_running_tools_or_opening_stores() {
    for provider in ["openai", "anthropic"] {
        let fixture = Fixture::new(provider, true);
        fixture.check_unchanged(fixture.args(), true);
        fixture.check_unchanged(fixture.args(), true);
        // A broken Markdown link is not checked, and no run identity is reserved.
        assert_eq!(
            fs::read_dir(fixture.config.events_path().parent().unwrap())
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn preflight_explicit_env_does_not_resolve_credentials_or_connect() {
    for provider in ["openai", "anthropic"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut fixture = Fixture::new(provider, true);
        fixture.config.model_profiles[0].endpoint =
            format!("https://{}/v1", listener.local_addr().unwrap());
        fixture
            .config
            .write_json(fixture.config.config_path())
            .unwrap();
        let mut args = fixture.args();
        args.extend(["--credential-source".into(), "env".into()]);
        fixture.check_unchanged(args, true);
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);

        let before = snapshot(fixture.directory.path());
        let mut args = fixture.args();
        args.extend(["--credential-source".into(), "env".into()]);
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), SUMMARY);
        assert!(output.stderr.is_empty());
        assert_eq!(snapshot(fixture.directory.path()), before);
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }
}

#[test]
fn preflight_does_not_create_missing_runtime_parent() {
    let fixture = Fixture::new("openai", false);
    fixture.check_unchanged(fixture.args(), false);
    assert!(!fixture.config.events_path().parent().unwrap().exists());
}

#[test]
fn preflight_rejects_all_eight_directory_symlink_and_hardlink_destinations() {
    for filename in DESTINATIONS {
        for kind in ["directory", "symlink", "hardlink"] {
            let fixture = Fixture::new("openai", true);
            let target = fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename);
            match kind {
                "directory" => fs::create_dir(&target).unwrap(),
                "symlink" => symlink(fixture.workspace.join("README.md"), &target).unwrap(),
                "hardlink" => {
                    let protected = fixture.directory.path().join("protected.txt");
                    fs::write(&protected, "Synthetic protected fixture\n").unwrap();
                    fs::hard_link(protected, &target).unwrap();
                }
                _ => unreachable!(),
            }
            fixture.check_unchanged(fixture.args(), false);
        }
    }
}

#[test]
fn preflight_rejects_store_aliases_and_workspace_collisions_without_writes() {
    for filename in &DESTINATIONS[1..] {
        let fixture = Fixture::new("openai", true);
        fs::write(fixture.config.events_path(), "synthetic-invalid-history\n").unwrap();
        fs::hard_link(
            fixture.config.events_path(),
            fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename),
        )
        .unwrap();
        fixture.check_unchanged(fixture.args(), false);
    }
    let fixture = Fixture::new("openai", false);
    symlink(
        &fixture.workspace,
        fixture.config.events_path().parent().unwrap(),
    )
    .unwrap();
    fixture.check_unchanged(fixture.args(), false);
}

#[test]
fn preflight_rejects_invalid_profile_and_scope_without_side_effects() {
    for invalid in [
        "missing-model",
        "capability",
        "auth",
        "missing-path",
        "iteration",
    ] {
        let mut fixture = Fixture::new("openai", true);
        let mut args = fixture.args();
        match invalid {
            "missing-model" => args[7] = "absent-model".into(),
            "capability" => fixture.config.model_profiles[0].capabilities = vec!["image".into()],
            "auth" => fixture.config.model_profiles[0].auth_ref = None,
            "missing-path" => args[15] = "absent.md".into(),
            "iteration" => args[13] = "0".into(),
            _ => unreachable!(),
        }
        fixture
            .config
            .write_json(fixture.config.config_path())
            .unwrap();
        fixture.check_unchanged(args, false);
    }
}

#[test]
fn preflight_rejects_authorized_file_aliases_and_runtime_inside_workspace() {
    for kind in ["symlink", "hardlink", "runtime-workspace"] {
        let fixture = Fixture::new("openai", true);
        let mut args = fixture.args();
        match kind {
            "symlink" => {
                symlink(
                    fixture.workspace.join("README.md"),
                    fixture.workspace.join("alias.md"),
                )
                .unwrap();
                args[15] = "alias.md".into();
            }
            "hardlink" => {
                fs::hard_link(
                    fixture.workspace.join("README.md"),
                    fixture.workspace.join("alias.md"),
                )
                .unwrap();
            }
            "runtime-workspace" => {
                let parent = fixture.config.events_path().parent().unwrap().to_owned();
                fs::write(parent.join("README.md"), "# Synthetic runtime collision\n").unwrap();
                args[11] = parent.display().to_string();
            }
            _ => unreachable!(),
        }
        fixture.check_unchanged(args, false);
    }
}

#[test]
fn preflight_parses_before_config_loading_and_does_not_echo_bad_flags() {
    let fixture = Fixture::new("openai", true);
    let mut args = fixture.args();
    args[2] = "/absent/synthetic-config.json".into();
    args.push("--synthetic-sensitive-invalid-flag".into());
    let output = fixture.check_unchanged(args, false);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("steerbox: invalid provider-readonly arguments"));
    assert!(!stderr.contains("synthetic-sensitive-invalid-flag"));
    assert!(!stderr.contains("configuration file read failed"));
}

#[test]
fn preflight_reports_history_unchecked_and_leaves_existing_logs_unchanged() {
    let fixture = Fixture::new("openai", true);
    for filename in DESTINATIONS {
        fs::write(
            fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename),
            "synthetic-invalid-history\n",
        )
        .unwrap();
    }
    fixture.check_unchanged(fixture.args(), true);
}
