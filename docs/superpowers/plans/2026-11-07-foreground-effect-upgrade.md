# Foreground Effect Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development with spec review before quality review. The master validates and commits the reviewed slice.

**Goal:** Wire the existing one-adjacent-level controller into an explicit pure-observation foreground runner without changing one-shot behavior.

**Architecture:** A new prepared runner freezes validated configuration and uses the existing effect candidate factory. A private shared transport forwards both requests to the same client. The existing controller owns quality, Attempt lineage, count/token limits and terminal decisions.

**Tech Stack:** Rust, existing serde/HTTP adapters and JSONL audit stores; no new dependencies.

## Task 1: Reproduce the missing runner seam

- [x] Inspect `src/provider_runner.rs`, existing effect candidate/controller APIs and retained synthetic tests.
- [x] Preserve the existing baseline log (25 tests passed on 2026-11-02); recheck baseline after the interruption.
- [x] Preserve the valid missing-API compile-RED in `implementer-api-red.log`. The earlier `implementer-red.log` failed because the fixture used an invalid auth reference, not because the target behavior was absent. The interrupted `red.log` contains no complete result; no behavior-level RED is claimed.

## Task 2: Implement the frozen contract

Files: `src/provider_runner.rs`, `tests/provider_foreground_upgrade.rs`. Full behavioral requirements and exclusions are in `docs/FOREGROUND_EFFECT_UPGRADE_CONTRACT.md`.

- [x] Add `ForegroundProviderRun::prepare_upgrade(config, request)` and the independent prepared type; reject absent opt-in/protocol mismatch before execution.
- [x] Add transport/resolver execution methods with a mandatory trusted evaluator. Reuse the effect factory and exact child route prompt resolution; keep all other authorization fields frozen.
- [x] Preconfigure the existing two-call count budget and reported-token ledger; retain identity guard ordering and one-shot behavior.
- [x] Add the direct seam cases listed in the contract and run focused tests with `--locked --offline`, `CARGO_BUILD_JOBS=1` and project-external `CARGO_TARGET_DIR`.
- [x] Freeze implementation and fingerprints. Spec review first, then quality/security review; resolve findings before master acceptance. The opt-in fixture gate was corrected and independently re-reviewed; redundant configuration parsing remains a non-blocking optimization suggestion.

## Task 3: Verify and deliver

This checklist is the pre-commit review snapshot. The last two gates run after this document is frozen into the delivery commit; their actual commit/tree, post-commit verification and integration results belong in `/temp/codex/steerbox-phase1-resume-20261031.md`, not a premature completion claim here.

- [x] Master reads the actual diff and runs all tests, strict all-targets clippy, lib/bin build, fmt and diff checks. Final result: 68 harnesses, 829 passed, 0 failed/ignored; project-external `master-final-*` logs preserve the evidence.
- [x] Update directly affected README, TODO, adaptive policy and validation evidence, preserving open Phase 1 boundaries.
- [ ] Stage only reviewed files, commit a buildable slice, recheck the committed tree and fast-forward into dev. Do not push or delete user reports/old worktrees.
- [ ] Append the resulting commit, verification logs and remaining work to the persistent project ledger under `/temp/codex/`.
