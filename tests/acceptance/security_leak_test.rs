// 敏感信息泄漏验收测试
//
// 测试目标：使用合成输入验证生产 Redactor 的已实现规则。
// 不声称 EventLog 集成、任意裸 API key 或通用 PII（邮箱、卡号、路径）识别。

use steerbox::redaction::{RedactionError, Redactor, FAIL_CLOSED_MARKER};

fn redact_sensitive_info(text: &str) -> String {
    Redactor::default()
        .redact_text(text)
        .expect("synthetic input is within the redaction limit")
}

#[test]
fn api_key_assignments_are_redacted() {
    let test_cases = vec![
        ("api_key=sk-1234567890abcdef", "api_key=[REDACTED:API_KEY]"),
        (
            "API_KEY: pk-9876543210fedcba",
            "API_KEY: [REDACTED:API_KEY]",
        ),
        ("api_key=abcdef123456", "api_key=[REDACTED:API_KEY]"),
    ];

    for (input, expected_pattern) in test_cases {
        let redacted = redact_sensitive_info(input);

        assert!(
            !redacted.contains("1234567890abcdef")
                && !redacted.contains("9876543210fedcba")
                && !redacted.contains("abcdef123456"),
            "API key 未被脱敏"
        );

        assert_eq!(redacted, expected_pattern);
    }
}

#[test]
fn passwords_are_redacted_in_urls() {
    let test_cases = vec![
        (
            "postgresql://user:MyP@ssw0rd@localhost:5432/db",
            "postgresql://user:[REDACTED:URL_PASSWORD]@localhost:5432/db",
        ),
        (
            "mysql://admin:secret123@db.example.com/mydb",
            "mysql://admin:[REDACTED:URL_PASSWORD]@db.example.com/mydb",
        ),
    ];

    for (input, expected_pattern) in test_cases {
        let redacted = redact_sensitive_info(input);

        assert!(
            !redacted.contains("MyP@ssw0rd") && !redacted.contains("secret123"),
            "密码未被脱敏"
        );
        assert!(!redacted.contains("ssw0rd"), "不得保留密码片段");

        assert_eq!(redacted, expected_pattern);
    }
}

#[test]
fn bearer_tokens_are_redacted() {
    let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.signature";
    let redacted = redact_sensitive_info(input);

    assert!(
        !redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
        "Bearer token 未被脱敏"
    );

    assert_eq!(redacted, "Authorization: [REDACTED:AUTHORIZATION]");
    assert_eq!(
        redact_sensitive_info("Bearer synthetic-bearer-token"),
        "Bearer [REDACTED:BEARER_TOKEN]"
    );
}

#[test]
fn jwt_tokens_are_redacted() {
    let input =
        "JWT fixture: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
    let redacted = redact_sensitive_info(input);

    assert!(
        !redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
        "JWT 未被脱敏"
    );
    assert_eq!(redacted, "JWT fixture: [REDACTED:JWT]");
}

#[test]
fn private_key_material_is_redacted() {
    for label in ["PRIVATE KEY", "RSA PRIVATE KEY", "OPENSSH PRIVATE KEY"] {
        let input = format!(
            "before\n-----BEGIN {label}-----\nsynthetic-key-material\n-----END {label}-----\nafter"
        );
        let redacted = redact_sensitive_info(&input);
        assert!(!redacted.contains("synthetic-key-material"));
        assert!(redacted.contains("[REDACTED:PRIVATE_KEY]"));
        assert!(redacted.starts_with("before\n"));
        assert!(redacted.ends_with("\nafter"));
    }
}

#[test]
fn nested_json_sensitive_fields_are_redacted() {
    let input = serde_json::json!({
        "account": "synthetic-build-bot",
        "password": "synthetic-json-password",
        "nested": [{"api_key": "synthetic-json-api-key"}]
    });
    let redacted = redact_sensitive_info(&input.to_string());
    let value: serde_json::Value = serde_json::from_str(&redacted).expect("redacted JSON parses");
    assert_eq!(value["account"], "synthetic-build-bot");
    assert_eq!(value["password"], "[REDACTED:PASSWORD]");
    assert_eq!(value["nested"][0]["api_key"], "[REDACTED:API_KEY]");
    assert!(!redacted.contains("synthetic-json-password"));
    assert!(!redacted.contains("synthetic-json-api-key"));
}

#[test]
fn oversized_sensitive_input_fails_closed() {
    let redactor = Redactor::with_max_input_bytes(8);
    let input = "password=synthetic-oversized-password";
    assert_eq!(
        redactor.redact_text(input),
        Err(RedactionError::InputTooLarge { limit: 8 })
    );
    assert_eq!(redactor.redact_text_fail_closed(input), FAIL_CLOSED_MARKER);
    assert!(!redactor
        .redact_text_fail_closed(input)
        .contains("synthetic-oversized-password"));
}

#[test]
fn password_assignments_are_redacted() {
    let test_cases = vec![
        ("password=MySecret123", "password=[REDACTED:PASSWORD]"),
        ("password: admin123", "password: [REDACTED:PASSWORD]"),
    ];

    for (input, expected_pattern) in test_cases {
        let redacted = redact_sensitive_info(input);

        assert!(
            !redacted.contains("MySecret123") && !redacted.contains("admin123"),
            "密码值未被脱敏"
        );

        assert_eq!(redacted, expected_pattern);
    }
}
