# Bounded Workspace Snapshot Implementation Plan

> For agentic workers: execute task-by-task with independent tests and master review. This implements the existing Phase 1 recovery direction; it does not add automatic rollback or a new model/tool authorization path.

**Goal:** Preserve one existing UTF-8 Markdown file in a private local store and explicitly restore it only when the current workspace/goal and expected current hash match.

**Architecture:** Separate private snapshot contents from serializable metadata. Reuse the existing scoped Markdown editor for the final conditional replacement; never use Git reset/restore, replay tools, or expose a restore tool to models. Store snapshots outside the workspace, with bounded reads and immutable IDs.

**Tech Stack:** Rust std filesystem, existing serde/sha2/libc; tempfile only in tests. No new dependency, network or real credential access.

## Scope and design choices

- Hash-only checkpoint metadata cannot restore bytes; Git/worktree-wide restoration could overwrite unrelated changes. Use a single-file local snapshot instead.
- Capture/restore only existing regular `.md`/`.markdown` files within current GoalContract scope, with no symlink components; reject traversal, control characters and oversized input (1 MiB).
- `SnapshotMetadata` exposes schema version, snapshot/project/goal IDs, relative path, before hash and byte count. Private records additionally contain canonical workspace binding and original text; they have no body-bearing Debug or error formatting.
- Snapshot hashes are `sha256:<SHA256(file bytes)>`. They are not the controller checkpoint's `SHA256(relative_path + NUL + file bytes)` and must not be interchanged.
- Bound the encoded private JSON record to `6 * 1 MiB + 64 KiB` (JSON escaping plus bounded metadata). Check effective-user ownership and owner-only store/record permissions; reject hardlinked source/record files.
- Store root is outside the workspace. New Unix root/file permissions are 0700/0600; existing non-private stores are rejected rather than silently chmod'ed. Equivalent Windows ACL isolation is not implemented: fail closed on non-Unix platforms.
- Snapshot IDs are 1–80 ASCII letters/digits/`-`/`_`; create-new prevents overwrite. Sync the record and store directory before returning successful capture. Partial/corrupt records are rejected on load, not interpreted as usable checkpoints.
- Restore requires explicit trusted-caller approval, matching project/goal/workspace, revalidated scope/path, a valid snapshot hash/size and `expected_after_hash` equal to the current file hash. Changes made after that expected boundary are not overwritten.
- Preserve exact bytes and current target Unix permission mode, not inode/owner/ACL/xattrs. Atomic replacement and sync do not constitute an adversarial multi-process lock or complete crash/recovery transaction. Errors after replacement may require inspection.
- Local filesystem permissions are not encryption or protection against the same OS user/root. The caller must classify/authorize data before capture; snapshots never automatically enter prompts, EventLog, trace, summaries or model-call stores.
- This primitive does not automatically capture controller writes, restore failed verifications, resume TaskRun/Attempt/budgets, restore processes or compensate external side effects. Checkpoint metadata queries remain read-only.

## Task 1: independent failing tests

Files: create `tests/workspace_snapshot.rs` (test agent only).

- [x] Test the production API below with tempfile workspace and sibling snapshot directory: CRLF capture/reopen/restore, metadata body exclusion, explicit approval, current-hash conflict, project/goal/workspace mismatch, corrupt record, invalid IDs/paths/scope, missing/nonregular/symlink files, size limits and Unix privacy/permission preservation.
- [x] Run `CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --locked --offline --test workspace_snapshot`; record the absent API/RED result before implementation.

```rust
let store = WorkspaceSnapshotStore::open(snapshot_dir);
let metadata = store.capture(&workspace, &goal, "README.md", "snapshot-1")?;
// expected_after_hash is supplied by the trusted caller from the known edit.
let result = store.restore(&workspace, &goal, "snapshot-1", &expected_after_hash, true)?;
```

## Task 2: bounded primitive

Files: create `src/workspace_snapshot.rs`; modify `src/lib.rs` and scoped helpers in `src/workspace_edit.rs` (master only).

- [x] Implement schema-v1 metadata and private record validation; safe ID/store/path checks; bounded no-follow file reads; private create-new record persistence and sync.
- [x] Implement restore with approval/binding/hash checks and exact conditional replacement. Extend only the shared atomic editor's permission preservation/private temporary file and sync required by restoration.
- [x] Run focused snapshot/edit tests; all must pass, with no changes to real project files during runtime tests.

## Task 3: review, documentation and validation

Files: `docs/PHASE_1_IMPLEMENTATION_SCOPE.md`, `docs/PHASE_1_TODO.md`, `docs/PHASE_1_VALIDATION_PLAN.md`, `docs/PHASE_1_DELIVERY_STATUS.md` and this plan.

- [x] Independently review implementation and tests for private body leakage, trust/permission boundaries, bounded reads, exact user-change rejection and honest race/crash guarantees; master verifies each finding.
- [x] Record actual implemented scope without marking full checkpoint/controller restoration complete.
- [x] Run locked/offline tests, strict lib/bin/tests clippy, lib/bin build, fmt and diff checks with `/tmp/steerbox-phase1-target`, jobs=2.

Commit gate: select only the reviewed slice files; exclude existing historical reviews/reference/benchmark files. Commit identity/status is recorded by Git, not predicted here. Do not push.

## Execution evidence (2026-10-14)

- Before implementation, both the independent test author and master observed exit 101 / E0432: `steerbox::workspace_snapshot` did not exist. This was a compile-time RED, not executed failing assertions.
- Final master focused run: snapshot 42 passed, editor 11 passed (53 total). The root-only foreign-owner fixture actually executed with UID 0 and changed ownership only within its own TempDir.
- Master `cargo test --locked --offline`: 563 passed, 0 failed, 0 ignored; log `/tmp/steerbox-phase1-snapshot-full-20261014.log`. Tests use local/fake/localhost fixtures, not live model validation.
- Master strict `cargo clippy --locked --offline --lib --bins --tests -- -D warnings`, `cargo build --locked --offline --lib --bins`, `cargo fmt --all --check` and `git diff --check` exited 0. Initial new-test Clippy cast/fold warnings were fixed and the complete strict check rerun.
- Snapshot/body isolation, scope/binding and bounded persistence were independently reviewed; master inspected the actual code/test diffs. Non-Unix ACL isolation and post-replacement sync fault injection were not runtime-validated. Controller snapshot wiring and complete Phase 1 remain open.
