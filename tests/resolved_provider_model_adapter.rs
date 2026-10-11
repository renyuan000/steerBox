use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use steerbox::model_adapter::{ModelAdapterError, ModelDecision, ModelRequest};
use steerbox::model_contracts::RouteDecision;
use steerbox::model_registry::{EndpointProfile, ModelProfile, ModelRegistry, ProviderProfile};
use steerbox::prompt_registry::{PromptProfile, PromptRegistry};
use steerbox::provider_adapter::{
    OpenAiCompatibleAdapter, ProviderTransport, TransportError, TransportResponse,
};
use steerbox::provider_model_adapter::ProviderModelAdapter;
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
        assert!(adapter(transport)
            .complete_resolved(request_for(&pack), &pack)
            .is_err());
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
fn resolved_bridge_rejects_invalid_decision_without_guessing() {
    let pack = resolved_pack();
    for text in [
        "The task is done.",
        r#"{"decision":"execute_tool","summary":"fixture"}"#,
        r#"{"decision":"complete_loop","summary":" "}"#,
        r#"{"decision":"complete_loop","summary":"done","extra":true}"#,
    ] {
        let transport = FakeTransport::response(text, None, Some("stop"));
        let sends = Arc::clone(&transport.sends);
        let error = adapter(transport)
            .complete_resolved(request_for(&pack), &pack)
            .expect_err("invalid decision must be rejected");
        assert_eq!(error, ModelAdapterError::ProviderTextUnmappable);
        assert_eq!(sends.lock().expect("capture lock").len(), 1);
    }
}
