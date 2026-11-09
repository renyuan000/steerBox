#![cfg(unix)]

use serde_json::json;
use std::os::unix::fs::symlink;
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::HumanSteeringMode;
use steerbox::event_log::EventLog;
use steerbox::http_transport::{CredentialError, CredentialResolver, ResolvedCredential};
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{
    BoundedMarkdownProviderRun, BoundedMarkdownRunConfig, ForegroundProviderRun, ProviderRunError,
    ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, StreamReporter};

struct Fixture {
    directory: tempfile::TempDir,
    workspace: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let runtime = directory.path().join("runtime");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&runtime).unwrap();
        fs::write(workspace.join("README.md"), "before\n").unwrap();
        Self {
            directory,
            workspace,
            runtime,
        }
    }

    fn run(&self) -> BoundedMarkdownRunConfig {
        BoundedMarkdownRunConfig {
            workspace: self.workspace.clone(),
            allowed_paths: vec!["README.md".into()],
            checkpoint_path: self.runtime.join("checkpoints.jsonl"),
            trace_path: self.runtime.join("traces.jsonl"),
            side_effect_path: self.runtime.join("effects.jsonl"),
            approved: true,
            human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
        }
    }

    fn prepare(&self) -> BoundedMarkdownProviderRun {
        self.prepare_with("openai-compatible", None, self.run())
            .unwrap()
    }

    fn prepare_with(
        &self,
        provider: &str,
        limit: Option<u64>,
        run: BoundedMarkdownRunConfig,
    ) -> Result<BoundedMarkdownProviderRun, ProviderRunError> {
        let config = RuntimeConfig::from_json_str(&json!({
            "schema_version":1,"project_ref":"markdown-gates","data_root":"/temp/steerbox/markdown-gates",
            "reported_token_limit":limit,
            "model_profiles":[{"model_id":"fixture-model","provider":provider,
                "endpoint":"https://fixture.example.test/v1","auth_ref":"secret-ref:synthetic-markdown",
                "capabilities":["text"]}]
        }).to_string()).unwrap();
        ForegroundProviderRun::prepare_markdown(
            &config,
            ProviderRunRequest {
                project_id: "project".into(),
                goal_id: "goal".into(),
                loop_run_id: "loop".into(),
                task_run_id: "task".into(),
                model_id: "fixture-model".into(),
                objective: "Update authorized Markdown".into(),
                observation: "before\n".into(),
            },
            run,
        )
    }

    fn execute(
        &self,
        prepared: BoundedMarkdownProviderRun,
        transport: &Transport,
    ) -> Result<(), ProviderRunError> {
        prepared
            .run_with_transport(
                transport,
                EventLog::open(self.runtime.join("events.jsonl")).unwrap(),
                ModelCallStore::open(self.runtime.join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            )
            .map(|_| ())
    }

    fn replace_workspace(&self) {
        fs::rename(&self.workspace, self.directory.path().join("original")).unwrap();
        fs::create_dir(&self.workspace).unwrap();
        fs::write(self.workspace.join("README.md"), "before\n").unwrap();
    }
}

