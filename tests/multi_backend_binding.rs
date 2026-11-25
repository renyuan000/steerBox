use serde_json::{json, Value};
use std::cell::Cell;
use std::time::Duration;
use steerbox::anthropic_adapter::{AnthropicHeaders, AnthropicTransport, AnthropicTransportError};
use steerbox::domain::LoopStatus;
use steerbox::event_log::EventLog;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_adapter::{ProviderTransport, TransportError, TransportResponse};
use steerbox::provider_runner::{ForegroundProviderRun, ProviderRunRequest};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::stream_reporter::StreamReporter;

struct Fixture {
    protocol: &'static str,
    model: &'static str,
    endpoint: &'static str,
    auth_ref: &'static str,
    calls: Cell<usize>,
}

impl Fixture {
    fn respond(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        streaming: bool,
    ) -> TransportResponse {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(endpoint, self.endpoint);
        assert_eq!(auth_ref, Some(self.auth_ref));
        assert!(!streaming);
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["model"], self.model);
        assert!(!body.to_string().contains(self.auth_ref));
        let decision =
            json!({"decision": "complete_loop", "summary": "fixture completed"}).to_string();
        let response = match self.protocol {
            "openai-compatible-chat" => {
                assert_eq!(body["messages"][0]["role"], "system");
                assert!(body.get("input").is_none());
                json!({"id": "chat-fixture", "model": self.model,
                    "choices": [{"message": {"content": decision}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 13, "completion_tokens": 5, "total_tokens": 18}})
            }
            "openai-responses" => {
                assert_eq!(body["input"][0]["role"], "system");
                assert_eq!(body["store"], false);
                assert!(body.get("messages").is_none());
                json!({"object": "response", "id": "responses-fixture", "model": self.model, "status": "completed",
                    "output": [{"type": "message", "role": "assistant", "status": "completed",
                        "content": [{"type": "output_text", "text": decision}]}],
                    "usage": {"input_tokens": 13, "output_tokens": 5, "total_tokens": 18}})
            }
            "anthropic-messages" => {
                assert!(body.get("system").is_some());
                assert_eq!(body["messages"][0]["role"], "user");
                assert!(body.get("input").is_none());
                json!({"id": "messages-fixture", "type": "message", "role": "assistant", "model": self.model,
                    "content": [{"type": "text", "text": decision}], "stop_reason": "end_turn",
                    "usage": {"input_tokens": 13, "output_tokens": 5}})
            }
            _ => panic!("unsupported fixture protocol"),
        };
        TransportResponse::body(200, response.to_string())
    }
}

impl ProviderTransport for &Fixture {
    fn send(
        &self,
        endpoint: &str,
        auth_ref: Option<&str>,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        assert_ne!(self.protocol, "anthropic-messages");
        Ok(self.respond(endpoint, auth_ref, body, streaming))
    }
}

impl AnthropicTransport for &Fixture {
    fn send(
        &self,
        endpoint: &str,
        headers: &AnthropicHeaders,
        body: &str,
        _: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        assert_eq!(self.protocol, "anthropic-messages");
        Ok(self.respond(endpoint, headers.api_key_ref.as_deref(), body, streaming))
    }
}

#[test]
fn mixed_protocol_config_keeps_model_endpoint_auth_and_audit_bound_to_selected_backend() {
    let profiles = [
        (
            "chat-model",
            "relay-chat",
            "openai-compatible-chat",
            "base_url",
            "https://chat.example.test/v1",
            "https://chat.example.test/v1/chat/completions",
            "env:CHAT_FIXTURE",
        ),
        (
            "responses-model",
            "relay-responses",
            "openai-responses",
            "full_url",
            "https://responses.example.test/custom-infer",
            "https://responses.example.test/custom-infer",
            "env:RESPONSES_FIXTURE",
        ),
        (
            "messages-model",
            "relay-messages",
            "anthropic-messages",
            "base_url",
            "https://messages.example.test/v1",
            "https://messages.example.test/v1/messages",
            "env:MESSAGES_FIXTURE",
        ),
    ];
    let config = RuntimeConfig::from_json_str(&json!({
        "schema_version": 1, "project_ref": "multi-backend-fixture", "data_root": "/temp/steerbox/multi-backend-fixture",
        "model_profiles": profiles.iter().map(|(model, provider, protocol, mode, base, _, auth)| json!({
            "model_id": model, "provider": provider, "endpoint_protocol": protocol,
            "endpoint_mode": mode, "endpoint": base, "auth_ref": auth, "capabilities": ["text"]
        })).collect::<Vec<_>>()
    }).to_string()).unwrap();

    // Select in a different order from the configuration to detect first-profile routing.
    for index in [2, 0, 1] {
        let (model, provider, protocol, _, _, endpoint, auth_ref) = profiles[index];
        let fixture = Fixture {
            protocol,
            model,
            endpoint,
            auth_ref,
            calls: Cell::new(0),
        };
        let directory = tempfile::tempdir().unwrap();
        let calls = ModelCallStore::open(directory.path().join("model-calls.jsonl")).unwrap();
        let events = EventLog::open(directory.path().join("events.jsonl")).unwrap();
        let run = ForegroundProviderRun::prepare(
            &config,
            ProviderRunRequest {
                project_id: "multi-backend-fixture".into(),
                goal_id: "fixture-goal".into(),
                loop_run_id: "fixture-loop".into(),
                task_run_id: "fixture-task".into(),
                model_id: model.into(),
                objective: "Inspect supplied observation".into(),
                observation: "Synthetic multi-backend fixture".into(),
            },
        )
        .unwrap();
        let sink = StreamReporter::new(Vec::new());
        let result = if protocol == "anthropic-messages" {
            run.run_anthropic_with_transport(&fixture, events, calls.clone(), sink)
        } else {
            run.run_with_transport(&fixture, events, calls.clone(), sink)
        }
        .unwrap();
        assert_eq!(result.loop_status, LoopStatus::Completed);
        assert_eq!(fixture.calls.get(), 1);
        let records = calls.read_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider_id, provider);
        assert_eq!(records[0].model_id, model);
        assert_eq!(records[0].input_tokens, Some(13));
        assert_eq!(records[0].output_tokens, Some(5));
        let audit = serde_json::to_string(&records).unwrap();
        for (_, _, _, _, base, _, auth) in profiles {
            assert!(!audit.contains(auth));
            assert!(!audit.contains(base));
        }
    }
}
