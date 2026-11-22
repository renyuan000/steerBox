#![cfg(unix)]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const PRIVATE: &str = "synthetic-private-body-and-identity";
const TIMESTAMP: &str = "2026-11-22T00:00:00Z";
const READONLY_ROOT: &str = "STEERBOX_RECOVERY_INSPECT_READONLY_TEST_ROOT";

struct BoundedChild(Option<Child>);

impl BoundedChild {
    fn run(command: &mut Command) -> Output {
        Self::run_with_timeout(command, Duration::from_secs(10))
    }

    fn run_with_timeout(command: &mut Command, timeout: Duration) -> Output {
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self::wait(child, timeout)
    }

    fn wait(child: Child, timeout: Duration) -> Output {
        let mut child = Self(Some(child));
        let deadline = Instant::now() + timeout;
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

fn event(sequence: u64, event_type: &str) -> Value {
    json!({
        "event_id": format!("{PRIVATE}-{sequence}"), "event_type": event_type,
        "schema_version": 1, "sequence_no": sequence, "aggregate_type": "loop_run",
        "aggregate_id": PRIVATE, "portfolio_id": null, "project_id": PRIVATE,
        "goal_id": PRIVATE, "task_id": null, "task_run_id": PRIVATE,
        "attempt_id": null, "loop_run_id": PRIVATE, "step_id": null,
        "correlation_id": PRIVATE, "causation_id": null, "producer": PRIVATE,
        "occurred_at": TIMESTAMP, "recorded_at": TIMESTAMP,
        "payload": {"note": PRIVATE}, "redaction_class": "summary"
    })
}

fn sealed(event: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(event).unwrap();
    bytes.push(b'\n');
    bytes
}

// Excludes atime: application zero-write does not promise kernel atime suppression.
type Snapshot = BTreeMap<PathBuf, (u64, u64, u32, u32, u32, u64, Vec<u8>)>;

fn snapshot(root: &Path) -> Snapshot {
    fn visit(root: &Path, path: &Path, entries: &mut Snapshot) {
        let meta = fs::symlink_metadata(path).unwrap();
        let bytes = if meta.is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if meta.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        entries.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (
                meta.dev(),
                meta.ino(),
                meta.mode(),
                meta.uid(),
                meta.gid(),
                meta.nlink(),
                bytes,
            ),
        );
        if meta.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn invoke(root: &Path, events: &Path, state: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_steerbox"));
    command.args(["recovery", "inspect"]).arg(events);
    if let Some(state) = state {
        command.arg("--state").arg(state);
    }
    BoundedChild::run(command.current_dir(root).env_clear())
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
    assert_eq!(actual, expected, "report fields must remain fixed");
}

fn inspect(root: &Path, events: &Path, state: Option<&Path>, exit: i32) -> Value {
    let before = snapshot(root);
    let output = invoke(root, events, state);
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        snapshot(root),
        before,
        "inspection must not mutate or create files"
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("exactly one JSON report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["mode"], "diagnostic_only");
    assert_eq!(report["coverage"], "events_state_projection_only");
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
        ],
    );
    keys(
        &report["events"],
        &[
            "status",
            "read_bytes",
            "sealed_prefix_bytes",
            "sealed_records",
            "last_sequence_no",
            "ignored_event_count",
        ],
    );
    keys(
        &report["projection"],
        &["status", "loop_count", "step_count"],
    );
    keys(&report["state"], &["status"]);
    assert_eq!(report["issues"].as_array().unwrap().is_empty(), exit == 0);
    for issue in report["issues"].as_array().unwrap() {
        keys(issue, &["source", "code", "line", "byte_offset"]);
    }
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(PRIVATE), "private fixture data leaked");
        assert!(
            !text.contains(&root.display().to_string()),
            "private path leaked"
        );
        assert!(!text.to_ascii_lowercase().contains("usage:"));
        assert!(!text.contains("resumable"));
        assert!(!text.contains("restored"));
    }
    report
}

fn issue(report: &Value, source: &str, code: &str, line: Option<u64>, offset: Option<u64>) {
    assert!(
        report["issues"].as_array().unwrap().iter().any(|issue| {
            issue == &json!({"source": source, "code": code, "line": line, "byte_offset": offset})
        }),
        "missing fixed issue {source}/{code}: {report}"
    );
}

