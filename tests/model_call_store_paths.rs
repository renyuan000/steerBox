#![cfg(unix)]

use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use steerbox::model_call_store::{ModelCallStore, ModelCallStoreError};
use steerbox::model_contracts::{ModelCallFailureRecord, ModelCallRecord};

const PRIVATE_PATH: &str = "synthetic-private-model-store-path";
const PROBE_ROOT: &str = "STEERBOX_MODEL_STORE_PATH_TEST_ROOT";
const PROBE_SLOT: &str = "STEERBOX_MODEL_STORE_PATH_TEST_SLOT";
const PROBE_SCENARIO: &str = "STEERBOX_MODEL_STORE_PATH_TEST_SCENARIO";
const PROBE_OPERATION: &str = "STEERBOX_MODEL_STORE_PATH_TEST_OPERATION";
const PERMISSION_SCENARIO: &str = "STEERBOX_MODEL_STORE_PERMISSION_TEST_SCENARIO";
const REPLACEMENTS: [&str; 5] = ["symlink", "dangling", "hardlink", "fifo", "directory"];

fn record(id: &str) -> ModelCallRecord {
    serde_json::from_value(serde_json::json!({
        "model_call_id": id, "task_id": "task", "task_run_id": "run",
        "agent_role": "observer", "provider_id": "fixture", "endpoint_id": "local://fixture",
        "model_id": "fixture-model", "route_decision_id": "route", "resolved_prompt_pack_id": "prompt",
        "input_context_hash": "sha256:synthetic", "output_artifact_ref": "trace://synthetic/output",
        "latency_ms": null, "input_tokens": null, "output_tokens": null, "estimated_cost": null,
        "streaming": false, "finish_reason": "stop", "tool_call_count": 0,
        "verification_status": "not_run", "error_class": null,
        "started_at": "2026-11-22T00:00:00Z", "completed_at": "2026-11-22T00:00:01Z",
        "created_at": "2026-11-22T00:00:01Z"
    }))
    .unwrap()
}

fn failure(id: &str) -> ModelCallFailureRecord {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "model_call_id": id, "task_id": "task", "task_run_id": "run",
        "agent_role": "observer", "provider_id": "fixture", "endpoint_id": "local://fixture",
        "model_id": "fixture-model", "route_decision_id": "route", "resolved_prompt_pack_id": "prompt",
        "started_at": "2026-11-22T00:00:00Z", "ended_at": "2026-11-22T00:00:01Z",
        "failure_stage": "provider_request", "failure_class": "timeout",
        "input_context_hash": null, "output_artifact_ref": null, "finish_reason": null,
        "input_tokens": null, "output_tokens": null, "estimated_cost": null
    }))
    .unwrap()
}

fn fixture(root: &Path) -> (ModelCallStore, [PathBuf; 4]) {
    let store = ModelCallStore::open(root.join(PRIVATE_PATH).with_extension("jsonl")).unwrap();
    let paths = store.storage_paths();
    fs::write(
        &paths[0],
        format!(
            "{}\n",
            serde_json::to_string(&record("original-success")).unwrap()
        ),
    )
    .unwrap();
    fs::write(
        &paths[1],
        format!(
            "{}\n",
            serde_json::to_string(&failure("original-failure")).unwrap()
        ),
    )
    .unwrap();
    for path in &paths[2..] {
        fs::write(path, b"").unwrap();
    }
    (store, paths)
}

fn assert_safe_error(error: &ModelCallStoreError) {
    assert!(
        matches!(error, ModelCallStoreError::Io(_)),
        "expected an I/O rejection: {error:?}"
    );
    assert!(!error.to_string().contains(PRIVATE_PATH));
    assert!(!format!("{error:?}").contains(PRIVATE_PATH));
}

fn assert_permission_denied(result: Result<(), ModelCallStoreError>) {
    let error = result.expect_err("permission boundary must reject the operation");
    assert_safe_error(&error);
    assert!(
        matches!(error, ModelCallStoreError::Io(ref source) if source.kind() == ErrorKind::PermissionDenied)
    );
}

