use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncPolicy {
    pub interval: Duration,
    pub byte_threshold: u64,
    pub max_total_bytes: u64,
}

impl Default for SyncPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            byte_threshold: 64 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
        }
    }
}

impl SyncPolicy {
    pub fn validate(&self) -> Result<(), WriterError> {
        if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&self.interval)
            || !(1..=4 * 1024 * 1024).contains(&self.byte_threshold)
            || !(1..=1024 * 1024 * 1024).contains(&self.max_total_bytes)
        {
            return Err(WriterError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DurabilityReceipt {
    pub written_sequence: u64,
    pub durable_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterError {
    InvalidPolicy,
    CapacityExceeded,
    Unavailable,
    Io(io::ErrorKind),
}

impl fmt::Display for WriterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy => formatter.write_str("invalid transcript sync policy"),
            Self::CapacityExceeded => formatter.write_str("transcript capacity exceeded"),
            Self::Unavailable => formatter.write_str("transcript writer unavailable"),
            Self::Io(kind) => write!(formatter, "transcript I/O failure: {kind:?}"),
        }
    }
}

impl std::error::Error for WriterError {}

trait Backend: Write + Send {
    fn sync_data(&mut self) -> io::Result<()>;
}

impl Backend for File {
    fn sync_data(&mut self) -> io::Result<()> {
        File::sync_data(self)
    }
}

struct WriterState {
    backend: Box<dyn Backend>,
    receipt: DurabilityReceipt,
    total_bytes: u64,
    pending_bytes: u64,
    dirty_since: Option<Instant>,
    failure: Option<WriterError>,
    stopping: bool,
}

impl WriterState {
    fn available(&self) -> Result<(), WriterError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.stopping {
            return Err(WriterError::Unavailable);
        }
        Ok(())
    }

    fn sync_pending(&mut self) -> Result<DurabilityReceipt, WriterError> {
        if self.dirty_since.is_some() {
            if let Err(error) = self.backend.sync_data() {
                let error = WriterError::Io(error.kind());
                self.failure = Some(error);
                return Err(error);
            }
            self.receipt.durable_sequence = self.receipt.written_sequence;
            self.pending_bytes = 0;
            self.dirty_since = None;
        }
        Ok(self.receipt)
    }
}

struct Shared {
    state: Mutex<WriterState>,
    wakeup: Condvar,
    worker_failed: AtomicBool,
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, WriterState>, WriterError> {
        let state = self.state.lock().map_err(|_| WriterError::Unavailable)?;
        if self.worker_failed.load(Ordering::Acquire) {
            return Err(WriterError::Unavailable);
        }
        state.available()?;
        Ok(state)
    }
}

pub(super) struct TimedWriter {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    policy: SyncPolicy,
}

impl TimedWriter {
    pub(super) fn new(file: File, policy: SyncPolicy) -> Result<Self, WriterError> {
        Self::new_with_backend(Box::new(file), policy)
    }

