# Ordinary foreground failure closure plan

Base: `6cb7684`. Contract: `docs/FOREGROUND_FAILURE_CLOSURE_CONTRACT.md`.
Use subagent-driven-development with sequential specification and quality
reviews, then independent master verification. No live provider calls.

## Ownership and scope

Implementation: `src/provider_runner.rs`, the smallest closure extraction in
`src/loop_controller.rs`, existing foreground tests and private state tests.
Master: contract, this plan, narrow Phase 1 status/TODO/validation updates.
Do not include the main checkout's future-requirement documents or user reports.

## Checkpoints

- [x] Baseline existing foreground tests (13 passed); observe behavior RED
  (12 passed, 10 failed, caused by missing terminal events).
- [x] Implement the two runner wrappers and state-first failure closure.
- [x] Verify error identity, terminal events, budgets, audit/progress faults,
  no-resend and unchanged successful/fallback/non-Running behavior.
- [x] Specification review, followed by quality/safety review; both passed
  without unresolved findings.
- [x] Master reads diff and runs full tests (68 harnesses, 852 passed, zero
  failed/ignored), strict clippy, lib/bin build, formatting and diff checks
  against the frozen implementation; all commands exited successfully.
- [ ] Update observed status, precisely commit, verify the clean committed
  tree, and fast-forward into dev without changing unrelated files.

## Verification environment

`CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`.
One Cargo writer at a time. Logs:
`/temp/codex/steerbox-validation/foreground-failure-close-20261108/`.

```bash
cargo test --locked --offline --test provider_foreground_runner
cargo test --locked --offline --lib
cargo test --locked --offline --tests --quiet -- --test-threads=1
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --offline --lib --bin steerbox
cargo fmt --all -- --check
git diff --check
```

Recheck tests/build on the committed tree. Scoped regression results (128 passed)
are included in the full 852; do not add them twice. `green.log` is a filtered
run; `regressions.log` contains the full selected targets. Full master evidence
is in `master-{tests,clippy,build,fmt}.log`.

Constructor startup-progress failure, durable recovery, live provider acceptance
and the rest of Phase 1 stay explicitly open.
