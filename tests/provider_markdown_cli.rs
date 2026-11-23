#![cfg(unix)]

use std::{fs, os::unix::fs::MetadataExt, path::PathBuf, process::Command};
use steerbox::runtime_config::{RuntimeConfig, OPENAI_COMPATIBLE_CHAT_PROTOCOL};

const SUCCESS: &str = "preflight=passed scope=markdown execution=disabled credentials=not_checked transport=not_checked history=not_checked\n";
const STORES: [&str; 11] = [
    "events.jsonl",
    "model-calls.jsonl",
    "model-calls.failures.jsonl",
    "model-calls.jsonl.lock",
    "model-calls.failures.jsonl.lock",
    "tool-calls.jsonl",
    "tool-calls.jsonl.lock",
    "checkpoints.jsonl",
    "traces.jsonl",
    "side-effects.jsonl",
    "state.json",
];

struct Fixture {
    _directory: tempfile::TempDir,
    config: RuntimeConfig,
    workspace: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new(protocol: &str) -> Self {
        fs::create_dir_all("/temp/steerbox").unwrap();
        let directory = tempfile::Builder::new()
            .prefix("markdown-preflight-cli-")
            .tempdir_in("/temp/steerbox")
            .unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("README.md"), b"# Synthetic fixture\n").unwrap();
        let config = RuntimeConfig::from_json_str(
            &serde_json::json!({
                "schema_version":1,
                "project_ref":directory.path().file_name().unwrap().to_str().unwrap(),
                "data_root":directory.path(),
                "model_profiles":[{
                    "model_id":"fixture-model",
                    "provider":if protocol == "anthropic-messages" {"anthropic"} else {"fixture"},
                    "endpoint_protocol":protocol,
                    "endpoint":"https://synthetic-hidden-endpoint.example.test/v1",
                    "auth_ref":"env:SYNTHETIC_MARKDOWN_PREFLIGHT_NOT_CONFIGURED",
                    "capabilities":["text"]
                }]
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
        }
    }

    fn args(&self) -> Vec<String> {
        vec![
            "loop".into(),
            "preflight-markdown".into(),
            self.config.config_path().display().to_string(),
            "project".into(),
            "goal".into(),
            "loop".into(),
            "task".into(),
            "fixture-model".into(),
            "synthetic-hidden-objective".into(),
            "synthetic-hidden-observation".into(),
            "--workspace".into(),
            self.workspace.display().to_string(),
            "--path".into(),
            "README.md".into(),
        ]
    }

    fn inventory(&self) -> Vec<Entry> {
        fn visit(path: PathBuf, entries: &mut Vec<Entry>) {
            let metadata = fs::symlink_metadata(&path).unwrap();
            let bytes = if metadata.is_file() {
                fs::read(&path).unwrap()
            } else if metadata.file_type().is_symlink() {
                fs::read_link(&path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else {
                Vec::new()
            };
            if metadata.is_dir() {
                let mut children: Vec<_> = fs::read_dir(&path)
                    .unwrap()
                    .map(|child| child.unwrap().path())
                    .collect();
                children.sort();
                for child in children {
                    visit(child, entries);
                }
            }
            entries.push(Entry {
                path,
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                links: metadata.nlink(),
                bytes,
            });
        }
        let mut entries = Vec::new();
        visit(self.config.data_root.clone(), &mut entries);
        entries
    }

    fn invoke(&self, args: Vec<String>) -> std::process::Output {
        let before = self.inventory();
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .env_remove("SYNTHETIC_MARKDOWN_PREFLIGHT_NOT_CONFIGURED")
            .output()
            .unwrap();
        assert_eq!(self.inventory(), before, "preflight changed fixture files");
        for text in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(text);
            for marker in [
                "synthetic-hidden-objective",
                "synthetic-hidden-observation",
                "synthetic-hidden-endpoint",
                "SYNTHETIC_MARKDOWN_PREFLIGHT_NOT_CONFIGURED",
                "untrusted-sensitive-value",
            ] {
                assert!(!text.contains(marker), "unexpected output: {text}");
            }
            assert!(!text.contains(self.config.data_root.to_str().unwrap()));
        }
        output
    }

    fn reject(&self, args: Vec<String>, prefix: &str) {
        let output = self.invoke(args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with(prefix), "{stderr}");
    }

    fn pass(&self, args: Vec<String>) {
        let output = self.invoke(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), SUCCESS);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn markdown_preflight_accepts_both_protocols_without_resolving_credentials_or_writing() {
    for protocol in [OPENAI_COMPATIBLE_CHAT_PROTOCOL, "anthropic-messages"] {
        let fixture = Fixture::new(protocol);
        fixture.pass(fixture.args());
    }
}

#[test]
fn markdown_preflight_accepts_interleaved_workspace_and_path_flags() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    fs::write(fixture.workspace.join("second.md"), "synthetic\n").unwrap();
    let mut args = fixture.args();
    args.truncate(10);
    args.extend([
        "--path".into(),
        "README.md".into(),
        "--workspace".into(),
        fixture.workspace.display().to_string(),
        "--path".into(),
        "second.md".into(),
    ]);
    fixture.pass(args);
}

#[test]
fn markdown_preflight_does_not_parse_workspace_contents_or_existing_bad_history() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    fs::write(fixture.workspace.join("README.md"), [0xff, 0x00, 0xfe]).unwrap();
    for filename in STORES {
        fs::write(
            fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename),
            b"not-json\xff\n",
        )
        .unwrap();
    }
    fixture.pass(fixture.args());
}

