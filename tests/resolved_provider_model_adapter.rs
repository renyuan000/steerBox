use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use steerbox::adaptive_routing::EffectLevel;
use steerbox::anthropic_adapter::{
    AnthropicAdapter, AnthropicHeaders, AnthropicTransport, AnthropicTransportError,
};
use steerbox::model_adapter::{
    DecisionOutputRejection, ModelAdapterError, ModelDecision, ModelRequest,
};
use steerbox::model_contracts::RouteDecision;
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::prompt_registry::{PromptProfile, PromptRegistry};
use steerbox::provider_adapter::{
    ModelEffectPolicy, OpenAiCompatibleAdapter, ProviderTransport, ReasoningEffort, TransportError,
    TransportResponse,
};
use steerbox::provider_model_adapter::{ProviderModelAdapter, ResolvedModelAdapter};
use steerbox::resolved_prompt::{
    resolve_prompt, AgentRolePrompt, ModelPromptFragment, ProviderPromptFragment,
    ResolvePromptInput, ResolvedPromptPack, TaskPromptPack, VersionedPromptFragment,
};

const BASE_URL: &str = "https://fixture.example.test/v1";

#[derive(Clone)]
struct CapturedSend {
    endpoint: String,
    body: String,
    streaming: bool,
}

#[derive(Clone)]
struct FakeTransport {
    response: TransportResponse,
    sends: Arc<Mutex<Vec<CapturedSend>>>,
}

