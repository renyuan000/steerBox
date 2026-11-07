# Resolved Model Allowlist Repair Plan

> **For agentic workers:** Use subagent-driven-development, with specification review before quality review. The master independently validates and commits the slice.

**Goal:** Enforce the existing Goal model allowlist before resolved candidate resolution and execution.

**Architecture:** Add narrowly scoped checks to the common resolved boundary and routed primary/fallback boundaries. Preserve empty-list behavior, existing binding checks, provider error classification and failed-run closure.

**Tech Stack:** Rust, existing synthetic adapters, JSONL audit stores; no new dependencies.

## Steps

- [x] Preserve behavior-level RED for direct, primary-resolver and fallback denial: 5 failed / 2 passed. The interrupted compile-only log is not RED evidence.
- [x] Change only `src/loop_controller.rs` and `tests/model_call_controller_audit.rs` for the repair; keep other error-closure work separate.
- [x] Verify the matrix in `docs/RESOLVED_MODEL_ALLOWLIST_CONTRACT.md`, including request/route disagreement, empty and explicitly allowed lists, and terminal audit failures. Focused 7 and full audit 68 passed.
- [x] Freeze the diff and obtain independent specification then quality/security review. Three rejection assertions were strengthened to `NoCompatibleModels`; the delta was independently approved with no remaining findings.
- [x] Master runs all tests, strict all-targets clippy, lib/bin build, fmt and diff checks with project-external target/logs: 835 passed, 0 failed/ignored; all checks exited 0.
- [ ] Update validation and delivery documents with actual results, precisely stage the slice, then verify the clean committed tree before integration.

This is a pre-commit checklist; final commit/tree and post-commit results are recorded in `/temp/codex/steerbox-phase1-resume-20261031.md`. It does not claim completion before evidence exists.
