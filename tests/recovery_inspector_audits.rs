#![cfg(unix)]

use serde_json::{json, Value};
use std::{fs, path::Path};
use steerbox::recovery_inspector::{inspect_files_with_audits, AuditInspectLimits, InspectLimits};

fn record(tool: bool, status: &str, class: &str) -> Vec<u8> {
    let mut value = json!({
        "tool_id":"private-identity", "goal_id":"private-identity",
        "loop_run_id":"private-identity", "status":status,
        "summary":"private-body", "created_at":"private-time"
    });
    if tool {
        for key in ["tool_call_id", "project_id", "task_run_id", "step_id"] {
            value[key] = json!("private-identity");
        }
        value["side_effect_class"] = json!(class);
    } else {
        value["side_effect_id"] = json!("private-identity");
        value["class"] = json!(class);
    }
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    bytes
}

fn inspect(
    path: &Path,
    tools: Option<&Path>,
    effects: Option<&Path>,
    limits: InspectLimits,
    audits: AuditInspectLimits,
) -> Value {
    let report = inspect_files_with_audits(path, None, tools, effects, limits, audits).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        report.has_findings(),
        !value["issues"].as_array().unwrap().is_empty()
    );
    for text in [format!("{report:?}"), value.to_string()] {
        for private in [
            "private-identity",
            "private-body",
            "private-time",
            "private-status",
            "private-class",
        ] {
            assert!(!text.contains(private));
        }
        assert!(!text.contains(&path.display().to_string()));
        for auxiliary in [tools, effects].into_iter().flatten() {
            assert!(!text.contains(&auxiliary.display().to_string()));
        }
    }
    value
}

