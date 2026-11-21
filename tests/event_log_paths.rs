#![cfg(unix)]

use std::ffi::{CString, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use steerbox::domain::EventEnvelope;
use steerbox::event_log::{EventLog, EventLogError};

const PATH_SENTINEL: &str = "synthetic-private-event-path-sentinel";
const PROBE_OPERATION: &str = "STEERBOX_EVENT_PATH_TEST_OPERATION";
const PROBE_SCENARIO: &str = "STEERBOX_EVENT_PATH_TEST_SCENARIO";
const PROBE_ROOT: &str = "STEERBOX_EVENT_PATH_TEST_ROOT";
const READ_ONLY_PROBE_ROOT: &str = "STEERBOX_EVENT_READ_ONLY_TEST_ROOT";
const REPLACEMENTS: [&str; 8] = [
    "leaf_symlink",
    "dangling_leaf",
    "parent_symlink",
    "dangling_parent",
    "hardlink",
    "fifo",
    "device",
    "directory",
];

fn event() -> EventEnvelope {
    EventEnvelope::new(
        "loop_run",
        "event-path-probe",
        "path_probe",
        "loop-path-probe",
        "loop-path-probe",
        serde_json::json!({"synthetic": true}),
    )
    .unwrap()
}

fn assert_safe_error(error: EventLogError) {
    assert!(!error.to_string().contains(PATH_SENTINEL));
    assert!(!format!("{error:?}").contains(PATH_SENTINEL));
}

fn assert_open_rejected(path: &Path) {
    match EventLog::open(path) {
        Err(error) => assert_safe_error(error),
        Ok(_) => panic!("unsafe event log open was accepted"),
    }
}

fn probe(scenario: &str, operation: &str) -> Output {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "event_path_probe", "--nocapture"])
        .env(PROBE_OPERATION, operation)
        .env(PROBE_SCENARIO, scenario)
        .env(PROBE_ROOT, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() >= Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("event path operation blocked: scenario={scenario} operation={operation}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_probe_succeeded(output: Output) {
    assert!(
        output.status.success(),
        "event path probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn invoke(log: &mut EventLog, operation: &str) -> Result<(), EventLogError> {
    match operation {
        "append" => log.append(event()).map(|_| ()),
        "read_all" => log.read_all().map(|_| ()),
        "read_page" => log.read_page(0, 1).map(|_| ()),
        "zero_page" => log.read_page(0, 0).map(|events| assert!(events.is_empty())),
        _ => panic!("unsupported event log probe operation"),
    }
}

#[test]
fn event_path_probe() {
    let Some(operation) = std::env::var_os(PROBE_OPERATION) else {
        return;
    };
    let operation = operation.to_str().unwrap();
    let scenario = std::env::var(PROBE_SCENARIO).unwrap();
    let root = PathBuf::from(std::env::var_os(PROBE_ROOT).unwrap());
    let parent = root.join("parent");
    fs::create_dir(&parent).unwrap();
    let mut path = parent.join(PATH_SENTINEL);
    fs::write(&path, b"").unwrap();
    let mut log = (operation != "open").then(|| EventLog::open(&path).unwrap());
    let mut preserved = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut missing = Vec::<PathBuf>::new();

    match scenario.as_str() {
        "parent_symlink" | "dangling_parent" => {
            let moved = root.join("moved");
            fs::rename(&parent, &moved).unwrap();
            preserved.push((moved.join(PATH_SENTINEL), Vec::new()));
            let target = root.join("target-parent");
            if scenario == "parent_symlink" {
                fs::create_dir(&target).unwrap();
                fs::write(target.join(PATH_SENTINEL), b"").unwrap();
                preserved.push((target.join(PATH_SENTINEL), Vec::new()));
            } else {
                missing.push(target.clone());
            }
            symlink(&target, &parent).unwrap();
        }
        _ => {
            fs::remove_file(&path).unwrap();
            match scenario.as_str() {
                "leaf_symlink" | "hardlink" => {
                    let target = root.join("target.jsonl");
                    fs::write(&target, b"").unwrap();
                    if scenario == "leaf_symlink" {
                        symlink(&target, &path).unwrap();
                    } else {
                        fs::hard_link(&target, &path).unwrap();
                    }
                    preserved.push((target, Vec::new()));
                }
                "dangling_leaf" => {
                    let target = root.join("missing-target.jsonl");
                    symlink(&target, &path).unwrap();
                    missing.push(target);
                }
                "fifo" => {
                    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
                "device" => {
                    assert!(fs::metadata("/dev/null")
                        .unwrap()
                        .file_type()
                        .is_char_device());
                    if operation == "open" {
                        path = PathBuf::from("/dev/null");
                    } else {
                        symlink("/dev/null", &path).unwrap();
                    }
                }
                "directory" => {
                    fs::create_dir(&path).unwrap();
                    let marker = path.join("marker");
                    fs::write(&marker, b"synthetic marker").unwrap();
                    preserved.push((marker, b"synthetic marker".to_vec()));
                }
                _ => panic!("unsupported event path probe scenario"),
            }
        }
    }

    let before = fs::symlink_metadata(&path).ok();
    let result = match log.as_mut() {
        Some(log) => invoke(log, operation),
        None => EventLog::open(&path).map(|_| ()),
    };
    for (file, expected) in preserved {
        assert_eq!(fs::read(file).unwrap(), expected);
    }
    for file in missing {
        assert!(matches!(
            fs::symlink_metadata(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ));
    }
    if let Some(before) = before {
        let after = fs::symlink_metadata(&path).unwrap();
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(after.mode(), before.mode());
        assert_eq!(after.nlink(), before.nlink());
    }
    if operation == "zero_page" {
        result.expect("zero-page query must not access unsafe replacement paths");
    } else {
        assert_safe_error(result.expect_err("unsafe event path operation must be rejected"));
    }
}

#[test]
fn open_rejects_final_symlink_without_changing_target() {
    assert_probe_succeeded(probe("leaf_symlink", "open"));
}

#[test]
fn open_rejects_parent_symlink_without_changing_target() {
    assert_probe_succeeded(probe("parent_symlink", "open"));
}

#[test]
fn open_rejects_dangling_final_symlink_without_creating_target() {
    assert_probe_succeeded(probe("dangling_leaf", "open"));
}

#[test]
fn open_rejects_dangling_parent_symlink_without_creating_target() {
    assert_probe_succeeded(probe("dangling_parent", "open"));
}

#[test]
fn open_rejects_hardlink_without_changing_target() {
    assert_probe_succeeded(probe("hardlink", "open"));
}

#[test]
fn open_rejects_fifo_without_blocking() {
    assert_probe_succeeded(probe("fifo", "open"));
}

#[test]
fn open_rejects_device_without_blocking() {
    assert_probe_succeeded(probe("device", "open"));
}

#[test]
fn open_rejects_directory_without_changing_contents() {
    assert_probe_succeeded(probe("directory", "open"));
}

#[test]
fn append_revalidates_unsafe_paths_replaced_after_open() {
    let outputs = REPLACEMENTS.map(|scenario| probe(scenario, "append"));
    for output in outputs {
        assert_probe_succeeded(output);
    }
}

#[test]
fn read_all_revalidates_unsafe_paths_replaced_after_open() {
    let outputs = REPLACEMENTS.map(|scenario| probe(scenario, "read_all"));
    for output in outputs {
        assert_probe_succeeded(output);
    }
}

#[test]
fn read_page_revalidates_unsafe_paths_replaced_after_open() {
    let outputs = REPLACEMENTS.map(|scenario| probe(scenario, "read_page"));
    for output in outputs {
        assert_probe_succeeded(output);
    }
}

#[test]
fn zero_limit_page_avoids_io_for_unsafe_paths_replaced_after_open() {
    let outputs = REPLACEMENTS.map(|scenario| probe(scenario, "zero_page"));
    for output in outputs {
        assert_probe_succeeded(output);
    }
}

#[test]
fn missing_file_queries_do_not_create_files_or_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(PATH_SENTINEL);
    let log = EventLog::open(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(log.read_all().unwrap().is_empty());
    assert!(log.read_page(0, 1).unwrap().is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn missing_parent_queries_do_not_recreate_directories_or_files() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("missing-parent");
    let path = parent.join(PATH_SENTINEL);
    let log = EventLog::open(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::remove_dir(&parent).unwrap();
    assert!(log.read_all().unwrap().is_empty());
    assert!(log.read_page(0, 1).unwrap().is_empty());
    assert!(!parent.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn open_rejects_parent_traversal_before_creating_any_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new/..").join(PATH_SENTINEL);
    assert_open_rejected(&path);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn open_rejects_raw_invalid_basenames_before_creating_directories() {
    for path in ["", "/", ".", ".."] {
        assert_open_rejected(Path::new(path));
    }
    let root = tempfile::tempdir().unwrap();
    for suffix in ["/", "/.", "/.."] {
        let mut raw = root.path().join("new").join(PATH_SENTINEL).into_os_string();
        raw.push(suffix);
        assert!(raw.as_bytes().ends_with(suffix.as_bytes()));
        assert_open_rejected(Path::new(&raw));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn open_rejects_embedded_nul_before_creating_earlier_parent_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join("new")
        .join(OsString::from_vec(b"event\0.jsonl".to_vec()));
    assert_open_rejected(&path);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn existing_read_only_history_can_be_opened_and_queried_without_changes() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "event_read_only_history_probe", "--nocapture"])
        .env(READ_ONLY_PROBE_ROOT, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            assert_probe_succeeded(child.wait_with_output().unwrap());
            return;
        }
        if started.elapsed() >= Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("read-only event history probe blocked");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn event_read_only_history_probe() {
    let Some(root) = std::env::var_os(READ_ONLY_PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let parent = root.join("history");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let path = parent.join("history.jsonl");
    let mut recorded = event();
    recorded.sequence_no = Some(41);
    let mut original = serde_json::to_vec(&recorded).unwrap();
    original.push(b'\n');
    fs::write(&path, &original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    if unsafe { libc::geteuid() } == 0 {
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 65534, 65534) }, 0);
        assert_eq!(unsafe { libc::setgroups(0, std::ptr::null()) }, 0);
        assert_eq!(unsafe { libc::setgid(65534) }, 0);
        assert_eq!(unsafe { libc::setuid(65534) }, 0);
        assert_eq!(unsafe { libc::geteuid() }, 65534);
        assert_eq!(unsafe { libc::getegid() }, 65534);
        assert_eq!(unsafe { libc::getgroups(0, std::ptr::null_mut()) }, 0);
    }
    let direct_error = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect_err("probe identity must not be allowed to open history for writing");
    assert_eq!(direct_error.kind(), std::io::ErrorKind::PermissionDenied);
    let mut log = EventLog::open(&path).unwrap();
    assert_eq!(log.read_all().unwrap(), vec![recorded.clone()]);
    assert_eq!(log.read_page(0, 1).unwrap(), vec![recorded.clone()]);
    let append_error = log
        .append(event())
        .expect_err("read-only history must reject append");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o400);
    assert!(matches!(append_error, EventLogError::Io(ref error)
        if error.kind() == std::io::ErrorKind::PermissionDenied));
    assert_eq!(log.read_all().unwrap(), vec![recorded]);
}

#[test]
fn nested_missing_parents_still_support_normal_append_and_queries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("first/second/events.jsonl");
    let mut log = EventLog::open(&path).unwrap();
    assert_eq!(log.append(event()).unwrap(), 1);
    assert_eq!(log.read_all().unwrap().len(), 1);
    assert_eq!(log.read_page(0, 1).unwrap().len(), 1);
}
