use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use steerbox::anthropic_adapter::{AnthropicAdapter, AnthropicTransport};
use steerbox::http_transport::{
    CredentialError, CredentialResolver, HttpAnthropicTransport, ResolvedCredential,
};
use steerbox::provider_adapter::{ProviderMessage, ProviderRequest};

#[derive(Clone)]
struct FixtureResolver {
    refs: Arc<Mutex<Vec<String>>>,
}

impl CredentialResolver for FixtureResolver {
    fn resolve(&self, auth_ref: &str) -> Result<ResolvedCredential, CredentialError> {
        self.refs
            .lock()
            .expect("resolver refs lock")
            .push(auth_ref.to_owned());
        ResolvedCredential::bearer("fixture-anthropic-key")
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer).expect("request reads");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes).expect("request is UTF-8")
}

#[test]
fn anthropic_http_transport_sends_protocol_headers_without_auth_body_leak() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener binds");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("listener address")
    );
    let captured = Arc::new(Mutex::new(String::new()));
    let captured_server = Arc::clone(&captured);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        *captured_server.lock().expect("request lock") = read_request(&mut stream);
        let body = r#"{"id":"msg_fixture","type":"message","role":"assistant","model":"claude-fixture","content":[{"type":"text","text":"{\"decision\":\"complete_loop\",\"summary\":\"done\"}"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("response writes");
    });

    let refs = Arc::new(Mutex::new(Vec::new()));
    let transport = HttpAnthropicTransport::new(FixtureResolver {
        refs: Arc::clone(&refs),
    })
    .expect("transport builds")
    .allow_loopback_http_for_tests();
    let adapter = AnthropicAdapter::new(
        "anthropic",
        &endpoint,
        transport,
        Some("env:ANTHROPIC_KEY".into()),
    )
    .with_model_id("claude-fixture");
    let response = adapter
        .complete(ProviderRequest {
            model: "claude-fixture".into(),
            messages: vec![ProviderMessage::user("fixture")],
            auth_ref: None,
            timeout_ms: 1_000,
            stream: false,
        })
        .expect("anthropic request succeeds");
    server.join().expect("server joins");

    assert_eq!(response.request_id, "msg_fixture");
    let request = captured.lock().expect("request lock").clone();
    assert!(request.contains("POST /v1/messages HTTP/1.1"));
    assert!(request.contains("x-api-key: fixture-anthropic-key"));
    assert!(request.contains("anthropic-version: 2023-06-01"));
    assert!(!request.contains("authorization:"));
    assert_eq!(
        refs.lock().expect("resolver refs lock").as_slice(),
        ["env:ANTHROPIC_KEY"]
    );
    assert!(!format!("{:?}", adapter).contains("fixture-anthropic-key"));
}

#[test]
fn anthropic_http_transport_rejects_plain_http_without_test_opt_in() {
    let transport = HttpAnthropicTransport::new(FixtureResolver {
        refs: Arc::new(Mutex::new(Vec::new())),
    })
    .expect("transport builds");
    let error = transport
        .send(
            "http://127.0.0.1:1/v1/messages",
            &steerbox::anthropic_adapter::AnthropicHeaders {
                api_key_ref: Some("env:ANTHROPIC_KEY".into()),
                anthropic_version: "2023-06-01".into(),
            },
            "{}",
            Duration::from_secs(1),
            false,
        )
        .expect_err("plain HTTP is rejected");
    assert_eq!(
        error,
        steerbox::anthropic_adapter::AnthropicTransportError::InvalidEndpoint
    );
}