#[test]
fn completed_only_and_existing_class_aliases_are_observations_not_missing_history() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    fs::write(&events, []).unwrap();
    for tool in [true, false] {
        let path = root.path().join("audit");
        let mut bytes = Vec::new();
        for class in [
            "none",
            "workspaceread",
            "workspacewrite",
            "externalnetwork",
            "externalsystem",
        ] {
            bytes.extend(record(tool, " COMPLETED ", class));
        }
        fs::write(&path, &bytes).unwrap();
        let report = inspect(
            &events,
            tool.then_some(path.as_path()),
            (!tool).then_some(path.as_path()),
            InspectLimits::default(),
            AuditInspectLimits::default(),
        );
        let source = if tool { "tools" } else { "side_effects" };
        let omitted = if tool { "side_effects" } else { "tools" };
        assert_eq!(report[source]["status"], "complete");
        assert_eq!(report[source]["sealed_records"], 5);
        assert_eq!(report[source]["status_counts"]["completed"], 5);
        for class in [
            "none",
            "workspace_read",
            "workspace_write",
            "external_network",
            "external_system",
        ] {
            assert_eq!(report[source]["class_counts"][class], 1);
        }
        assert_eq!(report[omitted]["status"], "not_requested");
        assert!(report["issues"].as_array().unwrap().is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn statuses_are_counted_without_joining_repeated_ids_or_claiming_action_success() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let path = root.path().join("effects");
    fs::write(&events, []).unwrap();
    let mut bytes = Vec::new();
    for (index, status) in [
        "started",
        "completed",
        "verification_failed",
        "failed",
        "blocked",
        "planned",
        " SIDE_EFFECT_UNCERTAIN ",
        "private-status",
        "private-status",
    ]
    .into_iter()
    .enumerate()
    {
        let mut value: Value =
            serde_json::from_slice(&record(false, status, "private-class")).unwrap();
        if index == 8 {
            value["loop_run_id"] = json!("private-identity-other-loop");
        }
        bytes.extend(serde_json::to_vec(&value).unwrap());
        bytes.push(b'\n');
    }
    fs::write(&path, bytes).unwrap();
    let report = inspect(
        &events,
        None,
        Some(&path),
        InspectLimits::default(),
        AuditInspectLimits::default(),
    );
    let source = &report["side_effects"];
    assert_eq!(source["status"], "complete");
    assert_eq!(source["sealed_records"], 9);
    for status in [
        "started",
        "completed",
        "verification_failed",
        "failed",
        "blocked",
        "planned",
        "uncertain",
    ] {
        assert_eq!(source["status_counts"][status], 1);
    }
    assert_eq!(source["status_counts"]["unknown"], 2);
    assert_eq!(source["class_counts"]["unknown"], 9);
    let issues = report["issues"].as_array().unwrap();
    assert_eq!(
        issues.len(),
        6,
        "one issue per source/code, not per repeated record"
    );
    for code in [
        "failed_status_observed",
        "blocked_status_observed",
        "uncertain_status_observed",
        "verification_failed_status_observed",
        "unknown_status_observed",
        "unknown_class_observed",
    ] {
        assert!(
            issues.iter().any(|issue| issue["code"] == code),
            "missing fixed code {code}"
        );
    }
}

#[test]
fn every_auxiliary_source_obeys_framing_byte_line_and_record_limits() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let path = root.path().join("audit");
    fs::write(&events, []).unwrap();
    for tool in [true, false] {
        let sealed = record(tool, "completed", "workspace_read");
        let source = if tool { "tools" } else { "side_effects" };
        let mut cases = vec![
            (
                sealed[..sealed.len() - 1].to_vec(),
                "unterminated_record",
                InspectLimits::default(),
                AuditInspectLimits::default(),
            ),
            (
                b"{bad}\n".to_vec(),
                "invalid_json",
                InspectLimits::default(),
                AuditInspectLimits::default(),
            ),
            (
                b"\xff\n".to_vec(),
                "invalid_utf8",
                InspectLimits::default(),
                AuditInspectLimits::default(),
            ),
            (
                sealed.clone(),
                "limit_reached",
                InspectLimits {
                    line_bytes: sealed.len() - 1,
                    ..InspectLimits::default()
                },
                AuditInspectLimits::default(),
            ),
        ];
        let exact = if tool {
            AuditInspectLimits {
                tool_bytes: sealed.len(),
                ..AuditInspectLimits::default()
            }
        } else {
            AuditInspectLimits {
                side_effect_bytes: sealed.len(),
                ..AuditInspectLimits::default()
            }
        };
        cases.push((
            sealed.clone(),
            "limit_reached",
            InspectLimits::default(),
            exact,
        ));
        let mut twice = sealed.clone();
        twice.extend_from_slice(&sealed);
        cases.push((
            twice,
            "limit_reached",
            InspectLimits::default(),
            AuditInspectLimits {
                records: 1,
                ..AuditInspectLimits::default()
            },
        ));
        for (bytes, status, limits, audits) in cases {
            fs::write(&path, &bytes).unwrap();
            let report = inspect(
                &events,
                tool.then_some(path.as_path()),
                (!tool).then_some(path.as_path()),
                limits,
                audits,
            );
            assert_eq!(report[source]["status"], status);
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let mut with_blank = sealed.clone();
        with_blank.extend_from_slice(b" \n\n");
        fs::write(&path, &with_blank).unwrap();
        let report = inspect(
            &events,
            tool.then_some(path.as_path()),
            (!tool).then_some(path.as_path()),
            InspectLimits {
                line_bytes: sealed.len(),
                ..InspectLimits::default()
            },
            AuditInspectLimits {
                records: 1,
                ..AuditInspectLimits::default()
            },
        );
        assert_eq!(report[source]["status"], "complete");
        assert_eq!(report[source]["sealed_records"], 1);
        assert_eq!(report[source]["sealed_prefix_bytes"], with_blank.len());
    }
}

#[test]
fn missing_and_empty_sources_are_distinct_and_requested_audits_are_not_hidden() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("absent/events");
    let missing = root.path().join("missing/audit");
    let empty = root.path().join("empty");
    fs::write(&empty, []).unwrap();
    let report = inspect(
        &events,
        Some(&missing),
        Some(&empty),
        InspectLimits::default(),
        AuditInspectLimits::default(),
    );
    assert_eq!(report["events"]["status"], "missing");
    assert_eq!(report["tools"]["status"], "missing");
    assert_eq!(report["side_effects"]["status"], "complete");
    assert_eq!(report["side_effects"]["sealed_records"], 0);
    assert!(!events.parent().unwrap().exists());
    assert!(!missing.parent().unwrap().exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn auxiliary_and_aggregate_limits_are_validated_before_any_file_io() {
    let root = tempfile::tempdir().unwrap();
    let absent = root.path().join("missing/path");
    let max = AuditInspectLimits::default();
    for audits in [
        AuditInspectLimits {
            tool_bytes: 0,
            ..max
        },
        AuditInspectLimits {
            side_effect_bytes: 0,
            ..max
        },
        AuditInspectLimits { records: 0, ..max },
        AuditInspectLimits {
            total_bytes: 0,
            ..max
        },
        AuditInspectLimits {
            tool_bytes: max.tool_bytes + 1,
            ..max
        },
        AuditInspectLimits {
            side_effect_bytes: max.side_effect_bytes + 1,
            ..max
        },
        AuditInspectLimits {
            records: max.records + 1,
            ..max
        },
        AuditInspectLimits {
            total_bytes: max.total_bytes + 1,
            ..max
        },
        AuditInspectLimits {
            total_bytes: max.total_bytes - 1,
            ..max
        },
    ] {
        let error = inspect_files_with_audits(
            &absent,
            Some(&absent),
            Some(&absent),
            Some(&absent),
            InspectLimits::default(),
            audits,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid recovery inspection limits");
        assert!(!format!("{error:?}").contains("missing"));
    }
    let report = inspect_files_with_audits(
        &absent,
        Some(&absent),
        Some(&absent),
        Some(&absent),
        InspectLimits::default(),
        max,
    )
    .unwrap();
    assert!(report.has_findings(), "exact aggregate cap is accepted");
    assert!(!absent.parent().unwrap().exists());
}

#[test]
fn aggregate_counts_each_requested_source_even_when_paths_are_identical() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("empty");
    fs::write(&path, []).unwrap();
    let limits = InspectLimits {
        events_bytes: 4,
        state_bytes: 3,
        ..InspectLimits::default()
    };
    let audits = AuditInspectLimits {
        tool_bytes: 2,
        side_effect_bytes: 2,
        total_bytes: 11,
        ..AuditInspectLimits::default()
    };
    assert!(inspect_files_with_audits(
        &path,
        Some(&path),
        Some(&path),
        Some(&path),
        limits,
        audits
    )
    .is_ok());
    assert!(inspect_files_with_audits(
        &path,
        Some(&path),
        Some(&path),
        Some(&path),
        limits,
        AuditInspectLimits {
            total_bytes: 10,
            ..audits
        }
    )
    .is_err());
    assert!(inspect_files_with_audits(
        &path,
        None,
        None,
        None,
        limits,
        AuditInspectLimits {
            total_bytes: 4,
            ..audits
        }
    )
    .is_ok());
    assert!(inspect_files_with_audits(
        &path,
        Some(&path),
        None,
        None,
        limits,
        AuditInspectLimits {
            total_bytes: 4,
            ..audits
        }
    )
    .is_err());
    assert!(inspect_files_with_audits(
        &path,
        None,
        Some(&path),
        None,
        InspectLimits {
            line_bytes: 0,
            ..limits
        },
        audits
    )
    .is_err());
}

#[test]
fn corrupted_suffix_retains_only_the_sealed_typed_prefix_and_fixed_positions() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let path = root.path().join("audit");
    fs::write(&events, []).unwrap();
    for tool in [true, false] {
        let sealed = record(tool, "completed", "none");
        for (suffix, code) in [
            (b"{bad}\n".as_slice(), "invalid_json"),
            (b"\xff\n".as_slice(), "invalid_utf8"),
            (b"{}".as_slice(), "unterminated_record"),
            (b"{}\n".as_slice(), "invalid_json"),
        ] {
            let mut bytes = sealed.clone();
            bytes.extend_from_slice(suffix);
            fs::write(&path, &bytes).unwrap();
            let report = inspect(
                &events,
                tool.then_some(path.as_path()),
                (!tool).then_some(path.as_path()),
                InspectLimits::default(),
                AuditInspectLimits::default(),
            );
            let source = if tool { "tools" } else { "side_effects" };
            assert_eq!(report[source]["status"], code);
            assert_eq!(report[source]["sealed_records"], 1);
            assert_eq!(report[source]["status_counts"]["completed"], 1);
            assert_eq!(report[source]["sealed_prefix_bytes"], sealed.len());
            let issue = &report["issues"][0];
            assert_eq!(issue["source"], source);
            assert_eq!(issue["line"], 2);
            assert_eq!(issue["byte_offset"], sealed.len());
        }
    }
}

#[test]
fn known_class_aliases_and_nonterminal_records_do_not_imply_a_history_gap() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let path = root.path().join("audit");
    fs::write(&events, []).unwrap();
    for tool in [true, false] {
        let mut bytes = Vec::new();
        for status in ["planned", "started", "completed"] {
            for class in [
                "read_only",
                "readonly",
                "reversible_local_change",
                " WORKSPACE_WRITE ",
            ] {
                bytes.extend(record(tool, status, class));
            }
        }
        fs::write(&path, bytes).unwrap();
        let report = inspect(
            &events,
            tool.then_some(path.as_path()),
            (!tool).then_some(path.as_path()),
            InspectLimits::default(),
            AuditInspectLimits::default(),
        );
        assert!(report["issues"].as_array().unwrap().is_empty());
        let source = if tool { "tools" } else { "side_effects" };
        assert_eq!(report[source]["sealed_records"], 12);
        assert_eq!(report[source]["class_counts"]["none"], 6);
        assert_eq!(report[source]["class_counts"]["workspace_write"], 6);
    }
}

#[test]
fn repeated_observation_findings_keep_first_position_separately_for_each_source() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let tools = root.path().join("tools");
    let effects = root.path().join("effects");
    fs::write(&events, []).unwrap();
    for (tool, path) in [(true, &tools), (false, &effects)] {
        let mut bytes = b" \n".to_vec();
        bytes.extend(record(tool, "failed", "none"));
        bytes.extend(record(tool, "failed", "none"));
        fs::write(path, bytes).unwrap();
    }
    let report = inspect(
        &events,
        Some(&tools),
        Some(&effects),
        InspectLimits::default(),
        AuditInspectLimits::default(),
    );
    let issues = report["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2);
    for (index, source) in ["tools", "side_effects"].into_iter().enumerate() {
        assert_eq!(report[source]["status_counts"]["failed"], 2);
        assert_eq!(issues[index]["source"], source);
        assert_eq!(issues[index]["code"], "failed_status_observed");
        assert_eq!(issues[index]["line"], 2);
        assert_eq!(issues[index]["byte_offset"], 2);
    }
}
