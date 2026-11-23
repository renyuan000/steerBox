# Bounded read-only provider loop

Date: 2026-11-08. Implements the previously approved Phase 1 direction:
connect the existing resolved model/tool loop to an explicitly authorized
foreground provider runner. This is not another fixture-only loop.

## Interface and authority

Add `ForegroundProviderRun::prepare_readonly(config, request, run)` returning
`BoundedReadOnlyProviderRun`. Its caller-owned configuration contains an absolute
workspace, explicit relative Markdown paths and a positive iteration limit
(1 through 16). The path list contains 1 through 256 existing regular files,
matching the existing action-scope contract; empty implicit workspace access is
not granted. Preparation freezes/revalidates model configuration and the run
scope, but does not resolve credentials, send requests or run tools.

Support the existing OpenAI-compatible and Anthropic transports separately and
a protocol-selecting HTTP credential-resolver entry. Do not change ordinary
observation, Markdown or effect-upgrade behavior. No new dependency is needed.

Only `git_status` (Git-discovered repository metadata) and `markdown_link_check`
on the listed Markdown files are executable, including verification. Git may
discover an ancestor repository when the workspace is a subdirectory; its
status metadata is not limited to the workspace subtree or Markdown path list.
The trusted caller authorizes that repository metadata and should supply the
intended repository root. This does not authorize arbitrary file bodies.
Validate both action and
verifier against caller authority before tool execution. Root must match the
frozen workspace, declared paths must be a subset of the caller's paths, and a
path-bearing action/verifier must name an authorized path within that scope.
No action may widen permissions, use shell, read an arbitrary file, or request
credentials. Prompts describe the actual action schema and allowed tools, and
include the authorized workspace/path scope without endpoint/auth material.

Reject root/relative/traversing/symlink workspaces and unauthorized, duplicate,
traversing, non-Markdown or symlink file paths. Recheck the workspace and files
at execution. Runtime stores (events, model success/failure and locks, tool
records and lock) must be distinct regular paths outside the workspace; reject
aliases and symlink/hardlink collisions before run start/provider/tool access.
This increment requires Unix file-identity/link-count checks; unsupported
platforms fail closed at preparation rather than silently dropping hardlink
protection. Windows support requires an equivalent separately verified gate.
The host owns these paths; this is not OS isolation from a malicious same-user
writer or inherited Git environment/configuration.

The library receives already opened EventLog/ModelCallStore handles; their
caller-side open operations are not covered by its preflight. A subsequent CLI
must validate its configured paths before opening stores, and must additionally
protect its state projection path. Runner checks do not make an unsafe caller
open operation safe retroactively.

## Execution and budgets

Reuse `ReadOnlyLoopController::run_resolved_model_loop`, PolicyGate, the fixed
read-only executor, model/tool stores and redacted follow-up observations. One
TaskRun and Attempt share model/step/tool and configured reported-token budgets
across all iterations. Each action and its verifier consume real tool budget.
The caller's iteration ceiling is not model-controlled. No automatic fallback,
upgrade, retry, write or budget reset is added.

Assembly must enable the existing strategy-aware tool accounting within the
same caller-authorized budget; merely counting ToolCallRecords is not enough.
Validate the frozen filesystem scope again after each provider response before
handing its action or verifier to the controller.

The read-only controller must distinguish executing a verifier from passing it:
a Git verifier with a nonzero exit code or a Markdown verifier with missing
local links must not emit `model_action_verified` or a passed model-call record,
and must not continue to another model call. Correct this directly related
controller defect without changing raw executor outputs or ordinary action
result semantics: a diagnostic action may report a nonzero Git exit or missing
links as evidence, whereas its explicitly selected verifier must pass its own
criterion. Passing a verifier does not mean every diagnostic action found no
issue. Tool execution errors and policy denials still stop the run.
The Markdown verifier scans complete UTF-8 input up to a 2 MiB byte budget;
over-limit, invalid UTF-8 and read failures stop verification rather than passing
a truncated prefix. This differs from the unchanged 16 KiB display truncation of
read_file. These failures must not emit verified/passed records or continue the
model loop. This bound does not replace the existing link syntax/target rules.
Propagate the two final human/no-action progress-report
errors so the runner's existing failure closure can handle them; do not change
successful stopped outcomes or initial-constructor progress semantics.

Return the existing loop outcome (including precise stop reason and iteration
count), actual loop status and read-only count/token-budget snapshots. The
count snapshot proves actual controller accounting, not durable recovery.
Complete is
not interchangeable with RequestHuman or ContinueWithoutAction: those return
their explicit existing stopped outcomes, do not spin or report completion.
Their inherited controller status may remain Running; this synchronous library
return does not mean a worker is still running or that durable resume exists.

Post-construction execution errors close a still-Running controller with the
existing state-first foreground failure closure. Preserve original errors,
unless terminal auditing itself fails. Existing completed/paused/terminal state
is not overwritten. Duplicate run-history rejection remains before send and
credential resolution. Initial constructor progress failure remains separate.

## Evidence and delivery

Use synthetic two-protocol responses and real temporary Git workspaces. Cover
two steps (action plus verification, then complete), distinct prompt/call IDs,
actual tool counts, bounded/no-action/human stops, verification/policy/provider
errors, zero and cumulative token limits, rejected action/verifier scope, bad
runtime paths, symlink/hardlink aliasing, duplicate history, progress/audit
failures and redacted follow-up wire. Test denied cases for zero tool records
and unchanged workspace content; preflight failures have zero sends/resolutions.
Rebuild StateStore and ExecutionQuery from actual runner events for complete,
explicit stopped, scope-rejection and verifier-failure cases; correlate loop,
task, attempt, step, model and tool evidence. The existing policy-decision
envelope has no attempt field value; other runner events retain their attempt
identity. Projection checks are not state-file persistence or durable recovery.

This increment delivers an injected/HTTP library runner.
Standalone CLI wiring, live-provider acceptance, arbitrary coding tools,
network streaming, background control and full Phase 1 remain separate work.
Do not claim live validation from synthetic tests.

Follow-up: the standalone CLI has a separate usage and pre-open/projection
contract in `BOUNDED_READONLY_PROVIDER_CLI.md`. Its delivery evidence is recorded
separately in the Phase 1 validation plan; the library increment above alone
does not prove caller-side opens, CLI operation or live-provider acceptance.
