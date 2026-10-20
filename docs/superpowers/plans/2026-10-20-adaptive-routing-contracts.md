# Bounded Adaptive Routing Contracts Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with tests before implementation.

**Goal:** Add secret-free, versioned contracts for effect levels, deterministic quality assessment, and bounded adaptation decisions without yet invoking live providers or changing endpoint/credential behavior.

**Architecture:** Keep assessment and adaptation pure. The contracts receive host-observable quality signals, enforce monotonic one-level upgrades, and reject attempts to alter security boundaries. Later controller work will consume these contracts and create new Attempt/RouteDecision records.

**Tech Stack:** Rust, serde, thiserror, Cargo integration tests.

### Task 1: Define the contracts

**Files:**
- Create: `src/adaptive_routing.rs`
- Modify: `src/lib.rs`
- Test: `tests/adaptive_routing.rs`

- [x] Write tests for effect-level ordering, quality-state serialization, one-level upgrade, budget/cooldown blocking, and security-boundary immutability.
- [x] Run `CARGO_TARGET_DIR=/tmp/steerbox-adaptive-target cargo test --locked --offline --test adaptive_routing`; the first run failed because the module/types did not exist.
- [x] Implement only pure contracts: `EffectLevel`, `QualityStatus`, `QualityAssessment`, `AdaptationDecision`, bounded upgrade helper, and secret-free validation.
- [x] Re-run the focused test target; 7 tests passed.
- [x] Run `cargo fmt --all -- --check`, clippy for the library/test target, and `git diff --check`.
- [ ] Commit the code/test/plan slice only after reviewing the staged diff.

### Task 2: Integrate with routing/controller

**Files:**
- Modify: `src/model_router.rs`, `src/loop_controller.rs`, `src/model_contracts.rs`
- Test: `tests/loop_model_routing.rs`, `tests/model_router.rs`

- [ ] Add current-task quality signals without changing endpoint/auth resolution.
- [ ] Create a child Attempt and new RouteDecision only for an allowed retryable adaptation.
- [ ] Add budget, maximum-attempt, and cooldown guards with metadata-only audit events.
- [ ] Run focused tests and review for no silent fallback or security-boundary changes.

### Task 3: Provider and UI integration

**Files:**
- Modify: `src/provider_runner.rs`, `src/stream_reporter.rs`, `src/main.rs`
- Test: focused provider/stream/runtime CLI tests

- [ ] Connect deterministic quality outcomes to compact progress events.
- [ ] Verify low-risk upgrades do not prompt repeatedly and that blocked decisions remain short and auditable.
- [ ] Defer live-provider validation until the user supplies an isolated test account.
