# Resolved model allowlist contract

Date: 2026-11-08. This document defines the resolved-call authorization boundary;
validation and delivery status belong in `PHASE_1_VALIDATION_PLAN.md`.

## Requirement

`GoalContract.allowed_models` applies to resolved calls as well as the existing
fixture/routed and effect-upgrade paths. An empty list remains unrestricted;
a nonempty list permits only exact model IDs. A resolved adapter, prompt pack,
or route does not grant permission by itself.

The common resolved entry must reject a disallowed request or route model before
step/model-call budget consumption, model-call lifecycle events, adapter calls,
or model-driven tool execution. Route/pack consistency checks remain in force;
an allowed route cannot authorize a different disallowed request model.

The routed entry must check the primary model before invoking the candidate
resolver or binding a route. It must check the next fallback candidate before
adaptation and the actual selected fallback before invoking the resolver or
creating a child Attempt. A denied fallback ends the failed run through the
existing closure path and preserves the original provider error when that
closure succeeds. It does not skip to another model or replenish any budget.

## Verification

Synthetic counting adapters/resolvers must cover direct denial, mismatched
request/route models, primary denial, fallback denial after one failed call,
allowed and empty lists, and failure while recording terminal audit events.
Denied fallback must retain first-call failure evidence without child Attempt,
fallback route, or second send. Terminal audit failure must not allow a later
invocation to send after the log is repaired. Existing foreground upgrade,
action/continuation, and routed regression suites remain required.

## Non-goals

No provider/endpoint allowlist, new approval flow, automatic model selection,
credential access, live-provider validation, or complete recovery is added.
Candidate-resolver/binding failure closure is a separate increment documented in
`RESOLVED_ROUTED_FAILURE_CLOSURE_CONTRACT.md`; it is not part of this allowlist
repair. Ordinary foreground failure closure is another separate increment,
documented in `FOREGROUND_FAILURE_CLOSURE_CONTRACT.md`.
