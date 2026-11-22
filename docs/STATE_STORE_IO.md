# StateStore IO contract (S1)

Status: **限定切片已实现，通过独立审核与全量验证**. Base: `ac0070c`.
This is a bounded Unix ordinary-file IO slice, not full crash recovery or
authorization to enable live write operations.

## Scope and compatibility

S1 hardens only `StateStore::load` and `StateStore::persist` file IO. It does not
change projection/rebuild business logic, public API signatures, error variants,
or the persisted schema, including loop-scoped steps and legacy-schema handling.
Persist retains pretty JSON without an added trailing LF. Ordinary relative
basenames must work without changing the process current directory.

Unix uses the shared `AuditParent` safety boundary. Non-Unix fails closed with
an IO error rather than silently using an unsafe path-based fallback.
This narrows the old non-Unix ordinary-file behavior; it is not a portability
claim. IO diagnostics now expose fixed stages and `ErrorKind`, not raw OS text.
Parent traversal rejects symlinks and `..` components and requires readable
directories. Persist synchronizes the traversed directory chain, including
existing directories; the extra cost has not been benchmarked. New parents
request `0700` and new temporaries `0600`, both subject to umask restriction.

## Load

- Open the parent with `AuditParent` in no-create mode and read through its safe
  read FD. A missing parent or file remains an IO error; load does not create it
  and does not substitute an empty/default projection.
- Keep UTF-8 and JSON/schema failure classification compatible with the current
  API. Do not migrate invalid or nonempty legacy flat-step projections.
- Safe-read validation is distinct from persist's rename replacement semantics.

## Persist and success acknowledgement

Keep one pinned parent directory for the whole persist operation:

1. Create missing parents through the safe parent helper.
2. Create a unique, exclusive temporary regular file in that same directory,
   requesting mode `0600`; do not reuse or truncate an existing temp entry.
3. Write the complete pretty-JSON buffer with `write_all` to that actual FD.
4. Complete `sync_all` on that same temporary-file FD.
5. Publish using same-directory, directory-FD-relative `renameat`.
6. Synchronize the same pinned parent directory before returning `Ok(())`.

The final target is not opened for writing. Rename replaces the target directory
entry rather than following a final symlink to write its referent. Do not
mechanically reject link targets using append-store rules: retain rename's
replacement semantics. This introduces no policy to preserve the old target's
UID, GID, mode, or inode.

## Failure boundaries

- Before a successful rename, return an error without replacing the old target.
  Best-effort cleanup may remove only the temporary entry created by this call.
- Cleanup failure may leave a temporary file. No crash/power-loss cleanup,
  orphan scan, or automatic recovery is promised.
- Cleanup is armed after exclusive create and FD validation return successfully.
  A failure in post-create FD validation can therefore also leave a temporary.
- After rename, a parent-sync failure returns an error even though the new
  projection may already be published. Do not roll back the target or
  automatically retry publication; an error is not proof that the old state is
  still present.
- Creating missing parents can leave directories after a later failure. There
  is no directory-creation rollback promise.

## Trust boundary and exclusions

The directory is trusted. Pinning its FD avoids redirecting later stages through
a replaced parent pathname, but does not provide fencing against noncooperating
writers replacing entries inside that directory. S1 adds no writer lock, CAS,
conflict arbitration, hostile-writer defense, or full crash-recovery guarantee.
Concurrent writers may publish complete projections with last-successful-rename
semantics; S1 does not choose a business winner.

## Verification status

The pre-fix ordinary-file regression was 8 passed / 1 failed: a bare relative
basename failed. The final ordinary-file harness has 10 passing entries,
including a subprocess probe and deterministic exclusive-collision/retry check.
Six actual-FD fault tests and 15 shared-helper tests also passed. Final full
verification: 86 harnesses / 1182 passed, zero failed or ignored; strict
all-targets Clippy, actual lib/bin build, fmt and diff checks passed.
Fault tests cover partial/full writes returning errors, file-sync and publish
errors, post-publish directory-sync errors, and parent-rename decoy/witness
checks. They establish currently readable bytes and actual FD/inode routing,
not power-loss durability. The private IO seam verifies orchestration; the four
`RealStateIo` delegates also require source review, not mock-only claims.

Final verification and delivery evidence:
`PHASE_1_VALIDATION_PLAN.md` and external
`/temp/codex/steerbox-validation/state-path-20261122/RECEIPT.md`.
This slice adds no new special-path, FIFO, privilege-drop, or permission matrix
and does not claim the previously refused A1 scope is verified.
