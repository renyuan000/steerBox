use serde_json::json;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;
use steerbox::domain::LoopStatus;
use steerbox::event_log::EventLog;
use steerbox::model_adapter::ModelDecision;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunRequest};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::StreamReporter;

struct Reply(String);

impl steerbox::anthropic_adapter::AnthropicTransport for Sequence {
    fn send(
        &self,
        endpoint: &str,
        _: &steerbox::anthropic_adapter::AnthropicHeaders,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, steerbox::anthropic_adapter::AnthropicTransportError> {
        ProviderTransport::send(self, endpoint, None, body, timeout, streaming).map_err(|error| {
            match error {
                TransportError::Timeout => {
                    steerbox::anthropic_adapter::AnthropicTransportError::Timeout
                }
                _ => steerbox::anthropic_adapter::AnthropicTransportError::Network,
            }
        })
    }
}

impl ProviderTransport for Reply {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        Ok(TransportResponse::body(200, self.0.clone()))
    }
}

fn config() -> RuntimeConfig {
    RuntimeConfig::from_json_str(&json!({"schema_version":1,"project_ref":"recovery-fixture",
        "data_root":"/temp/steerbox/recovery-fixture","model_profiles":[{
        "model_id":"fixture-model","provider":"relay","endpoint":"https://example.test/v1",
        "endpoint_protocol":"openai-compatible-chat","auth_ref":"env:SYNTHETIC","capabilities":["text"]}]}).to_string()).unwrap()
}

fn request() -> ProviderRunRequest {
    ProviderRunRequest {
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        model_id: "fixture-model".into(),
        objective: "Review observation only".into(),
        observation: "synthetic observation".into(),
    }
}

fn chat(reason: Option<&str>, content: &str) -> String {
    json!({"id":"reply-id","model":"fixture-model","choices":[{"message":{"content":content},
        "finish_reason":reason}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}).to_string()
}

#[test]
fn valid_json_cannot_override_truncated_or_unconfirmed_generation() {
    for reason in [
        Some("length"),
        None,
        Some("content_filter"),
        Some("unknown-terminal"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
        let event_path = dir.path().join("events.jsonl");
        let result = ForegroundProviderRun::prepare(&config(), request())
            .unwrap()
            .run_with_transport(
                Reply(chat(
                    reason,
                    r#"{"decision":"complete_loop","summary":"synthetic complete"}"#,
                )),
                EventLog::open(&event_path).unwrap(),
                calls.clone(),
                StreamReporter::new(Vec::new()),
            );
        assert!(
            result.is_err(),
            "unconfirmed generation must not succeed: {reason:?}"
        );
        assert!(calls.read_all().unwrap().is_empty());
        assert_eq!(calls.read_failures().unwrap().len(), 1);
        assert!(!EventLog::open(&event_path)
            .unwrap()
            .read_all()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "loop_completed"));
    }
}

#[derive(Clone)]
struct Sequence {
    replies: Rc<RefCell<VecDeque<Result<TransportResponse, TransportError>>>>,
    requests: Rc<RefCell<Vec<String>>>,
}

impl ProviderTransport for Sequence {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        body: &str,
        timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert!(!streaming);
        assert!(timeout <= Duration::from_secs(30));
        self.requests.borrow_mut().push(body.into());
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("no hidden extra calls")
    }
}

fn sequence(replies: Vec<Result<TransportResponse, TransportError>>) -> Sequence {
    Sequence {
        replies: Rc::new(RefCell::new(replies.into())),
        requests: Rc::new(RefCell::new(Vec::new())),
    }
}

fn answer(decision: &str) -> Result<TransportResponse, TransportError> {
    Ok(TransportResponse::body(
        200,
        chat(
            Some("stop"),
            &json!({"decision":decision,"summary":"fixture answer"}).to_string(),
        ),
    ))
}

#[test]
fn transient_observation_recovers_with_separate_audited_calls_and_child_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
    let events_path = dir.path().join("events.jsonl");
    let transport = sequence(vec![Err(TransportError::Timeout), answer("complete_loop")]);
    let outcome = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(&events_path).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(outcome.decision, ModelDecision::CompleteLoop);
    assert_eq!(outcome.loop_status, LoopStatus::Completed);
    assert_eq!(transport.requests.borrow().len(), 2);
    assert_eq!(calls.read_all().unwrap().len(), 1);
    assert_eq!(calls.read_failures().unwrap().len(), 1);
    let events = EventLog::open(events_path).unwrap().read_all().unwrap();
    let repair = events
        .iter()
        .find(|event| event.event_type == "model_recovery_planned")
        .unwrap();
    assert!(repair.attempt_id.is_some());
    assert_eq!(repair.payload["gap"], "transient_provider");
    let completed = events
        .iter()
        .find(|event| event.event_type == "loop_completed")
        .unwrap();
    assert_ne!(
        completed.attempt_id,
        events
            .iter()
            .find(|event| event.event_type == "model_call_failed")
            .unwrap()
            .attempt_id
    );
}