impl FakeTransport {
    fn response(text: &str, usage: Option<Value>, finish_reason: Option<&str>) -> Self {
        let mut body = json!({
            "id": "fixture-completion",
            "model": "wire-model",
            "choices": [{"message": {"role": "assistant", "content": text}}],
        });
        if let Some(usage) = usage {
            body["usage"] = usage;
        }
        if let Some(reason) = finish_reason {
            body["choices"][0]["finish_reason"] = json!(reason);
        }
        Self {
            response: TransportResponse::body(200, body.to_string()),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn valid() -> Self {
        Self::response(
            r#"{"decision":"complete_loop","summary":"fixture done"}"#,
            None,
            Some("stop"),
        )
    }
}

impl ProviderTransport for FakeTransport {
    fn send(
        &self,
        endpoint: &str,
        _auth_ref: Option<&str>,
        request_body: &str,
        _timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, TransportError> {
        self.sends.lock().expect("capture lock").push(CapturedSend {
            endpoint: endpoint.into(),
            body: request_body.into(),
            streaming,
        });
        Ok(self.response.clone())
    }
}

impl AnthropicTransport for FakeTransport {
    fn send(
        &self,
        endpoint: &str,
        _headers: &AnthropicHeaders,
        request_body: &str,
        _timeout: Duration,
        streaming: bool,
    ) -> Result<TransportResponse, AnthropicTransportError> {
        self.sends.lock().unwrap().push(CapturedSend {
            endpoint: endpoint.into(),
            body: request_body.into(),
            streaming,
        });
        Ok(self.response.clone())
    }
}

fn fragment(id: &str, content: &str) -> VersionedPromptFragment {
    VersionedPromptFragment {
        id: id.into(),
        version: 1,
        content: content.into(),
    }
}

fn resolved_pack() -> ResolvedPromptPack {
    resolved_pack_for_protocol("openai")
}

fn resolved_pack_for_protocol(protocol: &str) -> ResolvedPromptPack {
    let mut models = ModelRegistry::default();
    models
        .register_provider(ProviderProfile::new("fixture-provider", "Fixture", None).unwrap())
        .unwrap();
    models
        .register_endpoint(
            EndpointProfile::new("fixture-endpoint", "fixture-provider", BASE_URL, protocol)
                .unwrap(),
        )
        .unwrap();
    models
        .register_model(
            ModelProfile::new(
                "logical-model",
                "fixture-provider",
                "fixture-endpoint",
                "wire-model",
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let mut prompts = PromptRegistry::default();
    prompts
        .register(PromptProfile::new("inspect", 1, "reviewer", "review", "TASK_BODY").unwrap())
        .unwrap();
    let route = RouteDecision {
        route_decision_id: "route-fixture".into(),
        task_id: "task-fixture".into(),
        task_type: "review".into(),
        agent_role: "reviewer".into(),
        candidate_models: vec!["logical-model".into()],
        selected_provider: "fixture-provider".into(),
        selected_model: "logical-model".into(),
        selected_endpoint: "fixture-endpoint".into(),
        selection_reason: "explicit test fixture".into(),
        rejected_candidates: vec![],
        cost_estimate: None,
        latency_estimate_ms: None,
        risk_level: "low".into(),
        privacy_requirement: "standard".into(),
        fallback_chain: vec![],
        prompt_profile_id: "inspect".into(),
        prompt_profile_version: "1".into(),
        approval_required: false,
        created_at: "2026-10-10T00:00:00Z".into(),
    };
    resolve_prompt(
        &models,
        &prompts,
        ResolvePromptInput {
            route: &route,
            base: &fragment("base", "BASE_BODY"),
            provider: &ProviderPromptFragment {
                provider_id: "fixture-provider".into(),
                fragment: fragment("provider", "PROVIDER_BODY"),
            },
            model: &ModelPromptFragment {
                provider_id: "fixture-provider".into(),
                model_id: "logical-model".into(),
                fragment: fragment("model", "MODEL_BODY"),
            },
            role: &AgentRolePrompt {
                role: "reviewer".into(),
                fragment: fragment("reviewer", "ROLE_BODY"),
            },
            task: &TaskPromptPack {
                prompt_id: "inspect".into(),
                version: 1,
                task_type: "review".into(),
            },
            skills: &[],
            policy_summary: &json!({"mode": "read_only"}),
            tool_summary: &json!({"tools": ["read"]}),
            context_package: &json!({"file": "文档\".md\n", "evidence": "fixture"}),
            objective: "检查 \"quoted\" file\n保留换行",
            observation: "fixture evidence:\nUnicode 中文; quote: \"",
            resolution_reason: "explicit fake-only fixture",
            created_at: "2026-10-10T00:00:00Z",
        },
    )
    .expect("fixture prompt resolves")
}

fn request_for(pack: &ResolvedPromptPack) -> ModelRequest {
    ModelRequest {
        model_id: pack.model_id().into(),
        goal_id: "goal-fixture".into(),
        loop_run_id: "loop-fixture".into(),
        step_id: "step-fixture".into(),
        objective: pack.objective().into(),
        observation: pack.observation().into(),
    }
}

fn adapter(transport: FakeTransport) -> ProviderModelAdapter<FakeTransport> {
    ProviderModelAdapter::new(
        OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport),
        None,
    )
}

fn valid_action_response() -> String {
    json!({
        "decision": "continue_observation",
        "summary": "fixture action",
        "action": {
            "schema_version": 1,
            "action_id": "action-resolved-adapter",
            "tool_id": "git_status",
            "parameters": {"scope": "workspace"},
            "path_scope": {
                "root": "/workspace",
                "allowed_paths": ["README.md"]
            },
            "verification": {
                "verifier_tool_id": "markdown_link_check",
                "parameters": {"path": "README.md"}
            }
        }
    })
    .to_string()
}

#[test]
fn resolved_bridge_sends_wire_name_and_exact_resolved_messages() {
    let pack = resolved_pack();
    let transport = FakeTransport::valid();
    let sends = Arc::clone(&transport.sends);
    let result = adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .expect("resolved decision");

    assert_eq!(result.response.decision, ModelDecision::CompleteLoop);
    assert_eq!(result.response.summary, "fixture done");
    let debug = format!("{result:?}");
    assert!(debug.contains(&result.input_context_hash));
    assert!(!debug.contains("fixture done"));
    assert!(!debug.contains(&result.provider_response.text));
    let sends = sends.lock().expect("capture lock");
    assert_eq!(sends.len(), 1);
    let body: Value = serde_json::from_str(&sends[0].body).unwrap();
    assert_eq!(body["model"], "wire-model");
    assert_ne!(body["model"], pack.model_id());
    assert_eq!(
        body["messages"],
        serde_json::to_value(pack.messages()).unwrap()
    );
    for fragment in [
        "BASE_BODY",
        "PROVIDER_BODY",
        "MODEL_BODY",
        "ROLE_BODY",
        "TASK_BODY",
    ] {
        assert!(sends[0].body.contains(fragment));
    }
    assert_eq!(body["stream"], false);
    assert_eq!(sends[0].endpoint, format!("{BASE_URL}/chat/completions"));
    assert!(!sends[0].streaming);
}

#[test]
fn resolved_effect_binds_wire_model_and_hash_not_registry_identifier() {
    let pack = resolved_pack();
    let transport = FakeTransport::valid();
    let sends = Arc::clone(&transport.sends);
    let provider = OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport)
        .with_effect_policy(
            ModelEffectPolicy::new(
                "wire-model",
                BTreeMap::from([(EffectLevel::Strong, ReasoningEffort::High)]),
            )
            .unwrap(),
            EffectLevel::Strong,
        )
        .unwrap();
    let result = ProviderModelAdapter::new(provider, None)
        .complete_resolved(request_for(&pack), &pack)
        .unwrap();
    let sends = sends.lock().unwrap();
    let body: Value = serde_json::from_str(&sends[0].body).unwrap();
    assert_eq!(body["model"], "wire-model");
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(
        result.input_context_hash,
        format!("sha256:{:x}", Sha256::digest(sends[0].body.as_bytes()))
    );
}

#[test]
fn resolved_effect_cannot_use_logical_identifier_as_wire_binding() {
    let pack = resolved_pack();
    let transport = FakeTransport::valid();
    let sends = Arc::clone(&transport.sends);
    let provider = OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport)
        .with_effect_policy(
            ModelEffectPolicy::new(
                "logical-model",
                BTreeMap::from([(EffectLevel::Strong, ReasoningEffort::High)]),
            )
            .unwrap(),
            EffectLevel::Strong,
        )
        .unwrap();
    assert!(ProviderModelAdapter::new(provider, None)
        .complete_resolved(request_for(&pack), &pack)
        .is_err());
    assert!(sends.lock().unwrap().is_empty());
}

#[test]
fn resolved_action_bridge_retains_validated_action() {
    let pack = resolved_pack();
    let result = adapter(FakeTransport::response(
        &valid_action_response(),
        None,
        Some("stop"),
    ))
    .complete_resolved_with_action(request_for(&pack), &pack)
    .expect("resolved action response");

    let action = result.response.action.expect("resolved action is retained");
    assert_eq!(action.action_id, "action-resolved-adapter");
    assert_eq!(action.tool_id, "git_status");
    assert_eq!(result.response.summary, "fixture action");
}

#[test]
fn legacy_resolved_bridge_rejects_action_instead_of_dropping_it() {
    let pack = resolved_pack();
    let transport = FakeTransport::response(
        &valid_action_response(),
        Some(json!({"prompt_tokens": 0, "completion_tokens": 3})),
        Some("stop"),
    );
    let sends = Arc::clone(&transport.sends);
    let error = adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .expect_err("legacy resolved bridge must fail closed on action");
    assert_eq!(error.cause(), &ModelAdapterError::ProviderTextUnmappable);
    let evidence = error
        .evidence()
        .expect("actionless rejection retains evidence");
    assert_eq!(evidence.finish_reason(), Some("stop"));
    assert_eq!(evidence.input_tokens(), Some(0));
    assert_eq!(evidence.output_tokens(), Some(3));
    let bodies = sends.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        evidence.input_context_hash(),
        format!("sha256:{:x}", Sha256::digest(bodies[0].body.as_bytes()))
    );
}

#[test]
fn resolved_bridge_hash_covers_actual_unicode_quote_and_newline_wire_bytes() {
    let pack = resolved_pack();
    let transport = FakeTransport::valid();
    let sends = Arc::clone(&transport.sends);
    let result = adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .expect("resolved decision");
    let sends = sends.lock().expect("capture lock");
    assert_eq!(sends.len(), 1);
    let hash = format!("{:x}", Sha256::digest(sends[0].body.as_bytes()));
    assert_eq!(
        result
            .input_context_hash
            .strip_prefix("sha256:")
            .unwrap_or(&result.input_context_hash),
        hash
    );
    let body: Value = serde_json::from_str(&sends[0].body).unwrap();
    assert_eq!(
        body["messages"],
        serde_json::to_value(pack.messages()).unwrap()
    );
    assert!(sends[0].body.contains("中文"));
    assert!(sends[0].body.contains("\\n"));
    assert!(sends[0].body.contains("\\\""));
}

#[test]
fn resolved_bridge_rejects_foreign_provider_host_or_path_before_send() {
    let pack = resolved_pack();
    for (provider, base_url) in [
        ("other-provider", BASE_URL),
        ("fixture-provider", "https://other.example.test/v1"),
        ("fixture-provider", "https://fixture.example.test/v2"),
    ] {
        let transport = FakeTransport::valid();
        let sends = Arc::clone(&transport.sends);
        let adapter = ProviderModelAdapter::new(
            OpenAiCompatibleAdapter::new(provider, base_url, transport),
            None,
        );
        assert!(adapter
            .complete_resolved(request_for(&pack), &pack)
            .is_err());
        assert!(sends.lock().expect("capture lock").is_empty());
    }
}

#[test]
fn resolved_bridge_rejects_other_protocols_before_send() {
    for protocol in ["anthropic", "unknown"] {
        let pack = resolved_pack_for_protocol(protocol);
        let transport = FakeTransport::valid();
        let sends = Arc::clone(&transport.sends);
        let failure = adapter(transport)
            .complete_resolved(request_for(&pack), &pack)
            .unwrap_err();
        assert!(failure.evidence().is_none());
        assert!(sends.lock().expect("capture lock").is_empty());
    }
}

#[test]
fn resolved_bridge_accepts_existing_openai_compatible_registry_protocol() {
    let pack = resolved_pack_for_protocol("openai-compatible");
    let transport = FakeTransport::valid();
    let sends = Arc::clone(&transport.sends);
    adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .unwrap();
    assert_eq!(sends.lock().unwrap().len(), 1);
}

#[test]
fn resolved_bridge_rejects_request_model_goal_or_observation_mismatch_before_send() {
    let pack = resolved_pack();
    let mut model = request_for(&pack);
    model.model_id = "wire-model".into();
    let mut objective = request_for(&pack);
    objective.objective.push_str(" changed");
    let mut observation = request_for(&pack);
    observation.observation.push_str(" changed");
    for request in [model, objective, observation] {
        let transport = FakeTransport::valid();
        let sends = Arc::clone(&transport.sends);
        assert!(adapter(transport)
            .complete_resolved(request, &pack)
            .is_err());
        assert!(sends.lock().expect("capture lock").is_empty());
    }
}

#[test]
fn resolved_bridge_preserves_provider_usage_and_finish_reason() {
    let pack = resolved_pack();
    let transport = FakeTransport::response(
        r#"{"decision":"request_human","summary":"fixture review required"}"#,
        Some(json!({"prompt_tokens": 17, "completion_tokens": 5, "total_tokens": 22})),
        Some("stop"),
    );
    let result = adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .expect("resolved decision");
    assert_eq!(result.response.decision, ModelDecision::RequestHuman);
    assert_eq!(result.provider_response.request_id, "fixture-completion");
    assert_eq!(result.provider_response.model, "wire-model");
    assert_eq!(
        result.provider_response.finish_reason.as_deref(),
        Some("stop")
    );
    let usage = result.provider_response.usage.expect("real fixture usage");
    assert_eq!(usage.prompt_tokens, Some(17));
    assert_eq!(usage.completion_tokens, Some(5));
    assert_eq!(usage.total_tokens, Some(22));
}

#[test]
fn resolved_bridge_does_not_invent_missing_usage_or_finish_reason() {
    let pack = resolved_pack();
    let transport = FakeTransport::response(
        r#"{"decision":"continue_observation","summary":"fixture followup"}"#,
        None,
        None,
    );
    let result = adapter(transport)
        .complete_resolved(request_for(&pack), &pack)
        .expect("resolved decision");
    assert_eq!(result.response.decision, ModelDecision::ContinueObservation);
    assert!(result.provider_response.usage.is_none());
    assert!(result.provider_response.finish_reason.is_none());
}

#[test]
fn resolved_wire_failures_have_no_response_evidence_for_both_protocols() {
    for protocol in ["openai", "anthropic"] {
        let pack = resolved_pack_for_protocol(protocol);
        for (status, body) in [
            (500, "fixture-private-upstream-body"),
            (200, "fixture-private-malformed-wire"),
        ] {
            let transport = FakeTransport {
                response: TransportResponse::body(status, body),
                sends: Arc::new(Mutex::new(Vec::new())),
            };
            let sends = Arc::clone(&transport.sends);
            let adapter: Box<dyn ResolvedModelAdapter> = if protocol == "openai" {
                Box::new(adapter(transport))
            } else {
                Box::new(AnthropicAdapter::new(
                    "fixture-provider",
                    BASE_URL,
                    transport,
                    None,
                ))
            };
            let failure = adapter
                .complete_resolved_with_action(request_for(&pack), &pack)
                .err()
                .unwrap();
            assert!(failure.evidence().is_none());
            assert_eq!(sends.lock().unwrap().len(), 1);
            assert!(!format!("{failure}\n{failure:?}").contains("fixture-private"));
        }
    }
}

#[test]
fn resolved_parse_failures_preserve_only_known_safe_evidence_for_both_protocols() {
    let private_finish =
        "https://fixture-private.invalid/finish\n\u{1b}[31mfixture-synthetic-secret-marker";
    for (protocol, input_field, output_field, known_finish) in [
        ("openai", "prompt_tokens", "completion_tokens", "length"),
        ("anthropic", "input_tokens", "output_tokens", "max_tokens"),
    ] {
        let pack = resolved_pack_for_protocol(protocol);
        for (usage_present, input_tokens, output_tokens, finish, expected_finish) in [
            (false, None, None, None, None),
            (true, None, None, Some(private_finish), None),
            (true, Some(0), None, Some(known_finish), Some(known_finish)),
            (true, None, Some(7), Some(known_finish), Some(known_finish)),
            (
                true,
                Some(0),
                Some(0),
                Some(known_finish),
                Some(known_finish),
            ),
            (
                true,
                Some(17),
                Some(5),
                Some(known_finish),
                Some(known_finish),
            ),
        ] {
            let mut usage = json!({});
            if let Some(tokens) = input_tokens {
                usage[input_field] = json!(tokens);
            }
            if let Some(tokens) = output_tokens {
                usage[output_field] = json!(tokens);
            }
            if protocol == "openai" {
                usage["total_tokens"] = json!(999);
            }
            let mut body = if protocol == "openai" {
                json!({
                    "id": "fixture-response-private-id",
                    "model": "fixture-response-private-model",
                    "choices": [{"message": {"content": "fixture-response-private-body"}, "finish_reason": finish}],
                })
            } else {
                json!({
                    "id": "fixture-response-private-id",
                    "model": "fixture-response-private-model",
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "fixture-response-private-body"}],
                    "stop_reason": finish,
                })
            };
            if usage_present {
                body["usage"] = usage;
            }
            let transport = FakeTransport {
                response: TransportResponse::body(200, body.to_string()),
                sends: Arc::new(Mutex::new(Vec::new())),
            };
            let sends = Arc::clone(&transport.sends);
            let adapter: Box<dyn ResolvedModelAdapter> = if protocol == "openai" {
                Box::new(adapter(transport))
            } else {
                Box::new(AnthropicAdapter::new(
                    "fixture-provider",
                    BASE_URL,
                    transport,
                    None,
                ))
            };
            let failure = adapter
                .complete_resolved_with_action(request_for(&pack), &pack)
                .err()
                .expect("invalid decision text fails after one wire response");
            assert_eq!(
                failure.cause(),
                &ModelAdapterError::DecisionOutputRejected {
                    reason: DecisionOutputRejection::JsonSyntaxInvalid,
                }
            );
            let evidence = failure.evidence().expect("safe evidence retained");
            let bodies = sends.lock().unwrap();
            assert_eq!(bodies.len(), 1);
            assert_eq!(
                evidence.input_context_hash(),
                format!("sha256:{:x}", Sha256::digest(bodies[0].body.as_bytes()))
            );
            assert_eq!(evidence.input_tokens(), input_tokens);
            assert_eq!(evidence.output_tokens(), output_tokens);
            assert_eq!(evidence.finish_reason(), expected_finish);
            let diagnostic = format!("{failure}\n{failure:?}");
            for marker in [
                "fixture-response-private",
                "https://fixture-private.invalid",
                "fixture-synthetic-secret-marker",
            ] {
                assert!(!diagnostic.contains(marker));
            }
        }
    }
}

