use serde_json::json;
use steerbox::redaction::{redact_json, redact_text, RedactionError, Redactor, FAIL_CLOSED_MARKER};

#[test]
fn redacts_sensitive_assignments_and_auth_headers() {
    let input = "token=tok-123 password: p@ss authorization: Bearer abc.def secret";
    let output = redact_text(input).expect("text redaction should succeed");
    assert!(!output.contains("tok-123"));
    assert!(!output.contains("p@ss"));
    assert!(!output.contains("abc.def"));
    assert!(!output.contains("secret"));
    assert!(output.contains("[REDACTED:TOKEN]"));
    assert!(output.contains("[REDACTED:PASSWORD]"));
    assert!(output.contains("[REDACTED:AUTHORIZATION]"));
}

#[test]
fn redacts_private_keys_cookies_and_common_account_fields() {
    let input = concat!(
        "cookie=session-cookie\n",
        "client_secret=client-secret\n",
        "access_token=access-value\n",
        "session_token=session-value\n",
        "ssh_key=ssh-value\n",
        "-----BEGIN PRIVATE KEY-----\nprivate-material\n-----END PRIVATE KEY-----"
    );
    let output = redact_text(input).expect("text redaction should succeed");
    for secret in [
        "session-cookie",
        "client-secret",
        "access-value",
        "session-value",
        "ssh-value",
        "private-material",
    ] {
        assert!(!output.contains(secret), "secret leaked: {secret}");
    }
    assert!(output.contains("[REDACTED:PRIVATE_KEY]"));
}

#[test]
fn redacts_passwords_in_urls_without_destroying_url_shape() {
    let output = redact_text("https://alice:super-secret@example.test/api")
        .expect("URL redaction should succeed");
    assert!(!output.contains("super-secret"));
    assert!(output.contains("https://alice:[REDACTED:URL_PASSWORD]@example.test/api"));
}

#[test]
fn recursively_redacts_json_sensitive_keys_and_string_values() {
    let input = json!({
        "account": "build-bot",
        "password": "do-not-leak",
        "nested": [{"private_key": "key-material"}],
        "message": "Authorization: Bearer token-value"
    });
    let output = redact_json(&input.to_string()).expect("JSON redaction should succeed");
    let value: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
    assert_eq!(value["account"], "build-bot");
    assert_eq!(value["password"], "[REDACTED:PASSWORD]");
    assert_eq!(value["nested"][0]["private_key"], "[REDACTED:PRIVATE_KEY]");
    assert!(!output.contains("do-not-leak"));
    assert!(!output.contains("token-value"));
}

