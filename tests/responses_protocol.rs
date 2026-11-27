use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::Duration;
use steerbox::endpoint_mode::EndpointMode;
use steerbox::model_adapter::DecisionOutputRejection;
use steerbox::provider_adapter::{
    GenerationRejectionEvidence, ModelEffectPolicy, OpenAiCompatibleAdapter, OpenAiWireProtocol,
    ProviderAdapterError, ProviderMessage, ProviderRequest, ReasoningEffort,
};
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunRequest};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::{event_log::EventLog, model_call_store::ModelCallStore};

#[derive(Default)]
struct Fixture {
    calls: RefCell<Vec<(String, Value)>>,
}

impl ProviderTransport for &Fixture {
    fn send(
        &self,
        endpoint: &str,
        auth: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert_eq!(auth, Some("env:SYNTHETIC_KEY"));
        assert!(!streaming);
        let body: Value = serde_json::from_str(body).unwrap();
        self.calls.borrow_mut().push((endpoint.to_owned(), body));
        Ok(TransportResponse::body(200, json!({
            "object": "response", "id": "resp-fixture", "model": "fixture-model", "status": "completed",
            "output": [{"type": "message", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": "{\"decision\":\"complete_loop\",\"summary\":\"fixture complete\"}"}]}],
            "usage": {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18}
        }).to_string()))
    }
}

fn config(protocol: &str, mode: &str, endpoint: &str) -> RuntimeConfig {
    RuntimeConfig::from_json_str(&json!({
        "schema_version": 1, "project_ref": "responses-fixture", "data_root": "/temp/steerbox/responses-fixture",
        "model_profiles": [{"model_id": "fixture-model", "provider": "relay-alias", "endpoint": endpoint,
            "endpoint_protocol": protocol, "endpoint_mode": mode, "auth_ref": "env:SYNTHETIC_KEY", "capabilities": ["text"]}]
    }).to_string()).expect("explicit protocol and address mode accepted")
}

#[test]
fn responses_config_runs_through_existing_controller_and_audit() {
    let config = config(
        "openai-responses",
        "base_url",
        "https://relay.example.test/custom",
    );
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("model-calls.jsonl");
    let fixture = Fixture::default();
    let run = ForegroundProviderRun::prepare(
        &config,
        ProviderRunRequest {
            project_id: "responses-fixture".into(),
            goal_id: "goal-fixture".into(),
            loop_run_id: "loop-fixture".into(),
            task_run_id: "task-fixture".into(),
            model_id: "fixture-model".into(),
            objective: "Inspect supplied observation".into(),
            observation: "Synthetic fixture only".into(),
        },
    )
    .unwrap();
    let result = run
        .run_with_transport(
            &fixture,
            EventLog::open(dir.path().join("events.jsonl")).unwrap(),
            ModelCallStore::open(&calls).unwrap(),
            steerbox::stream_reporter::StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(result.loop_status, steerbox::domain::LoopStatus::Completed);
    let captured = fixture.calls.borrow();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0, "https://relay.example.test/custom/responses");
    assert_eq!(captured[0].1["store"], false);
    assert_eq!(captured[0].1["stream"], false);
    assert_eq!(captured[0].1["input"][0]["role"], "system");
    assert_eq!(captured[0].1["input"][1]["role"], "developer");
    assert!(captured[0].1.get("messages").is_none());
    assert!(captured[0].1.get("tools").is_none());
    assert!(!serde_json::to_string(&captured[0].1)
        .unwrap()
        .contains("SYNTHETIC_KEY"));
    let audit = std::fs::read_to_string(&calls).unwrap();
    assert!(audit.contains("sha256:"));
    assert!(audit.contains("\"input_tokens\":11"));
}

#[test]
fn full_url_mode_is_explicit_and_round_trips() {
    let config = config(
        "openai-compatible-chat",
        "full_url",
        "https://relay.example.test/arbitrary-inference",
    );
    let encoded = serde_json::to_value(config).unwrap();
    assert_eq!(encoded["model_profiles"][0]["endpoint_mode"], "full_url");
}

struct ReplyFixture {
    reply: TransportResponse,
    calls: RefCell<Vec<(String, Value)>>,
}

