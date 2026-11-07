# Resolved Routed Failure Closure Implementation Plan

> **For agentic workers:** Use subagent-driven-development, with specification
> review before quality review. The master independently checks the final diff,
> executes verification and precisely commits the completed slice.

**Goal:** Close active routed runs when primary/fallback candidate resolution or
binding validation fails, without losing evidence or attempting another send.

**Architecture:** Reuse `close_failed_routed_run()` at the four candidate failure
exits. Preserve preflight rejection, provider fallback policy and common resolved
call behavior. Do not add recovery machinery or dependencies.

**Tech Stack:** Rust; existing synthetic transports, JSONL audit stores and
`tests/model_call_controller_audit.rs`.

## Files and ownership

- Implementer: `src/loop_controller.rs`, `tests/model_call_controller_audit.rs`.
- Master: this plan, `RESOLVED_ROUTED_FAILURE_CLOSURE_CONTRACT.md`, narrow status
  corrections in the existing allowlist contract and Phase 1 status/TODO/validation.
- Preserve the main checkout's uncommitted future-requirement documents and user
  reports; they are outside this commit.

## Execution checklist

- [x] Run the existing routed audit regressions at baseline `2312381`; 4 passed
  (`baseline.log`). This is a focused baseline, not a full-suite baseline.
- [x] Add failing tests for the matrix in the companion closure contract; update
  the existing fallback-binding test's incorrect Running expectation. Confirm
  behavior-level RED, not a compiler failure.
- [x] At each primary/fallback resolution and validation error, invoke the
  existing failure closure before returning the original error. Self-review each
  hunk for unchanged success and preflight behavior.
- [x] Run focused tests (8 passed), then the complete model-call audit target
  (72 passed), including successful fallback. Actual RED: 5 failed / 3 passed;
  an earlier binding-error assertion was corrected before the final RED run.
- [x] Independently review specification compliance, then code quality/security;
  fix findings and repeat the affected review before acceptance.
- [x] Run full tests, strict all-targets clippy, lib/bin build, fmt and diff checks
  (68 harnesses, 839 passed, zero failed/ignored; all other checks passed).
- [ ] Record only observed results; stage this slice's explicit file list, commit,
  verify the committed tree and fast-forward the main checkout without touching
  unrelated uncommitted files. No push in this slice.

## Reproducible commands

Run from the isolated worktree. Use only synthetic fixtures; no live credentials.

```bash
export CARGO_BUILD_JOBS=1
export CARGO_TARGET_DIR=/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target
cargo test --locked --offline --test model_call_controller_audit
cargo test --locked --offline --tests
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --offline --lib --bin steerbox
cargo fmt --all -- --check
git diff --check
```

Logs: `/temp/codex/steerbox-validation/routed-resolution-close-20261108/`.
Do not run simultaneous writers to the shared target directory. Checkboxes are
updated only after actual evidence; final commit/merge evidence belongs in the
persistent Phase 1 handoff ledger, not a preemptive completion claim.