    fn new_with_backend(
        backend: Box<dyn Backend>,
        policy: SyncPolicy,
    ) -> Result<Self, WriterError> {
        policy.validate()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(WriterState {
                backend,
                receipt: DurabilityReceipt::default(),
                total_bytes: 0,
                pending_bytes: 0,
                dirty_since: None,
                failure: None,
                stopping: false,
            }),
            wakeup: Condvar::new(),
            worker_failed: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("transcript-sync".to_owned())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    Self::run_timer(&worker_shared, policy.interval)
                }));
                if !matches!(outcome, Ok(Ok(()))) {
                    worker_shared.worker_failed.store(true, Ordering::Release);
                    worker_shared.wakeup.notify_all();
                }
            })
            .map_err(|error| WriterError::Io(error.kind()))?;
        Ok(Self {
            shared,
            worker: Some(worker),
            policy,
        })
    }

    pub(super) fn append(
        &self,
        encoded: Vec<u8>,
        immediate: bool,
    ) -> Result<DurabilityReceipt, WriterError> {
        let mut state = self.shared.lock()?;
        let byte_count = u64::try_from(encoded.len()).map_err(|_| WriterError::CapacityExceeded)?;
        let total_bytes = state
            .total_bytes
            .checked_add(byte_count)
            .filter(|total| *total <= self.policy.max_total_bytes)
            .ok_or(WriterError::CapacityExceeded)?;
        let sequence = state
            .receipt
            .written_sequence
            .checked_add(1)
            .ok_or(WriterError::CapacityExceeded)?;
        let started = Instant::now();
        if let Err(error) = state.backend.write_all(&encoded) {
            let error = WriterError::Io(error.kind());
            state.failure = Some(error);
            self.shared.wakeup.notify_all();
            return Err(error);
        }
        state.total_bytes = total_bytes;
        state.pending_bytes += byte_count;
        state.receipt.written_sequence = sequence;
        if state.dirty_since.is_none() {
            state.dirty_since = Some(started);
            self.shared.wakeup.notify_one();
        }
        if immediate || state.pending_bytes >= self.policy.byte_threshold {
            let result = state.sync_pending();
            if result.is_err() {
                self.shared.wakeup.notify_all();
            }
            return result;
        }
        Ok(state.receipt)
    }

    pub(super) fn status(&self) -> Result<DurabilityReceipt, WriterError> {
        Ok(self.shared.lock()?.receipt)
    }

    pub(super) fn sync(&self) -> Result<DurabilityReceipt, WriterError> {
        let result = self.shared.lock()?.sync_pending();
        if result.is_err() {
            self.shared.wakeup.notify_all();
        }
        result
    }

    pub(super) fn close(mut self) -> Result<DurabilityReceipt, WriterError> {
        self.shutdown()
    }

    fn run_timer(shared: &Shared, interval: Duration) -> Result<(), WriterError> {
        let mut state = shared.state.lock().map_err(|_| WriterError::Unavailable)?;
        loop {
            if state.stopping || state.failure.is_some() {
                return Ok(());
            }
            if let Some(started) = state.dirty_since {
                let remaining = interval.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    if state.sync_pending().is_err() {
                        return Ok(());
                    }
                } else {
                    (state, _) = shared
                        .wakeup
                        .wait_timeout(state, remaining)
                        .map_err(|_| WriterError::Unavailable)?;
                }
            } else {
                state = shared
                    .wakeup
                    .wait(state)
                    .map_err(|_| WriterError::Unavailable)?;
            }
        }
    }

    fn shutdown(&mut self) -> Result<DurabilityReceipt, WriterError> {
        let result = match self.shared.state.lock() {
            Ok(mut state) => {
                let result = if self.shared.worker_failed.load(Ordering::Acquire) {
                    Err(WriterError::Unavailable)
                } else if let Some(error) = state.failure {
                    Err(error)
                } else {
                    match catch_unwind(AssertUnwindSafe(|| state.sync_pending())) {
                        Ok(result) => result,
                        Err(_) => {
                            state.failure = Some(WriterError::Unavailable);
                            Err(WriterError::Unavailable)
                        }
                    }
                };
                state.stopping = true;
                self.shared.wakeup.notify_all();
                result
            }
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.failure = Some(WriterError::Unavailable);
                state.stopping = true;
                self.shared.wakeup.notify_all();
                Err(WriterError::Unavailable)
            }
        };
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() || self.shared.worker_failed.load(Ordering::Acquire) {
                return Err(WriterError::Unavailable);
            }
        }
        result
    }
}

impl Drop for TimedWriter {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    #[derive(Default)]
    struct Observation {
        bytes: Vec<u8>,
        syncs: usize,
        write_limit: Option<usize>,
        fail_sync: bool,
        panic_sync: bool,
        dropped: bool,
    }

    struct FakeBackend(Arc<Mutex<Observation>>);