impl AnthropicTransport for &Transport<'_> {
    fn send(
        &self,
        _: &str,
        _: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        let response = ProviderTransport::send(self, "", None, "", Duration::ZERO, false).unwrap();
        let openai: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        Ok(TransportResponse::body(200, json!({"id":"anthropic-synthetic","type":"message","role":"assistant",
            "model":"fixture-model","content":[{"type":"text","text":openai["choices"][0]["message"]["content"]}],
            "stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}).to_string()))
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

const DESTINATIONS: [&str; 11] = [
    "events.jsonl",
    "checkpoints.jsonl",
    "traces.jsonl",
    "effects.jsonl",
    "tool-calls.jsonl",
    "tool-calls.jsonl.lock",
    "calls.jsonl",
    "calls.failures.jsonl",
    "calls.jsonl.lock",
    "calls.failures.jsonl.lock",
    "state.json",
];

#[test]
fn markdown_pure_preopen_checks_every_destination_without_writes() {
    for filename in DESTINATIONS {
        for kind in ["directory", "symlink", "hardlink"] {
            let fixture = Fixture::new();
            let prepared = fixture.prepare();
            let protected = fixture.directory.path().join("protected.txt");
            fs::write(&protected, "synthetic protected fixture\n").unwrap();
            let target = fixture.runtime.join(filename);
            match kind {
                "directory" => fs::create_dir(&target).unwrap(),
                "symlink" => symlink(&protected, &target).unwrap(),
                "hardlink" => fs::hard_link(&protected, &target).unwrap(),
                _ => unreachable!(),
            }
            let before = snapshot(fixture.directory.path());
            assert!(
                matches!(
                    prepared.validate_runtime_paths(
                        &fixture.runtime.join("events.jsonl"),
                        &fixture.runtime.join("calls.jsonl"),
                        Some(&fixture.runtime.join("state.json"))
                    ),
                    Err(ProviderRunError::InvalidRequest)
                ),
                "{filename}/{kind}"
            );
            assert_eq!(snapshot(fixture.directory.path()), before);
        }
    }
}

#[test]
fn markdown_pure_checks_reject_normalized_aliases_and_missing_parents() {
    for filename in DESTINATIONS {
        if filename == "traces.jsonl" {
            continue;
        }
        let fixture = Fixture::new();
        let mut run = fixture.run();
        run.trace_path = fixture.runtime.join(".").join(filename);
        let before = snapshot(fixture.directory.path());
        let result = fixture
            .prepare_with("openai-compatible", None, run)
            .and_then(|prepared| {
                prepared
                    .validate_runtime_paths(
                        &fixture.runtime.join("events.jsonl"),
                        &fixture.runtime.join("calls.jsonl"),
                        Some(&fixture.runtime.join("state.json")),
                    )
                    .map(|_| ())
            });
        assert!(
            matches!(result, Err(ProviderRunError::InvalidRequest)),
            "{filename}"
        );
        assert_eq!(snapshot(fixture.directory.path()), before);
    }
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let before = snapshot(fixture.directory.path());
    assert!(prepared
        .validate_runtime_paths(
            &fixture.runtime.join("missing/events.jsonl"),
            &fixture.runtime.join("calls.jsonl"),
            None
        )
        .is_err());
    assert_eq!(snapshot(fixture.directory.path()), before);
}

#[test]
fn markdown_scope_rejects_invalid_or_aliased_authorized_files_without_writes() {
    for invalid in [
        "missing",
        "symlink",
        "hardlink",
        "duplicate",
        "traversal",
        "too-many",
        "directory",
    ] {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        match invalid {
            "missing" => run.allowed_paths = vec!["missing.md".into()],
            "symlink" => {
                symlink(
                    fixture.workspace.join("README.md"),
                    fixture.workspace.join("alias.md"),
                )
                .unwrap();
                run.allowed_paths = vec!["alias.md".into()];
            }
            "hardlink" => fs::hard_link(
                fixture.workspace.join("README.md"),
                fixture.directory.path().join("alias.md"),
            )
            .unwrap(),
            "duplicate" => run.allowed_paths.push("README.md".into()),
            "traversal" => run.workspace = fixture.workspace.join("../workspace"),
            "too-many" => run.allowed_paths = vec!["README.md".into(); 257],
            "directory" => {
                fs::create_dir(fixture.workspace.join("dir.md")).unwrap();
                run.allowed_paths = vec!["dir.md".into()];
            }
            _ => unreachable!(),
        }
        let before = snapshot(fixture.directory.path());
        assert!(
            matches!(
                fixture.prepare_with("openai-compatible", None, run),
                Err(ProviderRunError::InvalidRequest)
            ),
            "{invalid}"
        );
        assert_eq!(snapshot(fixture.directory.path()), before);
    }
}

#[test]
fn markdown_frozen_check_survives_consumption_and_does_not_freeze_file_contents() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let before = snapshot(fixture.directory.path());
    let check = prepared
        .validate_runtime_paths(
            &fixture.runtime.join("events.jsonl"),
            &fixture.runtime.join("calls.jsonl"),
            Some(&fixture.runtime.join("state.json")),
        )
        .unwrap();
    check.revalidate().unwrap();
    assert_eq!(snapshot(fixture.directory.path()), before);
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: false,
    };
    fixture.execute(prepared, &transport).unwrap();
    check.revalidate().unwrap();
    fixture.replace_workspace();
    assert!(matches!(
        check.revalidate(),
        Err(ProviderRunError::InvalidRequest)
    ));
}

#[test]
fn markdown_both_protocols_preserve_failure_usage_and_never_edit_replaced_workspace() {
    for provider in ["openai-compatible", "anthropic"] {
        for replace in [false, true] {
            let fixture = Fixture::new();
            let prepared = fixture.prepare_with(provider, None, fixture.run()).unwrap();
            let events = EventLog::open(fixture.runtime.join("events.jsonl")).unwrap();
            let calls = ModelCallStore::open(fixture.runtime.join("calls.jsonl")).unwrap();
            let transport = Transport {
                fixture: &fixture,
                calls: Cell::new(0),
                replace_during_response: replace,
            };
            let result = if provider == "anthropic" {
                prepared.run_with_anthropic_transport(
                    &transport,
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            } else {
                prepared.run_with_transport(
                    &transport,
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            };
            assert_eq!(result.is_ok(), !replace);
            assert_eq!(transport.calls.get(), 1);
            assert_eq!(
                fs::read(fixture.workspace.join("README.md")).unwrap(),
                if replace {
                    b"before\n".as_slice()
                } else {
                    b"after\n".as_slice()
                }
            );
            if replace {
                let failures = calls.read_failures().unwrap();
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].input_tokens, Some(3));
                assert_eq!(failures[0].output_tokens, Some(2));
                assert!(calls.read_all().unwrap().is_empty());
                assert!(!EventLog::open(fixture.runtime.join("events.jsonl"))
                    .unwrap()
                    .read_all()
                    .unwrap()
                    .iter()
                    .any(|e| e.event_type == "model_action_started"));
                assert!(!fixture.runtime.join("checkpoints.jsonl").exists());
                assert!(!fixture.runtime.join("effects.jsonl").exists());
            }
        }
    }
}

struct RejectingResolver(Arc<AtomicUsize>);
impl CredentialResolver for RejectingResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }
}

#[test]
fn markdown_resolver_dispatches_both_protocols_and_gates_invalid_scope_and_zero_tokens() {
    for provider in ["openai-compatible", "anthropic"] {
        for mode in ["valid", "replaced", "invalid-lock", "zero-tokens"] {
            let fixture = Fixture::new();
            let prepared = fixture
                .prepare_with(
                    provider,
                    if mode == "zero-tokens" { Some(0) } else { None },
                    fixture.run(),
                )
                .unwrap();
            if mode == "replaced" {
                fixture.replace_workspace();
            }
            if mode == "invalid-lock" {
                fs::create_dir(fixture.runtime.join("tool-calls.jsonl.lock")).unwrap();
            }
            let resolutions = Arc::new(AtomicUsize::new(0));
            let result = prepared.run_with_resolver(
                RejectingResolver(resolutions.clone()),
                EventLog::open(fixture.runtime.join("events.jsonl")).unwrap(),
                ModelCallStore::open(fixture.runtime.join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            );
            assert!(result.is_err());
            assert!(!matches!(
                result,
                Err(ProviderRunError::UnsupportedProtocol)
            ));
            assert_eq!(
                resolutions.load(Ordering::SeqCst),
                usize::from(mode == "valid"),
                "{provider}/{mode}"
            );
            assert_eq!(
                fs::read(fixture.workspace.join("README.md")).unwrap(),
                b"before\n"
            );
        }
    }
}

struct SwapBeforeProvider {
    workspace: PathBuf,
    original: PathBuf,
    swapped: bool,
}

impl ProgressSink for SwapBeforeProvider {
    fn report(&mut self, _: ProgressEvent) -> std::io::Result<()> {
        if !self.swapped {
            fs::rename(&self.workspace, &self.original)?;
            fs::create_dir(&self.workspace)?;
            fs::write(self.workspace.join("README.md"), "before\n")?;
            self.swapped = true;
        }
        Ok(())
    }
}

#[test]
fn markdown_checks_scope_at_actual_model_boundary_after_progress_callbacks() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        let prepared = fixture.prepare_with(provider, None, fixture.run()).unwrap();
        let transport = Transport {
            fixture: &fixture,
            calls: Cell::new(0),
            replace_during_response: false,
        };
        let sink = SwapBeforeProvider {
            workspace: fixture.workspace.clone(),
            original: fixture.directory.path().join("original"),
            swapped: false,
        };
        let events = EventLog::open(fixture.runtime.join("events.jsonl")).unwrap();
        let calls = ModelCallStore::open(fixture.runtime.join("calls.jsonl")).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_with_anthropic_transport(&transport, events, calls.clone(), sink)
        } else {
            prepared.run_with_transport(&transport, events, calls.clone(), sink)
        };
        assert!(result.is_err());
        assert_eq!(transport.calls.get(), 0);
        assert_eq!(
            fs::read(fixture.workspace.join("README.md")).unwrap(),
            b"before\n"
        );
        let failures = calls.read_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].input_context_hash, None);
        assert_eq!(failures[0].input_tokens, None);
    }
}