#[test]
fn missing_events_reports_findings_without_creating_parent_data_or_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("absent/events.jsonl");
    let report = inspect(root.path(), &path, None, 2);
    assert_eq!(report["events"]["status"], "missing");
    assert_eq!(report["events"]["read_bytes"], 0);
    assert_eq!(report["events"]["sealed_records"], 0);
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    assert_eq!(report["state"]["status"], "not_requested");
    assert!(!report["issues"].as_array().unwrap().is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn empty_events_are_complete_and_rebuild_an_empty_projection() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    fs::write(&path, b"").unwrap();
    let report = inspect(root.path(), &path, None, 0);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["events"]["read_bytes"], 0);
    assert_eq!(report["events"]["sealed_prefix_bytes"], 0);
    assert_eq!(report["events"]["sealed_records"], 0);
    assert_eq!(report["events"]["last_sequence_no"], Value::Null);
    assert_eq!(
        report["projection"],
        json!({"status": "rebuilt", "loop_count": 0, "step_count": 0})
    );
    assert_eq!(report["issues"], json!([]));
}

#[test]
fn valid_loop_started_rebuilds_counts_without_exposing_identity_payload_or_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let bytes = sealed(&event(1, "loop_started"));
    fs::write(&path, &bytes).unwrap();
    let report = inspect(root.path(), &path, None, 0);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["events"]["sealed_prefix_bytes"], bytes.len());
    assert_eq!(report["events"]["sealed_records"], 1);
    assert_eq!(report["events"]["last_sequence_no"], 1);
    assert_eq!(
        report["projection"],
        json!({"status": "rebuilt", "loop_count": 1, "step_count": 0})
    );
    assert_eq!(report["issues"], json!([]));
}

#[test]
fn complete_loop_and_step_started_history_rebuilds_one_loop_and_one_step() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut step = event(2, "step_started");
    step["aggregate_type"] = json!("loop_step");
    step["aggregate_id"] = json!(format!("{PRIVATE}-step"));
    step["step_id"] = step["aggregate_id"].clone();
    step["payload"]["phase"] = json!("observe");
    step["payload"]["step_no"] = json!(1);
    let mut bytes = sealed(&event(1, "loop_started"));
    bytes.extend(sealed(&step));
    fs::write(&path, &bytes).unwrap();
    let report = inspect(root.path(), &path, None, 0);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["events"]["sealed_prefix_bytes"], bytes.len());
    assert_eq!(report["events"]["sealed_records"], 2);
    assert_eq!(report["events"]["last_sequence_no"], 2);
    assert_eq!(report["events"]["ignored_event_count"], 0);
    assert_eq!(
        report["projection"],
        json!({"status": "rebuilt", "loop_count": 1, "step_count": 1})
    );
    assert_eq!(report["issues"], json!([]));
}

#[test]
fn parseable_json_without_lf_is_not_sealed_and_never_repaired() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    fs::write(
        &path,
        serde_json::to_vec(&event(1, "loop_started")).unwrap(),
    )
    .unwrap();
    let report = inspect(root.path(), &path, None, 2);
    assert_eq!(report["events"]["status"], "unterminated_record");
    assert_eq!(report["events"]["sealed_prefix_bytes"], 0);
    assert_eq!(report["events"]["sealed_records"], 0);
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    issue(&report, "events", "unterminated_record", Some(1), Some(0));
}

#[test]
fn invalid_json_stops_at_first_bad_physical_line_and_preserves_prefix() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    for interior in [false, true] {
        let mut prefix = Vec::new();
        if interior {
            prefix.extend(sealed(&event(1, "loop_started")));
            prefix.extend(b" \t\n");
        }
        let mut bytes = prefix.clone();
        bytes.extend(format!("{{broken-{PRIVATE}}}\n").as_bytes());
        bytes.extend(sealed(&event(if interior { 2 } else { 1 }, "future_event")));
        fs::write(&path, bytes).unwrap();
        let report = inspect(root.path(), &path, None, 2);
        assert_eq!(report["events"]["status"], "invalid_json");
        assert_eq!(report["events"]["sealed_prefix_bytes"], prefix.len());
        assert_eq!(report["events"]["sealed_records"], u64::from(interior));
        assert_eq!(report["projection"]["status"], "not_rebuilt");
        issue(
            &report,
            "events",
            "invalid_json",
            Some(if interior { 3 } else { 1 }),
            Some(prefix.len() as u64),
        );
    }
}

