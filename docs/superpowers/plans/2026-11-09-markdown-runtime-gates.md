# Bounded Markdown runtime gates implementation plan

> **For agentic workers:** Use executing-plans and test-driven-development; independent specification and quality reviews precede delivery.

**Goal:** Give the existing single-action Markdown runner a non-persisting pre-open path check, frozen workspace identity checks, and the existing dual-protocol resolver dispatch.

**Architecture:** Keep controller policy/checkpoint/ledger/edit/verification as the only mutation path. Add a focused `provider_runner/markdown_paths.rs` helper and public `MarkdownRuntimePathCheck::revalidate`. Check the actual ten store/lock paths plus optional state, and authorized existing single-link Markdown files. Freeze canonical workspace and Unix device/inode at prepare; check again before opening tool stores, before provider access and after its response before accepting a model action. No global approval changes or automatic recovery.

**Tech Stack:** Rust, existing adapters/transports, tempfile and synthetic injected provider responses. Base `ee7d40a`.

## Scope and compatibility

- The existing trusted library may co-locate audit files inside its workspace if they do not alias editable files or one another; this increment does not silently impose whole-workspace runtime isolation. Future CLI must make its stronger boundary explicit.
- Workspace/files must exist, be normal UTF-8 Markdown paths, 1..256 distinct paths, no parent traversal, symlink or multiple hardlinks. Non-Unix identity protection fails closed; cross-platform support is not asserted.
- Paths: events, checkpoint, trace, side-effect, tool calls and tool lock, four model-call success/failure/lock destinations; optional state is eleventh. No audit-store lock is invented.
- Pre-open checks read metadata only and reserve nothing; no resolver, network, content read or store open. They do not prove writability or history validity, freeze file content, provide atomic OS isolation, authorize a write or resume a TaskRun. Existing history rejection remains in execution.
- No CLI, multi-step writing, Bash, fallback, new credential sources, snapshots/rollback, live account access or full Phase 1 claim in this independently buildable increment.

## Tasks and files

- [x] Confirm isolated baseline with full offline tests and one external Cargo target/jobs=1.
- [x] Add `tests/provider_markdown_gates.rs` using existing APIs first. Real regressions must show provider-before-path rejection and workspace replacement can reach old behavior; preserve legitimate dual-protocol writes as controls. Record actual RED, not an absent-method compilation error.
- [x] Add private `markdown_paths` module, `MarkdownRuntimePathCheck` and frozen workspace identity. Public API:

```rust
pub fn validate_runtime_paths(
    &self, events: &std::path::Path, model_calls: &std::path::Path,
    projection: Option<&std::path::Path>,
) -> Result<MarkdownRuntimePathCheck, ProviderRunError>;
```

- [x] Modify only Markdown preparation, its adapter and runner in `src/provider_runner.rs`; reuse pure `ModelCallStore::storage_paths_for` and existing path helpers. Replace the old opened-store-only validator with the new check; keep unrelated observation/upgrade/readonly runtime unchanged.
- [x] Add `run_with_resolver<R: CredentialResolver,S: ProgressSink + 'static>` matching readonly protocol dispatch. Validate frozen scope and history before client construction; keep production HTTPS and no-retry semantics. Synthetic rejecting resolver tests cover both protocols and zero-token zero resolution.
- [x] Extend tests for all ten/eleven destinations, file aliases and missing parents, scope constraints, frozen checks before execution and after response, valid replacement-in-place, no partial tool effects and unchanged credential data boundary. Independently inspect event/model/tool audit evidence; refusal before store open creates nothing.
- [x] Add `docs/BOUNDED_MARKDOWN_RUNTIME_GATES.md`, update only `PHASE_1_TODO.md`, `PHASE_1_DELIVERY_STATUS.md`, `PHASE_1_VALIDATION_PLAN.md` with actual results and remaining CLI/live gaps.
- [x] Independent spec/quality review and master direct diff review; full offline tests, strict all-target clippy, lib/bin build, fmt/diff on frozen source.
- [ ] Exact functional commit, clean committed-tree validation, safe local dev fast-forward and revalidation; no push or cleanup of existing worktrees. Record actual delivery externally after execution.

## Verification and rollback

Evidence: `/temp/codex/steerbox-validation/markdown-runtime-gates-20261108/`.
Target: `/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`.
Run `cargo test --locked --offline --test provider_markdown_gates --test provider_markdown_runner -- --test-threads=1`, then `cargo test --locked --offline --tests --quiet -- --test-threads=1`, `cargo clippy --locked --offline --all-targets -- -D warnings`, `cargo build --locked --offline --lib --bins`, `cargo fmt --all -- --check`, `git diff --check`.
Tracked baseline and exact hunks provide rollback without whole-source copies. Preserve user's unrelated main-worktree changes and all real credentials/configuration.
