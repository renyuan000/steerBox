#![cfg(unix)]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use steerbox::recovery_inspector::{inspect_files, InspectLimits};

const PRIVATE: &str = "synthetic-private-audit-identity-and-summary";
const TIMESTAMP: &str = "2026-11-22T00:00:00Z";

// Same bounded-child pattern as recovery_inspect_cli.rs: failures kill and reap.
struct BoundedChild(Option<Child>);

impl BoundedChild {
    fn run(command: &mut Command) -> Output {
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = Self(Some(child));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return child.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "inspection child exceeded its deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for BoundedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// Excludes atime: diagnostic zero-write does not promise kernel atime suppression.
type Snapshot = BTreeMap<PathBuf, (u64, u64, u32, u32, u32, u64, Vec<u8>)>;

fn snapshot(root: &Path) -> Snapshot {
    fn visit(root: &Path, path: &Path, entries: &mut Snapshot) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.is_symlink(), "only ordinary fixtures belong here");
        let bytes = if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        entries.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
                metadata.uid(),
                metadata.gid(),
                metadata.nlink(),
                bytes,
            ),
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn invoke(root: &Path, arguments: &[&str]) -> Output {
    let before = snapshot(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_steerbox"));
    let output = BoundedChild::run(
        command
            .args(["recovery", "inspect"])
            .args(arguments)
            .current_dir(root)
            .env_clear(),
    );
    assert_eq!(snapshot(root), before, "diagnostics must not write files");
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(PRIVATE), "synthetic private content leaked");
        assert!(!text.contains(TIMESTAMP), "synthetic timestamp leaked");
        assert!(
            !text.contains(&root.display().to_string()),
            "input path leaked"
        );
    }
    output
}

fn keys(value: &Value, expected: &[&str]) {
    let mut actual = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "diagnostic fields must remain fixed");
}

fn report(root: &Path, arguments: &[&str], exit: i32) -> Value {
    let output = invoke(root, arguments);
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "reports must not print usage");
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    let report: Value = serde_json::from_slice(&output.stdout).expect("one complete JSON report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["mode"], "diagnostic_only");
    assert_eq!(
        report["coverage"],
        "events_state_projection_and_audit_observations_only"
    );
    keys(
        &report,
        &[
            "schema_version",
            "mode",
            "coverage",
            "events",
            "projection",
            "state",
            "issues",
            "tools",
            "side_effects",
        ],
    );
    for field in ["tools", "side_effects"] {
        keys(
            &report[field],
            &[
                "status",
                "read_bytes",
                "sealed_prefix_bytes",
                "sealed_records",
                "status_counts",
                "class_counts",
            ],
        );
        assert!(!report[field]["status_counts"].is_null());
        assert!(!report[field]["class_counts"].is_null());
    }
    for issue in report["issues"].as_array().unwrap() {
        keys(issue, &["source", "code", "line", "byte_offset"]);
    }
    assert_eq!(report["issues"].as_array().unwrap().is_empty(), exit == 0);
    report
}

fn empty_inputs(root: &Path) {
    for name in ["events.jsonl", "tools.fixture", "effects.fixture"] {
        fs::write(root.join(name), b"").unwrap();
    }
    fs::write(root.join("state.json"), b"{\"loops\":{},\"steps\":{}}").unwrap();
}

fn sealed(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    bytes
}

fn tool_record() -> Value {
    json!({
        "tool_call_id": PRIVATE, "tool_id": PRIVATE, "project_id": PRIVATE,
        "goal_id": PRIVATE, "task_run_id": PRIVATE, "loop_run_id": PRIVATE,
        "step_id": PRIVATE, "side_effect_class": "none", "status": "completed",
        "summary": PRIVATE, "created_at": TIMESTAMP
    })
}

fn side_effect_record() -> Value {
    json!({
        "side_effect_id": PRIVATE, "goal_id": PRIVATE, "loop_run_id": PRIVATE,
        "tool_id": PRIVATE, "class": "none", "status": "completed",
        "summary": PRIVATE, "created_at": TIMESTAMP
    })
}

#[test]
fn explicit_empty_audits_select_the_new_report_and_omissions_are_not_requested() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    for (options, tools, effects) in [
        (vec!["--tools", "tools.fixture"], true, false),
        (vec!["--side-effects", "effects.fixture"], false, true),
        (
            vec![
                "--tools",
                "tools.fixture",
                "--side-effects",
                "effects.fixture",
            ],
            true,
            true,
        ),
    ] {
        let mut arguments = vec!["events.jsonl"];
        arguments.extend(options);
        let report = report(root.path(), &arguments, 0);
        assert_eq!(report["events"]["status"], "complete");
        assert_eq!(report["state"]["status"], "not_requested");
        for (field, requested) in [("tools", tools), ("side_effects", effects)] {
            assert_eq!(
                report[field]["status"],
                if requested {
                    "complete"
                } else {
                    "not_requested"
                }
            );
            assert_eq!(report[field]["read_bytes"], 0);
            assert_eq!(report[field]["sealed_prefix_bytes"], 0);
            assert_eq!(report[field]["sealed_records"], 0);
        }
    }
}

#[test]
fn state_tools_and_side_effect_options_accept_all_six_orders() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    let options = [
        ["--state", "state.json"],
        ["--tools", "tools.fixture"],
        ["--side-effects", "effects.fixture"],
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut arguments = vec!["events.jsonl"];
        for index in order {
            arguments.extend(options[index]);
        }
        let report = report(root.path(), &arguments, 0);
        assert_eq!(report["state"]["status"], "matches");
        assert_eq!(report["tools"]["status"], "complete");
        assert_eq!(report["side_effects"]["status"], "complete");
    }
}

