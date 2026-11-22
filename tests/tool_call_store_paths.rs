#![cfg(unix)]

use std::ffi::CString;
use std::fs::{self, Metadata, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use steerbox::domain::ToolCallRecord;
use steerbox::tool_call_store::{ToolCallRecordStore, ToolCallStoreError, MAX_TOOL_CALL_PAGE_SIZE};

const PRIVATE_PATH: &str = "synthetic-private-tool-store-path";
const PROBE_ROOT: &str = "STEERBOX_TOOL_STORE_PATH_ROOT";
const PROBE_MODE: &str = "STEERBOX_TOOL_STORE_PATH_MODE";
const PROBE_SCENARIO: &str = "STEERBOX_TOOL_STORE_PATH_SCENARIO";
const PROBE_SLOT: &str = "STEERBOX_TOOL_STORE_PATH_SLOT";
const PROBE_OPERATION: &str = "STEERBOX_TOOL_STORE_PATH_OPERATION";
const REPLACEMENTS: [&str; 5] = ["symlink", "dangling", "hardlink", "fifo", "directory"];

fn record(id: &str) -> ToolCallRecord {
    ToolCallRecord {
        tool_call_id: id.into(),
        tool_id: "git_status".into(),
        project_id: "project".into(),
        goal_id: "goal".into(),
        task_run_id: "run".into(),
        loop_run_id: "loop".into(),
        step_id: "step".into(),
        side_effect_class: "none".into(),
        status: "completed".into(),
        summary: "synthetic tool metadata".into(),
        created_at: "2026-11-22T00:00:00Z".into(),
    }
}

fn paths(root: &Path) -> [PathBuf; 2] {
    let data = root.join(PRIVATE_PATH).with_extension("jsonl");
    let mut lock = data.as_os_str().to_os_string();
    lock.push(".lock");
    [data, PathBuf::from(lock)]
}

fn fixture(root: &Path) -> (ToolCallRecordStore, [PathBuf; 2]) {
    let paths = paths(root);
    let store = ToolCallRecordStore::open(&paths[0]).unwrap();
    fs::write(
        &paths[0],
        format!("{}\n", serde_json::to_string(&record("original")).unwrap()),
    )
    .unwrap();
    fs::write(&paths[1], b"synthetic lock marker\n").unwrap();
    for path in &paths {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    (store, paths)
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

fn assert_safe_error(error: &ToolCallStoreError) {
    assert!(!error.to_string().contains(PRIVATE_PATH));
    assert!(!format!("{error:?}").contains(PRIVATE_PATH));
}

fn assert_permission_denied(result: Result<(), ToolCallStoreError>) {
    let error = result.expect_err("operation must require the actual OS permission");
    assert_safe_error(&error);
    assert!(matches!(error, ToolCallStoreError::Io(ref source)
        if source.kind() == ErrorKind::PermissionDenied));
}

fn assert_cap(result: Result<(), ToolCallStoreError>) {
    assert!(matches!(result, Err(ToolCallStoreError::PageLimitExceeded {
        requested, maximum
    }) if requested == MAX_TOOL_CALL_PAGE_SIZE + 1 && maximum == MAX_TOOL_CALL_PAGE_SIZE));
}

fn invoke(store: &ToolCallRecordStore, operation: &str) -> Result<(), ToolCallStoreError> {
    match operation {
        "append" => store.append(&record("new")),
        "read" => store.read_page(0, 1).map(|_| ()),
        "zero" => store.read_page(0, 0).map(|page| assert!(page.is_empty())),
        "cap" => store.read_page(0, MAX_TOOL_CALL_PAGE_SIZE + 1).map(|_| ()),
        _ => panic!("unsupported tool store operation"),
    }
}

struct ProbeChild(Option<Child>);

impl ProbeChild {
    fn spawn(root: &Path, mode: &str, scenario: &str, slot: usize, operation: &str) -> Self {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tool_store_process_probe", "--nocapture"])
            .env(PROBE_ROOT, root)
            .env(PROBE_MODE, mode)
            .env(PROBE_SCENARIO, scenario)
            .env(PROBE_SLOT, slot.to_string())
            .env(PROBE_OPERATION, operation)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self(Some(child))
    }

    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn finish(mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child().try_wait().unwrap().is_some() {
                assert_child_success(self.0.take().unwrap().wait_with_output().unwrap());
                return;
            }
            assert!(Instant::now() < deadline, "tool store child timed out");
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

fn assert_child_success(output: Output) {
    assert!(
        output.status.success(),
        "tool store probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn leaf_probe(root: &Path, scenario: &str, slot: usize, operation: &str) {
    let (store, paths) = fixture(root);
    let path = &paths[slot];
    let untouched = &paths[1 - slot];
    let untouched_bytes = fs::read(untouched).unwrap();
    let untouched_metadata = fs::metadata(untouched).unwrap();
    let original = fs::read(path).unwrap();
    fs::remove_file(path).unwrap();
    let target = root.join("protected-target");
    match scenario {
        "symlink" | "hardlink" => {
            fs::write(&target, &original).unwrap();
            // A hardlink must be UnsafePath even when it also has unsafe mode.
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
            if scenario == "symlink" {
                symlink(&target, path).unwrap();
            } else {
                fs::hard_link(&target, path).unwrap();
            }
        }
        "dangling" => symlink(&target, path).unwrap(),
        "fifo" => {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        "directory" => {
            fs::create_dir(path).unwrap();
            fs::write(path.join("marker"), b"protected marker").unwrap();
        }
        _ => panic!("unsupported replacement"),
    }
    let before = fs::symlink_metadata(path).unwrap();
    let result = if operation == "constructor" {
        ToolCallRecordStore::open(&paths[0]).map(|_| ())
    } else {
        invoke(&store, operation)
    };
    if operation == "cap" {
        assert_cap(result);
    } else if operation == "zero" && slot == 0 {
        result.expect("zero page skips data only after acquiring the safe lock");
    } else {
        let error = result.expect_err("unsafe leaf must fail closed");
        assert_safe_error(&error);
        assert!(matches!(error, ToolCallStoreError::UnsafePath));
    }
    assert_eq!(
        identity(&fs::symlink_metadata(path).unwrap()),
        identity(&before)
    );
    assert_eq!(fs::read(untouched).unwrap(), untouched_bytes);
    assert_eq!(
        identity(&fs::metadata(untouched).unwrap()),
        identity(&untouched_metadata)
    );
    match scenario {
        "symlink" | "hardlink" => assert_eq!(fs::read(target).unwrap(), original),
        "dangling" => assert!(!target.exists()),
        "directory" => assert_eq!(fs::read(path.join("marker")).unwrap(), b"protected marker"),
        _ => {}
    }
}

#[test]
fn unsafe_data_and_lock_leaves_are_rejected_without_mutation_or_blocking() {
    for scenario in REPLACEMENTS {
        for slot in 0..2 {
            for operation in ["append", "read", "zero", "cap"] {
                let root = tempfile::tempdir().unwrap();
                ProbeChild::spawn(root.path(), "leaf", scenario, slot, operation).finish();
            }
        }
        let root = tempfile::tempdir().unwrap();
        ProbeChild::spawn(root.path(), "leaf", scenario, 0, "constructor").finish();
    }
}

#[test]
fn missing_data_zero_and_cap_have_only_the_documented_side_effects() {
    for limit in [0, 1] {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("new/history");
        let paths = paths(&parent);
        let store = ToolCallRecordStore::open(&paths[0]).unwrap();
        for directory in [root.path().join("new"), parent.clone()] {
            assert_eq!(fs::metadata(directory).unwrap().mode() & 0o077, 0);
        }
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        assert_cap(invoke(&store, "cap"));
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        assert!(store.read_page(0, limit).unwrap().is_empty());
        assert!(!paths[0].exists());
        assert_eq!(fs::metadata(&paths[1]).unwrap().mode() & 0o077, 0);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }
}

#[test]
fn missing_parent_is_not_recreated_and_cap_precedes_parent_io() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join(PRIVATE_PATH);
    let store = ToolCallRecordStore::open(parent.join("calls.jsonl")).unwrap();
    fs::remove_dir(&parent).unwrap();
    assert_cap(invoke(&store, "cap"));
    for operation in ["append", "read", "zero"] {
        let error = invoke(&store, operation).expect_err("missing parent must fail");
        assert_safe_error(&error);
        assert!(matches!(error, ToolCallStoreError::Io(ref source)
            if source.kind() == ErrorKind::NotFound));
        assert!(!parent.exists());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn parent_symlinks_are_rejected_before_creating_or_touching_descendants() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join(PRIVATE_PATH);
    fs::create_dir(&target).unwrap();
    symlink(&target, &link).unwrap();
    let error = ToolCallRecordStore::open(link.join("must-not-exist/calls.jsonl"))
        .expect_err("constructor cannot follow a parent symlink");
    assert_safe_error(&error);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);

    let original = root.path().join("original");
    let moved = root.path().join("moved");
    let (store, paths) = fixture(&original);
    let snapshots = paths.map(|path| {
        (
            path.file_name().unwrap().to_os_string(),
            fs::read(&path).unwrap(),
            fs::metadata(path).unwrap(),
        )
    });
    fs::rename(&original, &moved).unwrap();
    symlink(&moved, &original).unwrap();
    assert_cap(invoke(&store, "cap"));
    for operation in ["append", "read", "zero"] {
        assert_safe_error(&invoke(&store, operation).expect_err("replaced parent must fail"));
        for (name, bytes, before) in &snapshots {
            let path = moved.join(name);
            assert_eq!(fs::read(&path).unwrap(), *bytes);
            assert_eq!(identity(&fs::metadata(path).unwrap()), identity(before));
        }
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 2);
        assert!(fs::symlink_metadata(&original).unwrap().is_symlink());
    }
}

#[test]
fn invalid_full_paths_are_rejected_before_any_mkdir() {
    let root = tempfile::tempdir().unwrap();
    let mut invalid = vec![
        PathBuf::from(""),
        PathBuf::from("/"),
        PathBuf::from("."),
        PathBuf::from(".."),
        root.path().join("never-created/../calls.jsonl"),
        root.path().join("never-created/nested/../calls.jsonl"),
        root.path()
            .join("never-created")
            .join(std::ffi::OsString::from_vec(b"bad\0name".to_vec())),
    ];
    for suffix in ["/", "/.", "/.."] {
        let mut raw = root
            .path()
            .join("never-created/calls.jsonl")
            .into_os_string();
        raw.push(suffix);
        invalid.push(PathBuf::from(raw));
    }
    for path in invalid {
        let error = ToolCallRecordStore::open(path).expect_err("invalid path must fail first");
        assert!(matches!(error, ToolCallStoreError::Io(ref source)
            if source.kind() == ErrorKind::InvalidInput));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

fn lock_probe(root: &Path, operation: &str) {
    let store = ToolCallRecordStore::open(&paths(root)[0]).unwrap();
    fs::write(root.join("ready"), b"ready").unwrap();
    if operation == "cap" {
        assert_cap(invoke(&store, operation));
    } else {
        invoke(&store, operation).unwrap();
    }
    fs::write(root.join("completed"), b"completed").unwrap();
}

#[test]
fn actual_sidecar_lock_blocks_append_read_and_zero_but_not_cap() {
    for operation in ["append", "read", "zero", "cap"] {
        let root = tempfile::tempdir().unwrap();
        let (store, paths) = fixture(root.path());
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&paths[1])
            .unwrap();
        held.lock().unwrap();
        let mut child = ProbeChild::spawn(root.path(), "lock", "", 0, operation);
        if operation == "cap" {
            child.finish();
            assert!(root.path().join("completed").exists());
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !root.path().join("ready").exists() {
            assert!(child.child().try_wait().unwrap().is_none());
            assert!(
                Instant::now() < deadline,
                "lock child never reached operation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let blocked_until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < blocked_until {
            assert!(
                !root.path().join("completed").exists(),
                "held lock was ignored"
            );
            assert!(child.child().try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(held);
        child.finish();
        assert!(root.path().join("completed").exists());
        assert_eq!(
            store.read_page(0, 10).unwrap().len(),
            if operation == "append" { 2 } else { 1 }
        );
        assert_eq!(fs::read(&paths[1]).unwrap(), b"synthetic lock marker\n");
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

fn drop_root_privileges(paths: &[PathBuf]) {
    if unsafe { libc::geteuid() } == 0 {
        for path in paths.iter().filter(|path| path.exists()) {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            // Only this child probe's synthetic regular files are chowned.
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
    let parent = root.join("history");
    fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
    let (store, paths) = fixture(&parent);
    let original_data = fs::read(&paths[0]).unwrap();
    let readonly_parent = scenario.starts_with("parent_");
    let data_mode = match scenario {
        "data_readonly" => 0o400,
        "data_writeonly" => 0o200,
        "data_no_access" => 0,
        "data_public" => 0o644,
        _ => 0o600,
    };
    fs::set_permissions(&paths[0], fs::Permissions::from_mode(data_mode)).unwrap();
    if scenario == "lock_readonly" {
        fs::set_permissions(&paths[1], fs::Permissions::from_mode(0o400)).unwrap();
    }
    if scenario == "parent_missing_lock" {
        fs::remove_file(&paths[1]).unwrap();
    }
    fs::set_permissions(
        &parent,
        fs::Permissions::from_mode(if readonly_parent { 0o555 } else { 0o755 }),
    )
    .unwrap();
    drop_root_privileges(&paths);
    let before = fs::metadata(&paths[0]).unwrap();
    let lock_before = paths[1].exists().then(|| fs::metadata(&paths[1]).unwrap());
    match scenario {
        "data_readonly" | "data_writeonly" | "data_no_access" => {
            // These must not require opening the existing data for R or W.
            ToolCallRecordStore::open(&paths[0]).expect("constructor is metadata-only");
            invoke(&store, "zero").unwrap();
            if scenario == "data_readonly" {
                assert_eq!(store.read_page(0, 1).unwrap(), vec![record("original")]);
            } else {
                assert_permission_denied(invoke(&store, "read"));
            }
            if scenario == "data_writeonly" {
                assert_permission_denied(
                    OpenOptions::new()
                        .read(true)
                        .open(&paths[0])
                        .map(|_| ())
                        .map_err(ToolCallStoreError::Io),
                );
                invoke(&store, "append").expect("append only requires O_WRONLY on data");
            } else {
                assert_permission_denied(
                    OpenOptions::new()
                        .write(true)
                        .open(&paths[0])
                        .map(|_| ())
                        .map_err(ToolCallStoreError::Io),
                );
                assert_permission_denied(invoke(&store, "append"));
            }
        }
        "lock_readonly" => {
            assert_permission_denied(
                OpenOptions::new()
                    .write(true)
                    .open(&paths[1])
                    .map(|_| ())
                    .map_err(ToolCallStoreError::Io),
            );
            for operation in ["append", "read", "zero"] {
                assert_permission_denied(invoke(&store, operation));
            }
        }
        "parent_existing_lock" | "parent_missing_lock" => {
            assert_permission_denied(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(parent.join("must-not-exist"))
                    .map(|_| ())
                    .map_err(ToolCallStoreError::Io),
            );
            for operation in ["read", "zero", "append"] {
                if scenario == "parent_existing_lock" {
                    invoke(&store, operation).unwrap();
                } else {
                    assert_permission_denied(invoke(&store, operation));
                    assert!(!paths[1].exists());
                }
            }
            assert!(!parent.join("must-not-exist").exists());
        }
        "data_public" => {
            assert!(matches!(
                ToolCallRecordStore::open(&paths[0]),
                Err(ToolCallStoreError::UnsafePermissions)
            ));
            for operation in ["read", "append"] {
                assert!(matches!(
                    invoke(&store, operation),
                    Err(ToolCallStoreError::UnsafePermissions)
                ));
            }
            invoke(&store, "zero").unwrap();
        }
        _ => panic!("unsupported permission scenario"),
    }
    assert_cap(invoke(&store, "cap"));
    assert_eq!(
        identity(&fs::metadata(&paths[0]).unwrap()),
        identity(&before)
    );
    if let Some(before) = lock_before {
        assert_eq!(
            identity(&fs::metadata(&paths[1]).unwrap()),
            identity(&before)
        );
        assert_eq!(fs::read(&paths[1]).unwrap(), b"synthetic lock marker\n");
    }
    assert_eq!(
        fs::metadata(&parent).unwrap().mode() & 0o777,
        if readonly_parent { 0o555 } else { 0o755 }
    );
    // Restore readability only after proving production preserved the mode.
    fs::set_permissions(&paths[0], fs::Permissions::from_mode(0o600)).unwrap();
    if matches!(scenario, "data_writeonly" | "parent_existing_lock") {
        assert_eq!(
            store.read_page(0, 10).unwrap(),
            vec![record("original"), record("new")]
        );
    } else {
        assert_eq!(fs::read(&paths[0]).unwrap(), original_data);
    }
}

#[test]
fn real_os_permissions_distinguish_data_lock_parent_and_metadata_only_open() {
    for scenario in [
        "data_readonly",
        "data_writeonly",
        "data_no_access",
        "data_public",
        "lock_readonly",
        "parent_existing_lock",
        "parent_missing_lock",
    ] {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _cleanup = PermissionCleanup(root.path().join("history"));
        ProbeChild::spawn(root.path(), "permission", scenario, 0, "").finish();
    }
}

#[test]
fn foreign_owner_is_rejected_when_privileged_fixture_is_available() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "foreign-owner fixture requires root; ordinary-user permission cases run separately"
        );
        return;
    }
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let (store, paths) = fixture(root.path());
    let original = fs::read(&paths[0]).unwrap();
    let name = CString::new(paths[0].as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::chown(name.as_ptr(), 65534, 65534) }, 0);
    let before = fs::metadata(&paths[0]).unwrap();
    assert_eq!(before.mode() & 0o777, 0o600);
    assert_ne!(before.uid(), unsafe { libc::geteuid() });
    assert!(matches!(
        ToolCallRecordStore::open(&paths[0]),
        Err(ToolCallStoreError::UnsafePermissions)
    ));
    for operation in ["read", "append"] {
        assert!(matches!(
            invoke(&store, operation),
            Err(ToolCallStoreError::UnsafePermissions)
        ));
    }
    invoke(&store, "zero").unwrap();
    assert_eq!(fs::read(&paths[0]).unwrap(), original);
    assert_eq!(
        identity(&fs::metadata(&paths[0]).unwrap()),
        identity(&before)
    );
}

#[test]
fn foreign_owner_lock_is_rejected_when_privileged_fixture_is_available() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("foreign-owner lock fixture requires root");
        return;
    }
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let (store, paths) = fixture(root.path());
    let snapshots = paths.each_ref().map(|path| {
        (
            fs::read(path).unwrap(),
            identity(&fs::metadata(path).unwrap()),
        )
    });
    let name = CString::new(paths[1].as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::chown(name.as_ptr(), 65534, 65534) }, 0);
    let lock_before = identity(&fs::metadata(&paths[1]).unwrap());
    ToolCallRecordStore::open(&paths[0]).expect("constructor does not open lock");
    for operation in ["append", "read", "zero"] {
        assert!(matches!(
            invoke(&store, operation),
            Err(ToolCallStoreError::UnsafePermissions)
        ));
    }
    assert_cap(invoke(&store, "cap"));
    assert_eq!(fs::read(&paths[0]).unwrap(), snapshots[0].0);
    assert_eq!(identity(&fs::metadata(&paths[0]).unwrap()), snapshots[0].1);
    assert_eq!(fs::read(&paths[1]).unwrap(), snapshots[1].0);
    assert_eq!(identity(&fs::metadata(&paths[1]).unwrap()), lock_before);
}

#[test]
fn public_lock_mode_is_rejected_before_data_access_without_chmod() {
    let root = tempfile::tempdir().unwrap();
    let (store, paths) = fixture(root.path());
    fs::set_permissions(&paths[1], fs::Permissions::from_mode(0o644)).unwrap();
    let snapshots = paths.each_ref().map(|path| {
        (
            fs::read(path).unwrap(),
            identity(&fs::metadata(path).unwrap()),
        )
    });
    ToolCallRecordStore::open(&paths[0]).unwrap();
    for operation in ["read", "append", "zero"] {
        assert!(matches!(
            invoke(&store, operation),
            Err(ToolCallStoreError::UnsafePermissions)
        ));
    }
    assert_cap(invoke(&store, "cap"));
    for (path, (bytes, metadata)) in paths.iter().zip(snapshots) {
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(identity(&fs::metadata(path).unwrap()), metadata);
    }
}

#[test]
fn append_retains_identical_records_without_scanning_existing_history() {
    let root = tempfile::tempdir().unwrap();
    let (store, paths) = fixture(root.path());
    fs::write(&paths[0], b"not-json\n").unwrap();
    store.append(&record("same-id")).unwrap();
    store.append(&record("same-id")).unwrap();
    let content = fs::read_to_string(&paths[0]).unwrap();
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "not-json");
    assert_eq!(lines[1], lines[2]);
    assert_eq!(
        serde_json::from_str::<ToolCallRecord>(lines[1]).unwrap(),
        record("same-id")
    );
    assert!(content.ends_with('\n'));
}

#[test]
fn pages_keep_physical_error_lines_offset_validation_and_early_stop() {
    let root = tempfile::tempdir().unwrap();
    let (store, paths) = fixture(root.path());
    let encoded = serde_json::to_string(&record("original")).unwrap();
    fs::write(&paths[0], format!("\n{encoded}\n \t\nnot-json\n")).unwrap();
    assert_eq!(store.read_page(0, 1).unwrap(), vec![record("original")]);
    assert!(matches!(
        store.read_page(10, 1),
        Err(ToolCallStoreError::InvalidJson { line: 4, .. })
    ));
    fs::write(&paths[0], format!("\n{encoded}\n").as_bytes()).unwrap();
    let mut bytes = fs::read(&paths[0]).unwrap();
    bytes.extend_from_slice(b"\xff\n");
    fs::write(&paths[0], bytes).unwrap();
    assert_eq!(store.read_page(0, 1).unwrap(), vec![record("original")]);
    assert!(
        matches!(store.read_page(10, 1), Err(ToolCallStoreError::Io(ref source)) if source.kind() == ErrorKind::InvalidData)
    );
    fs::write(&paths[0], encoded).unwrap();
    assert_eq!(store.read_page(0, 1).unwrap(), vec![record("original")]);
}

#[test]
fn tool_store_process_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var(PROBE_MODE).unwrap();
    let scenario = std::env::var(PROBE_SCENARIO).unwrap();
    let slot = std::env::var(PROBE_SLOT).unwrap().parse().unwrap();
    let operation = std::env::var(PROBE_OPERATION).unwrap();
    match mode.as_str() {
        "leaf" => leaf_probe(&root, &scenario, slot, &operation),
        "lock" => lock_probe(&root, &operation),
        "permission" => permission_probe(&root, &scenario),
        _ => panic!("unsupported probe mode"),
    }
}
