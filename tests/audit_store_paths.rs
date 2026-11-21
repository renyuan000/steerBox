#![cfg(unix)]

use std::ffi::{CString, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use steerbox::audit_store::{append_jsonl, read_jsonl, AuditStoreError};

const PATH_SENTINEL: &str = "synthetic-private-audit-path-sentinel";
const PROBE_OPERATION: &str = "STEERBOX_AUDIT_PATH_TEST_OPERATION";
const PROBE_PATH: &str = "STEERBOX_AUDIT_PATH_TEST_PATH";

fn assert_safe_error(error: AuditStoreError) {
    assert!(!error.to_string().contains(PATH_SENTINEL));
    assert!(!format!("{error:?}").contains(PATH_SENTINEL));
}

fn assert_read_write_rejected(path: &Path) {
    let append = append_jsonl(path, &17_u64);
    let read = read_jsonl::<serde_json::Value>(path);
    assert_safe_error(append.expect_err("unsafe append must be rejected"));
    assert_safe_error(read.expect_err("unsafe read must be rejected"));
}

fn rejection_probe(path: &Path, operation: &str) -> Output {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "audit_path_rejection_probe", "--nocapture"])
        .env(PROBE_OPERATION, operation)
        .env(PROBE_PATH, path)
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
            panic!("unsafe audit path operation blocked instead of rejecting");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_probe_rejected(output: Output) {
    assert!(
        output.status.success(),
        "audit path rejection probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn audit_path_rejection_probe() {
    let Some(operation) = std::env::var_os(PROBE_OPERATION) else {
        return;
    };
    let path = std::env::var_os(PROBE_PATH).expect("probe path is required");
    match operation.to_str().unwrap() {
        "append" => assert_safe_error(
            append_jsonl(Path::new(&path), &17_u64).expect_err("unsafe node must reject append"),
        ),
        "read" => assert_safe_error(
            read_jsonl::<serde_json::Value>(Path::new(&path))
                .expect_err("unsafe node must reject read"),
        ),
        _ => panic!("unsupported probe operation"),
    }
}

#[test]
fn nested_missing_parents_are_created_for_append_and_records_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("first/second/records.jsonl");
    append_jsonl(&path, &17_u64).unwrap();
    append_jsonl(&path, &serde_json::json!({"synthetic": true})).unwrap();
    assert_eq!(
        read_jsonl::<serde_json::Value>(&path).unwrap(),
        vec![
            serde_json::json!(17),
            serde_json::json!({"synthetic": true})
        ]
    );
}

#[test]
fn reading_missing_paths_does_not_create_files_or_parent_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing-parent/records.jsonl");
    assert!(read_jsonl::<u64>(&path).unwrap().is_empty());
    assert!(!path.exists());
    assert!(!root.path().join("missing-parent").exists());
    let direct = root.path().join("missing.jsonl");
    assert!(read_jsonl::<u64>(&direct).unwrap().is_empty());
    assert!(!direct.exists());
}

#[test]
fn relative_paths_work_without_changing_process_current_directory() {
    let current = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&current).unwrap();
    let absolute = root.path().join("nested/relative.jsonl");
    let relative = absolute.strip_prefix(&current).unwrap();
    append_jsonl(relative, &17_u64).unwrap();
    assert_eq!(read_jsonl::<u64>(relative).unwrap(), vec![17]);
    assert_eq!(std::env::current_dir().unwrap(), current);
}

#[test]
fn final_symlinks_are_rejected_without_changing_the_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.jsonl");
    let original = b"11\n";
    fs::write(&target, original).unwrap();
    let link = root.path().join(PATH_SENTINEL);
    symlink(&target, &link).unwrap();
    let append = append_jsonl(&link, &17_u64);
    let read = read_jsonl::<u64>(&link);
    assert_eq!(fs::read(&target).unwrap(), original);
    assert_eq!(fs::read_link(&link).unwrap(), target);
    assert_safe_error(append.expect_err("final symlink append must fail"));
    assert_safe_error(read.expect_err("final symlink read must fail"));
}