impl ReplyFixture {
    fn new(body: Value) -> Self {
        Self {
            reply: TransportResponse::body(200, body.to_string()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl ProviderTransport for &ReplyFixture {
    fn send(
        &self,
        endpoint: &str,
        _: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert!(!streaming);
        self.calls
            .borrow_mut()
            .push((endpoint.into(), serde_json::from_str(body).unwrap()));
        Ok(self.reply.clone())
    }
}

fn response_body() -> Value {
    json!({"object":"response", "id":"resp-fixture", "model":"fixture-model", "status":"completed",
        "output":[{"type":"message", "role":"assistant", "status":"completed", "content":[{"type":"output_text", "text":"fixture result"}]}],
        "usage":{"input_tokens":11, "output_tokens":7, "total_tokens":18}})
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "fixture-model".into(),
        messages: vec![ProviderMessage::user("synthetic observation")],
        auth_ref: None,
        timeout_ms: 1000,
        stream: false,
    }
}

fn responses_adapter(fixture: &ReplyFixture) -> OpenAiCompatibleAdapter<&ReplyFixture> {
    OpenAiCompatibleAdapter::new("relay-alias", "https://relay.example.test/custom/", fixture)
        .with_wire_protocol(OpenAiWireProtocol::Responses)
}

#[test]
fn exact_full_url_is_preserved_for_chat_and_responses_including_trailing_slash() {
    for protocol in [
        OpenAiWireProtocol::ChatCompletions,
        OpenAiWireProtocol::Responses,
    ] {
        let body = match protocol {
            OpenAiWireProtocol::Responses => response_body(),
            OpenAiWireProtocol::ChatCompletions => {
                json!({"id":"chat-fixture", "model":"fixture-model", "choices":[{"message":{"content":"fixture"},"finish_reason":"stop"}]})
            }
        };
        let fixture = ReplyFixture::new(body);
        let endpoint = "https://relay.example.test/nonstandard-inference/";
        let adapter = OpenAiCompatibleAdapter::new("relay-alias", endpoint, &fixture)
            .with_wire_protocol(protocol)
            .with_endpoint_mode(EndpointMode::FullUrl);
        adapter.complete(request()).unwrap();
        assert_eq!(fixture.calls.borrow()[0].0, endpoint);
    }
}

#[test]
fn prepared_requests_cannot_be_rebound_to_protocol_address_mode_or_exact_url() {
    let fixture = ReplyFixture::new(response_body());
    let prepared = responses_adapter(&fixture)
        .prepare_complete(request())
        .unwrap();
    let wrong_protocol = OpenAiCompatibleAdapter::new(
        "relay-alias",
        "https://relay.example.test/custom/",
        &fixture,
    );
    let wrong_mode = responses_adapter(&fixture).with_endpoint_mode(EndpointMode::FullUrl);
    let wrong_url = OpenAiCompatibleAdapter::new(
        "relay-alias",
        "https://relay.example.test/elsewhere/",
        &fixture,
    )
    .with_wire_protocol(OpenAiWireProtocol::Responses);
    for adapter in [wrong_protocol, wrong_mode, wrong_url] {
        assert!(matches!(
            adapter.complete_prepared(&prepared),
            Err(ProviderAdapterError::InvalidRequest { .. })
        ));
    }
    assert!(fixture.calls.borrow().is_empty());
}

#[test]
fn responses_streaming_and_unknown_roles_fail_before_transport() {
    let fixture = ReplyFixture::new(response_body());
    let mut streaming = request();
    streaming.stream = true;
    assert!(matches!(
        responses_adapter(&fixture).stream(streaming),
        Err(ProviderAdapterError::InvalidRequest { .. })
    ));
    let mut unknown = request();
    unknown.messages[0].role = "tool".into();
    assert!(matches!(
        responses_adapter(&fixture).complete(unknown),
        Err(ProviderAdapterError::InvalidRequest { .. })
    ));
    assert!(fixture.calls.borrow().is_empty());
}

#[test]
fn reasoning_effort_is_nested_and_usage_is_mapped_without_reasoning_content() {
    use steerbox::adaptive_routing::EffectLevel;
    let mut body = response_body();
    body["output"].as_array_mut().unwrap().insert(0, json!({"type":"reasoning", "summary":[{"type":"summary_text","text":"internal-fixture-not-for-context"}]}));
    body["output"][1]["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"output_text", "text":" continued"}));
    let fixture = ReplyFixture::new(body);
    let policy = ModelEffectPolicy::new(
        "fixture-model",
        [(EffectLevel::Strong, ReasoningEffort::High)].into(),
    )
    .unwrap();
    let adapter = responses_adapter(&fixture)
        .with_effect_policy(policy, EffectLevel::Strong)
        .unwrap();
    let mut req = request();
    req.messages
        .push(ProviderMessage::assistant("prior answer"));
    let result = adapter.complete(req).unwrap();
    assert_eq!(result.text, "fixture result continued");
    assert_eq!(result.finish_reason.as_deref(), Some("stop"));
    let usage = result.usage.unwrap();
    assert_eq!(usage.prompt_tokens, Some(11));
    assert_eq!(usage.completion_tokens, Some(7));
    assert_eq!(usage.total_tokens, Some(18));
    let captured = fixture.calls.borrow();
    assert_eq!(captured[0].0, "https://relay.example.test/custom/responses");
    assert_eq!(captured[0].1["reasoning"]["effort"], "high");
    assert!(captured[0].1.get("reasoning_effort").is_none());
    assert_eq!(captured[0].1["input"][1]["role"], "assistant");
    assert_eq!(captured[0].1["input"][1]["content"], "prior answer");
}

#[test]
fn responses_wire_rejections_preserve_known_usage_without_private_content() {
    for (status, details, output, reason) in [
        (
            "completed",
            Value::Null,
            json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"private-wire-fixture"}]}]),
            "UnsafeTermination",
        ),
        (
            "completed",
            Value::Null,
            json!([{"type":"function_call","arguments":"private-wire-fixture"}]),
            "UnsafeTermination",
        ),
        (
            "private-wire-fixture",
            Value::Null,
            json!([]),
            "UnsafeTermination",
        ),
        (
            "incomplete",
            json!({"reason":"content_filter"}),
            json!([]),
            "UnsafeTermination",
        ),
        (
            "incomplete",
            json!({"reason":"max_output_tokens"}),
            json!([]),
            "OutputTruncated",
        ),
    ] {
        let mut body = response_body();
        body["status"] = json!(status);
        body["incomplete_details"] = details;
        body["output"] = output;
        let fixture = ReplyFixture::new(body);
        let error = responses_adapter(&fixture).complete(request()).unwrap_err();
        let debug = format!("{error:?}");
        assert!(debug.contains("GenerationRejected"), "{debug}");
        assert!(debug.contains(reason), "{debug}");
        assert!(debug.contains("input_tokens: Some(11)"), "{debug}");
        assert!(debug.contains("output_tokens: Some(7)"), "{debug}");
        assert!(!debug.contains("private-wire-fixture"));
        assert!(!error.to_string().contains("private-wire-fixture"));
        assert_eq!(fixture.calls.borrow().len(), 1);
    }
}