    impl Write for FakeBackend {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut observation = self.0.lock().unwrap();
            let accepted = match observation.write_limit {
                Some(limit) if observation.bytes.len() >= limit => {
                    return Err(io::Error::other("synthetic private write details"));
                }
                Some(limit) => bytes.len().min(limit - observation.bytes.len()),
                None => bytes.len(),
            };
            observation.bytes.extend_from_slice(&bytes[..accepted]);
            Ok(accepted)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Backend for FakeBackend {
        fn sync_data(&mut self) -> io::Result<()> {
            let mut observation = self.0.lock().unwrap();
            observation.syncs += 1;
            if observation.panic_sync {
                drop(observation);
                panic!("synthetic backend panic");
            }
            if observation.fail_sync {
                return Err(io::Error::other("synthetic private sync details"));
            }
            Ok(())
        }
    }

    impl Drop for FakeBackend {
        fn drop(&mut self) {
            self.0.lock().unwrap().dropped = true;
        }
    }

    fn writer(policy: SyncPolicy) -> (TimedWriter, Arc<Mutex<Observation>>) {
        let observation = Arc::new(Mutex::new(Observation::default()));
        let writer =
            TimedWriter::new_with_backend(Box::new(FakeBackend(Arc::clone(&observation))), policy)
                .unwrap();
        (writer, observation)
    }