#[test]
fn text_mode_redacts_json_style_sensitive_assignments() {
    let output = redact_text(r#"{"password":"do-not-leak","token": "token-value"}"#)
        .expect("text redaction should succeed");
    assert!(!output.contains("do-not-leak"));
    assert!(!output.contains("token-value"));
    assert!(output.contains("[REDACTED:PASSWORD]"));
    assert!(output.contains("[REDACTED:TOKEN]"));
}

#[test]
fn over_limit_input_fails_closed_without_echoing_input() {
    let redactor = Redactor::with_max_input_bytes(8);
    let input = "password=super-secret";
    let error = redactor
        .redact_text(input)
        .expect_err("oversized input must fail closed");
    assert!(matches!(error, RedactionError::InputTooLarge { .. }));
    assert_eq!(redactor.redact_text_fail_closed(input), FAIL_CLOSED_MARKER);
    assert!(!redactor
        .redact_text_fail_closed(input)
        .contains("super-secret"));
}

#[test]
fn pem_blocks_are_redacted_without_neighboring_fields_hiding_the_bug() {
    for label in [
        "PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
    ] {
        let input = format!(
            "before\n-----BEGIN {label}-----\nsynthetic-key-body\n-----END {label}-----\nafter"
        );
        let output = redact_text(&input).unwrap();
        assert_eq!(
            output, "before\n[REDACTED:PRIVATE_KEY]\nafter",
            "PEM label {label}"
        );
    }
}

#[test]
fn incomplete_pem_blocks_fail_closed_from_the_header_onward() {
    for input in [
        "before\n-----BEGIN PRIVATE KEY-----\nsynthetic-key-body",
        "before\n-----BEGIN RSA PRIVATE KEY-----\nsynthetic-key-body\n-----END EC PRIVATE KEY-----",
    ] {
        assert_eq!(
            redact_text(input).unwrap(),
            "before\n[REDACTED:PRIVATE_KEY]"
        );
    }
}

#[test]
fn marker_prefixes_cannot_hide_credentials() {
    for input in [
        "password=[REDACTED:x]synthetic-secret",
        "password=[REDACTED:PASSWORD]synthetic-secret",
        "password=\"[REDACTED:PASSWORD]synthetic-secret\"",
        "Bearer [REDACTED:BEARER_TOKEN]synthetic-secret",
    ] {
        let output = redact_text(input).unwrap();
        assert!(!output.contains("synthetic-secret"));
        assert_eq!(redact_text(&output).unwrap(), output);
    }
}

#[test]
fn text_sensitive_keys_are_case_and_separator_normalized() {
    for key in [
        "private-key",
        "privateKey",
        "SSH-KEY",
        "sshKey",
        "clientSecret",
        "client-secret",
        "accessToken",
        "session-token",
    ] {
        assert!(
            !redact_text(&format!("{key}=synthetic-secret"))
                .unwrap()
                .contains("synthetic-secret"),
            "key {key}"
        );
    }
}

#[test]
fn public_pem_does_not_prevent_later_private_pem_redaction() {
    let public = "-----BEGIN CERTIFICATE-----\npublic\n-----END CERTIFICATE-----";
    let input = format!(
        "{public}\n-----BEGIN EC PRIVATE KEY-----\nsynthetic-secret\n-----END EC PRIVATE KEY-----"
    );
    let output = redact_text(&input).unwrap();
    assert!(output.starts_with(public));
    assert!(!output.contains("synthetic-secret"));
}

#[test]
fn unicode_json_fields_escaping_and_repeated_redaction_remain_safe() {
    let text = "说明：password=\"合成密码\\\"仍是密码\"；结果：安全";
    let output = redact_text(text).unwrap();
    assert!(!output.contains("合成密码"));
    assert!(!output.contains("仍是密码"));
    assert!(output.starts_with("说明："));
    assert_eq!(redact_text(&output).unwrap(), output);

    let input = r#"{"pass\u0077ord":"synthetic-secret","说明":"中文正常文本","nested":{"accessToken":"synthetic-token"}}"#;
    let json = redact_json(input).unwrap();
    assert!(!json.contains("synthetic-secret"));
    assert!(!json.contains("synthetic-token"));
    assert!(json.contains("中文正常文本"));
    assert_eq!(redact_json(&json).unwrap(), json);
}

#[test]
fn overlapping_sensitive_values_redact_the_full_union() {
    let input = "password=\"prefix authorization: synthetic-first\" synthetic-trailing";
    let output = redact_text(input).unwrap();
    assert!(!output.contains("synthetic-first"));
    assert!(!output.contains("synthetic-trailing"));
}

#[test]
fn invalid_json_and_oversized_json_fail_without_echoing_source() {
    let redactor = Redactor::with_max_input_bytes(32);
    let malformed = r#"{"password":"synthetic-secret""#;
    let error = redactor.redact_json(malformed).unwrap_err();
    assert_eq!(error, RedactionError::InvalidJson);
    assert!(!error.to_string().contains("synthetic-secret"));
    assert_eq!(
        redactor.redact_json_fail_closed(malformed),
        FAIL_CLOSED_MARKER
    );
    let oversized = json!({"message": "synthetic-secret".repeat(8)});
    assert!(matches!(
        redactor.redact_json_value(&oversized),
        Err(RedactionError::InputTooLarge { .. })
    ));
}

#[test]
fn ordinary_domains_and_code_names_are_not_jwt_tokens() {
    for input in [
        "https://api.example.com/v1",
        "package.module.function",
        "src.module.function",
    ] {
        assert_eq!(redact_text(input).unwrap(), input);
    }
}

#[test]
fn structurally_valid_signed_and_unsigned_jwts_are_redacted() {
    for token in [
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJmaXh0dXJlIn0.c3ludGhldGljLXNpZ25hdHVyZQ",
        "eyJhbGciOiJub25lIn0.e30.",
    ] {
        assert_eq!(redact_text(token).unwrap(), "[REDACTED:JWT]");
    }
}

#[test]
fn url_password_with_multiple_at_signs_is_fully_redacted() {
    let input = "postgres://alice:synthetic@remaining@example.test/db";
    let output = redact_text(input).unwrap();
    assert_eq!(
        output,
        "postgres://alice:[REDACTED:URL_PASSWORD]@example.test/db"
    );
    assert!(!output.contains("synthetic"));
    assert!(!output.contains("remaining"));
    assert_eq!(redact_text(&output).unwrap(), output);
}

#[test]
fn complete_json_in_text_mode_protects_escaped_sensitive_keys() {
    for input in [
        r#"{"pass\u0077ord":"synthetic-secret","note":"safe"}"#,
        r#"[{"access_\u0074oken":"synthetic-secret","note":"safe"}]"#,
    ] {
        let output = redact_text(input).unwrap();
        assert!(!output.contains("synthetic-secret"));
        assert!(output.contains("safe"));
        let _: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(redact_text(&output).unwrap(), output);
    }
}

#[test]
fn secret_key_is_redacted_consistently_in_text_and_json() {
    for key in ["secret_key", "secretKey", "SECRET-KEY"] {
        let text = redact_text(&format!("{key}=synthetic-secret")).unwrap();
        assert!(!text.contains("synthetic-secret"));
        assert!(text.contains("[REDACTED:SECRET_KEY]"));
        let input = json!({ key: "synthetic-secret" });
        let value = Redactor::default().redact_json_value(&input).unwrap();
        assert_eq!(value[key], "[REDACTED:SECRET_KEY]");
    }
}

#[test]
fn unicode_assignment_whitespace_does_not_expose_values() {
    for space in ['\u{00a0}', '\u{3000}'] {
        for input in [
            format!("password{space}={space}synthetic-secret"),
            format!("说明：\"password\"{space}:{space}\"synthetic-secret\""),
            format!("access_token{space}:{space}synthetic-secret"),
        ] {
            let output = redact_text(&input).unwrap();
            assert!(!output.contains("synthetic-secret"));
            assert_eq!(redact_text(&output).unwrap(), output);
        }
    }
}