#[test]
fn responses_wire_eof_has_no_invented_usage_and_truncation_cannot_mask_tools() {
    let fixture = ReplyFixture {
        reply: TransportResponse::body(200, "{\"object\":"),
        calls: RefCell::new(Vec::new()),
    };
    assert_eq!(
        responses_adapter(&fixture).complete(request()).unwrap_err(),
        ProviderAdapterError::GenerationRejected(GenerationRejectionEvidence {
            reason: DecisionOutputRejection::TerminationUnconfirmed,
            input_tokens: None,
            output_tokens: None,
        })
    );
    let mut body = response_body();
    body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    body["output"] = json!([{"type":"function_call","arguments":"private-wire-fixture"}]);
    body["usage"] = Value::Null;
    let fixture = ReplyFixture::new(body);
    let error = responses_adapter(&fixture).complete(request()).unwrap_err();
    assert_eq!(
        error,
        ProviderAdapterError::GenerationRejected(GenerationRejectionEvidence {
            reason: DecisionOutputRejection::UnsafeTermination,
            input_tokens: None,
            output_tokens: None,
        })
    );
    assert!(!format!("{error:?}").contains("private-wire-fixture"));
}

#[test]
fn responses_duplicate_block_type_cannot_override_refusal() {
    assert_duplicate_response_fields_rejected(
        "completed",
        "null",
        r#"[{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","type":"output_text","text":"{\"decision\":\"complete_loop\",\"summary\":\"private-duplicate-fixture\"}"}]}]"#,
    );
}