#[test]
fn state_match_and_mismatch_are_diagnostics_not_repairs_or_staleness_claims() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.jsonl");
    let state = root.path().join("state.json");
    fs::write(&events, sealed(&event(1, "loop_started"))).unwrap();
    let matching = json!({"loops": {(PRIVATE): {
        "loop_run_id": PRIVATE, "goal_id": PRIVATE, "task_run_id": PRIVATE,
        "status": "running", "current_step_id": null, "started_at": TIMESTAMP,
        "finished_at": null, "last_sequence_no": 1
    }}, "steps": {}});
    fs::write(&state, serde_json::to_vec(&matching).unwrap()).unwrap();
    let report = inspect(root.path(), &events, Some(&state), 0);
    assert_eq!(report["state"]["status"], "matches");
    assert_eq!(report["issues"], json!([]));
    fs::write(&state, b"{\"loops\":{},\"steps\":{}}").unwrap();
    let report = inspect(root.path(), &events, Some(&state), 2);
    assert_eq!(report["state"]["status"], "mismatch");
    issue(&report, "state", "mismatch", None, None);
    assert!(!report.to_string().contains("stale"));
}

#[test]
fn unsupported_schema_and_missing_repeated_or_skipped_sequences_stop_the_prefix() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let prefix = sealed(&event(1, "loop_started"));
    let mut bad_events = Vec::new();
    let mut unsupported = event(2, "future_event");
    unsupported["schema_version"] = json!(2);
    bad_events.push((unsupported, "unsupported_schema"));
    for sequence in [None, Some(0), Some(1), Some(3)] {
        let mut invalid = event(2, "future_event");
        if let Some(sequence) = sequence {
            invalid["sequence_no"] = json!(sequence);
        } else {
            invalid.as_object_mut().unwrap().remove("sequence_no");
        }
        bad_events.push((invalid, "invalid_sequence"));
    }
    for (bad, code) in bad_events {
        let mut bytes = prefix.clone();
        bytes.extend(sealed(&bad));
        bytes.extend(sealed(&event(3, "future_event")));
        fs::write(&path, bytes).unwrap();
        let report = inspect(root.path(), &path, None, 2);
        assert_eq!(report["events"]["status"], code);
        assert_eq!(report["events"]["sealed_records"], 1);
        assert_eq!(report["events"]["sealed_prefix_bytes"], prefix.len());
        assert_eq!(report["events"]["last_sequence_no"], 1);
        assert_eq!(report["events"]["ignored_event_count"], 0);
        assert_eq!(report["projection"]["status"], "not_rebuilt");
        issue(&report, "events", code, Some(2), Some(prefix.len() as u64));
    }
}

#[test]
fn unknown_events_are_counted_but_not_claimed_as_fully_reconciled() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut bytes = b"\n \t\n".to_vec();
    bytes.extend(sealed(&event(1, "loop_started")));
    bytes.extend(sealed(&event(2, "synthetic_future_event")));
    fs::write(&path, &bytes).unwrap();
    let report = inspect(root.path(), &path, None, 0);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["events"]["sealed_prefix_bytes"], bytes.len());
    assert_eq!(report["events"]["sealed_records"], 2);
    assert_eq!(report["events"]["last_sequence_no"], 2);
    assert_eq!(report["events"]["ignored_event_count"], 1);
    assert_eq!(report["projection"]["loop_count"], 1);
    assert_eq!(report["projection"]["step_count"], 0);
}

fn fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
}

#[test]
fn incomplete_or_unprojectable_history_does_not_compare_the_fifo_state() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.jsonl");
    let state = root.path().join("state.fifo");
    fifo(&state);
    for invalid_projection in [false, true] {
        let mut bytes = sealed(&event(1, "loop_started"));
        if invalid_projection {
            bytes.extend(sealed(&event(2, "loop_started")));
        } else {
            bytes.extend(b"\xff\n");
        }
        fs::write(&events, bytes).unwrap();
        let report = inspect(root.path(), &events, Some(&state), 2);
        assert_eq!(report["state"]["status"], "not_compared");
        assert!(!report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["source"] == "state"));
        if invalid_projection {
            assert_eq!(report["events"]["status"], "complete");
            assert_eq!(report["projection"]["status"], "invalid");
            issue(&report, "projection", "invalid_projection", None, None);
        } else {
            assert_eq!(report["events"]["status"], "invalid_utf8");
            assert_eq!(report["events"]["sealed_records"], 1);
            assert_eq!(report["projection"]["status"], "not_rebuilt");
        }
    }
}

