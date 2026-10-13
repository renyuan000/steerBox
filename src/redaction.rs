//! Deterministic, local redaction for model/tool/audit boundaries.
//!
//! This module does not call a model or a remote service. It is conservative:
//! oversized input fails closed and never appears in an error or marker.

use serde_json::{Map, Value};
use thiserror::Error;

pub const DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const FAIL_CLOSED_MARKER: &str = "[REDACTION_BLOCKED:INPUT_TOO_LARGE]";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RedactionError {
    #[error("redaction input exceeds configured limit of {limit} bytes")]
    InputTooLarge { limit: usize },
    #[error("redaction input is not valid JSON")]
    InvalidJson,
    #[error("redaction output could not be serialized")]
    SerializationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Redactor {
    max_input_bytes: usize,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::with_max_input_bytes(DEFAULT_MAX_INPUT_BYTES)
    }
}

impl Redactor {
    pub const fn with_max_input_bytes(max_input_bytes: usize) -> Self {
        Self { max_input_bytes }
    }

    pub const fn max_input_bytes(self) -> usize {
        self.max_input_bytes
    }

    pub fn redact_text(self, input: &str) -> Result<String, RedactionError> {
        self.check_size(input)?;
        if let Some(redacted_json) = redact_complete_json_text(input)? {
            return Ok(redacted_json);
        }
        Ok(redact_text_inner(input))
    }

    pub fn redact_json(self, input: &str) -> Result<String, RedactionError> {
        self.check_size(input)?;
        let value: Value = serde_json::from_str(input).map_err(|_| RedactionError::InvalidJson)?;
        let redacted = redact_json_value_inner(value);
        serde_json::to_string(&redacted).map_err(|_| RedactionError::SerializationFailed)
    }

    pub fn redact_json_value(self, value: &Value) -> Result<Value, RedactionError> {
        let encoded = serde_json::to_vec(value).map_err(|_| RedactionError::SerializationFailed)?;
        if encoded.len() > self.max_input_bytes {
            return Err(RedactionError::InputTooLarge {
                limit: self.max_input_bytes,
            });
        }
        Ok(redact_json_value_inner(value.clone()))
    }

    pub fn redact_text_fail_closed(self, input: &str) -> String {
        match self.redact_text(input) {
            Ok(redacted) => redacted,
            Err(_) => FAIL_CLOSED_MARKER.to_owned(),
        }
    }

    pub fn redact_json_fail_closed(self, input: &str) -> String {
        match self.redact_json(input) {
            Ok(redacted) => redacted,
            Err(_) => FAIL_CLOSED_MARKER.to_owned(),
        }
    }

    fn check_size(self, input: &str) -> Result<(), RedactionError> {
        if input.len() > self.max_input_bytes {
            Err(RedactionError::InputTooLarge {
                limit: self.max_input_bytes,
            })
        } else {
            Ok(())
        }
    }
}

pub fn redact_text(input: &str) -> Result<String, RedactionError> {
    Redactor::default().redact_text(input)
}

pub fn redact_json(input: &str) -> Result<String, RedactionError> {
    Redactor::default().redact_json(input)
}

fn redact_json_value_inner(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(redact_object(object)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(redact_json_value_inner).collect())
        }
        Value::String(text) => Value::String(redact_text_inner(&text)),
        scalar => scalar,
    }
}

fn redact_object(object: Map<String, Value>) -> Map<String, Value> {
    object
        .into_iter()
        .map(|(key, value)| {
            let value = if let Some(label) = sensitive_key_label(&key) {
                Value::String(redaction_marker(label))
            } else {
                redact_json_value_inner(value)
            };
            (key, value)
        })
        .collect()
}

fn redact_text_inner(input: &str) -> String {
    let without_private_keys = redact_private_key_blocks(input);
    let without_urls = redact_url_passwords(&without_private_keys);
    let without_assignments = redact_sensitive_assignments(&without_urls);
    let without_bearer = redact_bearer_tokens(&without_assignments);
    redact_jwt_like_tokens(&without_bearer)
}

fn redact_complete_json_text(input: &str) -> Result<Option<String>, RedactionError> {
    let trimmed = input.trim_start();
    if !matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
        return Ok(None);
    }
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Ok(None);
    };
    serde_json::to_string(&redact_json_value_inner(value))
        .map(Some)
        .map_err(|_| RedactionError::SerializationFailed)
}

