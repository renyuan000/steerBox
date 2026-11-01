# Reported-token runner configuration contract

## Scope and acceptance criteria

`RuntimeConfig.reported_token_limit` is an optional schema-v1 top-level `u64`
extension. Missing or JSON `null` disables the budget, and serializing `None`
omits the field. Integer zero enables the budget and rejects the first model
invocation with `ReportedTokenBudgetError::Exhausted`; it must not be treated as
disabled. Positive integers through `u64::MAX` enable the existing task-local
reported-token ledger. `default_for_project` defaults to `None`.

This is an optional schema-v1 extension. Older binaries with `deny_unknown_fields`
reject configurations containing this field; no legacy compatibility shim is added.

Negative numbers, fractional numbers, overflow, strings, booleans, arrays and
objects are invalid. Field-local deserialization returns a fixed metadata-only
error; invalid string values must never be repeated by configuration parsing,
loading, or `config validate` diagnostics. This is not a global configuration
error-format rewrite. Existing unknown-field rejection remains unchanged.

Preparation still serializes and revalidates the caller's public configuration
fields. It saves the checked limit in the prepared run; later changes to the
caller's configuration cannot change that prepared run. One small helper enables
the existing controller budget after stores are attached and before the first
resolved model call in all three paths: OpenAI foreground, Anthropic foreground,
and the shared Markdown path (both protocols).

Zero-limit runs perform no transport send or credential resolution and produce
no model/tool invocation audit records or Markdown edit. Local controller-start
events or opening caller-owned stores are permitted. Existing protocol, policy,
prompt, model-call audit, and count-budget behavior remains in force.

Successful `ProviderRunOutcome` values expose a read-only
`reported_token_budget()` getter returning `Option<&ReportedTokenBudget>` over a
private cloned snapshot. This supports checking actual task-local totals and
settled IDs without exposing a mutable controller or public mutable ledger
field. A disabled budget returns `None`.

Each runner currently creates a fresh TaskRun. The limit is only for that
TaskRun, not a cross-run cumulative quota. The getter is not persisted recovery;
CLI StateStore replay is not TaskRun-budget persistence. Same-ID rerun protection
and safe cross-run recovery require a separate slice. There are no new CLI
flags, credentials/config-file discovery, live calls, prices, tokenizer-based
estimates, global learning, or cross-process semantics in this slice.

## Ordered implementation and verification checkpoints

1. Run the existing configuration/provider/Markdown test targets unchanged and
   retain a baseline log outside the repository.
2. Add tests first and retain RED output before production changes. Cover
   missing/null/zero/positive/max integer serialization, parsing and load;
   invalid literal rejection and no marker reflection through parse/load/CLI;
   zero sends for all runner paths, zero resolver calls for both HTTP protocols,
   and known nonzero usage retained in the outcome ledger.
3. Add only the field-local parser/default, checked-limit capture, common enable
   helper, and private outcome snapshot/getter. No controller ledger semantics
   changes.
4. Run formatting, targeted tests with `--locked --offline` and one build job,
   then review the complete actual diff for scope and behavior. All generated
   artifacts and logs belong under
   `/temp/codex/steerbox-validation/reported-token-config-20261101/`.

Tests use synthetic configs, temporary stores, controlled fake transports, and a
counting resolver that returns unavailable without reading a credential. Source
edits are reversible using their exact Git diff; no separate backup or commit is
needed before the independent specification and quality reviews.
