use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use steerbox::http_transport::{
    CredentialError, CredentialResolver, HttpProviderTransport, ResolvedCredential,
};
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderAdapterError, ProviderMessage, ProviderRequest,
    ProviderTransport, TransportError,
};

#[derive(Clone)]
struct FixtureResolver {
    result: Result<String, CredentialError>,
    refs: Arc<Mutex<Vec<String>>>,
}

impl FixtureResolver {
    fn credential(token: &str) -> Self {
        Self {
            result: Ok(token.to_owned()),
            refs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn rejecting() -> Self {
        Self {
            result: Err(CredentialError::Missing),
            refs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CredentialResolver for FixtureResolver {
    fn resolve(&self, auth_ref: &str) -> Result<ResolvedCredential, CredentialError> {
        self.refs
            .lock()
            .expect("resolver refs lock")
            .push(auth_ref.into());
        self.result.clone().and_then(ResolvedCredential::bearer)
    }
}

fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind localhost");
    let address = format!("http://{}", listener.local_addr().expect("local address"));
    (listener, address)
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes).expect("fixture request is UTF-8")
}

fn write_response(mut stream: TcpStream, status: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");
}

#[test]
fn localhost_transport_resolves_opaque_auth_ref_and_sends_bounded_json_request() {
    let (listener, endpoint) = listener();
    let request = Arc::new(Mutex::new(String::new()));
    let captured = Arc::clone(&request);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        *captured.lock().expect("request lock") = read_request(&mut stream);
        write_response(stream, "200 OK", "", "{\"ok\":true}");
    });
    let resolver = FixtureResolver::credential("fixture-token-not-for-logs");
    let refs = Arc::clone(&resolver.refs);
    let transport = HttpProviderTransport::new(resolver)
        .expect("transport")
        .allow_loopback_http_for_tests();
    let response = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            Some("env:FIXTURE_PROVIDER_KEY"),
            r#"{"model":"fixture","messages":[]}"#,
            Duration::from_secs(2),
            false,
        )
        .expect("localhost response");
    server.join().expect("server join");
    let request = request.lock().expect("request lock").clone();
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, r#"{"ok":true}"#);
    assert!(request.contains("POST /chat/completions HTTP/1.1"));
    assert!(request.contains("content-type: application/json"));
    assert!(request.contains("authorization: Bearer fixture-token-not-for-logs"));
    assert!(request.contains(r#"{"model":"fixture","messages":[]}"#));
    assert_eq!(
        refs.lock().expect("resolver refs lock").as_slice(),
        ["env:FIXTURE_PROVIDER_KEY"]
    );
    let debug = format!("{transport:?}");
    assert!(!debug.contains("fixture-token-not-for-logs"));
}

#[test]
fn production_policy_rejects_plain_http_before_connecting() {
    let resolver = FixtureResolver::rejecting();
    let transport = HttpProviderTransport::new(resolver).expect("transport");
    let error = transport
        .send(
            "http://127.0.0.1:1/chat/completions",
            None,
            "{}",
            Duration::from_secs(1),
            false,
        )
        .expect_err("plain HTTP requires explicit test policy");
    assert_eq!(error, TransportError::InvalidEndpoint);
}

#[test]
fn credential_failure_happens_before_network_request() {
    let (listener, endpoint) = listener();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests();
    let error = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            Some("env:MISSING"),
            "{}",
            Duration::from_millis(100),
            false,
        )
        .expect_err("missing credential");
    assert_eq!(error, TransportError::Credential);
    assert!(
        listener.accept().is_err(),
        "resolver failure must not connect"
    );
}

#[test]
fn redirect_is_rejected_without_following_or_sending_a_second_request() {
    let (listener, endpoint) = listener();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept redirect request");
        write_response(
            stream,
            "302 Found",
            "Location: http://127.0.0.1:9/redirected\r\n",
            "redirect",
        );
    });
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests();
    let error = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            None,
            "{}",
            Duration::from_secs(1),
            false,
        )
        .expect_err("redirect is not an accepted provider response");
    server.join().expect("server join");
    assert_eq!(error, TransportError::Redirect);
}

