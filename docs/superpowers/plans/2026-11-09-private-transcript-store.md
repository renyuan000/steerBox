# Private Transcript Store Implementation Plan

> **For agentic workers:** Use subagent-driven-development for the isolated writer task;
> master implements filesystem/content boundary and independently verifies all results.

**Goal:** A private, bounded, fresh-session transcript library with per-file timed/critical durability.

**Architecture:** `transcript_store.rs` owns public schema/content checks; `transcript_store/private_file.rs`
owns Unix exclusive fd-relative creation; `transcript_store/writer.rs` owns serialized writes and timer.
No changes to production provider/controller/CLI behavior. Base `7998508`.

**Tech Stack:** Rust std threads/Mutex/Condvar/File, existing serde/Redactor/libc, tempfile tests; no new dependency.

## Task 1: Isolated baseline and failing tests

- [x] Run existing offline focused storage suites with external target and jobs=1.
- [x] Add `tests/transcript_store.rs` using `TranscriptStore::create(root, workspace, session, policy)`,
  `append(TranscriptEntry)`, `status()`, `sync()` and `close(self)`. Example expectations:

```rust
let receipt = store.append(entry(TranscriptKind::AssistantChunk, "你好\n🦀")).unwrap();
assert_eq!(receipt.written_sequence, 1);
assert_eq!(receipt.durable_sequence, 0);
assert_eq!(store.sync().unwrap().durable_sequence, 1);
```

- [x] Run tests before implementation; distinguish absent API compile RED from genuine failing timer/fault tests.

## Task 2: Writer (independent subagent)

- [x] Implement private `writer.rs`, only file it may edit. Shared API with master:

```rust
pub struct SyncPolicy {
    pub interval: std::time::Duration,
    pub byte_threshold: u64,
    pub max_total_bytes: u64,
}
pub struct DurabilityReceipt { pub written_sequence: u64, pub durable_sequence: u64 }
// Public exports of writer types retain private writer module implementation.
// WriterError: InvalidPolicy, CapacityExceeded, Unavailable, Io(std::io::ErrorKind).
// TimedWriter::new(file, policy), append(Vec<u8>, immediate: bool),
// status(), sync(), close(self) -> Result<DurabilityReceipt, WriterError>.
// Sequence allocated by successful append; master holds store append lock through encode+write.
```

- [x] First write private fake-backend tests for dirty deadline, threshold and fault watermarks;
  prove behavioral RED, then implement. No Cargo execution in agent; master serializes it.
- [x] Timer must wake without another append; failure sticky; idle no sync, explicit close joins thread.

## Task 3: Public/private boundary (master)

- [x] Export `pub mod transcript_store` only in `src/lib.rs`.
- [x] Add typed entries, bounded IDs and body gate; accepted original bytes not normalized.
  JSON comparison uses `serde_json::Value`; recursively reject known transport configuration keys.
- [x] Unix root traversal via openat NOFOLLOW directory fds, validate private owner root,
  create_new final filename `<session>.jsonl` 0600; fstat regular/single-link, sync file+directory.
  Non-Unix returns unsupported without filesystem writes. No raw paths/content in errors/Debug.
- [x] Tests cover critical kinds, actual timer without another append, fixed deadline under repeated chunks,
  max bytes before write, sticky injected failures, malicious path classes and unchanged protected files.

## Task 4: Review, validation and delivery

- [x] Update only Phase 1 TODO/delivery and crash-recovery document to reference actual library boundary.
- [x] Independent spec review then independent quality review; master reads diffs and all evidence.
- [x] Focused `cargo test --locked --offline --test transcript_store`; full `--tests`, strict all-targets
  clippy, lib/bin build, fmt/diff. All use one external target/jobs=1, synthetic bodies only.
- [ ] Exact independently buildable commit; clean committed-tree verification, local dev fast-forward
  preserving original docs, merged verification and external durable delivery receipt. No push.

Evidence directory `/temp/codex/steerbox-validation/transcript-store-20261109/`.
Rollback basis is exact Git hunks/commit. Do not copy whole source, modify global memory,
read real secrets, change old audit schema or claim automatic production transcript recording.
