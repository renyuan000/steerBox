//! Exception-only structural probe. Retain stop evidence even when later JSON
//! is truncated; never search model prose or reconstruct a partial response.
use crate::model_adapter::DecisionOutputRejection;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{cell::Cell, fmt};

#[derive(Clone, Copy)]
enum Scope {
    Root,
    Choices,
    Choice,
    ChatMessage,
    MessageBlocks,
    MessageBlock,
    Output,
    OutputItem,
    OutputBlocks,
    OutputBlock,
    Incomplete,
    Guard,
    Tools,
    Finish,
    Stop,
    Status,
    Reason,
    MessageType,
    ItemType,
    BlockType,
    Other,
}

impl Scope {
    fn field(self, key: &str) -> Self {
        match (self, key) {
            (Self::Root, "choices") => Self::Choices,
            (Self::Root, "content") => Self::MessageBlocks,
            (Self::Root, "output") => Self::Output,
            (Self::Root, "error") => Self::Guard,
            (Self::Root, "stop_reason") => Self::Stop,
            (Self::Root | Self::OutputItem, "status") => Self::Status,
            (Self::Root, "incomplete_details") => Self::Incomplete,
            (Self::Incomplete, "reason") => Self::Reason,
            (Self::Choice, "message" | "delta") => Self::ChatMessage,
            (Self::Choice, "finish_reason") => Self::Finish,
            (Self::ChatMessage, "refusal" | "function_call") => Self::Guard,
            (Self::ChatMessage, "tool_calls") => Self::Tools,
            (Self::MessageBlock, "type") => Self::MessageType,
            (Self::OutputItem, "type") => Self::ItemType,
            (Self::OutputItem, "content") => Self::OutputBlocks,
            (Self::OutputBlock, "type") => Self::BlockType,
            _ => Self::Other,
        }
    }

    fn element(self) -> Self {
        match self {
            Self::Choices => Self::Choice,
            Self::MessageBlocks => Self::MessageBlock,
            Self::Output => Self::OutputItem,
            Self::OutputBlocks => Self::OutputBlock,
            Self::Tools => Self::Guard,
            _ => Self::Other,
        }
    }

    fn scalar_is_unsafe(self) -> bool {
        matches!(
            self,
            Self::Guard
                | Self::Tools
                | Self::Finish
                | Self::Stop
                | Self::Status
                | Self::Reason
                | Self::MessageType
                | Self::ItemType
                | Self::BlockType
        )
    }
}

struct Probe<'a> {
    scope: Scope,
    unsafe_signal: &'a Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for Probe<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Probe<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("provider wire value")
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        if self.scope.scalar_is_unsafe() {
            self.unsafe_signal.set(true);
        }
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        self.visit_bool(false)
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        self.visit_bool(false)
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        self.visit_bool(false)
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        let unsafe_value = match self.scope {
            Scope::Finish => !matches!(value, "" | "stop" | "length"),
            Scope::Stop => !matches!(value, "" | "end_turn" | "stop_sequence" | "max_tokens"),
            Scope::Status => !matches!(value, "completed" | "incomplete" | "in_progress"),
            Scope::Reason => value != "max_output_tokens",
            Scope::MessageType => value != "text",
            Scope::ItemType => !matches!(value, "message" | "reasoning"),
            Scope::BlockType => value != "output_text",
            Scope::Guard | Scope::Tools => true,
            _ => false,
        };
        if unsafe_value {
            self.unsafe_signal.set(true);
        }
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        if self.scope.scalar_is_unsafe() {
            self.unsafe_signal.set(true);
        }
        while let Some(key) = map.next_key::<String>()? {
            map.next_value_seed(Probe {
                scope: self.scope.field(&key),
                unsafe_signal: self.unsafe_signal,
            })?;
        }
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        if self.scope.scalar_is_unsafe() && !matches!(self.scope, Scope::Tools) {
            self.unsafe_signal.set(true);
        }
        while seq
            .next_element_seed(Probe {
                scope: self.scope.element(),
                unsafe_signal: self.unsafe_signal,
            })?
            .is_some()
        {
            // Even a malformed nonempty list (e.g. [null]) is not permission
            // to retry a possible tool proposal. Empty/null lists stay benign.
            if matches!(self.scope, Scope::Tools) {
                self.unsafe_signal.set(true);
            }
        }
        Ok(())
    }
}

pub(crate) fn wire_eof_reason(body: &str) -> DecisionOutputRejection {
    let unsafe_signal = Cell::new(false);
    // Run only after the regular decoder reports EOF. The probe has no output
    // body, ignores prose, and keeps a monotonic flag across duplicate fields.
    let _ = Probe {
        scope: Scope::Root,
        unsafe_signal: &unsafe_signal,
    }
    .deserialize(&mut serde_json::Deserializer::from_str(body));
    if unsafe_signal.get() {
        DecisionOutputRejection::UnsafeTermination
    } else {
        DecisionOutputRejection::TerminationUnconfirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_evidence_is_sticky_across_duplicates_and_later_eof() {
        for body in [
            r#"{"choices":[{"finish_reason":"content_filter","finish_reason":"stop"}],"usage":"#,
            r#"{"choices":[{"message":{"refusal":"no","refusal":null}}],"usage":"#,
            r#"{"choices":[{"message":{"tool_calls":[null]}}],"usage":"#,
            r#"{"output":[{"type":"function_call","type":"reasoning"}],"usage":"#,
            r#"{"error":{"code":"blocked"},"usage":"#,
        ] {
            assert_eq!(
                wire_eof_reason(body),
                DecisionOutputRejection::UnsafeTermination
            );
        }
    }

    #[test]
    fn prose_and_benign_optional_fields_are_not_stop_signals() {
        for body in [
            "{",
            r#"{"choices":[{"message":{"refusal":null,"tool_calls":[],"function_call":null,"content":"refusal content_filter tool_calls"},"finish_reason":"stop"}],"usage":"#,
            r#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","status":"incomplete","content":[{"type":"output_text","text":"refusal"}]}],"usage":"#,
            r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"tool_use"}],"usage":"#,
            r#"{"extension":{"type":"function_call","refusal":"ignored extension"},"usage":"#,
        ] {
            assert_eq!(
                wire_eof_reason(body),
                DecisionOutputRejection::TerminationUnconfirmed
            );
        }
    }
}