#[test]
fn oversized_response_is_rejected_before_body_is_returned() {
    let (listener, endpoint) = listener();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept size request");
        write_response(stream, "200 OK", "", "123456789");
    });
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests()
        .with_max_response_bytes(4);
    let error = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            None,
            "{}",
            Duration::from_secs(1),
            false,
        )
        .expect_err("response exceeds configured limit");
    server.join().expect("server join");
    assert_eq!(error, TransportError::ResponseTooLarge);
}

#[test]
fn timeout_and_streaming_are_explicitly_rejected_without_retry() {
    let (listener, endpoint) = listener();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept timeout request");
        thread::sleep(Duration::from_millis(150));
        drop(stream);
    });
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests();
    let error = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            None,
            "{}",
            Duration::from_millis(20),
            false,
        )
        .expect_err("timeout expected");
    assert_eq!(error, TransportError::Timeout);
    assert_eq!(
        transport
            .send(
                &format!("{endpoint}/chat/completions"),
                None,
                "{}",
                Duration::from_secs(1),
                true,
            )
            .expect_err("streaming is not implemented"),
        TransportError::UnsupportedStreaming
    );
    server.join().expect("server join");
}

fn adapter_request() -> ProviderRequest {
    ProviderRequest {
        model: "fixture-model".into(),
        messages: vec![ProviderMessage::user("fixture")],
        auth_ref: None,
        timeout_ms: 1_000,
        stream: false,
    }
}

fn run_status_case(status: &str, expected: ProviderAdapterError) {
    let (listener, endpoint) = listener();
    let status = status.to_owned();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept status request");
        write_response(stream, &status, "", "sensitive provider error body");
    });
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests();
    let adapter = OpenAiCompatibleAdapter::new("fixture-provider", &endpoint, transport);
    let error = adapter
        .complete(adapter_request())
        .expect_err("status must be classified");
    server.join().expect("server join");
    assert_eq!(error, expected);
    assert!(!error.to_string().contains("sensitive provider error body"));
}

#[test]
fn provider_statuses_are_classified_without_returning_error_body() {
    run_status_case("401 Unauthorized", ProviderAdapterError::Authentication);
    run_status_case("403 Forbidden", ProviderAdapterError::Permission);
    run_status_case("408 Request Timeout", ProviderAdapterError::Timeout);
    run_status_case("429 Too Many Requests", ProviderAdapterError::RateLimit);
    run_status_case("500 Internal Server Error", ProviderAdapterError::Upstream);
}

#[test]
fn malformed_and_empty_success_bodies_are_response_parse_errors_without_body_echo() {
    for body in ["sensitive malformed provider body", ""] {
        let (listener, endpoint) = listener();
        let server = thread::spawn({
            let body = body.to_owned();
            move || {
                let (stream, _) = listener.accept().expect("accept parse request");
                write_response(stream, "200 OK", "", &body);
            }
        });
        let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
            .expect("transport")
            .allow_loopback_http_for_tests();
        let adapter = OpenAiCompatibleAdapter::new("fixture-provider", &endpoint, transport);
        let error = adapter
            .complete(adapter_request())
            .expect_err("invalid completion body");
        server.join().expect("server join");
        assert_eq!(error, ProviderAdapterError::ResponseParse);
        assert!(!error.to_string().contains("sensitive"));
    }
}

#[test]
fn disconnected_provider_is_not_retried() {
    let (listener, endpoint) = listener();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let accepted = Arc::new(AtomicUsize::new(0));
    let server_accepted = Arc::clone(&accepted);
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    server_accepted.fetch_add(1, Ordering::SeqCst);
                    let _ = read_request(&mut stream);
                    drop(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    let transport = HttpProviderTransport::new(FixtureResolver::rejecting())
        .expect("transport")
        .allow_loopback_http_for_tests();
    let error = transport
        .send(
            &format!("{endpoint}/chat/completions"),
            None,
            "{}",
            Duration::from_secs(1),
            false,
        )
        .expect_err("disconnect must be a transport error");
    server.join().expect("server join");
    assert_eq!(error, TransportError::Network);
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}