fn redact_private_key_blocks(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find("-----BEGIN ") {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let label_start = start + "-----BEGIN ".len();
        let header_end = input[label_start..]
            .find(['\n', '\r'])
            .map(|offset| label_start + offset)
            .unwrap_or(input.len());
        let header = &input[label_start..header_end];
        if !header.contains("PRIVATE KEY") {
            // Preserve public PEM blocks and keep searching later headers.
            output.push_str(&input[start..label_start]);
            cursor = label_start;
            continue;
        }
        output.push_str(&redaction_marker("PRIVATE_KEY"));
        let Some(label_end) = header.find("-----") else {
            // A truncated private header/body must never be returned verbatim.
            return output;
        };
        let label = &header[..label_end];
        let begin_end = label_start + label_end + "-----".len();
        let end_marker = format!("-----END {label}-----");
        let Some(relative_end) = input[begin_end..].find(&end_marker) else {
            return output;
        };
        cursor = begin_end + relative_end + end_marker.len();
    }
    output.push_str(&input[cursor..]);
    output
}

fn redact_url_passwords(input: &str) -> String {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while let Some(relative_scheme) = input[cursor..].find("://") {
        let scheme_end = cursor + relative_scheme;
        let authority_start = scheme_end + 3;
        let authority_end = input[authority_start..]
            .find(['/', '?', '#', '\n', '\r', ' ', '\t'])
            .map(|offset| authority_start + offset)
            .unwrap_or(input.len());
        let authority = &input[authority_start..authority_end];
        if let Some(at) = authority.rfind('@') {
            let user_info = &authority[..at];
            if let Some(colon) = user_info.find(':') {
                let password_start = authority_start + colon + 1;
                let password_end = authority_start + at;
                if password_start < password_end {
                    ranges.push((
                        password_start,
                        password_end,
                        redaction_marker("URL_PASSWORD"),
                    ));
                }
            }
        }
        cursor = authority_end;
    }
    replace_ranges(input, &ranges)
}

fn redact_sensitive_assignments(input: &str) -> String {
    let mut ranges = Vec::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor < input.len() {
        if !bytes[cursor].is_ascii_alphanumeric() && !matches!(bytes[cursor], b'_' | b'-') {
            cursor += 1;
            continue;
        }
        let key_start = cursor;
        while cursor < input.len()
            && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'_' | b'-'))
        {
            cursor += 1;
        }
        let Some(label) = sensitive_key_label(&input[key_start..cursor]) else {
            continue;
        };
        let mut value_start = cursor;
        value_start = skip_unicode_whitespace(input, value_start);
        if matches!(bytes.get(value_start), Some(b'"' | b'\'')) {
            value_start += 1;
            value_start = skip_unicode_whitespace(input, value_start);
        }
        if !matches!(bytes.get(value_start), Some(b'=' | b':')) {
            continue;
        }
        value_start += 1;
        value_start = skip_unicode_whitespace(input, value_start);
        if value_start >= input.len() {
            continue;
        }
        let quoted = matches!(bytes[value_start], b'"' | b'\'');
        let content_start = if quoted { value_start + 1 } else { value_start };
        let value_end = if quoted {
            find_quote_end(input, content_start, bytes[value_start]).unwrap_or(input.len())
        } else if matches!(label, "authorization" | "cookie") {
            find_unquoted_line_end(input, content_start)
        } else {
            find_unquoted_value_end(input, content_start)
        };
        if content_start < value_end {
            ranges.push((content_start, value_end, redaction_marker(label)));
        }
    }
    replace_ranges(input, &merge_ranges(ranges))
}

fn redact_bearer_tokens(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find("bearer") {
        let start = search_from + relative;
        let after = start + "bearer".len();
        if !is_token_boundary(&lower, start, after) {
            search_from = after;
            continue;
        }
        let mut value_start = after;
        while lower
            .as_bytes()
            .get(value_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            value_start += 1;
        }
        let value_end = find_unquoted_value_end(input, value_start);
        if value_start < value_end {
            ranges.push((value_start, value_end, redaction_marker("BEARER_TOKEN")));
        }
        search_from = value_end.max(after);
    }
    replace_ranges(input, &merge_ranges(ranges))
}

fn redact_jwt_like_tokens(input: &str) -> String {
    let mut ranges = Vec::new();
    let mut token_start = None;
    for (index, ch) in input.char_indices() {
        let valid = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if valid && token_start.is_none() {
            token_start = Some(index);
        } else if !valid {
            if let Some(start) = token_start.take() {
                maybe_add_jwt_range(input, start, index, &mut ranges);
            }
        }
    }
    if let Some(start) = token_start {
        maybe_add_jwt_range(input, start, input.len(), &mut ranges);
    }
    replace_ranges(input, &merge_ranges(ranges))
}