#[test]
fn missing_and_invalid_state_report_fixed_findings_without_persistence() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.jsonl");
    let state = root.path().join("absent/state.json");
    fs::write(&events, b"").unwrap();
    let report = inspect(root.path(), &events, Some(&state), 2);
    assert_eq!(report["state"]["status"], "missing");
    assert!(!root.path().join("absent").exists());
    let state = root.path().join("state.json");
    for bytes in [b"\xff".as_slice(), b"{broken}", b"[]"] {
        fs::write(&state, bytes).unwrap();
        let report = inspect(root.path(), &events, Some(&state), 2);
        assert_eq!(report["state"]["status"], "invalid");
        assert_eq!(report["projection"]["status"], "rebuilt");
    }
}

fn unsafe_fixture(root: &Path, shape: &str, target_bytes: &[u8]) -> PathBuf {
    let target = root.join("target");
    let path = root.join("unsafe");
    fs::write(&target, target_bytes).unwrap();
    match shape {
        "symlink" => symlink(&target, &path).unwrap(),
        "dangling" => symlink(root.join("missing"), &path).unwrap(),
        "hardlink" => fs::hard_link(&target, &path).unwrap(),
        "fifo" => fifo(&path),
        "directory" => {
            fs::create_dir(&path).unwrap();
            fs::write(path.join("marker"), PRIVATE).unwrap();
        }
        "parent_symlink" => {
            let real = root.join("real");
            fs::create_dir(&real).unwrap();
            fs::write(real.join("data"), target_bytes).unwrap();
            symlink(&real, &path).unwrap();
            return path.join("data");
        }
        _ => panic!("unknown unsafe fixture"),
    }
    path
}

#[test]
fn unsafe_events_and_state_paths_are_refused_without_touching_targets_or_blocking() {
    for source in ["events", "state"] {
        for shape in [
            "symlink",
            "dangling",
            "hardlink",
            "fifo",
            "directory",
            "parent_symlink",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = unsafe_fixture(
                root.path(),
                shape,
                if source == "events" {
                    b""
                } else {
                    b"{\"loops\":{},\"steps\":{}}"
                },
            );
            let report = if source == "events" {
                inspect(root.path(), &path, None, 2)
            } else {
                let events = root.path().join("events.jsonl");
                fs::write(&events, b"").unwrap();
                inspect(root.path(), &events, Some(&path), 2)
            };
            assert_eq!(report[source]["status"], "unsafe_path", "{source}/{shape}");
        }
    }
}

#[test]
fn default_line_and_event_record_limits_do_not_report_complete_or_project_a_prefix() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.jsonl");
    let mut oversized = event(1, "future_event");
    oversized["payload"]["note"] = json!("x".repeat(1024 * 1024));
    fs::write(&path, sealed(&oversized)).unwrap();
    let report = inspect(root.path(), &path, None, 2);
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["sealed_records"], 0);
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    let mut bytes = Vec::new();
    let mut prefix_len = 0;
    for sequence in 1..=10_001 {
        bytes.extend(sealed(&event(sequence, "future_event")));
        if sequence == 10_000 {
            prefix_len = bytes.len();
        }
    }
    assert!(bytes.len() < 16 * 1024 * 1024);
    fs::write(&path, bytes).unwrap();
    let report = inspect(root.path(), &path, None, 2);
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["sealed_records"], 10_000);
    assert_eq!(report["events"]["sealed_prefix_bytes"], prefix_len);
    assert_eq!(report["events"]["ignored_event_count"], 10_000);
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    assert!(report["events"]["read_bytes"].as_u64().unwrap() <= 16 * 1024 * 1024);
}

#[test]
fn exact_default_events_and_state_byte_caps_are_incomplete_without_an_eof_probe() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.jsonl");
    let cap = 16 * 1024 * 1024;
    let mut bytes = vec![b' '; cap];
    for end in (512 * 1024..=cap).step_by(512 * 1024) {
        bytes[end - 1] = b'\n';
    }
    fs::write(&events, bytes).unwrap();
    let report = inspect(root.path(), &events, None, 2);
    assert_eq!(report["events"]["status"], "limit_reached");
    assert_eq!(report["events"]["read_bytes"], cap);
    assert_eq!(report["projection"]["status"], "not_rebuilt");
    fs::write(&events, b"").unwrap();
    let state = root.path().join("state.json");
    let mut bytes = b"{\"loops\":{},\"steps\":{}}".to_vec();
    bytes.resize(2 * 1024 * 1024, b' ');
    fs::write(&state, bytes).unwrap();
    let report = inspect(root.path(), &events, Some(&state), 2);
    assert_eq!(report["events"]["status"], "complete");
    assert_eq!(report["state"]["status"], "limit_reached");
}

