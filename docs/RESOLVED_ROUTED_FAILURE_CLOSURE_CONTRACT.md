# Resolved routed candidate failure closure

Date: 2026-11-08. Scope: the primary/fallback candidate resolver and candidate
binding validation in `run_resolved_routed_model_step()` only.

## Requirement

Once an active routed run enters candidate resolution, failure to resolve or
validate either the primary or fallback candidate must not leave that run active.
Use the existing routed failure closure: mark the current nonterminal Attempt,
TaskRun and Loop Failed, synchronize the spent budget, and record `loop_failed`.

If closure succeeds, return the original resolution/validation error. If the
terminal audit write fails, return that audit error; the in-memory run must
already be terminal. This is not a promise of durable recovery after a failed
write. Repairing the audit destination must not allow the same controller to send
another provider request.

Primary resolution/validation rejection does not send a provider request or
spend step/model-call budget. The initial Attempt already exists and must not be
replaced. Fallback rejection preserves the primary failure record and evidence
references, does not create a child Attempt, send to a second provider, advance
to another fallback, or reset spent budgets.

## Boundaries

- Entry `route.validate()` and Goal model-allowlist rejection keep their existing
  preflight semantics. They are not included in this closure repair.
- Route binding after candidate validation, adaptation-planning failure, audit
  errors elsewhere, effect upgrades, ordinary one-shot foreground calls and the
  common resolved call implementation are unchanged.
- No new error payload or raw resolver/provider error text is persisted.
- Legal resolved fallback, direct calls and existing action/upgrade behavior
  remain covered by regression tests.
- No live-provider call, credential access, complete restart recovery or Phase 1
  completion is claimed.

## Required verification

1. Primary resolver failure and primary invalid candidate binding.
2. Fallback resolver failure and fallback invalid candidate binding after one
   retryable primary failure.
3. Exact original error identity, all three Failed states, no extra send/child,
   retained failure evidence and unchanged spent budgets.
4. Terminal-event write failure, followed by storage repair and attempted reuse:
   audit error returned, controller terminal, no new provider send.
5. Successful primary/fallback and existing allowlist preflight behavior.

Execution results belong in `PHASE_1_VALIDATION_PLAN.md`; this contract alone
does not establish implementation or passing tests.
