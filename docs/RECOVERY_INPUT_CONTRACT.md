# Protected recovery inputs (RI1)

Status: implemented host-library slice, 2026-11-23; final delivery evidence is
recorded in `PHASE_1_VALIDATION_PLAN.md`. This slice is not automatic task
recovery and does not complete Phase 1.

## Host-only capture

An explicit mutable controller call captures GoalContract, LoopRun, TaskRun and
its complete Attempt parent chain, cumulative eight-dimensional and reported-token
budgets, next step number, no-progress tracker, strategy and strategy revision.
Capture does not change the loop, consume an approval, reset a counter or call a
provider. The host must stop concurrent writers before capture and assessment.

The record contains private task text, including the no-progress fingerprint
(which can contain the original observation). It must not be used as model
context or emitted in progress, Debug or error messages. It contains no provider
configuration, credentials, endpoint, ApprovalStore or reusable authority.

Entry points are `ReadOnlyLoopController::capture_recovery_input`,
`RecoveryInput::persist_new`, `RecoveryInput::load` and `RecoveryInput::assess`.
There is no capture timer, automatic startup reload, recovery CLI or resumed
controller constructor. Existing runs do not begin saving private inputs by default.

## Source boundaries

Trusted callers supply source paths; records never supply filesystem read paths.
EventLog is required. Six explicitly optional sources are model successes, model
failures, tool calls, side effects, traces and checkpoint metadata. RuntimeConfig
can calculate these paths without reading config or credential files. Unconfigured,
missing and present-empty sources are distinct. Paths are not serialized.
Capture checks the supplied EventLog path against the controller's actual path.
It does not prove bindings to the controller's other stores: the trusted host must
supply those paths correctly. Configured sources are scanned globally, not filtered
to the current task; unrelated pending audits can conservatively block a verdict.

For each configured source retain raw byte count and SHA-256 of the same bounded
bytes that were parsed. Require complete UTF-8 JSONL, a final LF, no blank lines,
known typed records, at most 10,000 records and lines below 1 MiB. Events require
schema 1, unique IDs and contiguous global sequence numbers beginning at 1.
Cross-check the current loop identity, lifecycle and allocated step positions
against the StateStore projection rebuilt from these exact events. Budgets cannot
be reconstructed from step numbers: a failed action can consume budget first.
Explicit envelope/payload loop identities must agree; lifecycle aggregates must
also identify that loop. Allocated step numbers must be nonzero and strictly
increasing, with their last number agreeing with the saved next step. This does
not establish completeness of model-action lifecycle pairs or cross-store joins.

Read caps: recovery record 2 MiB, events 16 MiB, each other source 4 MiB. Filling
a cap is rejected without an extra EOF probe. These are input limits, not a claim
of constant peak RAM. Same-FD before/after metadata checks detect observed changes;
sequential reads do not constitute a cross-file transaction or writer fence.

## Persistence and validation

Use an existing owner-private directory and exclusive, no-follow, single-link
regular-file creation. Require effective-UID ownership, private directory/file
permissions, file sync_all and pinned-parent directory sync before success.
Recheck file ownership, private permissions and single-link status, and pinned
parent privacy after the write/sync window before acknowledging success. A renamed
parent remains pinned: success concerns that directory/file, not continued lookup
at the original pathname. This is neither a writer lock nor a malicious-UID fence.
Do not overwrite or automatically delete/retry an unconfirmed write. Source and
record destinations must not alias, including a configured missing source.
Non-Unix protected I/O fails closed in this slice.

Loading requires complete bounded JSON plus LF, schema 1, no duplicate or unknown
nested fields, coherent identities and Attempt parent chain. Preserve legitimate
over-budget and zero-limit states, blocked token accounting, exhausted trackers,
and individually valid Paused/Cancelled loop vs Running TaskRun states. Never
normalize them into runnable state. SHA-256 binds content; it is not authentication
against a malicious same-UID process or root.
Owner-private files are not encryption. A privileged local process can still read
them; private task inputs must stay outside model-facing context and diagnostics.

## Assessment, never authorization

Assess sources again. Changed, unavailable, malformed or identity-conflicting
sources block correspondence. Pending/uncertain side-effect or tool audit blocks
correspondence; this first slice does not prove cross-store request/tool/snapshot
joins or provider idempotency. Even `inputs_match_only` ALWAYS has
`resume_authorized: false`; there is no resume constructor or automatic mutation,
retry, fallback, rollback, approval restoration or live provider call.
Capture may retain a structurally valid but event-inconsistent diagnostic record;
assessment reports `inconsistent_state`. Persistence success means a confirmed
private save, not a successful recovery assessment. Nonempty auxiliary audits,
even completed ones, report `audit_reconciliation_required` until joins exist.

Absent capabilities include workspace snapshot bindings, command/approval cursor
reconciliation, scheduler recovery and a consistent multi-store commit protocol.
Matching inputs alone cannot prove that repeating an external effect is safe.

## Acceptance evidence

Require offline round-trip and fresh-process reload tests, precise preservation of
budget/lineage/tracker, source-content changes (including same-length edits),
missing/partial/malformed logs, invalid record schema/lineage, collision and path
protection tests, and static diagnostics without private content. Spec review
precedes quality review. Record actual test results separately, not as forecasts.
