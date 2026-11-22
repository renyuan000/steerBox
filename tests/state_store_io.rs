use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use steerbox::domain::EventEnvelope;
use steerbox::state_store::{StateStore, StateStoreError};

const PROBE_MODE: &str = "STEERBOX_STATE_STORE_IO_PROBE";
const ACK: &str = "ACK STATE_ROUND_TRIP";
const TIMEOUT: Duration = Duration::from_secs(10);

fn populated_state() -> StateStore {
    let event = EventEnvelope::new(
        "loop_run",
        "synthetic-started",
        "loop_started",
        "synthetic-loop",
        "synthetic-state-store-io",
        serde_json::json!({
            "status": "running",
            "goal_id": "synthetic-goal",
            "task_run_id": "synthetic-task"
        }),
    )
    .expect("synthetic event is valid");
    StateStore::rebuild(&[event]).expect("synthetic projection rebuilds")
}

fn assert_only_target_remains(directory: &Path, target: &Path) {
    let entries: Vec<_> = fs::read_dir(directory)
        .expect("inspect synthetic directory")
        .map(|entry| entry.expect("read synthetic directory entry").file_name())
        .collect();
    assert_eq!(
        entries,
        vec![target.file_name().expect("target basename").to_os_string()]
    );
    assert!(fs::symlink_metadata(target)
        .expect("inspect synthetic target")
        .file_type()
        .is_file());
}

#[test]
fn load_missing_file_does_not_create_it_or_its_parent() {
    let root = tempfile::tempdir().expect("synthetic root");
    let path = root.path().join("missing-parent").join("state.json");

    assert!(matches!(
        StateStore::load(&path),
        Err(StateStoreError::Io { .. })
    ));
    assert_eq!(fs::read_dir(root.path()).expect("inspect root").count(), 0);
}

#[test]
fn load_invalid_utf8_retains_io_error_classification() {
    let root = tempfile::tempdir().expect("synthetic root");
    let path = root.path().join("state.json");
    let bytes = b"{\xff}";
    fs::write(&path, bytes).expect("write synthetic non-UTF-8 bytes");

    assert!(matches!(
        StateStore::load(&path),
        Err(StateStoreError::Io { .. })
    ));
    assert_eq!(fs::read(&path).expect("reread fixture"), bytes);
    assert_only_target_remains(root.path(), &path);
}

#[test]
fn load_malformed_json_retains_invalid_json_error_classification() {
    let root = tempfile::tempdir().expect("synthetic root");
    let path = root.path().join("state.json");
    let bytes = b"{not-json}";
    fs::write(&path, bytes).expect("write synthetic malformed JSON");

    assert!(matches!(
        StateStore::load(&path),
        Err(StateStoreError::InvalidJson { .. })
    ));
    assert_eq!(fs::read(&path).expect("reread fixture"), bytes);
    assert_only_target_remains(root.path(), &path);
}

#[test]
fn persist_retains_pretty_json_without_a_trailing_line_feed() {
    let root = tempfile::tempdir().expect("synthetic root");
    let path = root.path().join("state.json");
    let state = StateStore::default();

    state.persist(&path).expect("persist empty projection");

    assert_eq!(
        fs::read(&path).expect("read synthetic projection"),
        b"{\n  \"loops\": {},\n  \"steps\": {}\n}"
    );
    assert_eq!(StateStore::load(&path).expect("load projection"), state);
    assert_only_target_remains(root.path(), &path);
}

#[test]
fn persist_creates_parents_and_replaces_snapshots_without_temporary_residue() {
    let root = tempfile::tempdir().expect("synthetic root");
    let directory = root.path().join("new-parent");
    let path = directory.join("state.json");

    for state in [
        StateStore::default(),
        populated_state(),
        StateStore::default(),
    ] {
        state.persist(&path).expect("persist complete snapshot");

        assert_eq!(
            StateStore::load(&path).expect("load latest snapshot"),
            state
        );
        assert_eq!(
            fs::read(&path).expect("read latest snapshot bytes"),
            serde_json::to_vec_pretty(&state).expect("encode expected snapshot")
        );
        assert_only_target_remains(&directory, &path);
    }
}

#[cfg(unix)]
#[test]
fn persist_and_load_accept_a_raw_non_utf8_regular_filename() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = tempfile::tempdir().expect("synthetic root");
    let filename = OsString::from_vec(b"state-\xff.json".to_vec());
    let path = root.path().join(filename);
    assert!(path.to_str().is_none());

    for state in [populated_state(), StateStore::default()] {
        state.persist(&path).expect("persist raw filename");

        assert_eq!(StateStore::load(&path).expect("load raw filename"), state);
        assert_only_target_remains(root.path(), &path);
    }
}

