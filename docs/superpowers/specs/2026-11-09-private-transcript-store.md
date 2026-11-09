# Private Transcript Store — bounded library contract

Approved direction: the user requested timely local conversation/model-body history,
suggested roughly one-second file syncing, and said to proceed after the mixed policy proposal.
This increment is a **library foundation**, not automatic capture of real conversations.

## Scope

- New `transcript_store` module, independent of metadata-only EventLog/ModelCallStore.
- A fresh per-session JSONL file in an existing absolute private runtime directory, outside
  the explicit workspace. Unix only; other platforms fail closed. Never reopen/overwrite a session.
- Root path traversal opens directories one component at a time with NOFOLLOW; fixed directory
  fd plus exclusive final creation; root owned by effective uid and no group/other permission,
  final file ordinary/single-link/private. No chmod or cleanup of preexisting files.
- One file writer per store, no public Clone. Held fd prevents later pathname redirection;
  not a defense against arbitrary same-uid inode tampering/unlink/permission changes.
- Typed entries: user message, assistant chunk/complete, model request, model response chunk/complete.
  Entry fields: message_id, optional model_call_id, kind, content. Session ID and monotonically
  increasing sequence are supplied by the store; no generic header/endpoint/config map.
- Trusted callers supply **already approved body content**. Reject recognized secrets using
  existing Redactor, preserving accepted UTF-8 content exactly. Compare JSON values semantically
  when applicable, so harmless JSON formatting is not rejected or silently normalized.
  Structured-looking input beginning with `{`, `[` or `"` after leading whitespace must
  pass bounded JSON parsing; malformed/deep/overflow input fails closed, not plaintext fallback.
  This restriction also rejects incomplete structured chunks and some ordinary code/quote fragments.
  No guarantee of recognizing arbitrary secrets or split-across-chunk secrets: automatic raw
  streaming capture must remain disabled until its whole-stream policy is implemented.
- No API keys, auth headers or endpoints should be supplied. Structured request JSON with transport
  auth/header/endpoint fields is rejected. Body protection is not encryption; no automatic LLM access,
  reader, export, network transport, credential resolver or tool registration in this increment.

## Durability

- Append writes the full encoded record and newline directly to the file while serialized by a mutex;
  no unbounded channel or deferred userspace content queue.
- Default background sync interval 1s measured from first dirty write; more chunks must not
  postpone its deadline. Pending byte threshold defaults to 64 KiB; total file cap 64 MiB.
  Bounds: interval 1ms..60s, threshold 1..4MiB, total cap 1..1GiB. Public policy is validated
  **before filesystem creation**. Record content cap 64KiB, identifiers short ASCII safe IDs.
- UserMessage, AssistantComplete, ModelRequest and ModelResponseComplete require successful sync
  before append returns. Chunk entries batch. Explicit sync, and explicit close/normal Drop, flush
  remaining data; Drop is best-effort, so durable success must be based on explicit receipts.
- Receipts expose written_sequence and durable_sequence; durable advances only after successful
  per-file sync. First creation syncs file + correct pinned directory before returning a store.
- Partial write or sync failure is sticky; block further appends and never advance durable watermark.
  Error messages expose kinds only, no paths, content, credentials or original error text.
- Idle timer waits, no periodic fsync with no dirty data, no sync(2)/syncfs or per-token fsync.
- Timer target is not a hard RPO guarantee; blocking I/O/scheduling/storage faults can exceed 1s.
  No OS-power-loss, VHDX-loss, remote-provider exactly-once or complete controller recovery claim.
- Capacity/validation errors before write do not poison the store. Persisted records can still
  contain an incomplete final tail after crash; no auto truncation/replay/reopen in this increment.

## Acceptance

Real timer (including idle-no-new-append), byte threshold, immediate critical records, close,
sticky simulated sync/write failures, exact multibyte content/JSON whitespace, sequence/watermark,
private permissions, forbidden root/symlink/hardlink/existing file behavior, bounded bytes/policy,
no sensitive synthetic text in errors/Debug/persisted file, and a controlled ACK-then-SIGKILL check.
Failure injection is internal test-only; never kill the user's OS or inspect real conversation data.

Follow-up: secure read/recovery inspection, protected runtime capture seams and transcript references,
stream sanitization, immutable prompt-pack dedup, rotation, then foreground/provider opt-in wiring.
Actual shared audit I/O and checkpoint synchronization remain independent open Phase 1 blockers.
