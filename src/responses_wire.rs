//! Bounded, stateless Responses wire format. Never decode it as Chat choices.
use crate::model_adapter::DecisionOutputRejection;
use crate::provider_adapter::{
    GenerationRejectionEvidence, ProviderAdapterError, ProviderMessage, ProviderResponse,
    ProviderUsage, ReasoningEffort,
};
use serde::Deserialize;
use serde_json::json;

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
    error: Option<serde::de::IgnoredAny>,
    incomplete_details: Option<IncompleteDetails>,
    output: Vec<OutputItem>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct IncompleteDetails {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct OutputItem {
    #[serde(rename = "type")]
    kind: String,
    role: Option<String>,
    status: Option<String>,
    content: Option<Vec<OutputBlock>>,
}

#[derive(Deserialize)]
struct OutputBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

pub(crate) fn parse(body: &str) -> Result<ProviderResponse, ProviderAdapterError> {
    let response: Response = serde_json::from_str(body).map_err(|error| {
        if error.is_eof() {
            ProviderAdapterError::GenerationRejected(GenerationRejectionEvidence {
                reason: crate::provider_recovery::wire_eof_reason(body),
                input_tokens: None,
                output_tokens: None,
            })
        } else {
            ProviderAdapterError::ResponseParse
        }
    })?;
    if response.object != "response"
        || response.id.trim().is_empty()
        || response.model.trim().is_empty()
    {
        return Err(ProviderAdapterError::ResponseParse);
    }
    let rejection = |reason| {
        ProviderAdapterError::GenerationRejected(GenerationRejectionEvidence {
            reason,
            input_tokens: response.usage.as_ref().map(|usage| usage.input_tokens),
            output_tokens: response.usage.as_ref().map(|usage| usage.output_tokens),
        })
    };
    if response.error.is_some() {
        return Err(rejection(DecisionOutputRejection::UnsafeTermination));
    }
    let truncated = match response.status.as_str() {
        "completed" if response.incomplete_details.is_none() => false,
        "incomplete"
            if response
                .incomplete_details
                .as_ref()
                .and_then(|details| details.reason.as_deref())
                == Some("max_output_tokens") =>
        {
            true
        }
        _ => return Err(rejection(DecisionOutputRejection::UnsafeTermination)),
    };
    let mut text = None;
    for item in &response.output {
        match item.kind.as_str() {
            // Do not persist or expose reasoning content to the action parser.
            "reasoning" => continue,
            "message"
                if text.is_none()
                    && item.role.as_deref() == Some("assistant")
                    && (item.status.as_deref() == Some("completed")
                        || (truncated
                            && matches!(
                                item.status.as_deref(),
                                Some("incomplete" | "in_progress")
                            ))) =>
            {
                let blocks = item
                    .content
                    .as_ref()
                    .ok_or(ProviderAdapterError::ResponseParse)?;
                let mut message = String::new();
                for block in blocks {
                    if block.kind != "output_text" {
                        return Err(rejection(DecisionOutputRejection::UnsafeTermination));
                    }
                    message.push_str(
                        block
                            .text
                            .as_deref()
                            .ok_or(ProviderAdapterError::ResponseParse)?,
                    );
                }
                if message.trim().is_empty() && !truncated {
                    return Err(ProviderAdapterError::ResponseParse);
                }
                text = Some(message);
            }
            _ => return Err(rejection(DecisionOutputRejection::UnsafeTermination)),
        }
    }
    if truncated {
        return Err(rejection(DecisionOutputRejection::OutputTruncated));
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
