#![cfg(unix)]

use std::ffi::CString;
use std::fs::{self, Metadata, OpenOptions};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use steerbox::audit_store::{AuditStoreError, RunSummary};

const PRIVATE_PATH: &str = "synthetic-private-run-summary";
const PROBE_ROOT: &str = "STEERBOX_RUN_SUMMARY_TEST_ROOT";
const PROBE_KIND: &str = "STEERBOX_RUN_SUMMARY_TEST_KIND";
const PROBE_SCENARIO: &str = "STEERBOX_RUN_SUMMARY_TEST_SCENARIO";
const PROBE_OPERATION: &str = "STEERBOX_RUN_SUMMARY_TEST_OPERATION";

fn summary(label: &str) -> RunSummary {
    RunSummary {
        schema_version: 1,
        project_id: "project".into(),
        goal_id: "goal".into(),
        loop_run_id: "loop".into(),
        task_run_id: "task".into(),
        final_status: label.into(),
        step_ids: vec!["step".into()],
        file_changes: vec!["README.md".into()],
        verification_status: "passed".into(),
        event_ids: vec!["event".into()],
        verification_event_ids: vec!["verification".into()],
        trace_ids: vec!["trace".into()],
        checkpoint_ids: vec!["checkpoint".into()],
        side_effect_ids: vec!["effect".into()],
        event_count: 7,
        generated_at: "2026-11-22T00:00:00Z".into(),
    }
}

fn identity(metadata: &Metadata) -> (u64, u64, u32, u32, u32, u64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
        metadata.nlink(),
    )
}

fn assert_io<T: std::fmt::Debug>(result: Result<T, AuditStoreError>) {
    assert!(
        matches!(result, Err(AuditStoreError::Io { .. })),
        "expected I/O error: {result:?}"
    );
}

fn invoke(path: &Path, operation: &str) -> Result<(), AuditStoreError> {
    match operation {
        "read" => RunSummary::read_json(path).map(|_| ()),
        "write" => summary("new").write_json(path),
        _ => panic!("unknown summary operation"),
    }
}

#[test]
fn exact_pretty_without_final_lf_and_long_to_short_in_place_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/summary.json");
    let long = summary(&"long-status-".repeat(64));
    long.write_json(&path).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&long).unwrap()
    );
    assert_ne!(fs::read(&path).unwrap().last(), Some(&b'\n'));
    assert_eq!(RunSummary::read_json(&path).unwrap(), long);
    // Existing non-owner-only mode is intentionally accepted and preserved.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let short = summary("done");
    short.write_json(&path).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&short).unwrap()
    );
    assert_eq!(identity(&fs::metadata(&path).unwrap()), identity(&before));
    assert_eq!(RunSummary::read_json(&path).unwrap(), short);
    // Compact JSON and surrounding whitespace remain valid single documents.
    fs::write(&path, serde_json::to_vec(&short).unwrap()).unwrap();
    assert_eq!(RunSummary::read_json(&path).unwrap(), short);
    fs::write(
        &path,
        format!(" \n{}\n\t", serde_json::to_string_pretty(&short).unwrap()),
    )
    .unwrap();
    assert_eq!(RunSummary::read_json(&path).unwrap(), short);
}

#[test]
fn newly_created_directories_and_summary_request_private_modes() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("new/history");
    let path = parent.join("summary.json");
    summary("new").write_json(&path).unwrap();
    for directory in [root.path().join("new"), parent] {
        assert_eq!(fs::metadata(directory).unwrap().mode() & 0o077, 0);
    }
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o077, 0);
    assert_eq!(RunSummary::read_json(&path).unwrap(), summary("new"));
}

#[test]
fn missing_leaf_and_missing_parent_are_io_errors_without_creation() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        root.path().join("missing.json"),
        root.path().join("missing/parent/summary.json"),
    ] {
        assert_io(RunSummary::read_json(path));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn invalid_utf8_is_io_but_json_and_schema_errors_keep_line_zero() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("summary.json");
    fs::write(&path, b"\xff").unwrap();
    assert_io(RunSummary::read_json(&path));
    for bytes in [
        b"".as_slice(),
        b"{",
        b"not-json",
        b"{}",
        b"{\"schema_version\":\"wrong-type\"}",
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            RunSummary::read_json(&path),
            Err(AuditStoreError::InvalidJson { line: 0, .. })
        ));
    }
    let document = serde_json::to_string(&summary("done")).unwrap();
    fs::write(&path, format!("{document}\n{document}")).unwrap();
    assert!(matches!(
        RunSummary::read_json(&path),
        Err(AuditStoreError::InvalidJson { line: 0, .. })
    ));
}

