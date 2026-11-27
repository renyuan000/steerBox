//! Cheap, body-free integrity checks and a deliberately narrow recovery policy.
use crate::model_adapter::{DecisionOutputRejection, ModelAdapterError};
use crate::provider_adapter::{ProviderErrorClass, ProviderResponse};

mod wire_eof;
pub(crate) use wire_eof::wire_eof_reason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryGap {
    TransientProvider,
    DecisionSchema,
    OutputTruncated,
    TerminationUnconfirmed,
}

impl RecoveryGap {
    pub fn code(self) -> &'static str {
        match self {
            Self::TransientProvider => "transient_provider",
            Self::DecisionSchema => "decision_schema",
            Self::OutputTruncated => "output_truncated",
            Self::TerminationUnconfirmed => "termination_unconfirmed",
        }
    }

    /// Only for a pure-observation invocation. Never authority to replay tools.
    pub fn from_error(error: &ModelAdapterError) -> Option<Self> {
        match error {
            ModelAdapterError::Provider {
                class:
                    ProviderErrorClass::Timeout
                    | ProviderErrorClass::Transport
                    | ProviderErrorClass::Upstream,
            } => Some(Self::TransientProvider),
            // Untyped parse errors may hide refusal or tool states: stop safely.
            ModelAdapterError::DecisionOutputRejected { reason } => match reason {
                // Data/summary errors may mask an invalid action. Do not add a
                // second parse or reinterpret them as permission to retry.
                DecisionOutputRejection::JsonSyntaxInvalid => Some(Self::DecisionSchema),
                DecisionOutputRejection::OutputTruncated => Some(Self::OutputTruncated),
                DecisionOutputRejection::TerminationUnconfirmed => {
                    Some(Self::TerminationUnconfirmed)
                }
                _ => None,
            },
            _ => None,
        }
    }
}

/// No text scan or LLM call. Protocol termination is not task success.
pub(crate) fn validate_generation(
    response: &ProviderResponse,
    messages_protocol: bool,
) -> Result<(), ModelAdapterError> {
    let reason = match response.finish_reason.as_deref() {
        Some("end_turn" | "stop_sequence") if messages_protocol => return Ok(()),
        Some("stop") if !messages_protocol => return Ok(()),
        Some("length") if !messages_protocol => DecisionOutputRejection::OutputTruncated,
        Some("max_tokens") if messages_protocol => DecisionOutputRejection::OutputTruncated,
        None | Some("") => DecisionOutputRejection::TerminationUnconfirmed,
        // Includes refusals, filters, tool calls, pause_turn and unknown values.
        // Never reinterpret these as authorization to ask a different model.
        _ => DecisionOutputRejection::UnsafeTermination,
    };
    Err(ModelAdapterError::DecisionOutputRejected { reason })
}