#[test]
fn responses_duplicate_item_type_cannot_override_tool_call() {
    assert_duplicate_response_fields_rejected(
        "completed",
        "null",
        r#"[{"type":"function_call","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"decision\":\"complete_loop\",\"summary\":\"private-duplicate-fixture\"}"}]}]"#,
    );
}

#[test]
fn responses_duplicate_incomplete_reason_cannot_override_filter() {
    assert_duplicate_response_fields_rejected(
        "incomplete",
        r#"{"reason":"content_filter","reason":"max_output_tokens"}"#,
        "[]",
    );
}

fn assert_duplicate_response_fields_rejected(status: &str, details: &str, output: &str) {
    let body = format!(
        r#"{{"object":"response","id":"fixture","model":"fixture","status":"{status}","incomplete_details":{details},"output":{output},"usage":{{"input_tokens":11,"output_tokens":7,"total_tokens":18}}}}"#
    );
    let fixture = ReplyFixture {
        reply: TransportResponse::body(200, body),
        calls: RefCell::new(Vec::new()),
    };
    let error = responses_adapter(&fixture).complete(request()).unwrap_err();
    assert_eq!(error, ProviderAdapterError::ResponseParse);
    assert!(!format!("{error:?}").contains("private-duplicate-fixture"));
    assert!(!error.to_string().contains("private-duplicate-fixture"));
    assert_eq!(fixture.calls.borrow().len(), 1);
}