struct ProbeChild(Option<Child>);

impl ProbeChild {
    fn spawn(root: &Path, kind: &str, scenario: &str, operation: &str) -> Self {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "run_summary_process_probe", "--nocapture"])
            .env(PROBE_ROOT, root)
            .env(PROBE_KIND, kind)
            .env(PROBE_SCENARIO, scenario)
            .env(PROBE_OPERATION, operation)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self(Some(child))
    }

    fn finish(mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = self.0.take().unwrap().wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "summary probe failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                return;
            }
            assert!(Instant::now() < deadline, "summary operation timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn leaf_probe(root: &Path, scenario: &str, operation: &str) {
    let path = root.join(PRIVATE_PATH);
    let target = root.join("protected-target");
    let sentinel = serde_json::to_vec_pretty(&summary("protected")).unwrap();
    fs::write(&target, &sentinel).unwrap();
    match scenario {
        "symlink" => symlink(&target, &path).unwrap(),
        "dangling" => {
            fs::remove_file(&target).unwrap();
            symlink(&target, &path).unwrap();
        }
        "hardlink" => fs::hard_link(&target, &path).unwrap(),
        "fifo" => {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        "directory" => {
            fs::create_dir(&path).unwrap();
            fs::write(path.join("marker"), b"protected marker").unwrap();
        }
        _ => panic!("unknown leaf scenario"),
    }
    let before = fs::symlink_metadata(&path).unwrap();
    let target_before = target.exists().then(|| fs::metadata(&target).unwrap());
    let result = invoke(&path, operation);
    assert_eq!(
        identity(&fs::symlink_metadata(&path).unwrap()),
        identity(&before)
    );
    if let Some(before) = target_before {
        assert_eq!(
            fs::read(&target).unwrap(),
            sentinel,
            "protected target was modified"
        );
        assert_eq!(identity(&fs::metadata(&target).unwrap()), identity(&before));
    } else {
        assert!(!target.exists(), "dangling target was created");
    }
    if scenario == "directory" {
        assert_eq!(fs::read(path.join("marker")).unwrap(), b"protected marker");
        assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    }
    assert_io(result);
}

fn check_leaf(scenario: &str) {
    for operation in ["write", "read"] {
        let root = tempfile::tempdir().unwrap();
        ProbeChild::spawn(root.path(), "leaf", scenario, operation).finish();
    }
}

#[test]
fn leaf_symlinks_are_rejected_before_reading_or_overwriting_targets() {
    check_leaf("symlink");
}
#[test]
fn dangling_symlinks_never_create_targets() {
    check_leaf("dangling");
}
#[test]
fn leaf_hardlinks_are_rejected_without_truncating_aliases() {
    check_leaf("hardlink");
}
#[test]
fn fifo_read_and_write_fail_within_the_process_deadline() {
    check_leaf("fifo");
}
#[test]
fn directory_leaves_are_io_errors_and_preserve_contents() {
    check_leaf("directory");
}

#[test]
fn parent_symlink_read_and_write_never_follow_or_create_descendants() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join(PRIVATE_PATH);
    fs::create_dir(&target).unwrap();
    symlink(&target, &link).unwrap();
    let path = target.join("summary.json");
    let sentinel = serde_json::to_vec_pretty(&summary("protected")).unwrap();
    fs::write(&path, &sentinel).unwrap();
    let before = fs::metadata(&path).unwrap();
    assert_io(RunSummary::read_json(link.join("summary.json")));
    assert_io(summary("new").write_json(link.join("must-not-exist/summary.json")));
    assert_eq!(fs::read(&path).unwrap(), sentinel);
    assert_eq!(identity(&fs::metadata(&path).unwrap()), identity(&before));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
}

