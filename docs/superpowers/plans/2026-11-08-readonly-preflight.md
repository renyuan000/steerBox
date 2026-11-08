# Offline read-only preflight implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to execute these gates; request independent review before delivery.

**Goal:** Expose the existing non-persisting, non-egress preparation and runtime-path checks without starting a run. These checks read configuration, filesystem metadata and the clock; they are not I/O-free.

**Architecture:** Add `loop preflight-readonly` beside `loop provider-readonly`, accepting exactly the same arguments and using its parser, `prepare_readonly`, and eight-destination `validate_runtime_paths`. Return a fixed local-check summary. Never open stores, construct HTTP clients, resolve credentials, send requests, execute tools, project state or reserve run identities.

**Tech Stack:** Rust, existing CLI/runtime modules, synthetic subprocess tests with tempfile.

Base: `e2ff59f`. This is the offline part of delivery package 4, not live acceptance or full Phase 1 completion. Existing credential syntax may be parsed but is not resolved; permissions/writability, transport/TLS, historical identity conflicts and actual Markdown semantics remain unchecked. The run command rechecks its own gates; preflight is not a permit or race-proof isolation.

## Files

- Modify `src/main.rs`: sibling command and usage only.
- Modify `src/provider_readonly_cli.rs`: non-persisting `preflight` wrapper; existing run behavior stays unchanged.
- Create `tests/readonly_preflight_cli.rs`: real subprocess checks and synthetic no-write/no-network fixtures.
- Update `docs/BOUNDED_READONLY_PROVIDER_CLI.md`, `docs/PHASE_1_TODO.md`, `docs/PHASE_1_DELIVERY_STATUS.md`, `docs/PHASE_1_VALIDATION_PLAN.md`: narrow usage and measured evidence.

## Execution gates

- [x] Baseline full tests on isolated tree, with one Cargo writer and external target.
- [x] RED: invoke `loop preflight-readonly` with a valid synthetic profile and initialized empty runtime directory; assert exit 0, exact fixed summary and byte-for-byte filesystem snapshot unchanged. Before command wiring it must fail because the command is absent.
- [x] Add regressions: two protocols, explicit env source with synthetic invalid/missing env, loopback listener accepting no connection, missing runtime parent, all eight directory/symlink/hardlink destinations, alias/workspace collisions, invalid scope/model/capability/auth metadata, malformed arguments before config loading, existing corrupt logs unchanged and explicitly not checked.
- [x] GREEN: add the following function in the existing CLI module and the sibling match arm in `main.rs`:

```rust
pub fn preflight(args: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let args = parse(args)?;
    let config = RuntimeConfig::load(args.config_path)?;
    let prepared = ForegroundProviderRun::prepare_readonly(&config, args.request, args.run)?;
    prepared.validate_runtime_paths(
        &config.events_path(),
        &config.model_calls_path(),
        Some(&config.state_path()),
    )?;
    Ok("preflight=passed scope=readonly credentials=not_checked transport=not_checked history=not_checked".into())
}
```

```rust
(Some("loop"), Some("preflight-readonly")) => {
    println!("{}", provider_readonly_cli::preflight(args)?);
}
```

- [x] Run `cargo test --locked --offline --test readonly_preflight_cli --test provider_readonly_cli -- --test-threads=1`; focused green and existing run regressions must pass. Independently review source call graph for absence of resolver/client/store-open/tool/Git calls, because exit 0 alone does not prove absence of all side effects.
- [x] Update scoped docs, marking only local preparation checks implemented; retain live, recovery and foreground/background control gaps.
- [x] Independent review, master diff review, full `cargo test --locked --offline --tests --quiet -- --test-threads=1`, strict all-targets clippy, lib/bin build, fmt and diff checks.
- [ ] Exact functional commit after all gates; verify committed tree, locally fast-forward dev only if still safe, then reverify. Keep user's unrelated files and new context-design discussion unchanged. No fetch/push or worktree cleanup.

## Evidence and rollback

Use `/temp/codex/steerbox-validation/readonly-preflight-20261108/` for actual red/green/full logs and `/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target` for build output, jobs=1. Synthetic data only. Git baseline plus exact hunks provide rollback; no source copies or full-file backups are needed. Actual receipt goes into the existing durable task ledger only after execution.