fn maybe_add_jwt_range(
    input: &str,
    start: usize,
    end: usize,
    ranges: &mut Vec<(usize, usize, String)>,
) {
    let token = &input[start..end];
    let segments: Vec<&str> = token.split('.').collect();
    if is_structurally_valid_jwt(&segments) {
        ranges.push((start, end, redaction_marker("JWT")));
    }
}

fn is_structurally_valid_jwt(segments: &[&str]) -> bool {
    if segments.len() != 3 || segments[0].is_empty() || segments[1].is_empty() {
        return false;
    }
    if !segments.iter().all(|segment| {
        segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return false;
    }
    let Some(header) = decode_base64url_json_object(segments[0]) else {
        return false;
    };
    let Some(_payload) = decode_base64url_json_object(segments[1]) else {
        return false;
    };
    let Some(algorithm) = header.get("alg").and_then(Value::as_str) else {
        return false;
    };
    if algorithm.eq_ignore_ascii_case("none") {
        segments[2].is_empty()
    } else {
        !segments[2].is_empty() && decode_base64url(segments[2]).is_some()
    }
}

fn decode_base64url_json_object(segment: &str) -> Option<Map<String, Value>> {
    let bytes = decode_base64url(segment)?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    if input.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

fn find_quote_end(input: &str, start: usize, quote: u8) -> Option<usize> {
    let mut escaped = false;
    for (offset, byte) in input.as_bytes()[start..].iter().enumerate() {
        if escaped {
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else if *byte == quote {
            return Some(start + offset);
        }
    }
    None
}

fn find_unquoted_line_end(input: &str, start: usize) -> usize {
    input[start..]
        .find(['\n', '\r'])
        .map(|offset| start + offset)
        .unwrap_or(input.len())
}

fn find_unquoted_value_end(input: &str, start: usize) -> usize {
    // Consume the closing bracket and any suffix, never trust marker prefixes.
    let scan_start = if input[start..].starts_with('[') {
        input[start..]
            .find(']')
            .map(|offset| start + offset + 1)
            .unwrap_or(input.len())
    } else {
        start
    };
    for (offset, character) in input[scan_start..].char_indices() {
        if matches!(character, ',' | ';' | '}' | ']' | '\n' | '\r') || character.is_whitespace() {
            return scan_start + offset;
        }
    }
    input.len()
}

fn skip_unicode_whitespace(input: &str, mut start: usize) -> usize {
    while start < input.len() {
        let Some(character) = input[start..].chars().next() else {
            break;
        };
        if !character.is_whitespace() {
            break;
        }
        start += character.len_utf8();
    }
    start
}

fn is_token_boundary(input: &str, start: usize, end: usize) -> bool {
    let before_ok = start == 0
        || (!input.as_bytes()[start - 1].is_ascii_alphanumeric()
            && input.as_bytes()[start - 1] != b'_');
    let after_ok = end == input.len()
        || (!input.as_bytes()[end].is_ascii_alphanumeric() && input.as_bytes()[end] != b'_');
    before_ok && after_ok
}

fn replace_ranges(input: &str, ranges: &[(usize, usize, String)]) -> String {
    if ranges.is_empty() {
        return input.to_owned();
    }
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    for (start, end, replacement) in ranges {
        if *start < cursor || *end <= *start || *end > input.len() {
            continue;
        }
        output.push_str(&input[cursor..*start]);
        output.push_str(replacement);
        cursor = *end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn merge_ranges(mut ranges: Vec<(usize, usize, String)>) -> Vec<(usize, usize, String)> {
    ranges.sort_by_key(|(start, _, _)| *start);
    let mut merged: Vec<(usize, usize, String)> = Vec::new();
    for range in ranges {
        if let Some((_, end, _)) = merged.last_mut() {
            if range.0 < *end {
                *end = (*end).max(range.1);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

fn sensitive_key_label(key: &str) -> Option<&'static str> {
    let normalized = key
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    SENSITIVE_KEYS
        .iter()
        .find(|candidate| {
            candidate
                .bytes()
                .filter(|byte| byte.is_ascii_alphanumeric())
                .map(|byte| byte.to_ascii_lowercase())
                .eq(normalized.iter().copied())
        })
        .copied()
}

fn redaction_marker(label: &str) -> String {
    format!("[REDACTED:{}]", label.to_ascii_uppercase())
}

const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "authorization",
    "bearer_token",
    "client_secret",
    "cookie",
    "credential_response",
    "password",
    "private_key",
    "refresh_token",
    "secret",
    "secret_key",
    "session_token",
    "ssh_key",
    "token",
    "access_token",
];