fn invoke(store: &ModelCallStore, slot: usize, operation: &str) -> Result<(), ModelCallStoreError> {
    let success = slot == 0 || slot == 2;
    match (success, operation) {
        (true, "append") => store.append(&record("new-success")),
        (false, "append") => store.append_failure(&failure("new-failure")),
        (true, "read") => {
            let all = store.read_all();
            let page = store.read_page(0, 1);
            match (all, page) {
                (Err(error), Err(page_error)) => {
                    assert_safe_error(&page_error);
                    Err(error)
                }
                (Ok(_), Ok(_)) => Ok(()),
                _ => panic!("read_all and read_page disagree on the same path boundary"),
            }
        }
        (false, "read") => store.read_failures().map(|_| ()),
        (true, "page") => store.read_page(0, 1).map(|_| ()),
        (true, "zero") => store
            .read_page(0, 0)
            .map(|records| assert!(records.is_empty())),
        _ => panic!("unsupported model store operation"),
    }
}

struct ProbeChild(Option<Child>);

impl ProbeChild {
    fn spawn(name: &str, root: &Path, variables: &[(&str, String)]) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", name, "--nocapture"])
            .env(PROBE_ROOT, root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in variables {
            command.env(key, value);
        }
        Self(Some(command.spawn().unwrap()))
    }

    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child().try_wait().unwrap().is_some() {
                return self.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "model store child operation timed out"
            );
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
        "model store child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_path_probe(slot: usize, scenario: &str, operation: &str) {
    let root = tempfile::tempdir().unwrap();
    let child = ProbeChild::spawn(
        "model_store_path_probe",
        root.path(),
        &[
            (PROBE_SLOT, slot.to_string()),
            (PROBE_SCENARIO, scenario.into()),
            (PROBE_OPERATION, operation.into()),
        ],
    );
    let output = child.finish();
    assert!(
        output.status.success(),
        "slot={slot} scenario={scenario} operation={operation}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn model_store_path_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let slot: usize = std::env::var(PROBE_SLOT).unwrap().parse().unwrap();
    let scenario = std::env::var(PROBE_SCENARIO).unwrap();
    let operation = std::env::var(PROBE_OPERATION).unwrap();
    let (store, paths) = fixture(&root);
    let path = &paths[slot];
    let mut preserved = paths
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != slot)
        .map(|(_, path)| (path.clone(), fs::read(path).unwrap()))
        .collect::<Vec<_>>();
    let original = fs::read(path).unwrap();
    fs::remove_file(path).unwrap();
    let target = root.join(format!("{PRIVATE_PATH}-protected-target"));
    match scenario.as_str() {
        "symlink" | "hardlink" => {
            fs::write(&target, &original).unwrap();
            if scenario == "symlink" {
                symlink(&target, path).unwrap();
            } else {
                fs::hard_link(&target, path).unwrap();
            }
            preserved.push((target.clone(), original));
        }
        "dangling" => symlink(&target, path).unwrap(),
        "fifo" => {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        "directory" => {
            fs::create_dir(path).unwrap();
            let marker = path.join("marker");
            fs::write(&marker, b"synthetic protected marker").unwrap();
            preserved.push((marker, b"synthetic protected marker".to_vec()));
        }
        _ => panic!("unsupported replacement"),
    }
    let before = fs::symlink_metadata(path).unwrap();
    let result = invoke(&store, slot, &operation);
    for (file, expected) in preserved {
        assert_eq!(fs::read(file).unwrap(), expected);
    }
    let after = fs::symlink_metadata(path).unwrap();
    assert_eq!(
        (after.dev(), after.ino(), after.mode(), after.nlink()),
        (before.dev(), before.ino(), before.mode(), before.nlink())
    );
    if scenario == "dangling" {
        assert!(!target.exists());
    }
    if operation == "zero" && slot == 0 {
        result.expect("zero page must not access the data path after locking");
    } else {
        assert_safe_error(&result.expect_err("unsafe data or lock path must be rejected"));
    }
}

fn check_replacement(scenario: &str) {
    for slot in 0..4 {
        for operation in ["append", "read"] {
            run_path_probe(slot, scenario, operation);
        }
    }
}

#[test]
fn post_open_symlinks_on_both_data_and_lock_paths_are_rejected() {
    check_replacement("symlink");
}
#[test]
fn post_open_dangling_symlinks_never_create_targets() {
    check_replacement("dangling");
}
#[test]
fn post_open_hardlinks_on_both_data_and_lock_paths_are_rejected() {
    check_replacement("hardlink");
}
#[test]
fn post_open_fifos_on_both_data_and_lock_paths_do_not_block() {
    check_replacement("fifo");
}
#[test]
fn post_open_directories_on_both_data_and_lock_paths_are_rejected() {
    check_replacement("directory");
}

#[test]
fn zero_page_ignores_unsafe_data_after_acquiring_a_safe_lock() {
    for scenario in REPLACEMENTS {
        run_path_probe(0, scenario, "zero");
    }
}

#[test]
fn zero_page_rejects_unsafe_main_lock_before_skipping_data() {
    for scenario in REPLACEMENTS {
        run_path_probe(2, scenario, "zero");
    }
}

#[test]
fn directory_data_targets_still_open_but_fail_when_operated_on() {
    for slot in 0..2 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("calls.jsonl");
        let paths = ModelCallStore::storage_paths_for(&path);
        fs::create_dir(&paths[slot]).unwrap();
        let store =
            ModelCallStore::open(&path).expect("open must preserve deferred data validation");
        for operation in ["append", "read"] {
            assert_safe_error(
                &invoke(&store, slot, operation).expect_err("directory data is not a log"),
            );
        }
    }
}