#[test]
fn symlinked_parent_chains_are_rejected_without_creating_descendants() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    let original_path = target.join("records.jsonl");
    fs::write(&original_path, b"11\n").unwrap();
    let link = root.path().join(PATH_SENTINEL);
    symlink(&target, &link).unwrap();
    let append = append_jsonl(link.join("records.jsonl"), &17_u64);
    let read = read_jsonl::<u64>(link.join("records.jsonl"));
    assert_eq!(fs::read(&original_path).unwrap(), b"11\n");
    assert_safe_error(append.expect_err("parent symlink append must fail"));
    assert_safe_error(read.expect_err("parent symlink read must fail"));
    assert_read_write_rejected(&link.join("new/nested/records.jsonl"));
    assert!(!target.join("new").exists());
}

#[test]
fn dangling_final_symlinks_are_rejected_without_creating_targets() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("missing-target.jsonl");
    let link = root.path().join(PATH_SENTINEL);
    symlink(&target, &link).unwrap();
    let append = append_jsonl(&link, &17_u64);
    let read = read_jsonl::<u64>(&link);
    assert!(!target.exists());
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_safe_error(append.expect_err("dangling final symlink append must fail"));
    assert_safe_error(read.expect_err("dangling final symlink read must fail"));
}

#[test]
fn dangling_parent_symlinks_are_rejected_without_creating_targets() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("missing-parent");
    let link = root.path().join(PATH_SENTINEL);
    symlink(&target, &link).unwrap();
    assert_read_write_rejected(&link.join("records.jsonl"));
    assert!(!target.exists());
    assert_eq!(fs::read_link(&link).unwrap(), target);
}

#[test]
fn fifo_is_rejected_for_read_and_append_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(PATH_SENTINEL);
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let before = fs::symlink_metadata(&path).unwrap();
    let append = rejection_probe(&path, "append");
    let read = rejection_probe(&path, "read");
    let after = fs::symlink_metadata(&path).unwrap();
    assert!(after.file_type().is_fifo());
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(after.len(), before.len());
    assert_probe_rejected(append);
    assert_probe_rejected(read);
}

#[test]
fn character_device_is_rejected_for_read_and_append_without_blocking() {
    let path = Path::new("/dev/null");
    let before = fs::symlink_metadata(path).unwrap();
    assert!(before.file_type().is_char_device());
    let append = rejection_probe(path, "append");
    let read = rejection_probe(path, "read");
    let after = fs::symlink_metadata(path).unwrap();
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(after.rdev(), before.rdev());
    assert_probe_rejected(append);
    assert_probe_rejected(read);
}

#[test]
fn directories_are_rejected_for_read_and_append_without_changing_contents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(PATH_SENTINEL);
    fs::create_dir(&path).unwrap();
    let original_path = path.join("original.jsonl");
    fs::write(&original_path, b"11\n").unwrap();
    let append = rejection_probe(&path, "append");
    let read = rejection_probe(&path, "read");
    assert_eq!(fs::read(&original_path).unwrap(), b"11\n");
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    assert_probe_rejected(append);
    assert_probe_rejected(read);
}

#[test]
fn hard_links_are_rejected_for_read_and_append_without_changing_either_name() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original.jsonl");
    fs::write(&original, b"11\n").unwrap();
    let linked = root.path().join(PATH_SENTINEL);
    fs::hard_link(&original, &linked).unwrap();
    let append = rejection_probe(&linked, "append");
    let read = rejection_probe(&linked, "read");
    assert_eq!(fs::read(&original).unwrap(), b"11\n");
    assert_eq!(fs::read(&linked).unwrap(), b"11\n");
    assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
    assert_probe_rejected(append);
    assert_probe_rejected(read);
}

#[test]
fn parent_traversal_is_rejected_without_changing_the_destination() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    let target = root.path().join(PATH_SENTINEL);
    fs::write(&target, b"11\n").unwrap();
    let traversal = root.path().join("child/..").join(PATH_SENTINEL);
    let append = append_jsonl(&traversal, &17_u64);
    let read = read_jsonl::<u64>(&traversal);
    assert_eq!(fs::read(&target).unwrap(), b"11\n");
    assert_safe_error(append.expect_err("parent traversal append must fail"));
    assert_safe_error(read.expect_err("parent traversal read must fail"));
}

