#![cfg(unix)]

use chrono::{Duration as ChronoDuration, Utc};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use steerbox::approval_contract::{
    ApprovalDecision, ApprovalError, ApprovalRecord, ApprovalRequest, ApprovalScope,
    ApprovalStatus, ApprovalStore,
};

const PROBE_ROOT: &str = "STEERBOX_APPROVAL_PROCESS_ROOT";
const PROBE_MODE: &str = "STEERBOX_APPROVAL_PROCESS_MODE";
const TIMEOUT: Duration = Duration::from_secs(10);
const BUSY_MESSAGE: &str = "approval store is busy or locking is unsupported";

fn request() -> ApprovalRequest {
    let now = Utc::now();
    ApprovalRequest::at(
        "synthetic-once",
        "synthetic-project",
        "synthetic-session",
        "synthetic-task",
        "read_file",
        "workspace-source",
        "D1",
        "inspect synthetic source",
        now - ChronoDuration::minutes(1),
        now + ChronoDuration::days(1),
    )
    .expect("synthetic request is valid")
}

fn data_path(root: &Path) -> PathBuf {
    root.join("synthetic-approvals.jsonl")
}

fn approved_fixture(root: &Path) -> ApprovalRequest {
    let request = request();
    let store = ApprovalStore::open(data_path(root));
    store
        .submit(request.clone())
        .expect("submit synthetic request");
    store
        .resolve(
            &request.request_id,
            ApprovalDecision::Approve(ApprovalScope::Once),
        )
        .expect("approve synthetic once grant");
    request
}

struct ProbeChild {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl ProbeChild {
    fn spawn(root: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "approval_store_process_probe",
                "--nocapture",
                "--quiet",
                "--test-threads=1",
            ])
            .env_clear()
            .env(PROBE_ROOT, root)
            .env(PROBE_MODE, "consume")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn synthetic approval probe");
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
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
            input: Some(input),
            messages,
            reader: Some(reader),
        }
    }

    fn send(&mut self, command: &str) {
        let input = self.input.as_mut().expect("probe stdin remains open");
        writeln!(input, "{command}").expect("send fixed probe command");
        input.flush().expect("flush fixed probe command");
    }

    fn receive(&self) -> String {
        self.messages
            .recv_timeout(TIMEOUT)
            .expect("probe must ACK before timeout or disconnect")
    }

    fn ready(&self) {
        assert_eq!(self.receive(), "ACK READY");
    }

    fn finish(mut self) {
        self.send("EXIT");
        assert_eq!(self.receive(), "ACK EXIT");
        drop(self.input.take());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("inspect probe exit") {
                assert!(status.success(), "synthetic approval probe failed");
                break;
            }
            assert!(Instant::now() < deadline, "approval probe exit timed out");
            thread::yield_now();
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
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn assert_consumed_history(root: &Path, request: &ApprovalRequest) {
    let bytes = fs::read(data_path(root)).expect("read synthetic terminal history");
    assert!(bytes.ends_with(b"\n"));
    let history: Vec<ApprovalRecord> = std::str::from_utf8(&bytes)
        .expect("synthetic history is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid approval snapshot"))
        .collect();
    assert_eq!(history.len(), 3, "exactly one consumption snapshot");
    assert_eq!(history[0].status, ApprovalStatus::Pending);
    assert_eq!(history[1].status, ApprovalStatus::Approved);
    assert_eq!(history[2].status, ApprovalStatus::Consumed);
    for record in &history {
        assert_eq!(&record.request, request);
    }
    assert_eq!(
        ApprovalStore::open(data_path(root))
            .read_all()
            .expect("replay terminal history"),
        vec![history[2].clone()]
    );
}

#[test]
fn held_actual_lock_returns_busy_before_release_then_retry_succeeds() {
    let root = tempfile::tempdir().expect("synthetic root");
    let request = approved_fixture(root.path());
    let mut consumer = ProbeChild::spawn(root.path());
    consumer.ready();
    let mut lock_path = data_path(root.path()).into_os_string();
    lock_path.push(".lock");
    let held_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(PathBuf::from(lock_path))
        .expect("open existing actual approval sidecar");
    held_lock.try_lock().expect("hold actual approval lock");

    consumer.send("GO");
    assert_eq!(consumer.receive(), "ACK BUSY");
    // Other tests can spawn while this descriptor is open. Explicitly release
    // the lock instead of relying on all inherited descriptors being closed.
    held_lock.unlock().expect("release actual approval lock");
    drop(held_lock);
    consumer.send("RETRY");
    assert_eq!(consumer.receive(), "ACK TRUE");
    consumer.finish();
    assert_consumed_history(root.path(), &request);
}

#[test]
fn preopened_process_reloads_consumed_after_other_process_ack() {
    let root = tempfile::tempdir().expect("synthetic root");
    let request = approved_fixture(root.path());
    let mut first = ProbeChild::spawn(root.path());
    first.ready();
    let mut second = ProbeChild::spawn(root.path());
    second.ready();

    first.send("GO");
    assert_eq!(first.receive(), "ACK TRUE");
    second.send("GO");
    assert_eq!(second.receive(), "ACK FALSE");
    first.finish();
    second.finish();
    assert_consumed_history(root.path(), &request);
}

#[test]
fn competing_processes_consume_once_and_busy_retry_observes_consumed() {
    let root = tempfile::tempdir().expect("synthetic root");
    let request = approved_fixture(root.path());
    let mut first = ProbeChild::spawn(root.path());
    first.ready();
    let mut second = ProbeChild::spawn(root.path());
    second.ready();

    first.send("GO");
    second.send("GO");
    let first_result = first.receive();
    let second_result = second.receive();
    match (first_result.as_str(), second_result.as_str()) {
        ("ACK TRUE", "ACK FALSE") | ("ACK FALSE", "ACK TRUE") => {}
        ("ACK TRUE", "ACK BUSY") => {
            second.send("RETRY");
            assert_eq!(second.receive(), "ACK FALSE");
        }
        ("ACK BUSY", "ACK TRUE") => {
            first.send("RETRY");
            assert_eq!(first.receive(), "ACK FALSE");
        }
        _ => panic!("unexpected fixed approval ACK pair"),
    }
    first.finish();
    second.finish();
    assert_consumed_history(root.path(), &request);
}

fn ack(value: &str) {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "ACK {value}").expect("write fixed probe ACK");
    output.flush().expect("flush fixed probe ACK");
}

#[test]
fn approval_store_process_probe() {
    let Some(mode) = std::env::var_os(PROBE_MODE) else {
        return;
    };
    assert_eq!(mode, "consume");
    let root = PathBuf::from(std::env::var_os(PROBE_ROOT).expect("synthetic probe root"));
    let store = ApprovalStore::open(data_path(&root));
    let approved = store
        .latest("synthetic-once")
        .expect("preopen approved snapshot");
    assert_eq!(approved.status, ApprovalStatus::Approved);
    ack("READY");

    for command in std::io::stdin().lock().lines() {
        match command.expect("read fixed probe command").as_str() {
            "GO" | "RETRY" => match store.consume(&approved.request) {
                Ok(true) => ack("TRUE"),
                Ok(false) => ack("FALSE"),
                Err(ApprovalError::Initialization(message)) if message == BUSY_MESSAGE => {
                    ack("BUSY");
                }
                Err(_) => panic!("unexpected approval consume error"),
            },
            "EXIT" => {
                ack("EXIT");
                return;
            }
            _ => panic!("unexpected fixed probe command"),
        }
    }
    panic!("probe stdin closed without EXIT");
}