#[test]
fn markdown_preflight_rejects_flags_before_loading_missing_config() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    for flags in [
        vec![],
        vec!["--workspace"],
        vec!["--workspace", "relative", "--path", "README.md"],
        vec!["--workspace", "/tmp"],
        vec![
            "--workspace",
            "/tmp",
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
            "--path",
            "README.md",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
            "--approved",
            "true",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
            "--credential-source",
            "env",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
            "--max-iterations",
            "1",
        ],
        vec![
            "--workspace",
            "/tmp",
            "--path",
            "README.md",
            "--untrusted-sensitive-value",
            "untrusted-sensitive-value",
        ],
    ] {
        let mut args = fixture.args();
        args.truncate(10);
        args[2] = "/missing/config-untrusted-sensitive-value.json".into();
        args.extend(flags.into_iter().map(str::to_owned));
        fixture.reject(args, "steerbox: invalid preflight-markdown arguments");
    }
    for length in 0..8 {
        let mut args = fixture.args();
        args.truncate(2 + length);
        fixture.reject(args, "steerbox: invalid preflight-markdown arguments");
    }
}

#[test]
fn markdown_preflight_rejects_unsafe_relative_paths_before_loading_config() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    for path in [
        "",
        "../README.md",
        "/README.md",
        "./README.md",
        "dir//README.md",
        "dir/../README.md",
        "dir\\README.md",
        "bad\nREADME.md",
        "README.txt",
        "--untrusted-sensitive-value.md",
    ] {
        let mut args = fixture.args();
        args[2] = "/missing/config.json".into();
        args[13] = path.into();
        fixture.reject(args, "steerbox: invalid preflight-markdown arguments");
    }
}

#[test]
fn markdown_preflight_sanitizes_config_and_profile_failures() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args[2] = "/missing/untrusted-sensitive-value.json".into();
    fixture.reject(args, "steerbox: invalid preflight-markdown configuration");
    let mut args = fixture.args();
    args[7] = "untrusted-sensitive-value".into();
    fixture.reject(
        args,
        "steerbox: preflight-markdown scope or runtime check failed",
    );
    fs::write(
        fixture.config.config_path(),
        "{\"untrusted-sensitive-value\":bad-json",
    )
    .unwrap();
    fixture.reject(
        fixture.args(),
        "steerbox: invalid preflight-markdown configuration",
    );
}