#[test]
fn all_decision_rejection_classes_preserve_safe_evidence_for_both_protocols() {
    let cases = [
        (
            "fixture-response-private-body".to_owned(),
            DecisionOutputRejection::JsonSyntaxInvalid,
        ),
        (
            r#"{"decision":"fixture-response-private-body","summary":"done"}"#.to_owned(),
            DecisionOutputRejection::EnvelopeInvalid,
        ),
        (
            r#"{"decision":"complete_loop","summary":" "}"#.to_owned(),
            DecisionOutputRejection::EmptySummary,
        ),
        (
            r#"{"decision":"continue_observation","summary":"fixture","action":{"schema_version":1,"action_id":"action-1","tool_id":"git_status","parameters":{"shell":"fixture-response-private-body"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},"verification":{"verifier_tool_id":"verify","parameters":{"path":"README.md"}}}}"#.to_owned(),
            DecisionOutputRejection::ActionRejected,
        ),
        (
            json!({
                "decision": "complete_loop",
                "summary": format!("fixture-response-private-body{}", "x".repeat(steerbox::redaction::DEFAULT_MAX_INPUT_BYTES)),
            })
            .to_string(),
            DecisionOutputRejection::SummaryRedactionRejected,
        ),
    ];
    for protocol in ["openai", "anthropic"] {
        let pack = resolved_pack_for_protocol(protocol);
        for (text, reason) in &cases {
            let body = if protocol == "openai" {
                json!({
                    "id": "fixture-response-private-id", "model": "fixture-response-private-model",
                    "choices": [{"message": {"content": text}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 17, "completion_tokens": 5},
                })
            } else {
                json!({
                    "id": "fixture-response-private-id", "model": "fixture-response-private-model",
                    "type": "message", "role": "assistant",
                    "content": [{"type": "text", "text": text}],
                    "stop_reason": "end_turn", "usage": {"input_tokens": 17, "output_tokens": 5},
                })
            };
            let transport = FakeTransport {
                response: TransportResponse::body(200, body.to_string()),
                sends: Arc::new(Mutex::new(Vec::new())),
            };
            let sends = Arc::clone(&transport.sends);
            let adapter: Box<dyn ResolvedModelAdapter> = if protocol == "openai" {
                Box::new(adapter(transport))
            } else {
                Box::new(AnthropicAdapter::new(
                    "fixture-provider",
                    BASE_URL,
                    transport,
                    None,
                ))
            };
            let failure = adapter
                .complete_resolved_with_action(request_for(&pack), &pack)
                .err()
                .expect("decision output must be rejected");
            assert_eq!(
                failure.cause(),
                &ModelAdapterError::DecisionOutputRejected { reason: *reason }
            );
            let evidence = failure.evidence().expect("safe response evidence retained");
            let bodies = sends.lock().unwrap();
            assert_eq!(bodies.len(), 1);
            assert_eq!(
                evidence.input_context_hash(),
                format!("sha256:{:x}", Sha256::digest(bodies[0].body.as_bytes()))
            );
            assert_eq!(evidence.input_tokens(), Some(17));
            assert_eq!(evidence.output_tokens(), Some(5));
            assert_eq!(
                evidence.finish_reason(),
                Some(if protocol == "openai" {
                    "stop"
                } else {
                    "end_turn"
                })
            );
            assert!(!format!("{failure}\n{failure:?}").contains("fixture-response-private-body"));
        }
    }
}

#[test]
fn resolved_parse_failure_hash_tracks_effect_and_default_wire_bytes() {
    let pack = resolved_pack();
    let mut hashes = Vec::new();
    for configured in [false, true] {
        let transport = FakeTransport::response("fixture-invalid-decision", None, Some("stop"));
        let sends = Arc::clone(&transport.sends);
        let mut provider = OpenAiCompatibleAdapter::new("fixture-provider", BASE_URL, transport);
        if configured {
            provider = provider
                .with_effect_policy(
                    ModelEffectPolicy::new(
                        "wire-model",
                        BTreeMap::from([(EffectLevel::Strong, ReasoningEffort::High)]),
                    )
                    .unwrap(),
                    EffectLevel::Strong,
                )
                .unwrap();
        }
        let failure = ProviderModelAdapter::new(provider, None)
            .complete_resolved(request_for(&pack), &pack)
            .unwrap_err();
        let bodies = sends.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        let wire: Value = serde_json::from_str(&bodies[0].body).unwrap();
        assert_eq!(
            wire.get("reasoning_effort").and_then(Value::as_str),
            configured.then_some("high")
        );
        let hash = failure.evidence().unwrap().input_context_hash();
        assert_eq!(
            hash,
            format!("sha256:{:x}", Sha256::digest(bodies[0].body.as_bytes()))
        );
        hashes.push(hash.to_owned());
    }
    assert_ne!(hashes[0], hashes[1]);
}

#[test]
fn resolved_bridge_rejects_invalid_decision_without_guessing() {
    let pack = resolved_pack();
    let cases = [
        "The task is done.",
        r#"{"decision":"execute_tool","summary":"fixture"}"#,
        r#"{"decision":"complete_loop","summary":" "}"#,
        r#"{"decision":"complete_loop","summary":"done","extra":true}"#,
        r#"{"decision":"complete_loop","summary":" ","action":{"schema_version":1,"action_id":"action-1","tool_id":"git_status","parameters":{"shell":"fixture-response-body"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},"verification":{"verifier_tool_id":"verify","parameters":{"path":"README.md"}}}}"#,
        r#"{"decision":"execute_tool","summary":"fixture-response-body","action":{"schema_version":1,"action_id":"action-1","tool_id":"git_status","parameters":{"shell":"fixture-response-body"},"path_scope":{"root":"/workspace","allowed_paths":["README.md"]},"verification":{"verifier_tool_id":"verify","parameters":{"path":"README.md"}}}}"#,
    ];
    let reasons = [
        DecisionOutputRejection::JsonSyntaxInvalid,
        DecisionOutputRejection::EnvelopeInvalid,
        DecisionOutputRejection::EmptySummary,
        DecisionOutputRejection::EnvelopeInvalid,
        DecisionOutputRejection::EmptySummary,
        DecisionOutputRejection::EnvelopeInvalid,
    ];
    assert_eq!(cases.len(), reasons.len());
    for (text, reason) in cases.into_iter().zip(reasons) {
        let transport = FakeTransport::response(text, None, Some("stop"));
        let sends = Arc::clone(&transport.sends);
        let error = adapter(transport)
            .complete_resolved(request_for(&pack), &pack)
            .expect_err("invalid decision must be rejected");
        assert_eq!(
            error.cause(),
            &ModelAdapterError::DecisionOutputRejected { reason }
        );
        assert!(!format!("{error}\n{error:?}").contains("fixture-response-body"));
        assert_eq!(sends.lock().expect("capture lock").len(), 1);
    }
}