    fn slow_policy() -> SyncPolicy {
        SyncPolicy {
            interval: Duration::from_secs(60),
            ..SyncPolicy::default()
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "timed writer did not make progress"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn policy_defaults_and_inclusive_limits() {
        assert_eq!(SyncPolicy::default().interval, Duration::from_secs(1));
        assert_eq!(SyncPolicy::default().byte_threshold, 64 * 1024);
        assert_eq!(SyncPolicy::default().max_total_bytes, 64 * 1024 * 1024);
        for policy in [
            SyncPolicy {
                interval: Duration::from_millis(1),
                byte_threshold: 1,
                max_total_bytes: 1,
            },
            SyncPolicy {
                interval: Duration::from_secs(60),
                byte_threshold: 4 * 1024 * 1024,
                max_total_bytes: 1024 * 1024 * 1024,
            },
        ] {
            assert_eq!(policy.validate(), Ok(()));
        }
    }

    #[test]
    fn invalid_policies_fail_before_writer_starts() {
        for policy in [
            SyncPolicy {
                interval: Duration::ZERO,
                ..slow_policy()
            },
            SyncPolicy {
                interval: Duration::from_micros(999),
                ..slow_policy()
            },
            SyncPolicy {
                interval: Duration::from_secs(61),
                ..slow_policy()
            },
            SyncPolicy {
                byte_threshold: 0,
                ..slow_policy()
            },
            SyncPolicy {
                byte_threshold: 4 * 1024 * 1024 + 1,
                ..slow_policy()
            },
            SyncPolicy {
                max_total_bytes: 0,
                ..slow_policy()
            },
            SyncPolicy {
                max_total_bytes: 1024 * 1024 * 1024 + 1,
                ..slow_policy()
            },
        ] {
            assert_eq!(policy.validate(), Err(WriterError::InvalidPolicy));
            let observation = Arc::new(Mutex::new(Observation::default()));
            let result = TimedWriter::new_with_backend(
                Box::new(FakeBackend(Arc::clone(&observation))),
                policy,
            );
            assert!(matches!(result, Err(WriterError::InvalidPolicy)));
            assert!(observation.lock().unwrap().bytes.is_empty());
            assert_eq!(observation.lock().unwrap().syncs, 0);
        }
    }

    #[test]
    fn append_writes_exact_bytes_but_does_not_claim_durability() {
        let (writer, observation) = writer(slow_policy());
        let bytes = "{ \"body\": \"你好🦀\" }\n".as_bytes().to_vec();
        let receipt = writer.append(bytes.clone(), false).unwrap();
        assert_eq!(
            receipt,
            DurabilityReceipt {
                written_sequence: 1,
                durable_sequence: 0
            }
        );
        assert_eq!(observation.lock().unwrap().bytes, bytes);
        assert_eq!(observation.lock().unwrap().syncs, 0);
    }

    #[test]
    fn explicit_sync_advances_only_the_durable_watermark() {
        let (writer, observation) = writer(slow_policy());
        writer.append(b"one\n".to_vec(), false).unwrap();
        writer.append(b"two\n".to_vec(), false).unwrap();
        assert_eq!(
            writer.sync().unwrap(),
            DurabilityReceipt {
                written_sequence: 2,
                durable_sequence: 2
            }
        );
        assert_eq!(writer.sync().unwrap().durable_sequence, 2);
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn threshold_syncs_accumulated_pending_bytes() {
        let (writer, observation) = writer(SyncPolicy {
            byte_threshold: 4,
            ..slow_policy()
        });
        assert_eq!(
            writer
                .append(b"ab".to_vec(), false)
                .unwrap()
                .durable_sequence,
            0
        );
        assert_eq!(
            writer
                .append(b"cd".to_vec(), false)
                .unwrap()
                .durable_sequence,
            2
        );
        assert_eq!(observation.lock().unwrap().syncs, 1);
        assert_eq!(
            writer
                .append(b"e".to_vec(), false)
                .unwrap()
                .durable_sequence,
            2
        );
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn immediate_append_syncs_prior_pending_records() {
        let (writer, observation) = writer(slow_policy());
        writer.append(b"chunk\n".to_vec(), false).unwrap();
        assert_eq!(
            writer
                .append(b"done\n".to_vec(), true)
                .unwrap()
                .durable_sequence,
            2
        );
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn timer_syncs_without_another_append_and_stays_idle_afterward() {
        let (writer, observation) = writer(SyncPolicy {
            interval: Duration::from_millis(20),
            ..slow_policy()
        });
        thread::sleep(Duration::from_millis(50));
        assert_eq!(observation.lock().unwrap().syncs, 0);
        writer.append(b"chunk\n".to_vec(), false).unwrap();
        wait_until(|| writer.status().unwrap().durable_sequence == 1);
        assert_eq!(observation.lock().unwrap().syncs, 1);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn repeated_appends_do_not_postpone_the_dirty_deadline() {
        let (writer, _) = writer(SyncPolicy {
            interval: Duration::from_millis(80),
            ..slow_policy()
        });
        writer.append(b"chunk\n".to_vec(), false).unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut observed_sync = false;
        while Instant::now() < deadline {
            writer.append(b"chunk\n".to_vec(), false).unwrap();
            if writer.status().unwrap().durable_sequence > 0 {
                observed_sync = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            observed_sync,
            "continuous writes postponed the first dirty deadline"
        );
    }

    #[test]
    fn capacity_rejection_writes_nothing_and_does_not_poison() {
        let (writer, observation) = writer(SyncPolicy {
            max_total_bytes: 4,
            ..slow_policy()
        });
        writer.append(b"ab".to_vec(), false).unwrap();
        assert_eq!(
            writer.append(b"cde".to_vec(), false),
            Err(WriterError::CapacityExceeded)
        );
        assert_eq!(observation.lock().unwrap().bytes, b"ab");
        assert_eq!(writer.status().unwrap().written_sequence, 1);
        assert_eq!(
            writer
                .append(b"cd".to_vec(), true)
                .unwrap()
                .durable_sequence,
            2
        );
        assert_eq!(observation.lock().unwrap().bytes, b"abcd");
        assert_eq!(
            writer.append(b"e".to_vec(), false),
            Err(WriterError::CapacityExceeded)
        );
    }

    #[test]
    fn partial_write_failure_is_sticky_and_leaves_prior_durability_intact() {
        let (writer, observation) = writer(slow_policy());
        writer.append(b"ok".to_vec(), true).unwrap();
        observation.lock().unwrap().write_limit = Some(4);
        assert_eq!(
            writer.append(b"tail".to_vec(), false),
            Err(WriterError::Io(io::ErrorKind::Other))
        );
        assert_eq!(
            writer.shared.state.lock().unwrap().receipt,
            DurabilityReceipt {
                written_sequence: 1,
                durable_sequence: 1,
            }
        );
        assert_eq!(observation.lock().unwrap().bytes, b"okta");
        observation.lock().unwrap().write_limit = None;
        assert_eq!(
            writer.append(b"blocked".to_vec(), false),
            Err(WriterError::Io(io::ErrorKind::Other))
        );
        assert_eq!(writer.status(), Err(WriterError::Io(io::ErrorKind::Other)));
        assert_eq!(writer.sync(), Err(WriterError::Io(io::ErrorKind::Other)));
        assert_eq!(observation.lock().unwrap().bytes, b"okta");
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn sync_failure_is_sticky_and_never_acknowledged_as_durable() {
        let (writer, observation) = writer(slow_policy());
        observation.lock().unwrap().fail_sync = true;
        assert_eq!(
            writer.append(b"body\n".to_vec(), true),
            Err(WriterError::Io(io::ErrorKind::Other))
        );
        assert_eq!(
            writer.shared.state.lock().unwrap().receipt,
            DurabilityReceipt {
                written_sequence: 1,
                durable_sequence: 0,
            }
        );
        observation.lock().unwrap().fail_sync = false;
        assert_eq!(writer.status(), Err(WriterError::Io(io::ErrorKind::Other)));
        assert_eq!(
            writer.append(b"blocked\n".to_vec(), true),
            Err(WriterError::Io(io::ErrorKind::Other))
        );
        assert_eq!(writer.close(), Err(WriterError::Io(io::ErrorKind::Other)));
        assert_eq!(observation.lock().unwrap().bytes, b"body\n");
        assert_eq!(observation.lock().unwrap().syncs, 1);
    }

    #[test]
    fn timer_sync_failure_stops_subsequent_appends() {
        let (writer, observation) = writer(SyncPolicy {
            interval: Duration::from_millis(10),
            ..slow_policy()
        });
        observation.lock().unwrap().fail_sync = true;
        writer.append(b"body\n".to_vec(), false).unwrap();
        wait_until(|| observation.lock().unwrap().syncs > 0);
        assert_eq!(writer.status(), Err(WriterError::Io(io::ErrorKind::Other)));
        assert_eq!(
            writer.append(b"blocked".to_vec(), false),
            Err(WriterError::Io(io::ErrorKind::Other))
        );
        assert_eq!(observation.lock().unwrap().bytes, b"body\n");
    }

    #[test]
    fn worker_backend_panic_fails_closed() {
        let (writer, observation) = writer(SyncPolicy {
            interval: Duration::from_millis(10),
            ..slow_policy()
        });
        observation.lock().unwrap().panic_sync = true;
        writer.append(b"body\n".to_vec(), false).unwrap();
        wait_until(|| writer.status().is_err());
        assert_eq!(
            writer.append(b"blocked".to_vec(), false),
            Err(WriterError::Unavailable)
        );
        assert_eq!(writer.close(), Err(WriterError::Unavailable));
    }

    #[test]
    fn close_syncs_pending_data_and_releases_backend() {
        let (writer, observation) = writer(slow_policy());
        writer.append(b"body\n".to_vec(), false).unwrap();
        assert_eq!(
            writer.close().unwrap(),
            DurabilityReceipt {
                written_sequence: 1,
                durable_sequence: 1
            }
        );
        let observation = observation.lock().unwrap();
        assert_eq!(observation.syncs, 1);
        assert!(observation.dropped);
    }

    #[test]
    fn drop_best_effort_syncs_and_releases_backend() {
        let (writer, observation) = writer(slow_policy());
        writer.append(b"body\n".to_vec(), false).unwrap();
        drop(writer);
        let observation = observation.lock().unwrap();
        assert_eq!(observation.syncs, 1);
        assert!(observation.dropped);
    }

    #[test]
    fn errors_never_reveal_original_io_messages() {
        let error = WriterError::Io(io::ErrorKind::PermissionDenied);
        assert_eq!(
            error.to_string(),
            "transcript I/O failure: PermissionDenied"
        );
        assert!(!format!("{error:?}").contains("synthetic private"));
    }
}