#[test]
fn markdown_preflight_requires_existing_runtime_parent_without_creating_it() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    fs::remove_dir(fixture.config.events_path().parent().unwrap()).unwrap();
    fixture.reject(
        fixture.args(),
        "steerbox: preflight-markdown scope or runtime check failed",
    );
    assert!(!fixture.config.events_path().parent().unwrap().exists());
}

#[test]
fn markdown_preflight_sanitizes_unknown_config_field_and_invalid_opaque_reference() {
    for unknown_field in [true, false] {
        let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
        let mut value = serde_json::to_value(&fixture.config).unwrap();
        if unknown_field {
            value["untrusted-sensitive-value"] = serde_json::json!(true);
        } else {
            value["model_profiles"][0]["auth_ref"] =
                serde_json::json!("env:untrusted-sensitive-value=invalid");
        }
        fs::write(fixture.config.config_path(), value.to_string()).unwrap();
        fixture.reject(
            fixture.args(),
            "steerbox: invalid preflight-markdown configuration",
        );
    }
}

#[test]
fn markdown_preflight_rejects_all_unsafe_store_destinations_without_writes() {
    use std::os::unix::fs::symlink;
    for filename in STORES {
        for kind in ["directory", "symlink", "hardlink"] {
            let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
            let target = fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename);
            let original = fixture.config.data_root.join("protected.md");
            fs::write(&original, "protected synthetic fixture\n").unwrap();
            match kind {
                "directory" => fs::create_dir(&target).unwrap(),
                "symlink" => symlink(&original, &target).unwrap(),
                "hardlink" => fs::hard_link(&original, &target).unwrap(),
                _ => unreachable!(),
            }
            fixture.reject(
                fixture.args(),
                "steerbox: preflight-markdown scope or runtime check failed",
            );
        }
    }
}

#[test]
fn markdown_preflight_rejects_store_alias_without_parsing_existing_bytes() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    fs::write(fixture.config.events_path(), "not-json\n").unwrap();
    fs::hard_link(
        fixture.config.events_path(),
        fixture.config.model_calls_path(),
    )
    .unwrap();
    fixture.reject(
        fixture.args(),
        "steerbox: preflight-markdown scope or runtime check failed",
    );
}

#[test]
fn markdown_preflight_rejects_symlink_runtime_parent_without_creating_any_files() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let runtime = fixture.config.events_path().parent().unwrap().to_owned();
    fs::remove_dir(&runtime).unwrap();
    let alternate = fixture.config.data_root.join("alternate");
    fs::create_dir(&alternate).unwrap();
    symlink(alternate, &runtime).unwrap();
    fixture.reject(
        fixture.args(),
        "steerbox: preflight-markdown scope or runtime check failed",
    );
}

#[test]
fn markdown_preflight_rejects_symlink_workspace_root() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let alias = fixture.config.data_root.join("workspace-alias");
    symlink(&fixture.workspace, &alias).unwrap();
    let mut args = fixture.args();
    args[11] = alias.display().to_string();
    fixture.reject(
        args,
        "steerbox: preflight-markdown scope or runtime check failed",
    );
}

#[test]
fn markdown_preflight_rejects_missing_directory_symlink_and_hardlink_scope() {
    use std::os::unix::fs::symlink;
    for kind in [
        "missing",
        "directory",
        "symlink",
        "hardlink",
        "symlink-parent",
    ] {
        let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
        let mut args = fixture.args();
        args[13] = "other.md".into();
        let target = fixture.workspace.join("other.md");
        match kind {
            "missing" => (),
            "directory" => fs::create_dir(&target).unwrap(),
            "symlink" => symlink(fixture.workspace.join("README.md"), target).unwrap(),
            "hardlink" => fs::hard_link(fixture.workspace.join("README.md"), target).unwrap(),
            "symlink-parent" => {
                symlink(&fixture.workspace, fixture.workspace.join("alias")).unwrap();
                args[13] = "alias/README.md".into();
            }
            _ => unreachable!(),
        }
        fixture.reject(
            args,
            "steerbox: preflight-markdown scope or runtime check failed",
        );
    }
}

