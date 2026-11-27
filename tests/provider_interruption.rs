use serde_json::json;
use std::cell::Cell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::event_log::EventLog;
use steerbox::http_transport::{
    CredentialError, CredentialResolver, HttpAnthropicTransport, HttpProviderTransport,
    ResolvedCredential,
};
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderAdapterError, ProviderMessage, ProviderRequest,
    ProviderStreamEvent, ProviderTransport, TransportError, TransportResponse,
};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunRequest};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::StreamReporter;

struct ScriptedTransport {
    body: String,
    calls: Cell<usize>,
}

impl ProviderTransport for &ScriptedTransport {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.calls.set(self.calls.get() + 1);
        Ok(TransportResponse::body(200, self.body.clone()))
    }
}

fn chunk(content: Option<&str>, reason: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id": "fixture-id", "model": "fixture-model",
        "choices": [{"delta": {"content": content}, "finish_reason": reason}]})
    )
}

fn parse_stream(body: String) -> Result<Vec<ProviderStreamEvent>, ProviderAdapterError> {
    let transport = ScriptedTransport {
        body,
        calls: Cell::new(0),
    };
    let result =
        OpenAiCompatibleAdapter::new("relay-fixture", "https://example.test/v1", &transport)
            .stream(ProviderRequest {
                model: "fixture-model".into(),
                messages: vec![ProviderMessage::user("synthetic request")],
                auth_ref: None,
                timeout_ms: 1000,
                stream: true,
            });
    assert_eq!(transport.calls.get(), 1, "no implicit retry");
    result
}

#[test]
fn truncated_stream_never_returns_partial_text_as_success() {
    let partial = chunk(Some("partial-private-fixture"), None);
    let finish = chunk(None, Some("stop"));
    for body in [
        partial.clone(),
        format!("{partial}{finish}"),
        format!("{partial}data: [DONE]\n\n"),
    ] {
        let error = parse_stream(body).expect_err("EOF or DONE alone must not confirm completion");
        assert_eq!(
            error.class(),
            steerbox::provider_adapter::ProviderErrorClass::ResponseParse
        );
        assert!(!format!("{error:?} {error}").contains("partial-private-fixture"));
    }
}

#[test]
fn reasoning_only_stream_cannot_be_treated_as_a_usable_answer() {
    let reasoning = format!(
        "data: {}\n\n",
        json!({"id": "fixture-id", "model": "fixture-model",
        "choices": [{"delta": {"reasoning_content": "private-reasoning-fixture"}, "finish_reason": null}]})
    );
    let body = format!("{reasoning}{}data: [DONE]\n\n", chunk(None, Some("stop")));
    let error = parse_stream(body).expect_err("a finish event without text is not a usable answer");
    assert!(!format!("{error:?} {error}").contains("private-reasoning-fixture"));
}

#[test]
fn terminal_stream_must_not_hide_later_errors_or_conflicting_chunks() {
    let complete = chunk(Some("fixture answer"), Some("stop"));
    for body in [
        format!("{complete}data: [DONE]\n\ndata: {{\"error\":\"private-error-fixture\"}}\n\n"),
        format!(
            "{complete}{}data: [DONE]\n\n",
            chunk(Some("unexpected continuation"), None)
        ),
        format!(
            "{}data: [DONE]\n\n",
            chunk(Some("fixture answer"), Some(" "))
        ),
        format!(
            "{}{complete}data: [DONE]\n\n",
            chunk(Some("other identity"), None).replacen("fixture-id", "other-id", 1)
        ),
    ] {
        assert!(
            parse_stream(body).is_err(),
            "conflicting stream must fail closed"
        );
    }
}

#[test]
fn completed_unicode_stream_accepts_comments_and_trailing_usage_chunk() {
    let body = format!(
        ": keepalive\n\n{}: heartbeat\n\n{}data: {}\n\ndata: [DONE]\n\n: closed\n",
        chunk(Some("中文项目检查完成"), None),
        chunk(None, Some("stop")),
        json!({"id":"fixture-id", "model":"fixture-model", "choices":[],
            "usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"extra_counter":1}})
    );
    assert_eq!(
        parse_stream(body).unwrap(),
        vec![
            ProviderStreamEvent::delta("fixture-id", "fixture-model", "中文项目检查完成"),
            ProviderStreamEvent::completed("fixture-id", "fixture-model", "stop"),
        ]
    );
}

#[derive(Clone, Copy)]
struct SyntheticResolver;

impl CredentialResolver for SyntheticResolver {
    fn resolve(&self, _: &str) -> Result<ResolvedCredential, CredentialError> {
        ResolvedCredential::bearer("synthetic-local-only")
    }
}

fn heartbeat_server() -> (String, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                return 0;
            }
            request.push(byte[0]);
            assert!(request.len() < 16 * 1024);
        }
        // Deliberately never finish this body; frequent bytes must not extend its total deadline.
        if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n").is_err() { return 0; }
        let mut heartbeats = 0;
        for _ in 0..100 {
            if stream.write_all(b": keepalive\n\n").is_err() {
                break;
            }
            heartbeats += 1;
            thread::sleep(Duration::from_millis(10));
        }
        heartbeats
    });
    (endpoint, server)
}