#[test]
fn schema_repair_sends_only_fixed_gap_and_original_observation() {
    let dir = tempfile::tempdir().unwrap();
    let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
    let transport = sequence(vec![
        Ok(TransportResponse::body(
            200,
            chat(Some("stop"), "private-broken-output"),
        )),
        answer("complete_loop"),
    ]);
    ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(dir.path().join("events.jsonl")).unwrap(),
            calls.clone(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    let requests = transport.requests.borrow();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains("recovery_gap"));
    assert!(requests[1].contains("decision_schema"));
    assert!(requests[1].contains("synthetic observation"));
    assert!(!requests[1].contains("private-broken-output"));
    let failure = calls.read_failures().unwrap();
    assert_eq!(failure[0].input_tokens, Some(3));
    assert_eq!(failure[0].output_tokens, Some(2));
}

#[test]
fn repeated_outage_stops_after_two_calls_without_false_completion() {
    let dir = tempfile::tempdir().unwrap();
    let transport = sequence(vec![
        Err(TransportError::Network),
        Err(TransportError::Network),
    ]);
    let events_path = dir.path().join("events.jsonl");
    let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
    assert!(
        ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(&events_path).unwrap(),
                calls.clone(),
                StreamReporter::new(Vec::new())
            )
            .is_err()
    );
    assert_eq!(transport.requests.borrow().len(), 2);
    assert_eq!(calls.read_failures().unwrap().len(), 2);
    let events = EventLog::open(events_path).unwrap().read_all().unwrap();
    assert!(events.iter().any(|event| event.event_type == "loop_failed"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "loop_completed"));
}

#[test]
fn valid_stop_decisions_never_trigger_recovery_or_get_rewritten() {
    for (decision, expected_status) in [
        ("complete_loop", LoopStatus::Completed),
        ("request_human", LoopStatus::Running),
        ("continue_observation", LoopStatus::Running),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let transport = sequence(vec![answer(decision)]);
        let outcome = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(dir.path().join("events.jsonl")).unwrap(),
                ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            )
            .unwrap();
        assert_eq!(transport.requests.borrow().len(), 1);
        assert_eq!(outcome.loop_status, expected_status);
        assert_eq!(serde_json::to_value(outcome.decision).unwrap(), decision);
    }
}