#[test]
fn invalid_full_paths_are_rejected_before_mkdir() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = vec![
        PathBuf::from(""),
        PathBuf::from("/"),
        PathBuf::from("."),
        PathBuf::from(".."),
        root.path().join("must-not-exist/../summary.json"),
        root.path().join("must-not-exist/nested/../summary.json"),
        root.path()
            .join("must-not-exist")
            .join(std::ffi::OsString::from_vec(b"bad\0name".to_vec())),
    ];
    for suffix in ["/", "/.", "/.."] {
        let mut raw = root
            .path()
            .join("must-not-exist/summary.json")
            .into_os_string();
        raw.push(suffix);
        paths.push(PathBuf::from(raw));
    }
    for path in paths {
        assert_io(RunSummary::read_json(&path));
        assert_io(summary("new").write_json(path));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

struct PermissionCleanup(PathBuf);

impl Drop for PermissionCleanup {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
}

fn drop_root_privileges(path: &Path) {
    if unsafe { libc::geteuid() } == 0 {
        if path.exists() {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            // Exclusively this probe's synthetic summary file.
            assert_eq!(unsafe { libc::chown(name.as_ptr(), 65534, 65534) }, 0);
        }
        assert_eq!(unsafe { libc::setgroups(0, std::ptr::null()) }, 0);
        assert_eq!(unsafe { libc::setgid(65534) }, 0);
        assert_eq!(unsafe { libc::setuid(65534) }, 0);
        assert_eq!(unsafe { libc::geteuid() }, 65534);
        assert_eq!(unsafe { libc::getegid() }, 65534);
        assert_eq!(unsafe { libc::getgroups(0, std::ptr::null_mut()) }, 0);
    }
}

fn permission_probe(root: &Path, scenario: &str) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
    let parent = root.join("history");
    fs::create_dir(&parent).unwrap();
    let path = parent.join(PRIVATE_PATH);
    let original = summary("original");
    fs::write(&path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    let mode = match scenario {
        "0200" => 0o200,
        "0400" => 0o400,
        "0000" => 0,
        _ => 0o600,
    };
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    let readonly_parent = scenario.starts_with("parent_");
    if scenario == "parent_missing" {
        fs::remove_file(&path).unwrap();
    }
    fs::set_permissions(
        &parent,
        fs::Permissions::from_mode(if readonly_parent { 0o555 } else { 0o755 }),
    )
    .unwrap();
    drop_root_privileges(&path);
    let before = path.exists().then(|| fs::metadata(&path).unwrap());
    let expected = match scenario {
        "0200" => {
            assert_eq!(
                OpenOptions::new()
                    .read(true)
                    .open(&path)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert_io(RunSummary::read_json(&path));
            summary("new").write_json(&path).unwrap();
            summary("new")
        }
        "0400" | "0000" => {
            assert_eq!(
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            if scenario == "0400" {
                assert_eq!(RunSummary::read_json(&path).unwrap(), original);
            } else {
                assert_eq!(
                    OpenOptions::new()
                        .read(true)
                        .open(&path)
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::PermissionDenied
                );
                assert_io(RunSummary::read_json(&path));
            }
            assert_io(summary("new").write_json(&path));
            original
        }
        "parent_existing" | "parent_missing" => {
            assert_eq!(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(parent.join("must-not-exist"))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert!(!parent.join("must-not-exist").exists());
            if scenario == "parent_existing" {
                assert_eq!(RunSummary::read_json(&path).unwrap(), original);
                summary("new").write_json(&path).unwrap();
                summary("new")
            } else {
                assert_io(RunSummary::read_json(&path));
                assert_io(summary("new").write_json(&path));
                assert!(!path.exists());
                assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
                original
            }
        }
        _ => panic!("unknown permissions scenario"),
    };
    assert_eq!(
        fs::metadata(&parent).unwrap().mode() & 0o777,
        if readonly_parent { 0o555 } else { 0o755 }
    );
    if let Some(before) = before {
        assert_eq!(identity(&fs::metadata(&path).unwrap()), identity(&before));
        // Restore readability after verifying the production call preserved mode.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec_pretty(&expected).unwrap()
        );
    }
}

#[test]
fn real_permissions_preserve_write_only_read_only_no_access_and_parent_behavior() {
    for scenario in ["0200", "0400", "0000", "parent_existing", "parent_missing"] {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _cleanup = PermissionCleanup(root.path().join("history"));
        ProbeChild::spawn(root.path(), "permissions", scenario, "").finish();
    }
}

#[test]
fn run_summary_process_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let scenario = std::env::var(PROBE_SCENARIO).unwrap();
    let operation = std::env::var(PROBE_OPERATION).unwrap();
    match std::env::var(PROBE_KIND).unwrap().as_str() {
        "leaf" => leaf_probe(&root, &scenario, &operation),
        "permissions" => permission_probe(&root, &scenario),
        _ => panic!("unknown summary probe kind"),
    }
}
