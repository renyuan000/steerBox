# Bounded read-only provider loop implementation plan

> Use subagent-driven-development: a single implementer, specification review,
> then quality/security review. Master verifies the frozen diff independently.

Goal: deliver the approved model-driven read-only runner by reusing the existing
loop; do not construct another fixture-only command or a new orchestration core.
Base: `d103e92`. Contract: `BOUNDED_READONLY_PROVIDER_LOOP_CONTRACT.md`.

## Files and ownership

- Implementer: `src/provider_runner.rs`, new focused child module
  `src/provider_runner/readonly.rs`, `tests/readonly_provider_loop.rs`.
  Direct prerequisite fixes in `src/loop_controller.rs` are also included:
  semantic read-only verification failures and the two swallowed final
  human/no-action progress errors, identified by the preflight audit.
- Master: contract, this plan, narrow Phase 1 status/validation/TODO updates.
- Do not touch `main.rs` in this independently buildable library increment.
  Standalone CLI wiring follows after the runner is accepted.
- Preserve main-worktree future documents, user reports and benchmarks.

## Task 1: Preparation and frozen scope

- [x] Run the existing foreground/model-call/Markdown regression baseline.
- [x] Add the new configuration/factory tests and observe missing-API RED.
- [x] Add the focused child module and re-export its types. Extend the private
  shared prompt preparation with an optional read-only context; existing caller
  branches and signatures stay behaviorally unchanged.
- [x] Reject invalid iterations/workspace/files and freeze the caller's scope.
  Supply model-visible schema/tool/context descriptions without auth/endpoint.

API shape:

```rust
pub struct BoundedReadOnlyRunConfig {
    pub workspace: std::path::PathBuf,
    pub allowed_paths: Vec<String>,
    pub max_iterations: u32,
}
// ForegroundProviderRun::prepare_readonly(&config, request, run)
//   -> Result<BoundedReadOnlyProviderRun, ProviderRunError>
```

## Task 2: Two-protocol bounded execution

- [x] Write two-protocol two-response/action/verifier tests and failure matrices.
- [x] Bind only authorized actions/verifiers in the adapter; construct one
  Goal/controller/runtime and invoke `run_resolved_model_loop` once. Enable the
  frozen token budget before any call; no reinitialization per iteration.
- [x] Enable existing strategy-aware tool accounting without changing the
  authorized cap. Assert real TaskRun tool counters, not only record counts.
- [x] Recheck frozen scope after each response; RED/GREEN semantic verifier
  failure and final human/no-action progress errors before fixing their narrow
  shared-controller exits. Keep ordinary executor output contracts unchanged.
- [x] Add distinct OpenAI/Anthropic transport entries plus HTTP resolver dispatch.
  The outer boundary must reject protocol/history/path failures before resolving
  credentials, preserve failure evidence, and close active errors state-first.
- [x] Verify actual call/tool counts, stopped outcomes, error identity, immutable
  files, scope/data-store boundaries, cumulative limits and outbound redaction.

## Task 3: Review and independent delivery

- [x] Rebuild StateStore/ExecutionQuery from the new runner's real events and
  assert correlated identities, steps and complete/stopped/failure projections.
  Initial specification review requested this test-evidence addition; it is
  not state-file persistence or durable recovery.
- [x] Freeze code hashes; specification review before quality/security review.
- [x] Master reads all diffs, checks contract coverage, runs full tests, strict
  all-targets clippy, lib/bin build, fmt and diff checks.
- [x] Review narrow status/TODO/validation updates and prepare the exact owned
  file list; independent specification and quality/security reviews pass.

Post-commit delivery evidence belongs in the existing
`/temp/codex/steerbox-phase1-resume-20261031.md` ledger: exact commit, clean-tree
tests/build, fast-forward into dev and the remaining CLI/live boundary. Record
those outcomes only after execution, not as pre-commit completion claims.

Commands (jobs=1, one Cargo writer at a time):

```bash
export CARGO_BUILD_JOBS=1
export CARGO_TARGET_DIR=/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target
cargo test --locked --offline --test readonly_provider_loop
cargo test --locked --offline --tests --quiet -- --test-threads=1
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --offline --lib --bin steerbox
cargo fmt --all -- --check
git diff --check
```

Keep baseline/RED/GREEN/master/committed logs under
`/temp/codex/steerbox-validation/readonly-provider-loop-20261108/`.
Do not call real providers, read real credentials, push, or remove old worktrees.