#[test]
fn paths_without_a_file_name_are_rejected_without_creating_directories() {
    assert_read_write_rejected(Path::new(""));
    assert_read_write_rejected(Path::new("/"));
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join(PATH_SENTINEL);
    let path = missing.join("..");
    assert_read_write_rejected(&path);
    assert!(!missing.exists());
}

#[test]
fn non_utf8_file_names_round_trip_without_lossy_path_conversion() {
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join(OsString::from_vec(b"audit-\xff.jsonl".to_vec()));
    assert!(path.file_name().unwrap().to_str().is_none());
    append_jsonl(&path, &17_u64).unwrap();
    assert_eq!(read_jsonl::<u64>(&path).unwrap(), vec![17]);
    assert_eq!(fs::read(path).unwrap(), b"17\n");
}

#[test]
fn parse_errors_do_not_expose_synthetic_private_path_names() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(PATH_SENTINEL);
    fs::write(&path, b"{\n").unwrap();
    let error = read_jsonl::<serde_json::Value>(&path).expect_err("invalid JSON must fail");
    assert_safe_error(error);
    assert_eq!(fs::read(path).unwrap(), b"{\n");
}

#[test]
fn raw_trailing_separator_and_dot_are_rejected_before_path_normalization() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join(PATH_SENTINEL);
    fs::write(&target, b"11\n").unwrap();
    for suffix in ["/", "/."] {
        let mut raw = target.as_os_str().to_os_string();
        raw.push(suffix);
        assert!(raw.as_bytes().ends_with(suffix.as_bytes()));
        let append = append_jsonl(Path::new(&raw), &17_u64);
        let read = read_jsonl::<u64>(Path::new(&raw));
        assert_eq!(fs::read(&target).unwrap(), b"11\n");
        assert_safe_error(append.expect_err("raw trailing path suffix must reject append"));
        assert_safe_error(read.expect_err("raw trailing path suffix must reject read"));

        let missing = root.path().join("missing-parent");
        let mut raw_missing = missing.join(PATH_SENTINEL).into_os_string();
        raw_missing.push(suffix);
        let append = append_jsonl(Path::new(&raw_missing), &17_u64);
        let read = read_jsonl::<u64>(Path::new(&raw_missing));
        assert!(!missing.exists());
        assert_safe_error(append.expect_err("invalid suffix must not create parent directories"));
        assert_safe_error(read.expect_err("missing path with invalid suffix must reject read"));
    }
}

#[test]
fn explicit_dot_prefix_remains_usable_without_changing_current_directory() {
    let current = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&current).unwrap();
    let absolute = root.path().join("nested/dot-prefixed.jsonl");
    let relative = absolute.strip_prefix(&current).unwrap();
    let prefixed = Path::new(".").join(relative);
    assert!(prefixed.as_os_str().as_bytes().starts_with(b"./"));
    append_jsonl(&prefixed, &17_u64).unwrap();
    assert_eq!(read_jsonl::<u64>(&prefixed).unwrap(), vec![17]);
    assert_eq!(fs::read(&absolute).unwrap(), b"17\n");
    assert_eq!(std::env::current_dir().unwrap(), current);
}

#[test]
fn nul_in_file_name_is_rejected_before_creating_any_parent_directory() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join(PATH_SENTINEL);
    let path = missing
        .join("nested")
        .join(OsString::from_vec(b"records\0.jsonl".to_vec()));
    let append = append_jsonl(&path, &17_u64);
    let read = read_jsonl::<u64>(&path);
    assert!(!missing.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_safe_error(append.expect_err("NUL file name must reject append"));
    assert_safe_error(read.expect_err("NUL file name must reject read"));
}

#[test]
fn nul_in_parent_component_is_rejected_before_creating_earlier_directories() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join(PATH_SENTINEL);
    let path = missing
        .join(OsString::from_vec(b"invalid\0parent".to_vec()))
        .join("nested/records.jsonl");
    let append = append_jsonl(&path, &17_u64);
    let read = read_jsonl::<u64>(&path);
    assert!(!missing.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_safe_error(append.expect_err("NUL parent must reject append"));
    assert_safe_error(read.expect_err("NUL parent must reject read"));
}