#[test]
fn responses_reject_partial_refusal_tools_unknown_and_ambiguous_outputs() {
    let mut invalid = Vec::new();
    for status in ["incomplete", "failed", "queued", "in_progress", "cancelled"] {
        let mut body = response_body();
        body["status"] = json!(status);
        invalid.push((body, Some(DecisionOutputRejection::UnsafeTermination)));
    }
    for (field, value, reason) in [
        ("object", json!("chat.completion"), None),
        ("id", json!(""), None),
        ("model", json!(""), None),
        (
            "error",
            json!({"message":"sensitive-fixture"}),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
        (
            "incomplete_details",
            json!({"reason":"max_output_tokens"}),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
    ] {
        let mut body = response_body();
        body[field] = value;
        invalid.push((body, reason));
    }
    for (item, reason) in [
        (
            json!({"type":"function_call","name":"dangerous-fixture"}),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
        (
            json!({"type":"future-item"}),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
        (json!({"type":"reasoning"}), None),
    ] {
        let mut body = response_body();
        body["output"] = json!([item]);
        invalid.push((body, reason));
    }
    for (content, reason) in [
        (json!([]), None),
        (
            json!([{"type":"refusal","refusal":"fixture"}]),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
        (json!([{"type":"output_text","text":"  "}]), None),
        (json!([{"type":"output_text","text":17}]), None),
        (
            json!([{"type":"image","data":"fixture"}]),
            Some(DecisionOutputRejection::UnsafeTermination),
        ),
    ] {
        let mut body = response_body();
        body["output"][0]["content"] = content;
        invalid.push((body, reason));
    }
    let mut duplicate = response_body();
    duplicate["output"] = json!([
        duplicate["output"][0].clone(),
        duplicate["output"][0].clone()
    ]);
    invalid.push((duplicate, Some(DecisionOutputRejection::UnsafeTermination)));
    let mut unfinished_message = response_body();
    unfinished_message["output"][0]["status"] = json!("in_progress");
    invalid.push((
        unfinished_message,
        Some(DecisionOutputRejection::UnsafeTermination),
    ));
    for value in [json!(-1), json!("11"), json!(1.5), Value::Null] {
        let mut body = response_body();
        body["usage"]["input_tokens"] = value;
        invalid.push((body, None));
    }
    for (body, reason) in invalid {
        let fixture = ReplyFixture::new(body);
        let error = responses_adapter(&fixture).complete(request()).unwrap_err();
        let expected = match reason {
            Some(reason) => ProviderAdapterError::GenerationRejected(GenerationRejectionEvidence {
                reason,
                input_tokens: Some(11),
                output_tokens: Some(7),
            }),
            None => ProviderAdapterError::ResponseParse,
        };
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("sensitive-fixture"));
    }
}

#[test]
fn responses_http_errors_and_malformed_json_do_not_echo_upstream_bodies() {
    for (status, expected) in [
        (401, ProviderAdapterError::Authentication),
        (403, ProviderAdapterError::Permission),
        (429, ProviderAdapterError::RateLimit),
        (500, ProviderAdapterError::Upstream),
        (200, ProviderAdapterError::ResponseParse),
    ] {
        let fixture = ReplyFixture {
            reply: TransportResponse::body(status, "private-fixture-body-not-json"),
            calls: RefCell::new(Vec::new()),
        };
        let error = responses_adapter(&fixture).complete(request()).unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error}").contains("private-fixture-body"));
    }
}

#[test]
fn default_address_mode_is_omitted_and_invalid_mode_is_rejected() {
    let default_config = config("openai-responses", "base_url", "https://relay.example.test");
    let mut encoded = serde_json::to_value(&default_config).unwrap();
    assert!(encoded["model_profiles"][0].get("endpoint_mode").is_none());
    assert_eq!(
        RuntimeConfig::from_json_str(&encoded.to_string()).unwrap(),
        default_config
    );
    encoded["model_profiles"][0]["endpoint_mode"] = json!("auto-detect");
    assert!(RuntimeConfig::from_json_str(&encoded.to_string()).is_err());
}

#[test]
fn responses_fixed_effect_is_supported_but_automatic_upgrade_is_not() {
    let config = config("openai-responses", "base_url", "https://relay.example.test");
    let mut encoded = serde_json::to_value(&config).unwrap();
    encoded["model_profiles"][0]["effect"] =
        json!({"selected_level":"fast", "mapping":{"fast":"low","strong":"high"}});
    assert!(RuntimeConfig::from_json_str(&encoded.to_string()).is_ok());
    encoded["model_profiles"][0]["effect"]["automatic_upgrade"] = json!(true);
    encoded["model_profiles"][0]["effect"]["maximum_level"] = json!("strong");
    assert!(RuntimeConfig::from_json_str(&encoded.to_string()).is_err());
}

fn run_request() -> ProviderRunRequest {
    ProviderRunRequest {
        project_id: "responses-fixture".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "fixture-model".into(),
        objective: "Inspect README.md".into(),
        observation: "Synthetic observation".into(),
    }
}

#[test]
fn responses_readonly_and_markdown_cli_preflight_accept_explicit_protocol() {
    std::fs::create_dir_all("/temp/steerbox").unwrap();
    let dir = tempfile::Builder::new()
        .prefix("responses-preflight-")
        .tempdir_in("/temp/steerbox")
        .unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("README.md"), "fixture\n").unwrap();
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version":1,"project_ref":dir.path().file_name().unwrap().to_str().unwrap(),"data_root":dir.path(),
        "model_profiles":[{"model_id":"fixture-model","provider":"relay-alias",
            "endpoint":"https://relay.example.test/custom-wire/","endpoint_protocol":"openai-responses",
            "endpoint_mode":"full_url","auth_ref":"env:SYNTHETIC_KEY","capabilities":["text"]}]
    }).to_string()).unwrap();
    config.write_json(config.config_path()).unwrap();
    // CLI preflight requires an initialized store directory but must create no
    // history records itself. Keep that existing runtime gate intact.
    std::fs::create_dir(config.events_path().parent().unwrap()).unwrap();
    for command in ["preflight-readonly", "preflight-markdown"] {
        let mut cli = std::process::Command::new(env!("CARGO_BIN_EXE_steerbox"));
        cli.args(["loop", command])
            .arg(config.config_path())
            .args([
                "project",
                "goal",
                "loop",
                "task",
                "fixture-model",
                "Inspect README.md",
                "Synthetic observation",
            ])
            .arg("--workspace")
            .arg(&workspace)
            .args(["--path", "README.md"])
            .env_remove("SYNTHETIC_KEY");
        if command == "preflight-readonly" {
            cli.args(["--max-iterations", "1"]);
        }
        let output = cli.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("preflight=passed"));
        assert!(!config.events_path().exists());
        assert!(!config.model_calls_path().exists());
    }
}

