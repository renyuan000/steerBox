#![cfg(unix)]

use serde_json::{json, Value};
use std::fs;
use steerbox::recovery_inspector::{inspect_files, InspectLimits};

fn event(sequence: u64) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({
        "event_id":"private-id", "event_type":"unprojected_event", "schema_version":1,
        "sequence_no":sequence, "aggregate_type":"test", "aggregate_id":"private-id",
        "correlation_id":"private-id", "producer":"test", "occurred_at":"test",
        "recorded_at":"test", "payload":{"note":"private-body"}, "redaction_class":"summary"
    }))
    .unwrap();
    bytes.push(b'\n');
    bytes
}

fn inspect(bytes: &[u8], limits: InspectLimits) -> Value {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events");
    fs::write(&path, bytes).unwrap();
    let report = inspect_files(&path, None, limits).unwrap();
    let encoded = serde_json::to_value(&report).unwrap();
    assert_eq!(
        report.has_findings(),
        !encoded["issues"].as_array().unwrap().is_empty()
    );
    for text in [format!("{report:?}"), encoded.to_string()] {
        assert!(!text.contains("private-id"));
        assert!(!text.contains("private-body"));
        assert!(!text.contains(&root.path().display().to_string()));
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    encoded
}

#[test]
fn invalid_limits_fail_before_path_io_and_have_fixed_errors() {
    let defaults = InspectLimits::default();
    let root = tempfile::tempdir().unwrap();
    let absent = root.path().join("absent/events");
    for limits in [
        InspectLimits {
            events_bytes: 0,
            ..defaults
        },
        InspectLimits {
            events_bytes: defaults.events_bytes + 1,
            ..defaults
        },
        InspectLimits {
            line_bytes: 0,
            ..defaults
        },
        InspectLimits {
            line_bytes: defaults.line_bytes + 1,
            ..defaults
        },
        InspectLimits {
            event_records: 0,
            ..defaults
        },
        InspectLimits {
            event_records: defaults.event_records + 1,
            ..defaults
        },
        InspectLimits {
            state_bytes: 0,
            ..defaults
        },
        InspectLimits {
            state_bytes: defaults.state_bytes + 1,
            ..defaults
        },
    ] {
        let error = inspect_files(&absent, Some(&absent), limits).unwrap_err();
        assert_eq!(error.to_string(), "invalid recovery inspection limits");
        assert!(!format!("{error:?}").contains("absent"));
        assert!(!absent.parent().unwrap().exists());
    }
}

#[test]
fn byte_probe_is_within_cap_and_exact_cap_is_not_complete() {
    let bytes = event(1);
    for extra in [0, 1] {
        let limits = InspectLimits {
            events_bytes: bytes.len() + extra,
            ..InspectLimits::default()
        };
        let report = inspect(&bytes, limits);
        assert_eq!(report["events"]["read_bytes"], bytes.len());
        assert_eq!(report["events"]["sealed_prefix_bytes"], bytes.len());
        assert_eq!(
            report["events"]["status"],
            if extra == 0 {
                "limit_reached"
            } else {
                "complete"
            }
        );
        assert_eq!(
            report["projection"]["status"],
            if extra == 0 { "not_rebuilt" } else { "rebuilt" }
        );
    }
    let report = inspect(
        &bytes,
        InspectLimits {
            events_bytes: 3,
            ..InspectLimits::default()
        },
    );
    assert_eq!(report["events"]["read_bytes"], 3);
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["sealed_records"], 0);
}

#[test]
fn line_cap_includes_lf_and_exact_record_cap_allows_trailing_blank_lines() {
    let mut bytes = event(1);
    let size = bytes.len();
    let report = inspect(
        &bytes,
        InspectLimits {
            line_bytes: size - 1,
            ..InspectLimits::default()
        },
    );
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["sealed_records"], 0);
    bytes.extend(b" \t\n\n");
    let limits = InspectLimits {
        line_bytes: size,
        event_records: 1,
        ..InspectLimits::default()
    };
    let report = inspect(&bytes, limits);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["events"]["ignored_event_count"], 1);
    let prefix_len = bytes.len();
    bytes.extend(event(2));
    let report = inspect(&bytes, limits);
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["sealed_prefix_bytes"], prefix_len);
    assert_eq!(report["issues"][0]["line"], 4);
    assert_eq!(report["issues"][0]["byte_offset"], prefix_len);
}

#[test]
fn unsealed_eof_precedes_decoding_and_sealed_utf8_failure_stops_prefix() {
    for (bytes, status) in [
        (vec![255], "unterminated_record"),
        (vec![255, 10], "invalid_utf8"),
    ] {
        let report = inspect(&bytes, InspectLimits::default());
        assert_eq!(report["events"]["status"], status);
        assert_eq!(report["events"]["sealed_prefix_bytes"], 0);
    }
}

#[test]
fn state_byte_cap_is_conservative_and_malformed_state_is_not_repaired() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let state = root.path().join("state");
    fs::write(&events, b"").unwrap();
    let contents = b"{\"loops\":{},\"steps\":{}}";
    fs::write(&state, contents).unwrap();
    for (cap, expected) in [
        (contents.len(), "limit_reached"),
        (contents.len() + 1, "matches"),
    ] {
        let report = inspect_files(
            &events,
            Some(&state),
            InspectLimits {
                state_bytes: cap,
                ..InspectLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(report).unwrap()["state"]["status"],
            expected
        );
        assert_eq!(fs::read(&state).unwrap(), contents);
    }
}
