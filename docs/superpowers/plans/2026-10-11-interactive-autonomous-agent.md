# Interactive + Autonomous Agent Vertical Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 steerBox 从“可审计的本地/fake harness”推进到一个有实时反馈、真实 provider 边界、受控动作闭环和可恢复长任务的双模式 Agent，同时保持 EventLog、预算、PolicyGate、Attempt lineage 和审计护城河。

**Architecture:** 保留 shared harness core；Interactive 模式只改变反馈和用户控制入口，不绕过 PolicyGate、预算或事件事实源。Autonomous 模式复用同一 LoopController、ModelCallStore、Attempt/TaskRun 和恢复协议；真实 provider 通过 provider-specific adapter/transport 接入，Anthropic 不伪装成 OpenAI-compatible URL。

**Tech Stack:** Rust 2021、Cargo、JSONL EventLog/Trace/SideEffect、serde、reqwest blocking + rustls（non-stream first）、本地 localhost 集成测试；构建产物统一写入 `/tmp/steerbox-phase1-target`。

---

## Evidence rules

- 不再使用未经测量的“59%/92%”“300ms/500ms”或“某产品没有某能力”作为完成依据。
- 每项能力必须有：代码路径、focused test、全量回归、失败边界和文档状态。
- 真实 provider 验收只允许使用用户明确授权的 `auth_ref`；测试不读取或打印 credential 正文。
- “流式”只有在 transport 逐增量交付时才可使用；完整 body 解析不得命名为实时 streaming。

## Task 1: Secret-free provider profile binding

**Files:**
- Modify: `src/runtime_config.rs`
- Modify: `src/provider_model_adapter.rs`
- Test: `tests/config_provider_binding.rs`
- Docs: `docs/PHASE_1_TODO.md`, `docs/PHASE_1_DELIVERY_STATUS.md`