#[test]
fn missing_explicit_audit_files_report_findings_without_creating_their_parents() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    let report = report(
        root.path(),
        &[
            "events.jsonl",
            "--tools",
            "missing-tools/tools.jsonl",
            "--side-effects",
            "missing-effects/effects.jsonl",
        ],
        2,
    );
    assert_eq!(report["tools"]["status"], "missing");
    assert_eq!(report["side_effects"]["status"], "missing");
    assert!(!root.path().join("missing-tools").exists());
    assert!(!root.path().join("missing-effects").exists());
}

#[test]
fn malformed_events_do_not_suppress_independent_audit_observations_or_leak_bodies() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    fs::write(root.path().join("events.jsonl"), b"{not-json}\n").unwrap();
    let tools = sealed(&tool_record());
    let effects = sealed(&side_effect_record());
    fs::write(root.path().join("tools.fixture"), &tools).unwrap();
    fs::write(root.path().join("effects.fixture"), &effects).unwrap();
    let report = report(
        root.path(),
        &[
            "events.jsonl",
            "--tools",
            "tools.fixture",
            "--side-effects",
            "effects.fixture",
        ],
        2,
    );
    assert_eq!(report["events"]["status"], "invalid_json");
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    for (field, bytes) in [("tools", tools), ("side_effects", effects)] {
        assert_eq!(report[field]["status"], "complete");
        assert_eq!(report[field]["read_bytes"], bytes.len());
        assert_eq!(report[field]["sealed_prefix_bytes"], bytes.len());
        assert_eq!(report[field]["sealed_records"], 1);
    }
}

#[test]
fn a_bad_auxiliary_log_does_not_hide_the_other_requested_log() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    for bad in ["tools", "side_effects"] {
        fs::write(root.path().join("tools.fixture"), sealed(&tool_record())).unwrap();
        fs::write(
            root.path().join("effects.fixture"),
            sealed(&side_effect_record()),
        )
        .unwrap();
        let name = if bad == "tools" {
            "tools.fixture"
        } else {
            "effects.fixture"
        };
        fs::write(root.path().join(name), b"{not-json}\n").unwrap();
        let report = report(
            root.path(),
            &[
                "events.jsonl",
                "--tools",
                "tools.fixture",
                "--side-effects",
                "effects.fixture",
            ],
            2,
        );
        let good = if bad == "tools" {
            "side_effects"
        } else {
            "tools"
        };
        assert_eq!(report[bad]["status"], "invalid_json");
        assert_eq!(report[good]["status"], "complete");
        assert_eq!(report[good]["sealed_records"], 1);
    }
}

#[test]
fn explicit_paths_do_not_initialize_config_or_read_derived_audit_names() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    for name in ["config.json", "tool-calls.jsonl", "side-effects.jsonl"] {
        fs::write(root.path().join(name), PRIVATE.as_bytes()).unwrap();
    }
    let report = report(
        root.path(),
        &[
            "events.jsonl",
            "--tools",
            "tools.fixture",
            "--side-effects",
            "effects.fixture",
        ],
        0,
    );
    assert_eq!(report["tools"]["status"], "complete");
    assert_eq!(report["side_effects"]["status"], "complete");
    assert!(!root.path().join("tools.fixture.lock").exists());
    assert!(!root.path().join("effects.fixture.lock").exists());
}

#[test]
fn existing_calls_keep_the_exact_original_json_without_audit_fields() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    fs::write(root.path().join("tool-calls.jsonl"), PRIVATE.as_bytes()).unwrap();
    fs::write(root.path().join("side-effects.jsonl"), PRIVATE.as_bytes()).unwrap();
    let events = root.path().join("events.jsonl");
    let state = root.path().join("state.json");
    for state_requested in [false, true] {
        let mut arguments = vec!["events.jsonl"];
        if state_requested {
            arguments.extend(["--state", "state.json"]);
        }
        let output = invoke(root.path(), &arguments);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let expected = inspect_files(
            &events,
            state_requested.then_some(state.as_path()),
            InspectLimits::default(),
        )
        .unwrap();
        let mut encoded = serde_json::to_vec(&expected).unwrap();
        encoded.push(b'\n');
        assert_eq!(output.stdout, encoded);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report.get("tools").is_none());
        assert!(report.get("side_effects").is_none());
    }
}

#[test]
fn malformed_new_option_requests_are_syntax_errors_not_diagnostic_reports() {
    let root = tempfile::tempdir().unwrap();
    empty_inputs(root.path());
    let mut invalid = vec![
        vec![],
        vec![""],
        vec!["--tools", "tools.fixture"],
        vec!["events.jsonl", "--unknown", "tools.fixture"],
        vec!["events.jsonl", "--tools", "tools.fixture", "extra"],
        vec![
            "events.jsonl",
            "--side-effects",
            "effects.fixture",
            "--repair",
        ],
    ];
    for flag in ["--state", "--tools", "--side-effects"] {
        invalid.extend([
            vec!["events.jsonl", flag],
            vec!["events.jsonl", flag, ""],
            vec!["events.jsonl", flag, "--unknown"],
            vec!["events.jsonl", flag, "file.json", flag, "again.json"],
        ]);
        for value in ["--state", "--tools", "--side-effects"] {
            invalid.push(vec!["events.jsonl", flag, value, "file.json"]);
        }
    }
    for arguments in invalid {
        let output = invoke(root.path(), &arguments);
        assert_eq!(output.status.code(), Some(1), "arguments={arguments:?}");
        assert!(output.stdout.is_empty(), "syntax errors are not reports");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid recovery inspect arguments"),
            "syntax validation must precede diagnostic inspection"
        );
    }
}