#[test]
fn chat_body_timeout_is_classified_and_not_extended_by_heartbeat_bytes() {
    let (endpoint, server) = heartbeat_server();
    let transport = HttpProviderTransport::new(SyntheticResolver)
        .unwrap()
        .allow_loopback_http_for_tests();
    let started = Instant::now();
    let error = transport
        .send(&endpoint, None, "{}", Duration::from_millis(200), false)
        .unwrap_err();
    let elapsed = started.elapsed();
    let heartbeats = server.join().unwrap();
    assert!(
        heartbeats > 1,
        "must exercise an active body, not only a header timeout"
    );
    assert!(elapsed < Duration::from_secs(2));
    assert_eq!(error, TransportError::Timeout);
}

#[test]
fn messages_body_timeout_is_classified_and_not_extended_by_heartbeat_bytes() {
    let (endpoint, server) = heartbeat_server();
    let transport = HttpAnthropicTransport::new(SyntheticResolver)
        .unwrap()
        .allow_loopback_http_for_tests();
    let started = Instant::now();
    let error = transport
        .send(
            &endpoint,
            &AnthropicHeaders {
                api_key_ref: Some("env:SYNTHETIC".into()),
                anthropic_version: "2023-06-01".into(),
            },
            "{}",
            Duration::from_millis(200),
            false,
        )
        .unwrap_err();
    let elapsed = started.elapsed();
    let heartbeats = server.join().unwrap();
    assert!(heartbeats > 1);
    assert!(elapsed < Duration::from_secs(2));
    assert_eq!(error, AnthropicTransportError::Timeout);
}

#[test]
fn truncated_success_body_is_audited_as_failure_under_unicode_directory_without_retry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("中文项目 空格");
    std::fs::create_dir(&root).unwrap();
    let calls = ModelCallStore::open(root.join("model-calls.jsonl")).unwrap();
    let events_path = root.join("events.jsonl");
    let transport = ScriptedTransport {
        body: "{\"id\":\"private-partial-body\",".into(),
        calls: Cell::new(0),
    };
    let config = RuntimeConfig::from_json_str(&json!({"schema_version":1,"project_ref":"interruption-fixture",
        "data_root":"/temp/steerbox/interruption-fixture", "model_profiles":[{"model_id":"fixture-model",
        "provider":"relay-fixture","endpoint":"https://example.test/v1","endpoint_protocol":"openai-compatible-chat",
        "auth_ref":"env:SYNTHETIC","capabilities":["text"]}]}).to_string()).unwrap();
    let run = ForegroundProviderRun::prepare(
        &config,
        ProviderRunRequest {
            project_id: "fixture-project".into(),
            goal_id: "fixture-goal".into(),
            loop_run_id: "fixture-loop".into(),
            task_run_id: "fixture-task".into(),
            model_id: "fixture-model".into(),
            objective: "Inspect synthetic observation".into(),
            observation: "中文项目路径检查".into(),
        },
    )
    .unwrap();
    let error = run
        .run_with_transport(
            &transport,
            EventLog::open(&events_path).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap_err();
    assert_eq!(transport.calls.get(), 1);
    assert!(calls.read_all().unwrap().is_empty());
    let failures = calls.read_failures().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure_class, "response_unmappable");
    assert!(failures[0]
        .input_context_hash
        .as_deref()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(failures[0].input_tokens, None);
    assert_eq!(failures[0].output_tokens, None);
    let events = EventLog::open(events_path).unwrap().read_all().unwrap();
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
    let audit = format!(
        "{error} {} {}",
        serde_json::to_string(&failures).unwrap(),
        serde_json::to_string(&events).unwrap()
    );
    assert!(!audit.contains("private-partial-body"));
    assert!(!audit.contains("env:SYNTHETIC"));
}