#[test]
fn permission_refusal_and_unknown_termination_never_retry() {
    for reply in [TransportResponse::body(401,"private-credential-error"),
        TransportResponse::body(200,chat(Some("content_filter"),r#"{"decision":"complete_loop","summary":"fixture"}"#)),
        TransportResponse::body(200,json!({"id":"r","model":"fixture-model","choices":[{"message":{"content":null},"finish_reason":"content_filter"}]}).to_string()),
        TransportResponse::body(200,json!({"id":"r","model":"fixture-model","choices":[{"message":{"content":null},"finish_reason":"tool_calls"}]}).to_string()),
        TransportResponse::body(200,chat(Some("unknown-terminal"),r#"{"decision":"complete_loop","summary":"fixture"}"#)),
        TransportResponse::body(200,json!({"id":"reply","model":"fixture-model","choices":[{"finish_reason":"stop","message":{"refusal":"private-refusal","content":null}}]}).to_string())] {
        let dir = tempfile::tempdir().unwrap();
        let transport = sequence(vec![Ok(reply)]);
        let error = ForegroundProviderRun::prepare_recovering(&config(),request(),30_000).unwrap()
            .run_with_transport(transport.clone(),EventLog::open(dir.path().join("events.jsonl")).unwrap(),ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),StreamReporter::new(Vec::new())).unwrap_err();
        assert_eq!(transport.requests.borrow().len(),1);
        assert!(!error.to_string().contains("private-"));
    }
}

#[test]
fn strict_usage_budget_stops_recovery_without_resetting_known_or_unknown_costs() {
    for (limit, reply, expected_sends) in [
        (0, Err(TransportError::Timeout), 0),
        (100, Err(TransportError::Timeout), 1),
        (
            5,
            Ok(TransportResponse::body(
                200,
                chat(Some("stop"), "broken decision"),
            )),
            1,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config();
        cfg.reported_token_limit = Some(limit);
        let transport = sequence(vec![reply]);
        let events_path = dir.path().join("events.jsonl");
        let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
        assert!(
            ForegroundProviderRun::prepare_recovering(&cfg, request(), 30_000)
                .unwrap()
                .run_with_transport(
                    transport.clone(),
                    EventLog::open(&events_path).unwrap(),
                    calls.clone(),
                    StreamReporter::new(Vec::new())
                )
                .is_err()
        );
        assert_eq!(transport.requests.borrow().len(), expected_sends);
        assert_eq!(calls.read_failures().unwrap().len(), expected_sends);
        assert!(calls.read_all().unwrap().is_empty());
        let events = EventLog::open(events_path).unwrap().read_all().unwrap();
        assert!(!events
            .iter()
            .any(|e| e.event_type == "model_recovery_planned" || e.event_type == "loop_completed"));
    }
}

#[test]
fn output_limit_recovers_once_even_without_text_and_accounts_for_both_calls() {
    for content in [
        json!(null),
        json!("{\"decision\":\"complete_loop\",\"summary\":\"fixture\"}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config();
        cfg.reported_token_limit = Some(10);
        let body = json!({"id":"r","model":"fixture-model","choices":[{"message":{"content":content},"finish_reason":"length"}],"usage":{"prompt_tokens":3,"completion_tokens":2}});
        let transport = sequence(vec![
            Ok(TransportResponse::body(200, body.to_string())),
            answer("complete_loop"),
        ]);
        let outcome = ForegroundProviderRun::prepare_recovering(&cfg, request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(dir.path().join("events.jsonl")).unwrap(),
                ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            )
            .unwrap();
        assert_eq!(transport.requests.borrow().len(), 2);
        let budget = outcome.reported_token_budget().unwrap();
        assert_eq!(budget.known_total_tokens(), 10);
        assert_eq!(budget.records().len(), 2);
    }
}

#[test]
fn wire_eof_recovers_but_unknown_wire_shapes_do_not() {
    for (body, recover) in [
        (r#"{"id":"unfinished""#, true),
        ("not-json", false),
        ("{}", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let transport = sequence(vec![
            Ok(TransportResponse::body(200, body)),
            answer("complete_loop"),
        ]);
        let result = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(dir.path().join("events.jsonl")).unwrap(),
                ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new()),
            );
        assert_eq!(result.is_ok(), recover);
        assert_eq!(
            transport.requests.borrow().len(),
            if recover { 2 } else { 1 }
        );
    }
}

#[test]
fn explicit_nontext_stop_signals_keep_safe_usage_and_never_retry() {
    for message in [
        json!({"content":null,"refusal":"private-refusal"}),
        json!({"content":null,"tool_calls":[{"arguments":"private-tool"}]}),
        json!({"content":"{\"decision\":\"complete_loop\",\"summary\":\"fixture\"}","function_call":{"arguments":"private-tool"}}),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let body = json!({"id":"r","model":"fixture-model","choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}});
        let transport = sequence(vec![Ok(TransportResponse::body(200, body.to_string()))]);
        let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
        let error = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(dir.path().join("events.jsonl")).unwrap(),
                calls.clone(),
                StreamReporter::new(Vec::new()),
            )
            .unwrap_err();
        assert_eq!(transport.requests.borrow().len(), 1);
        let failures = calls.read_failures().unwrap();
        assert_eq!(failures[0].input_tokens, Some(3));
        assert_eq!(failures[0].output_tokens, Some(2));
        assert!(!format!("{error:?} {failures:?}").contains("private-"));
    }
}

#[test]
fn empty_tool_list_is_not_mistaken_for_a_tool_request() {
    let dir = tempfile::tempdir().unwrap();
    let body = json!({"id":"r","model":"fixture-model","choices":[{"message":{"content":"{\"decision\":\"complete_loop\",\"summary\":\"fixture\"}","tool_calls":[]},"finish_reason":"stop"}]});
    let transport = sequence(vec![Ok(TransportResponse::body(200, body.to_string()))]);
    let result = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(dir.path().join("events.jsonl")).unwrap(),
            ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        )
        .unwrap();
    assert_eq!(result.loop_status, LoopStatus::Completed);
    assert_eq!(transport.requests.borrow().len(), 1);
}

#[test]
fn observation_recovery_never_executes_proposed_actions() {
    let dir = tempfile::tempdir().unwrap();
    let action = json!({"decision":"continue_observation","summary":"fixture", "action":{
        "schema_version":1,"action_id":"action-1","tool_id":"git_status","parameters":{"scope":"workspace"},
        "path_scope":{"root":"/workspace","allowed_paths":["README.md"]},
        "verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}}});
    let transport = sequence(vec![Ok(TransportResponse::body(
        200,
        chat(Some("stop"), &action.to_string()),
    ))]);
    let events_path = dir.path().join("events.jsonl");
    assert!(
        ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport.clone(),
                EventLog::open(&events_path).unwrap(),
                ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
                StreamReporter::new(Vec::new())
            )
            .is_err()
    );
    assert_eq!(transport.requests.borrow().len(), 1);
    let events = EventLog::open(events_path).unwrap().read_all().unwrap();
    assert!(!events.iter().any(|e| matches!(
        e.event_type.as_str(),
        "tool_call_started" | "loop_completed" | "model_recovery_planned"
    )));
}

#[test]
fn transient_failure_cannot_sleep_past_total_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let transport = sequence(vec![Err(TransportError::Timeout)]);
    let events_path = dir.path().join("events.jsonl");
    let result = ForegroundProviderRun::prepare_recovering(&config(), request(), 100)
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(&events_path).unwrap(),
            ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
            StreamReporter::new(Vec::new()),
        );
    assert!(matches!(
        result,
        Err(steerbox::provider_runner::ProviderRunError::RecoveryDeadlineExceeded)
    ));
    assert!(transport.requests.borrow().len() <= 1);
    assert!(!EventLog::open(events_path)
        .unwrap()
        .read_all()
        .unwrap()
        .iter()
        .any(|e| e.event_type == "loop_completed"));
}

struct RejectModelProgress;

impl steerbox::stream_reporter::ProgressSink for RejectModelProgress {
    fn report(&mut self, event: steerbox::stream_reporter::ProgressEvent) -> std::io::Result<()> {
        if event.subject == steerbox::stream_reporter::ProgressSubject::Model {
            return Err(std::io::Error::other("synthetic progress unavailable"));
        }
        Ok(())
    }
}

#[test]
fn progress_failure_stops_before_network_and_never_starts_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let transport = sequence(Vec::new());
    let events_path = dir.path().join("events.jsonl");
    let result = ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
        .unwrap()
        .run_with_transport(
            transport.clone(),
            EventLog::open(&events_path).unwrap(),
            ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap(),
            RejectModelProgress,
        );
    assert!(matches!(
        result,
        Err(steerbox::provider_runner::ProviderRunError::Controller(
            steerbox::loop_controller::LoopControllerError::ProgressReport(_)
        ))
    ));
    assert!(transport.requests.borrow().is_empty());
    assert!(!EventLog::open(events_path)
        .unwrap()
        .read_all()
        .unwrap()
        .iter()
        .any(|e| e.event_type == "model_recovery_planned"));
}

struct BreakFailureAudit {
    failure_path: std::path::PathBuf,
    requests: Rc<RefCell<usize>>,
}

impl ProviderTransport for BreakFailureAudit {
    fn send(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: Duration,
        _: bool,
    ) -> Result<TransportResponse, TransportError> {
        *self.requests.borrow_mut() += 1;
        // Only a fresh per-test tempdir is touched. A directory cannot accept JSONL.
        std::fs::create_dir(&self.failure_path).unwrap();
        Err(TransportError::Timeout)
    }
}

#[test]
fn failed_failure_audit_prevents_a_second_provider_call() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Rc::new(RefCell::new(0));
    let calls_path = dir.path().join("calls.jsonl");
    let calls = ModelCallStore::open(&calls_path).unwrap();
    let transport = BreakFailureAudit {
        failure_path: calls_path.with_extension("failures.jsonl"),
        requests: requests.clone(),
    };
    let events_path = dir.path().join("events.jsonl");
    assert!(
        ForegroundProviderRun::prepare_recovering(&config(), request(), 30_000)
            .unwrap()
            .run_with_transport(
                transport,
                EventLog::open(&events_path).unwrap(),
                calls,
                StreamReporter::new(Vec::new())
            )
            .is_err()
    );
    assert_eq!(*requests.borrow(), 1);
    assert!(!EventLog::open(events_path)
        .unwrap()
        .read_all()
        .unwrap()
        .iter()
        .any(|e| matches!(
            e.event_type.as_str(),
            "model_recovery_planned" | "loop_completed"
        )));
}

#[test]
fn malformed_action_envelopes_never_authorize_a_repair_call_for_any_protocol() {
    for protocol in [
        "openai-compatible-chat",
        "openai-responses",
        "anthropic-messages",
    ] {
        for (summary, action) in [
            ("fixture", json!({})),
            ("fixture", json!("private-malformed-action")),
            ("fixture", json!({"unknown":"private-malformed-action"})),
            (
                " ",
                json!({"schema_version":999,"action_id":"action-1","tool_id":"git_status","parameters":{"scope":"workspace"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},"verification":{"verifier_tool_id":"markdown_link_check","parameters":{"path":"README.md"}}}),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut cfg = config();
            cfg.model_profiles[0].endpoint_protocol = protocol.into();
            let decision =
                json!({"decision":"continue_observation","summary":summary,"action":action})
                    .to_string();
            let body = match protocol {
                "anthropic-messages" => json!({"id":"r","type":"message","role":"assistant","model":"fixture-model","content":[{"type":"text","text":decision}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}).to_string(),
                "openai-responses" => json!({"object":"response","id":"r","model":"fixture-model","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":decision}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}).to_string(),
                _ => chat(Some("stop"), &decision),
            };
            let transport = sequence(vec![
                Ok(TransportResponse::body(200, body.clone())),
                Ok(TransportResponse::body(200, body)),
            ]);
            let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
            let prepared =
                ForegroundProviderRun::prepare_recovering(&cfg, request(), 30_000).unwrap();
            let events = EventLog::open(dir.path().join("events.jsonl")).unwrap();
            let result = if protocol == "anthropic-messages" {
                prepared.run_anthropic_with_transport(
                    transport.clone(),
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            } else {
                prepared.run_with_transport(
                    transport.clone(),
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            };
            assert!(result.is_err());
            assert_eq!(
                transport.requests.borrow().len(),
                1,
                "{protocol} must not retry action errors"
            );
            let failures = calls.read_failures().unwrap();
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].input_tokens, Some(3));
            assert!(!format!("{result:?} {failures:?}").contains("private-malformed-action"));
        }
    }
}

fn complete_wire(protocol: &str, truncated: bool) -> String {
    let decision = json!({"decision":"complete_loop","summary":"fixture answer"}).to_string();
    match protocol {
        "anthropic-messages" => json!({"id":"r","type":"message","role":"assistant","model":"fixture-model",
            "content":[{"type":"text","text":decision}],"stop_reason":if truncated {"max_tokens"} else {"end_turn"},
            "usage":{"input_tokens":3,"output_tokens":2}}).to_string(),
        "openai-responses" => json!({"object":"response","id":"r","model":"fixture-model",
            "status":if truncated {"incomplete"} else {"completed"},
            "incomplete_details":if truncated {json!({"reason":"max_output_tokens"})} else {json!(null)},
            "output":[{"type":"message","role":"assistant","status":if truncated {"incomplete"} else {"completed"},
                "content":[{"type":"output_text","text":decision}]}],
            "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}).to_string(),
        _ => chat(Some(if truncated { "length" } else { "stop" }), &decision),
    }
}

#[test]
fn all_three_protocols_recover_transient_eof_and_output_limit_with_one_shared_audit() {
    for protocol in [
        "openai-compatible-chat",
        "openai-responses",
        "anthropic-messages",
    ] {
        for failure in [
            Err(TransportError::Timeout),
            Ok(TransportResponse::body(200, "{")),
            Ok(TransportResponse::body(200, complete_wire(protocol, true))),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut cfg = config();
            cfg.model_profiles[0].endpoint_protocol = protocol.into();
            let transport = sequence(vec![
                failure,
                Ok(TransportResponse::body(200, complete_wire(protocol, false))),
            ]);
            let calls = ModelCallStore::open(dir.path().join("calls.jsonl")).unwrap();
            let events_path = dir.path().join("events.jsonl");
            let events = EventLog::open(&events_path).unwrap();
            let prepared =
                ForegroundProviderRun::prepare_recovering(&cfg, request(), 30_000).unwrap();
            let outcome = if protocol == "anthropic-messages" {
                prepared.run_anthropic_with_transport(
                    transport.clone(),
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            } else {
                prepared.run_with_transport(
                    transport.clone(),
                    events,
                    calls.clone(),
                    StreamReporter::new(Vec::new()),
                )
            }
            .unwrap();
            assert_eq!(outcome.loop_status, LoopStatus::Completed);
            assert_eq!(transport.requests.borrow().len(), 2);
            assert_eq!(calls.read_all().unwrap().len(), 1);
            assert_eq!(calls.read_failures().unwrap().len(), 1);
            assert_eq!(
                EventLog::open(events_path)
                    .unwrap()
                    .read_all()
                    .unwrap()
                    .iter()
                    .filter(|e| e.event_type == "model_recovery_planned")
                    .count(),
                1
            );
        }
    }
}