#[test]
fn missing_data_queries_create_only_the_corresponding_lock_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let store = ModelCallStore::open(root.path().join("calls.jsonl")).unwrap();
    let paths = store.storage_paths();
    assert!(store.read_all().unwrap().is_empty());
    assert!(store.read_failures().unwrap().is_empty());
    assert!(store.read_page(0, 1).unwrap().is_empty());
    assert!(!paths[0].exists());
    assert!(!paths[1].exists());
    assert!(paths[2].is_file());
    assert!(paths[3].is_file());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn zero_page_of_missing_data_still_creates_the_main_lock() {
    let root = tempfile::tempdir().unwrap();
    let store = ModelCallStore::open(root.path().join("calls.jsonl")).unwrap();
    let paths = store.storage_paths();
    assert!(store.read_page(0, 0).unwrap().is_empty());
    assert!(paths[2].is_file());
    for slot in [0, 1, 3] {
        assert!(!paths[slot].exists());
    }
}

#[test]
fn queries_with_a_missing_parent_fail_without_recreating_it() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("history");
    let store = ModelCallStore::open(parent.join("calls.jsonl")).unwrap();
    fs::remove_dir(&parent).unwrap();
    for (slot, operation) in [(0, "read"), (1, "read"), (0, "zero")] {
        assert_safe_error(
            &invoke(&store, slot, operation).expect_err("missing parent cannot host the lock"),
        );
    }
    assert!(!parent.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

fn check_cross_process_lock(slot: usize) {
    let operations: &[&str] = if slot == 0 {
        &["append", "read", "page", "zero"]
    } else {
        &["append", "read"]
    };
    for operation in operations {
        let root = tempfile::tempdir().unwrap();
        let (store, paths) = fixture(root.path());
        let lock_path = &paths[slot + 2];
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        held.lock().unwrap();
        let mut child = ProbeChild::spawn(
            "model_store_lock_probe",
            root.path(),
            &[
                (PROBE_SLOT, slot.to_string()),
                (PROBE_OPERATION, (*operation).into()),
            ],
        );
        let ready = root.path().join("ready");
        let completed = root.path().join("completed");
        let ready_deadline = Instant::now() + Duration::from_secs(3);
        while !ready.exists() {
            assert!(
                child.child().try_wait().unwrap().is_none(),
                "lock child exited before ready"
            );
            assert!(
                Instant::now() < ready_deadline,
                "lock child never reached the operation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // Ready is emitted immediately before the real store operation. Only
        // observe blocking while this process still owns the actual sidecar lock.
        let blocked_deadline = Instant::now() + Duration::from_millis(100);
        loop {
            assert!(
                !completed.exists(),
                "operation ignored the held sidecar lock"
            );
            assert!(
                child.child().try_wait().unwrap().is_none(),
                "operation exited while its lock was held"
            );
            if Instant::now() >= blocked_deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(held);
        assert_child_success(child.finish());
        assert!(completed.exists());
        let expected = if *operation == "append" { 2 } else { 1 };
        if slot == 0 {
            assert_eq!(store.read_all().unwrap().len(), expected);
            assert_eq!(store.read_failures().unwrap().len(), 1);
        } else {
            assert_eq!(store.read_failures().unwrap().len(), expected);
            assert_eq!(store.read_all().unwrap().len(), 1);
        }
    }
}

#[test]
fn success_operations_wait_for_the_real_cross_process_lock_then_complete() {
    check_cross_process_lock(0);
}

#[test]
fn failure_operations_wait_for_the_real_cross_process_lock_then_complete() {
    check_cross_process_lock(1);
}

#[test]
fn model_store_lock_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let slot: usize = std::env::var(PROBE_SLOT).unwrap().parse().unwrap();
    let operation = std::env::var(PROBE_OPERATION).unwrap();
    let store = ModelCallStore::open(root.join(PRIVATE_PATH).with_extension("jsonl")).unwrap();
    fs::write(root.join("ready"), b"about to invoke the locked store").unwrap();
    invoke(&store, slot, &operation).unwrap();
    fs::write(root.join("completed"), b"store operation returned").unwrap();
}

struct PermissionCleanup(PathBuf);

impl Drop for PermissionCleanup {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
}

fn run_permission_probe(scenario: &str) {
    // The parent retains the TempDir and can restore its own synthetic directory
    // permissions even if a downgraded child panics or has to be killed.
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let _cleanup = PermissionCleanup(root.path().join("history"));
    let child = ProbeChild::spawn(
        "model_store_permission_probe",
        root.path(),
        &[(PERMISSION_SCENARIO, scenario.into())],
    );
    assert_child_success(child.finish());
}

#[test]
fn read_only_data_allows_queries_and_identical_retries_but_rejects_new_records() {
    run_permission_probe("data");
}

#[test]
fn read_only_locks_still_require_read_write_access_even_for_queries() {
    run_permission_probe("lock");
}

#[test]
fn read_only_parent_distinguishes_existing_locks_from_missing_locks() {
    run_permission_probe("parent_existing_locks");
    run_permission_probe("parent_missing_locks");
}

fn drop_root_privileges(paths: &[PathBuf]) {
    if unsafe { libc::geteuid() } == 0 {
        for path in paths.iter().filter(|path| path.exists()) {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            // These are exclusively this probe's synthetic regular files.
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

fn assert_direct_write_denied(path: &Path) {
    let error = OpenOptions::new().write(true).open(path).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn model_store_permission_probe() {
    let Some(root) = std::env::var_os(PROBE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let scenario = std::env::var(PERMISSION_SCENARIO).unwrap();
    let parent = root.join("history");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let (store, paths) = fixture(&parent);
    for (slot, path) in paths.iter().enumerate() {
        let readonly = (scenario == "data" && slot < 2) || (scenario == "lock" && slot >= 2);
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if readonly { 0o400 } else { 0o600 }),
        )
        .unwrap();
    }
    if scenario == "parent_missing_locks" {
        for path in &paths[2..] {
            fs::remove_file(path).unwrap();
        }
    }
    let readonly_parent = scenario.starts_with("parent_");
    fs::set_permissions(
        &parent,
        fs::Permissions::from_mode(if readonly_parent { 0o555 } else { 0o755 }),
    )
    .unwrap();
    drop_root_privileges(&paths);
    let snapshots = paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            (
                path.clone(),
                fs::read(path).unwrap(),
                fs::metadata(path).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    match scenario.as_str() {
        "data" => {
            for path in &paths[..2] {
                assert_direct_write_denied(path);
            }
            assert_eq!(store.read_all().unwrap(), vec![record("original-success")]);
            assert_eq!(
                store.read_page(0, 1).unwrap(),
                vec![record("original-success")]
            );
            assert_eq!(
                store.read_failures().unwrap(),
                vec![failure("original-failure")]
            );
            store
                .append(&record("original-success"))
                .expect("identical retry uses existing read-only data");
            store
                .append_failure(&failure("original-failure"))
                .expect("identical failure retry uses existing read-only data");
            assert_permission_denied(invoke(&store, 0, "append"));
            assert_permission_denied(invoke(&store, 1, "append"));
        }
        "lock" => {
            for path in &paths[2..] {
                assert_direct_write_denied(path);
            }
            for slot in 0..2 {
                assert_permission_denied(invoke(&store, slot, "read"));
                assert_permission_denied(invoke(&store, slot, "append"));
            }
            assert_permission_denied(invoke(&store, 0, "zero"));
        }
        "parent_existing_locks" | "parent_missing_locks" => {
            let direct_error = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(parent.join("new-file-must-not-exist"))
                .unwrap_err();
            assert_eq!(direct_error.kind(), ErrorKind::PermissionDenied);
            assert!(!parent.join("new-file-must-not-exist").exists());
            if scenario == "parent_existing_locks" {
                assert_eq!(store.read_all().unwrap(), vec![record("original-success")]);
                assert_eq!(
                    store.read_failures().unwrap(),
                    vec![failure("original-failure")]
                );
                invoke(&store, 0, "zero").unwrap();
                // Existing writable files do not require directory write permission.
                invoke(&store, 0, "append").unwrap();
                invoke(&store, 1, "append").unwrap();
                assert_eq!(
                    store.read_all().unwrap(),
                    vec![record("original-success"), record("new-success")]
                );
                assert_eq!(
                    store.read_failures().unwrap(),
                    vec![failure("original-failure"), failure("new-failure")]
                );
            } else {
                for slot in 0..2 {
                    assert_permission_denied(invoke(&store, slot, "read"));
                    assert_permission_denied(invoke(&store, slot, "append"));
                }
                assert_permission_denied(invoke(&store, 0, "zero"));
                assert!(!paths[2].exists());
                assert!(!paths[3].exists());
            }
        }
        _ => panic!("unsupported permission scenario"),
    }
    for (path, bytes, before) in snapshots {
        if scenario != "parent_existing_locks" || path == paths[2] || path == paths[3] {
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let after = fs::metadata(path).unwrap();
        assert_eq!(
            (
                after.dev(),
                after.ino(),
                after.mode(),
                after.uid(),
                after.gid(),
                after.nlink()
            ),
            (
                before.dev(),
                before.ino(),
                before.mode(),
                before.uid(),
                before.gid(),
                before.nlink()
            )
        );
    }
    assert_eq!(
        fs::metadata(parent).unwrap().mode() & 0o777,
        if readonly_parent { 0o555 } else { 0o755 }
    );
}

#[test]
fn constructor_and_post_open_parent_symlinks_are_rejected_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("constructor-target");
    let link = root.path().join("constructor-link");
    fs::create_dir(&target).unwrap();
    symlink(&target, &link).unwrap();
    let error = ModelCallStore::open(link.join("must-not-be-created").join("calls.jsonl"))
        .expect_err("constructor must reject a symlink parent before mkdir");
    assert_safe_error(&error);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());

    let original = root.path().join("original-parent");
    let moved = root.path().join("moved-parent");
    let (store, paths) = fixture(&original);
    let snapshots = paths
        .iter()
        .map(|path| {
            (
                path.file_name().unwrap().to_os_string(),
                fs::read(path).unwrap(),
                fs::metadata(path).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    fs::rename(&original, &moved).unwrap();
    symlink(&moved, &original).unwrap();
    for (slot, operation) in [
        (0, "append"),
        (1, "append"),
        (0, "read"),
        (1, "read"),
        (0, "page"),
        (0, "zero"),
    ] {
        let result = invoke(&store, slot, operation);
        for (name, bytes, before) in &snapshots {
            let path = moved.join(name);
            assert_eq!(fs::read(&path).unwrap(), *bytes);
            let after = fs::metadata(path).unwrap();
            assert_eq!(
                (after.dev(), after.ino(), after.mode(), after.nlink()),
                (before.dev(), before.ino(), before.mode(), before.nlink())
            );
        }
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 4);
        assert!(fs::symlink_metadata(&original).unwrap().is_symlink());
        assert_safe_error(
            &result.expect_err("every access, including zero, must reject the replaced parent"),
        );
    }
}

#[test]
fn constructor_rejects_invalid_paths_before_mkdir_but_storage_names_remain_pure() {
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
            .join(std::ffi::OsString::from_vec(b"bad\0basename".to_vec())),
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
        let names = ModelCallStore::storage_paths_for(&path);
        assert_eq!(names[0], path);
        assert_eq!(names[1], path.with_extension("failures.jsonl"));
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            0,
            "pure naming must not mkdir"
        );
        let error =
            ModelCallStore::open(&path).expect_err("invalid full path must fail before mkdir");
        assert_safe_error(&error);
        assert!(
            matches!(error, ModelCallStoreError::Io(ref source) if source.kind() == ErrorKind::InvalidInput)
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
