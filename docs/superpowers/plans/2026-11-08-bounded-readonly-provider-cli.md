# Bounded read-only provider CLI

Date: 2026-11-08. Base: `034967f`. User requests immediate CLI wiring and
measured startup CPU/memory, not a second loop implementation or live calls.

## Deliverable

Add a sibling command, preserving existing command behavior:

```text
steerbox loop provider-readonly <config.json> <project_id> <goal_id>
  <loop_run_id> <task_run_id> <model_id> <objective> <observation>
  --workspace <absolute-path> --max-iterations <1..16>
  --path <relative.md>... [--credential-source env]
```

- Parse all arguments before loading configuration. Reject unknown flags,
  repeated singleton flags, missing values and invalid bounds with fixed errors.
- Require an explicit workspace and 1..256 existing authorized Markdown paths.
- Prepare the existing bounded runner once. Do not reset task or token budgets.
- Before opening stores, validate all seven store/lock paths plus the state
  projection path: distinct regular destinations outside the workspace, without
  symlink/hardlink aliases. Pure path derivation must not create files. Require
  existing runtime parents rather than create directories before this gate.
- Default credentials are unavailable; only explicit `--credential-source env`
  enables the existing environment resolver. No inline credentials or insecure
  HTTP bypass. Dispatch both supported protocols through the existing runner.
- Revalidate projection destination before persistence. Preserve execution
  errors over subsequent projection errors. Do not project after preflight fails.
- Print actual decision, stop reason, iteration count and loop status. A returned
  human/no-action outcome is not completion or a running background worker.
- Keep Unix-only scope and Git ancestor-discovery boundary from the library.

## Verification gates

- [x] TDD evidence: parser-before-config, path gates before file creation,
  credential default, both injected protocols, stopped summaries, original-error
  precedence and actual event/model/tool/state identity correlations.
- [x] Existing CLI and provider regressions remain passing.
- [x] Independent specification review, then quality/security review; master
  reads the final diff and resolves findings.
- [x] Master full tests, strict all-target clippy, lib/bin build, fmt and diff.
- [x] Release startup measurements and bounded synthetic execution/wait resource
  measurements, with workload/build mode/repeats/raw evidence reported.
- [x] Update scoped status and usage documents, including the corrected resource
  baseline. Final documentation delta remains subject to independent review.
- [ ] Commit only this functional unit after final documentation review, then
  verify and integrate locally. Actual commit/integration IDs belong in the
  external delivery ledger rather than a pre-claimed completion checkbox.

## Resource evidence boundaries

No live provider or real credentials are used. A synchronous command that exits
has no daemon-idle measurement. Report startup, active execution and controlled
provider waiting separately; synthetic waiting is not real-provider latency.
Build/test artifacts and raw measurements stay outside the source tree.
Git baseline and exact diffs provide rollback evidence; no source-copy backup
is needed for this isolated version-controlled worktree.