fn action_response(workspace: &std::path::Path, path: &str) -> Value {
    let mut body = response_body();
    body["output"][0]["content"][0]["text"] = json!(json!({
        "decision":"complete_loop", "summary":"fixture edit",
        "action":{"schema_version":1, "action_id":"fixture-edit", "tool_id":"edit_markdown",
            "parameters":{"path":path,"expected":"before\n","replacement":"after\n"},
            "path_scope":{"root":workspace,"allowed_paths":["README.md"]},
            "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":path}}}
    })
    .to_string());
    body
}

#[test]
fn responses_markdown_uses_existing_allowlist_snapshot_and_verification_gates() {
    use steerbox::domain::{HumanSteeringMode, LoopStatus};
    use steerbox::provider_runner::BoundedMarkdownRunConfig;
    for path in ["README.md", "OTHER.md"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("README.md"), "before\n").unwrap();
        std::fs::write(root.join("OTHER.md"), "before\n").unwrap();
        let config = config(
            "openai-responses",
            "full_url",
            "https://relay.example.test/custom-inference/",
        );
        let transport = ReplyFixture::new(action_response(root, path));
        let calls = ModelCallStore::open(root.join("calls.jsonl")).unwrap();
        let result = ForegroundProviderRun::prepare_markdown(
            &config,
            run_request(),
            BoundedMarkdownRunConfig {
                workspace: root.into(),
                allowed_paths: vec!["README.md".into()],
                checkpoint_path: root.join("checkpoints.jsonl"),
                trace_path: root.join("traces.jsonl"),
                side_effect_path: root.join("effects.jsonl"),
                approved: true,
                human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
            },
        )
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(root.join("events.jsonl")).unwrap(),
            calls.clone(),
            steerbox::stream_reporter::StreamReporter::new(Vec::new()),
        );
        assert_eq!(
            transport.calls.borrow()[0].0,
            "https://relay.example.test/custom-inference/"
        );
        if path == "README.md" {
            assert_eq!(result.unwrap().loop_status, LoopStatus::Completed);
            assert_eq!(
                std::fs::read_to_string(root.join("README.md")).unwrap(),
                "after\n"
            );
            assert_eq!(calls.read_all().unwrap()[0].verification_status, "passed");
            assert!(!std::fs::read_to_string(root.join("checkpoints.jsonl"))
                .unwrap()
                .is_empty());
        } else {
            assert!(result.is_err());
            assert_eq!(
                std::fs::read_to_string(root.join("README.md")).unwrap(),
                "before\n"
            );
            assert!(calls.read_all().unwrap().is_empty());
            assert_eq!(calls.read_failures().unwrap().len(), 1);
        }
        assert_eq!(
            std::fs::read_to_string(root.join("OTHER.md")).unwrap(),
            "before\n"
        );
    }
}

#[test]
fn responses_readonly_runner_completes_without_mutation_and_honors_zero_budget() {
    use steerbox::provider_runner::BoundedReadOnlyRunConfig;
    for zero_budget in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("README.md"), "before\n").unwrap();
        let mut config = config(
            "openai-responses",
            "base_url",
            "https://relay.example.test/custom",
        );
        if zero_budget {
            config.reported_token_limit = Some(0);
        }
        let transport = Fixture::default();
        let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
        let result = ForegroundProviderRun::prepare_readonly(
            &config,
            run_request(),
            BoundedReadOnlyRunConfig {
                workspace: workspace.clone(),
                allowed_paths: vec!["README.md".into()],
                max_iterations: 1,
            },
        )
        .unwrap()
        .run_with_transport(
            &transport,
            EventLog::open(dir.path().join("events.jsonl")).unwrap(),
            calls.clone(),
            steerbox::stream_reporter::StreamReporter::new(Vec::new()),
        );
        if zero_budget {
            assert!(result.is_err());
            assert!(transport.calls.borrow().is_empty());
            assert!(calls.read_all().unwrap().is_empty());
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(transport.calls.borrow().len(), 1);
            assert_eq!(calls.read_all().unwrap().len(), 1);
        }
        assert_eq!(
            std::fs::read_to_string(workspace.join("README.md")).unwrap(),
            "before\n"
        );
    }
}