- [ ] Write tests proving a valid model profile creates an adapter using only provider, endpoint, model and opaque `auth_ref`; unknown model fails before transport construction; config serialization contains no credential material.
- [ ] Run `CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --locked --test config_provider_binding` and confirm the new tests fail before implementation.
- [ ] Implement `RuntimeConfig::build_http_provider_adapter(model_id, resolver)` without reading environment variables or invoking the resolver during construction.
- [ ] Run the focused test, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo fmt --all -- --check`.

## Task 2: Measured interactive reporting

**Files:**
- Modify: `src/stream_reporter.rs`
- Modify: `src/loop_controller.rs`
- Modify: `src/main.rs`
- Test: `tests/stream_reporter.rs`, `tests/loop_controller.rs`

- [ ] Add event-to-report mapping for loop start, step start, tool request, policy wait, model call outcome, verification and terminal state; report metadata only and never prompt/credential bodies.
- [ ] Add a deterministic buffered writer test asserting ordering and that a long event payload is not duplicated into user output.
- [ ] Integrate the reporter at safe controller boundaries without changing EventLog as the source of truth.
- [ ] Verify with `cargo test --locked --test stream_reporter --test loop_controller` and full `cargo test --locked --all-targets --all-features --quiet`.

## Task 3: Interactive command polling and permission boundary

**Files:**
- Modify: `src/user_command_inbox.rs`
- Modify: `src/loop_controller.rs`
- Modify: `src/policy_gate.rs`
- Modify: `src/main.rs`
- Test: `tests/user_command_inbox.rs`, `tests/loop_controller.rs`, `tests/tool_policy.rs`

- [ ] Define a non-blocking poll API that returns complete lines or no input and never consumes partial bytes as a command.
- [ ] Apply pause/resume/cancel/takeover only at declared safe boundaries; preserve `task_run` budget, Attempt lineage and side-effect-uncertain status.
- [ ] Add an approval prompt adapter that emits a policy event, waits for an explicit decision, and defaults to denial on EOF/invalid input.
- [ ] Verify that a denied or interrupted action performs zero side effects and that repeated `command_id` remains idempotent.

## Task 4: Controlled model action loop

**Files:**
- Modify: `src/model_contracts.rs`
- Modify: `src/model_adapter.rs`
- Modify: `src/loop_controller.rs`
- Modify: `src/tool_executor.rs`
- Test: `tests/tool_policy.rs`, `tests/loop_controller.rs`, `tests/dev_vertical_slice.rs`

- [ ] Define a versioned action envelope containing tool id, validated arguments, requested path scope and expected verification contract; unknown fields and free-form shell strings fail closed.
- [ ] Parse model action output separately from decision/summary; do not allow `complete` to bypass verification.
- [ ] Route every action through PolicyGate, ToolRegistry, workspace boundary, budget consumption and side-effect ledger before execution.
- [ ] Return verification evidence to the next model step and test denied, out-of-scope, malformed, execution-failed and verification-failed paths.

## Task 5: Real provider capability slices

**Files:**
- Modify: `src/provider_model_adapter.rs`
- Create: `src/anthropic_adapter.rs`
- Test: `tests/provider_model_adapter.rs`, `tests/anthropic_adapter.rs`
- Docs: `docs/PHASE_1_IMPLEMENTATION_SCOPE.md`

- [ ] Keep OpenAI/DeepSeek on the OpenAI-compatible contract only after provider capability validation.
- [ ] Implement Anthropic as a separate protocol adapter with its own auth header, request envelope, response envelope and non-stream parser; do not substitute a URL.
- [ ] Add localhost protocol fixtures for success, malformed response, status classification and secret-free errors; no live key is required for CI.
- [ ] Do not enable fallback until Task 6 proves new Attempt creation, parent evidence retention and total TaskRun budget enforcement.

## Task 6: Recovery, fallback and measured comparison

**Files:**
- Modify: `src/domain.rs`, `src/loop_controller.rs`, `src/state_store.rs`, `src/event_log.rs`
- Test: `tests/state_store.rs`, `tests/loop_controller.rs`, `tests/task_attempt_lineage.rs`
- Docs: `docs/PHASE_1_DELIVERY_STATUS.md`, `docs/STEERBOX_VS_CODEX_VS_CLAUDE_COMPARISON.md`

- [ ] Persist TaskRun/Attempt transitions and restore controller state from EventLog/StateStore without resetting budgets or replaying uncertain side effects.
- [ ] Implement fallback as a new Attempt with `parent_attempt_id`, preserved failure evidence, a new route/prompt resolution and the same total TaskRun budget ceiling.
- [ ] Add crash/reopen, duplicate command, version conflict and side-effect-uncertain tests.
- [ ] Replace subjective comparison scores with a benchmark table containing command, environment, sample count, latency percentiles, first-visible-event time, cancellation delay and correctness/evidence results.

## Checkpoints

### Checkpoint A: provider + audit

- [ ] Task 1 complete; fake/localhost provider path is constructible from config metadata only.
- [ ] Existing full test, clippy, build, fmt and diff gates pass.

### Checkpoint B: interactive controlled loop

- [ ] Tasks 2–4 complete; a local fixture run visibly reports progress, accepts safe-boundary commands, and cannot bypass policy or verification.

### Checkpoint C: first restricted real-model Agent

- [ ] Tasks 1–5 complete; one authorized low-risk provider call, one controlled action and one verification path are auditable end-to-end.
- [ ] Automatic fallback remains disabled unless Task 6 Attempt/recovery gates pass.

### Checkpoint D: complete Phase 1

- [ ] Task 6 complete; cross-process recovery and memory/wiki scope gates are separately verified.
- [ ] No document claims complete Phase 1 without the corresponding command output and artifact references.

## Known gaps intentionally outside this plan

- No distributed workers, multi-agent scheduler, TUI/Web frontend, vector/graph index or global automatic memory promotion.
- No claim that steerBox is faster or more capable than Claude Code/Codex until the benchmark in Task 6 is executed.