#[test]
fn markdown_preflight_rejects_both_runtime_workspace_nesting_directions_and_equality() {
    for relation in [
        "same",
        "runtime-inside-workspace",
        "workspace-inside-runtime",
    ] {
        let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
        let runtime = fixture.config.events_path().parent().unwrap().to_owned();
        let workspace = match relation {
            "same" => runtime,
            "runtime-inside-workspace" => fixture.config.data_root.clone(),
            "workspace-inside-runtime" => {
                let child = runtime.join("workspace");
                fs::create_dir(&child).unwrap();
                child
            }
            _ => unreachable!(),
        };
        fs::write(workspace.join("README.md"), "synthetic\n").unwrap();
        let mut args = fixture.args();
        args[11] = workspace.display().to_string();
        fixture.reject(
            args,
            "steerbox: preflight-markdown scope or runtime check failed",
        );
    }
}

#[test]
fn markdown_preflight_accepts_sibling_prefix_and_unicode_space_path() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let workspace = fixture.config.data_root.join(".steerbox-other");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("说明 文档.markdown"), "synthetic\n").unwrap();
    let mut args = fixture.args();
    args[11] = workspace.display().to_string();
    args[13] = "说明 文档.markdown".into();
    fixture.pass(args);
}

#[test]
fn markdown_preflight_accepts_256_paths_and_rejects_257_before_config_io() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args.truncate(12);
    for index in 0..256 {
        let path = format!("file-{index}.md");
        fs::write(fixture.workspace.join(&path), "synthetic\n").unwrap();
        args.extend(["--path".into(), path]);
    }
    fixture.pass(args.clone());
    args[2] = "/missing/config.json".into();
    args.extend(["--path".into(), "file-256.md".into()]);
    fixture.reject(args, "steerbox: invalid preflight-markdown arguments");
}

#[test]
fn markdown_execution_requires_scope_authorization_before_configuration_io() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args[1] = "provider-markdown".into();
    args[2] = "/missing/untrusted-sensitive-value.json".into();
    fixture.reject(
        args.clone(),
        "steerbox: invalid provider-markdown arguments",
    );
    for flags in [
        vec!["--authorize-markdown"],
        vec!["--authorize-markdown", "always"],
        vec![
            "--authorize-markdown",
            "once",
            "--authorize-markdown",
            "once",
        ],
        vec!["--authorize-markdown", "once", "--approved", "true"],
        vec!["--authorize-markdown", "once", "--max-iterations", "2"],
        vec![
            "--authorize-markdown",
            "once",
            "--credential-source",
            "file",
        ],
        vec![
            "--authorize-markdown",
            "once",
            "--credential-source",
            "env",
            "--credential-source",
            "env",
        ],
    ] {
        let mut invalid = args.clone();
        invalid.extend(flags.into_iter().map(str::to_owned));
        fixture.reject(invalid, "steerbox: invalid provider-markdown arguments");
    }
}

#[test]
fn markdown_execution_rejects_bad_runtime_before_opening_stores() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args[1] = "provider-markdown".into();
    args.extend(["--authorize-markdown".into(), "once".into()]);
    fs::create_dir(fixture.config.state_path()).unwrap();
    fixture.reject(
        args,
        "steerbox: provider-markdown scope or runtime check failed",
    );
}

#[test]
fn markdown_execution_sanitizes_configuration_failure_without_writes() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args[1] = "provider-markdown".into();
    args[2] = "/missing/untrusted-sensitive-value.json".into();
    args.extend(["--authorize-markdown".into(), "once".into()]);
    fixture.reject(args, "steerbox: invalid provider-markdown configuration");
}

