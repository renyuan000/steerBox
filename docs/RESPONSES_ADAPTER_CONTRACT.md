# Bounded Responses adapter and endpoint contract

Date: 2026-11-24. Scope: the approved incremental provider-support work.

## Decisions

- `openai-responses` is an explicit wire protocol, separate from
  `openai-compatible-chat` and `anthropic-messages`. Provider aliases, hostnames
  and model names cannot override an explicit protocol. Old configurations
  without the field retain their existing deserialize-time normalization
  (`anthropic` provider to Messages, other aliases to Chat); runners use the
  resulting explicit field, and Responses must always be explicitly selected.
- `endpoint_mode` is `base_url` by default. Chat appends `/chat/completions`,
  Responses appends `/responses`; neither inserts `/v1`. Anthropic retains its
  existing base-address behavior. With `full_url`, every adapter uses the exact
  configured endpoint without appending a suffix. Existing endpoint security
  checks, credential lookup and redirect restrictions still apply.
- The first Responses slice is stateless, synchronous and non-streaming. It
  sends typed `input` messages with the existing roles, `store: false` and no
  tools, prior response ID, background mode or protocol fallback. Native
  reasoning uses `reasoning.effort`, not Chat's `reasoning_effort`.
- Only a completed response with one completed assistant message consisting of
  output-text blocks is mapped to the existing strict Harness decision/action
  contract. Reasoning items may be ignored; refusals, tool calls, unknown items,
  incomplete/failed responses and ambiguous multiple messages fail closed.
- Preserve `input_context_hash` as the SHA-256 digest of the exact encoded body
  (not a routing/binding digest). Separately bind prepared requests to provider,
  endpoint, protocol and address mode, and recheck the current effect policy
  before sending. Resolved prompts must match the selected protocol before
  transport or credential lookup. Equal bodies may have equal body hashes on
  different routes; that does not authorize cross-route prepared-request reuse.
- Preserve the existing controller's policy, budget, audit, duplicate-identity,
  Markdown and read-only execution gates. Supporting Responses does not grant
  new tool authority. Automatic effect upgrade remains Chat-only in this slice.

## Verification

Offline injected transports must verify request paths, roles, wire fields,
reasoning, usage, completion parsing and secret-free credential references.
Negative tests cover protocol/address-mode rebinding, unsupported streaming,
refusal/tool/unknown output items, partial responses and invalid bodies.
Runner tests must prove a completed decision reaches the existing success audit.
Run existing provider/config/runner regressions and strict lint checks.

No private endpoint or real credential belongs in fixtures or docs. No live
acceptance, token-stream responsiveness, native tool calling, continuation or
full Phase 1 completion is established by these offline tests.

References: OpenAI Responses create schema (official documentation); the existing
`PROVIDER_PROTOCOL_MATRIX.md`; local OpenCode/Codex public protocol adapters.

## Configuration

The SDK package name `@ai-sdk/openai-compatible` is not a steerBox protocol ID.
Select the actual request/response format, independently from provider aliases.
An explicit Responses model profile can be added to `model_profiles`:

```json
{
  "model_id": "example-model",
  "provider": "example-relay",
  "endpoint": "https://relay.example.test/custom",
  "endpoint_protocol": "openai-responses",
  "endpoint_mode": "base_url",
  "auth_ref": "env:EXAMPLE_LLM_KEY",
  "capabilities": ["text"]
}
```

This sends to `https://relay.example.test/custom/responses`. No `/v1` is inserted.
If the relay gives a complete URL, use `endpoint_mode: "full_url"` and put that
exact URL in `endpoint`; trailing slashes are preserved. This mode also applies
to `openai-compatible-chat` and `anthropic-messages`. Omitting the mode preserves
the existing base-URL behavior. Unknown modes/protocols are rejected.

Authentication references never become input messages. Use the existing trusted
credential resolver; this example is not an instruction to put a key in a prompt
or an invocation argument. No credential source or private config was changed.

Existing commands `loop provider`, `loop provider-readonly` and
`loop provider-markdown` accept Responses through the explicit model profile.
The Markdown command still requires explicit scope and authorization; choosing
Responses does not grant Bash, arbitrary edits or native provider tools.

Schema cross-check: OpenAI's generated `openai-python` types
`EasyInputMessageParam` and `ResponseInputMessageContentListParam`.
Assistant history uses the permitted easy-input text string, not a partial
output-message object containing `output_text` blocks.

## Offline acceptance receipt — 2026-11-24

- `cargo test --offline -j 2 --target-dir /temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`:
  1,341 passed, 0 failed, 0 ignored across 94 result groups (including doc tests).
- `cargo clippy --offline --all-targets -j 2 --target-dir /temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Dedicated coverage: 13 Responses tests, plus new Anthropic full-URL, localhost
  HTTP transport and bidirectional resolved-protocol mismatch regressions.
  CLI coverage here is offline read-only/Markdown **preflight**, not a live
  provider invocation from the CLI. Injected runners cover observation, read-only
  completion/zero budget, allowed Markdown edits and rejected out-of-scope edits.
- Independent review identified and corrected the assistant input encoding,
  clarified body-hash versus binding semantics, and caught a missing test-only
  loopback HTTP flag. Production HTTPS/redirect/credential policy is unchanged.
- No real provider or credential was used. Live Responses acceptance, SSE,
  native tools, continuation and Responses automatic effect upgrade remain open.