struct InvalidResponseTransport<'a>(Transport<'a>);
impl ProviderTransport for &InvalidResponseTransport<'_> {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        let response = ProviderTransport::send(&&self.0, "", None, "", Duration::ZERO, false)?;
        let mut wire: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        wire["choices"][0]["message"]["content"] = "{synthetic-invalid-decision".into();
        Ok(TransportResponse::body(200, wire.to_string()))
    }
}
impl AnthropicTransport for &InvalidResponseTransport<'_> {
    fn send(
        &self,
        _: &str,
        headers: &AnthropicHeaders,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        let response = AnthropicTransport::send(&&self.0, "", headers, "", Duration::ZERO, false)?;
        let mut wire: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        wire["content"][0]["text"] = "{synthetic-invalid-decision".into();
        Ok(TransportResponse::body(200, wire.to_string()))
    }
}

#[test]
fn markdown_provider_error_with_replaced_scope_keeps_existing_safe_usage_evidence() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        let prepared = fixture.prepare_with(provider, None, fixture.run()).unwrap();
        let transport = InvalidResponseTransport(Transport {
            fixture: &fixture,
            calls: Cell::new(0),
            replace_during_response: true,
        });
        let events = EventLog::open(fixture.runtime.join("events.jsonl")).unwrap();
        let calls = ModelCallStore::open(fixture.runtime.join("calls.jsonl")).unwrap();
        let result = if provider == "anthropic" {
            prepared.run_with_anthropic_transport(
                &transport,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        } else {
            prepared.run_with_transport(
                &transport,
                events,
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
        };
        assert!(result.is_err());
        assert_eq!(transport.0.calls.get(), 1);
        assert_eq!(
            fs::read(fixture.workspace.join("README.md")).unwrap(),
            b"before\n"
        );
        let failures = calls.read_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].input_context_hash.is_some());
        assert_eq!(failures[0].input_tokens, Some(3));
        assert_eq!(failures[0].output_tokens, Some(2));
        assert!(
            !fs::read_to_string(fixture.runtime.join("calls.failures.jsonl"))
                .unwrap()
                .contains("synthetic-invalid-decision")
        );
    }
}