#[test]
fn unsupported_or_extra_arguments_exit_one_without_creating_or_mutating_files() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.fifo");
    fifo(&events);
    let before = snapshot(root.path());
    for suffix in [
        vec![],
        vec!["--state"],
        vec!["--repair"],
        vec!["extra"],
        vec!["--state", "state.json", "--state", "again.json"],
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_steerbox"));
        command
            .args(["recovery", "inspect"])
            .current_dir(root.path())
            .env_clear();
        if !suffix.is_empty() {
            command.arg(&events).args(suffix);
        }
        let output = BoundedChild::run(&mut command);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            output.stdout.is_empty(),
            "syntax errors are not diagnostic reports"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE));
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains(&root.path().display().to_string())
        );
        assert_eq!(snapshot(root.path()), before);
    }
}

#[test]
fn closed_stdout_pipe_is_output_failure_exit_one_for_clean_and_finding_reports() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events.jsonl");
    for finding in [false, true] {
        fs::write(&events, if finding { PRIVATE.as_bytes() } else { b"" }).unwrap();
        let report = inspect(root.path(), &events, None, if finding { 2 } else { 0 });
        assert_eq!(
            report["events"]["status"],
            if finding {
                "unterminated_record"
            } else {
                "complete"
            }
        );
        let before = snapshot(root.path());
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        // SAFETY: pipe returned two distinct new descriptors, owned exactly once.
        let read_end = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let write_end = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        // No reader exists before spawn; EPIPE does not depend on scheduling.
        drop(read_end);
        let child = Command::new(env!("CARGO_BIN_EXE_steerbox"))
            .args(["recovery", "inspect"])
            .arg(&events)
            .current_dir(root.path())
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::from(fs::File::from(write_end)))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = BoundedChild::wait(child, Duration::from_secs(10));
        assert_eq!(
            output.status.code(),
            Some(1),
            "finding={finding}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.lines().next(),
            Some("steerbox: recovery inspection output failed")
        );
        for forbidden in [
            PRIVATE,
            root.path().to_str().unwrap(),
            "Broken pipe",
            "os error",
        ] {
            assert!(
                !stderr.contains(forbidden),
                "output failure leaked private or raw error details"
            );
        }
        assert_eq!(snapshot(root.path()), before);
    }
}

struct ReadonlyCleanup(PathBuf);

impl Drop for ReadonlyCleanup {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn diagnostic_succeeds_with_real_readonly_files_and_unwritable_parent() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let _cleanup = ReadonlyCleanup(root.path().join("readonly"));
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "recovery_readonly_probe", "--nocapture"])
        .env_clear()
        .env(READONLY_ROOT, root.path());
    // Let the probe's inner CLI deadline fire and reap that child first.
    let output = BoundedChild::run_with_timeout(&mut command, Duration::from_secs(20));
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn recovery_readonly_probe() {
    let Some(root) = std::env::var_os(READONLY_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let parent = root.join("readonly");
    fs::create_dir(&parent).unwrap();
    let events = parent.join("events.jsonl");
    let state = parent.join("state.json");
    fs::write(&events, b"").unwrap();
    fs::write(&state, b"{\"loops\":{},\"steps\":{}}").unwrap();
    for path in [&events, &state] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o444)).unwrap();
    }
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    if unsafe { libc::geteuid() } == 0 {
        assert_eq!(unsafe { libc::setgroups(0, std::ptr::null()) }, 0);
        assert_eq!(unsafe { libc::setgid(65534) }, 0);
        assert_eq!(unsafe { libc::setuid(65534) }, 0);
        assert_eq!(unsafe { libc::geteuid() }, 65534);
        assert_eq!(unsafe { libc::getegid() }, 65534);
        assert_eq!(unsafe { libc::getgroups(0, std::ptr::null_mut()) }, 0);
    }
    for path in [&events, &state] {
        assert_eq!(
            OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    assert_eq!(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(parent.join("must-not-exist"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let report = inspect(&root, &events, Some(&state), 0);
    assert_eq!(report["state"]["status"], "matches");
}