struct ProbeChild {
    child: Child,
    messages: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl ProbeChild {
    fn spawn(root: &Path, mode: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "state_store_relative_path_probe",
                "--nocapture",
                "--quiet",
                "--test-threads=1",
            ])
            .env_clear()
            .env(PROBE_MODE, mode)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn synthetic relative-path probe");
        let output = child.stdout.take().expect("piped probe stdout");
        let (sender, messages) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) if line.starts_with("ACK ") => {
                        if sender.send(line).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        let _ = sender.send("ACK READER_ERROR".into());
                        return;
                    }
                }
            }
        });
        Self {
            child,
            messages,
            reader: Some(reader),
        }
    }

    fn finish(mut self) {
        assert_eq!(
            self.messages
                .recv_timeout(TIMEOUT)
                .expect("probe must ACK before timeout or disconnect"),
            ACK
        );
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("inspect probe exit") {
                assert!(status.success(), "synthetic relative-path probe failed");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "relative-path probe exit timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.reader
            .take()
            .expect("stdout reader exists")
            .join()
            .expect("stdout reader joins");
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn persist_and_load_accept_a_bare_relative_basename() {
    let root = tempfile::tempdir().expect("synthetic root");
    ProbeChild::spawn(root.path(), "basename").finish();
    assert_only_target_remains(root.path(), &root.path().join("state.json"));
}

#[test]
fn persist_and_load_accept_an_explicit_current_directory_basename() {
    let root = tempfile::tempdir().expect("synthetic root");
    ProbeChild::spawn(root.path(), "dot-basename").finish();
    assert_only_target_remains(root.path(), &root.path().join("state.json"));
}

#[test]
fn exclusive_temporary_collision_preserves_existing_files_and_allows_retry() {
    let root = tempfile::tempdir().expect("synthetic root");
    ProbeChild::spawn(root.path(), "exclusive-conflict").finish();
}

fn assert_exclusive_temporary_collision_and_retry() {
    let path = Path::new("./state.json");
    let old_state = StateStore::default();
    let old_bytes = serde_json::to_vec_pretty(&old_state).expect("encode old snapshot");
    fs::write(path, &old_bytes).expect("precreate ordinary target without consuming temp id");
    let temporary = path.with_extension(format!("json.tmp.{}.1", std::process::id()));
    let sentinel = b"preexisting synthetic temporary file";
    fs::write(&temporary, sentinel).expect("precreate ordinary conflicting temporary");
    let replacement = populated_state();

    assert!(matches!(
        replacement.persist(path),
        Err(StateStoreError::Io { .. })
    ));
    assert_eq!(fs::read(path).expect("read protected target"), old_bytes);
    assert_eq!(
        StateStore::load(path).expect("load old snapshot"),
        old_state
    );
    assert_eq!(
        fs::read(&temporary).expect("read protected conflicting temporary"),
        sentinel
    );

    replacement.persist(path).expect("retry with next temp id");

    assert_eq!(
        StateStore::load(path).expect("load replacement snapshot"),
        replacement
    );
    assert_eq!(
        fs::read(path).expect("read replacement bytes"),
        serde_json::to_vec_pretty(&replacement).expect("encode replacement snapshot")
    );
    assert_eq!(
        fs::read(&temporary).expect("conflicting temporary survives retry"),
        sentinel
    );
    let retry_temporary = path.with_extension(format!("json.tmp.{}.2", std::process::id()));
    assert!(matches!(
        fs::symlink_metadata(retry_temporary),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ));
}

#[test]
fn state_store_relative_path_probe() {
    let mode = match std::env::var(PROBE_MODE) {
        Ok(mode) => mode,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => panic!("invalid synthetic probe mode: {error}"),
    };
    match mode.as_str() {
        "exclusive-conflict" => assert_exclusive_temporary_collision_and_retry(),
        "basename" | "dot-basename" => {
            let path = if mode == "basename" {
                Path::new("state.json")
            } else {
                Path::new("./state.json")
            };
            let state = populated_state();
            state.persist(path).expect("persist relative-path snapshot");
            assert_eq!(
                StateStore::load(path).expect("load relative snapshot"),
                state
            );
            assert_only_target_remains(Path::new("."), path);
        }
        _ => panic!("unknown synthetic probe mode"),
    }
    let mut output = std::io::stdout().lock();
    writeln!(output, "{ACK}").expect("write fixed probe ACK");
    output.flush().expect("flush fixed probe ACK");
}