#[test]
fn markdown_resolver_rejects_reused_identity_without_resolving_again() {
    for provider in ["openai-compatible", "anthropic"] {
        let fixture = Fixture::new();
        let resolutions = Arc::new(AtomicUsize::new(0));
        for attempt in 0..2 {
            let prepared = fixture.prepare_with(provider, None, fixture.run()).unwrap();
            let result = prepared.run_with_resolver(
                RejectingResolver(resolutions.clone()),
                EventLog::open(fixture.runtime.join("events.jsonl")).unwrap(),
                ModelCallStore::open(fixture.runtime.join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            );
            assert!(result.is_err());
            if attempt == 1 {
                assert!(matches!(
                    result,
                    Err(ProviderRunError::DuplicateRunIdentity)
                ));
            }
            assert_eq!(resolutions.load(Ordering::SeqCst), 1);
        }
    }
}
struct Transport<'a> {
    fixture: &'a Fixture,
    calls: Cell<usize>,
    replace_during_response: bool,
}

impl ProviderTransport for &Transport<'_> {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.calls.set(self.calls.get() + 1);
        if self.replace_during_response {
            self.fixture.replace_workspace();
        }
        let envelope = json!({"decision":"complete_loop","summary":"Synthetic update","action":{
            "schema_version":1,"action_id":"edit-1","tool_id":"edit_markdown",
            "parameters":{"path":"README.md","expected":"before\n","replacement":"after\n"},
            "path_scope":{"root":self.fixture.workspace,"allowed_paths":["README.md"]},
            "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}
        }});
        Ok(TransportResponse::body(
            200,
            json!({"id":"synthetic-markdown-call","model":"fixture-model","choices":[{"message":{"content":envelope.to_string()},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":2}})
            .to_string(),
        ))
    }
}

#[test]
fn markdown_valid_control_reaches_verified_edit() {
    let fixture = Fixture::new();
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: false,
    };
    fixture.execute(fixture.prepare(), &transport).unwrap();
    assert_eq!(transport.calls.get(), 1);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"after\n"
    );
}

