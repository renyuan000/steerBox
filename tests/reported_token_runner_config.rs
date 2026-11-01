use std::{fs, process::Command};
use steerbox::runtime_config::{ConfigError, RuntimeConfig};

fn config_with_literal(literal: &str) -> String {
    // Keep numeric literals raw so JSON overflow is not rounded by a Value.
    format!(
        r#"{{"schema_version":1,"project_ref":"token-config-fixture","data_root":"/temp/steerbox/token-config-fixture","reported_token_limit":{literal}}}"#
    )
}

#[test]
fn missing_and_null_limits_default_off_and_serialize_without_the_field() {
    let default = RuntimeConfig::default_for_project("token-config-fixture").unwrap();
    let encoded = serde_json::to_value(&default).unwrap();
    assert!(encoded.get("reported_token_limit").is_none());
    assert_eq!(encoded["schema_version"], 1);
    let missing = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap();
    let null = RuntimeConfig::from_json_str(&config_with_literal("null")).unwrap();
    assert_eq!(missing, default);
    assert_eq!(null, default);
    assert!(serde_json::to_value(null)
        .unwrap()
        .get("reported_token_limit")
        .is_none());
}

#[test]
fn zero_positive_and_max_u64_limits_round_trip_and_load_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-config.json");
    for limit in [0, 1000, u64::MAX] {
        let raw = config_with_literal(&limit.to_string());
        let parsed = RuntimeConfig::from_json_str(&raw).unwrap();
        let encoded = serde_json::to_value(&parsed).unwrap();
        assert_eq!(encoded["reported_token_limit"].as_u64(), Some(limit));
        let roundtrip = RuntimeConfig::from_json_str(&encoded.to_string()).unwrap();
        assert_eq!(roundtrip, parsed);
        fs::write(&path, &raw).unwrap();
        assert_eq!(RuntimeConfig::load(&path).unwrap(), parsed);
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(["config", "validate"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("valid project=token-config-fixture "));
    }
}

#[test]
fn invalid_limit_literals_fail_parse_load_and_cli_without_echoing_values() {
    const MARKER: &str = "REPORTED_TOKEN_PRIVATE_MARKER";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-config.json");
    for literal in [
        "-1",
        "1.5",
        "1e2",
        "18446744073709551616",
        "true",
        "[]",
        "{}",
        r#""REPORTED_TOKEN_PRIVATE_MARKER""#,
        r#"["REPORTED_TOKEN_PRIVATE_MARKER"]"#,
        r#"{"REPORTED_TOKEN_PRIVATE_MARKER":"REPORTED_TOKEN_PRIVATE_MARKER"}"#,
    ] {
        let raw = config_with_literal(literal);
        fs::write(&path, &raw).unwrap();
        for error in [
            RuntimeConfig::from_json_str(&raw).unwrap_err(),
            RuntimeConfig::load(&path).unwrap_err(),
        ] {
            assert!(
                matches!(error, ConfigError::InvalidJson { .. }),
                "{error:?}"
            );
            for diagnostic in [error.to_string(), format!("{error:?}")] {
                assert!(diagnostic.contains("reported_token_limit"));
                assert!(!diagnostic.contains(MARKER));
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(["config", "validate"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("reported_token_limit"));
        assert!(!stderr.contains(MARKER));
    }
}

#[test]
fn reported_token_extension_does_not_relax_unknown_field_rejection() {
    let raw = r#"{"schema_version":1,"project_ref":"token-config-fixture","data_root":"/temp/steerbox/token-config-fixture","unrecognized_budget":1}"#;
    assert!(matches!(
        RuntimeConfig::from_json_str(raw),
        Err(ConfigError::UnknownField { field }) if field == "unrecognized_budget"
    ));
}
