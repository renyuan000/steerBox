# Ordinary foreground failure closure

Date: 2026-11-08. This is a bounded failure-lifecycle repair, not a general
foreground execution or recovery redesign.

## Requirement

After an ordinary OpenAI-compatible or Anthropic foreground runner has obtained
its controller, errors from budget configuration or its single resolved model
step must close a still-Running run. Preserve spent budgets and existing
Attempt artifacts/evidence, mark Attempt/TaskRun/Loop Failed, then append exactly
one fixed, metadata-only `loop_failed` event. Do not record the raw error,
provider response, endpoint or credential reference in that event.

Return the original error when closure succeeds. Return the closure error when
terminal auditing fails; in-memory state must already be closed. The same
controller must not send again after the audit destination is repaired. No
additional model call, retry, fallback, credential resolution or budget reset
is authorized by failure handling.

Already Completed, Paused, Cancelled or Failed controllers are not overwritten.
In particular, a progress callback failing after successful completion must not
turn the successful run into a failed run or append a contradictory failure.

## Implementation boundary

- Apply the wrapper only to the two ordinary foreground runner entry points.
- Share the existing state-first closure primitive, preserving the routed
  wrapper's event semantics. Do not unconditionally close errors inside the
  common resolved model step: a routed caller may still have a legal fallback.
- Preparation, unsupported protocol and history-identity rejection happen
  before a controller exists and must not create a fictional failed run.
- Initial `Loop Started` progress failure inside the constructor remains a
  separate lifecycle issue. So do non-error Continue/RequestHuman semantics,
  Markdown execution, upgrade orchestration and durable recovery.

## Required evidence

1. Both protocols: authentication, retryable provider failure and invalid
   decision output return their original typed error and one terminal event,
   with one send and retained failure evidence but no successful/tool record.
2. Zero reported-token limit: zero sends and credential resolutions, no model
   failure record, but a failed lifecycle for the already-started run.
3. Progress failure while Running closes the run; progress failure after
   completion preserves Completed and its completion event.
4. Controller-level assertions cover all three state layers, budget/evidence
   preservation, audit-failure/no-resend and non-Running no-ops.
5. Existing legal routed fallback and successful foreground regressions pass.

Use synthetic fixtures only. Record actual RED/GREEN, reviews and validation
in the plan and Phase 1 validation record; this contract is not proof of delivery.