#[test]
fn markdown_rejects_unrelated_hardlinked_audit_before_provider() {
    let fixture = Fixture::new();
    let protected = fixture.directory.path().join("protected.txt");
    fs::write(&protected, "synthetic protected bytes\n").unwrap();
    fs::hard_link(&protected, fixture.runtime.join("traces.jsonl")).unwrap();
    let prepared = fixture.prepare();
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: false,
    };
    let result = fixture.execute(prepared, &transport);
    assert!(result.is_err(), "unexpected acceptance");
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read(&protected).unwrap(),
        b"synthetic protected bytes\n"
    );
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn markdown_rejects_tool_lock_alias_before_provider() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fs::hard_link(
        fixture.workspace.join("README.md"),
        fixture.runtime.join("tool-calls.jsonl.lock"),
    )
    .unwrap();
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: false,
    };
    assert!(fixture.execute(prepared, &transport).is_err());
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn markdown_rejects_replaced_workspace_before_provider() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.replace_workspace();
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: false,
    };
    assert!(fixture.execute(prepared, &transport).is_err());
    assert_eq!(transport.calls.get(), 0);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn markdown_rejects_workspace_replacement_during_provider_without_edit() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let transport = Transport {
        fixture: &fixture,
        calls: Cell::new(0),
        replace_during_response: true,
    };
    assert!(fixture.execute(prepared, &transport).is_err());
    assert_eq!(transport.calls.get(), 1);
    assert_eq!(
        fs::read(fixture.workspace.join("README.md")).unwrap(),
        b"before\n"
    );
    assert_eq!(
        fs::read(fixture.directory.path().join("original/README.md")).unwrap(),
        b"before\n"
    );
}
