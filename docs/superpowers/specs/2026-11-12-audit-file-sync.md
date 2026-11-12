# Audit JSONL file synchronization increment

Approved direction: continue the recovery mainline by adding real critical-audit/checkpoint
durability barriers before inspection/resume. Base `904606a`; this is a narrow prerequisite,
not completion of runtime safe I/O or crash recovery.

`audit_store::append_jsonl`, shared by CheckpointStore, SideEffectLedger and TraceStore, must
return success only after the complete JSON record and newline have been written and that same
open file's `sync_data` succeeded. Existing record format, paths, APIs and redaction remain.
No global sync, retry, truncation, replay, new credential/body collection or production LLM call.

Write errors must not call sync or return success; sync errors must propagate with sanitized
stage/kind only, without claiming the bytes were rolled back. A failed call may leave a partial
or complete record. This stateless helper does not add a sticky writer or permission to retry.

Directory creation/fsync, descriptor-relative trusted path traversal, read hardening, concurrent
writers, bad-tail detection, run-summary replacement and cross-store transactions remain TODO.
Even a successful file sync is not proof of a new pathname's durability, OS/VM power-loss safety,
workspace mutation durability, exactly-once external requests or restored TaskRun/authorization.
TraceStore shares the barrier deliberately; no relaxed trace mode is introduced in this patch.
Each trace append now also blocks on file synchronization; its latency cost has not been measured.

Verification: baseline audit/gates; behavior RED for absent sync and hidden sync failure; fake
backend write/sync/order/partial-failure tests plus actual append with Linux `/dev/null` (write
success but sync failure); real checkpoint/ledger/trace append-ACK then self-child SIGKILL test.
SIGKILL is not a power-loss test. Focused and full offline tests, strict Clippy/build/fmt/diff,
independent review, exact independently buildable commit and local dev fast-forward with recheck.
No remote push. External target/evidence under `/temp/codex`; jobs=1, one Cargo at a time.

Plan: write tests + extraction with unchanged behavior; collect RED; add sync; collect GREEN;
review actual error propagation and docs; run gates; commit; verify committed tree and merged dev.
Rollback basis is exact Git hunks/commit, not whole-source copies. Preserve preexisting user docs.