#[test]
fn markdown_preflight_never_accepts_execution_authorization() {
    let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
    let mut args = fixture.args();
    args[2] = "/missing/config.json".into();
    args.extend(["--authorize-markdown".into(), "once".into()]);
    fixture.reject(args, "steerbox: invalid preflight-markdown arguments");
}

#[test]
fn markdown_execution_rechecks_all_destinations_and_workspace_runtime_separation() {
    use std::os::unix::fs::symlink;
    for filename in STORES {
        for kind in ["directory", "symlink", "hardlink"] {
            let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
            let target = fixture
                .config
                .events_path()
                .parent()
                .unwrap()
                .join(filename);
            let protected = fixture.config.data_root.join("protected.md");
            fs::write(&protected, "protected synthetic content\n").unwrap();
            match kind {
                "directory" => fs::create_dir(&target).unwrap(),
                "symlink" => symlink(&protected, &target).unwrap(),
                "hardlink" => fs::hard_link(&protected, &target).unwrap(),
                _ => unreachable!(),
            }
            let mut args = fixture.args();
            args[1] = "provider-markdown".into();
            args.extend(["--authorize-markdown".into(), "once".into()]);
            fixture.reject(
                args,
                "steerbox: provider-markdown scope or runtime check failed",
            );
        }
    }
    for relation in [
        "same",
        "runtime-inside-workspace",
        "workspace-inside-runtime",
    ] {
        let fixture = Fixture::new(OPENAI_COMPATIBLE_CHAT_PROTOCOL);
        let runtime = fixture.config.events_path().parent().unwrap().to_owned();
        let workspace = match relation {
            "same" => runtime,
            "runtime-inside-workspace" => fixture.config.data_root.clone(),
            "workspace-inside-runtime" => {
                let child = runtime.join("workspace");
                fs::create_dir(&child).unwrap();
                child
            }
            _ => unreachable!(),
        };
        fs::write(workspace.join("README.md"), "synthetic\n").unwrap();
        let mut args = fixture.args();
        args[1] = "provider-markdown".into();
        args[11] = workspace.display().to_string();
        args.extend(["--authorize-markdown".into(), "once".into()]);
        fixture.reject(
            args,
            "steerbox: provider-markdown scope or runtime check failed",
        );
    }
}

#[test]
fn markdown_execution_default_credentials_fail_closed_with_safe_lifecycle_only() {
    for protocol in [OPENAI_COMPATIBLE_CHAT_PROTOCOL, "anthropic-messages"] {
        let fixture = Fixture::new(protocol);
        let original = fs::read(fixture.workspace.join("README.md")).unwrap();
        let mut args = fixture.args();
        args[1] = "provider-markdown".into();
        args.extend(["--authorize-markdown".into(), "once".into()]);
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(args)
            .env_remove("SYNTHETIC_MARKDOWN_PREFLIGHT_NOT_CONFIGURED")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("steerbox: provider-markdown execution failed; audit or workspace may have changed; no automatic rollback\n"), "{stderr}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.contains("decision="));
        for line in stdout.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 3, "{line}");
            assert_eq!(fields[1], "bounded-markdown");
            assert!(["step", "tool", "model", "action", "loop"].contains(&fields[0]));
            assert!([
                "started",
                "progress",
                "completed",
                "failed",
                "paused",
                "resumed",
                "cancelled"
            ]
            .contains(&fields[2]));
        }
        assert_eq!(
            fs::read(fixture.workspace.join("README.md")).unwrap(),
            original
        );
        assert!(fixture.config.state_path().is_file());
        for marker in [
            "synthetic-hidden",
            "SYNTHETIC_MARKDOWN_PREFLIGHT_NOT_CONFIGURED",
            "untrusted-sensitive-value",
        ] {
            assert!(!stdout.contains(marker));
            assert!(!stderr.contains(marker));
        }
    }
}
