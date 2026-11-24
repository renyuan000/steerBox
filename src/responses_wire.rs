//! Bounded, stateless Responses wire format. Never decode it as Chat choices.
use crate::provider_adapter::{
    ProviderAdapterError, ProviderMessage, ProviderResponse, ProviderUsage, ReasoningEffort,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn encode(
    messages: &[ProviderMessage],
    model: &str,
    effort: Option<ReasoningEffort>,
) -> Result<String, ProviderAdapterError> {
    let input = messages
        .iter()
        .map(|message| {
            let content = match message.role.as_str() {
                "system" | "developer" | "user" => {
                    json!([{"type": "input_text", "text": message.content}])
                }
                // EasyInputMessage accepts assistant text directly. An output_text
                // block belongs to a full ResponseOutputMessage, not this shape.
                "assistant" => json!(message.content),
                _ => {
                    return Err(ProviderAdapterError::InvalidRequest {
                        reason: "Responses requires system, developer, user or assistant roles",
                    });
                }
            };
            Ok(json!({
                "role": message.role,
                "content": content
            }))
        })
        .collect::<Result<Vec<_>, ProviderAdapterError>>()?;
    let mut body = json!({"model": model, "input": input, "stream": false, "store": false});
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort": effort});
    }
    serde_json::to_string(&body).map_err(|_| ProviderAdapterError::ResponseParse)
}

#[derive(Deserialize)]
struct Response {
    object: String,
    id: String,
    model: String,
    status: String,
    error: Option<Value>,
    incomplete_details: Option<Value>,
    output: Vec<Value>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

pub(crate) fn parse(body: &str) -> Result<ProviderResponse, ProviderAdapterError> {
    let response: Response =
        serde_json::from_str(body).map_err(|_| ProviderAdapterError::ResponseParse)?;
    if response.object != "response"
        || response.id.trim().is_empty()
        || response.model.trim().is_empty()
        || response.status != "completed"
        || response.error.is_some()
        || response.incomplete_details.is_some()
    {
        return Err(ProviderAdapterError::ResponseParse);
    }
    let mut text = None;
    for item in response.output {
        match item.get("type").and_then(Value::as_str) {
            // Do not persist or expose reasoning content to the action parser.
            Some("reasoning") => continue,
            Some("message")
                if text.is_none()
                    && item.get("role").and_then(Value::as_str) == Some("assistant")
                    && item.get("status").and_then(Value::as_str) == Some("completed") =>
            {
                let blocks = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(ProviderAdapterError::ResponseParse)?;
                let mut message = String::new();
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) != Some("output_text") {
                        return Err(ProviderAdapterError::ResponseParse);
                    }
                    message.push_str(
                        block
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or(ProviderAdapterError::ResponseParse)?,
                    );
                }
                if message.trim().is_empty() {
                    return Err(ProviderAdapterError::ResponseParse);
                }
                text = Some(message);
            }
            _ => return Err(ProviderAdapterError::ResponseParse),
        }
    }
    Ok(ProviderResponse {
        request_id: response.id,
        model: response.model,
        text: text.ok_or(ProviderAdapterError::ResponseParse)?,
        finish_reason: Some("stop".to_owned()),
        usage: response.usage.map(|usage| ProviderUsage {
            prompt_tokens: Some(usage.input_tokens),
            completion_tokens: Some(usage.output_tokens),
            total_tokens: Some(usage.total_tokens),
        }),
    })
}
