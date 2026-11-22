# Workspace Markdown edit IO contract (W1)

Status: **限定 Unix 普通文件切片已实现并验证** (2026-11-22), against base
`c7df098`. Independent source review and the checks below cover this IO slice,
not complete recovery or permission to enable live writes. Commit and local-dev
integration evidence is recorded in the external receipt below.

## Scope and compatibility

W1 changes only the actual file IO of `workspace_edit::edit_markdown`. Keep its
public API, `EditResult`, explicit approval gate, normalized relative path,
goal scope and Markdown checks. Keep existing error categories and the bounded
UTF-8 expected-content comparison: read at most `expected.len() + 1` bytes and
reject a mismatch before staging a replacement. If expected and replacement
match, return `changed = false` without creating a temporary or publishing.

Retain the existing canonical workspace-root/containment semantics, including
resolution of the workspace root itself. Do not silently impose audit's raw
ancestor policy on paths above that root. Continue rejecting target and
workspace-internal ancestor symlinks; this precheck is not a lifetime workspace
identity or race-free authorization guarantee.

Regular editor source files may have hardlinks. Do not mechanically reuse the
audit reader's `nlink == 1` rule or introduce UID/owner-only/mode rejection.
Replacing the selected directory entry does not overwrite other hardlink names.
No new link-specific validation matrix is included in W1.

## Read and replacement sequence

On Unix, bind the canonical workspace path and normalized relative target to a
no-create pinned parent. Do not create a missing target or parent directory.

1. Open the existing source through that parent with non-following,
   nonblocking, close-on-exec flags; validate regular-file status on the actual
   read FD without imposing the audit hardlink/UID policy.
2. Obtain expected-content bytes and the permissions to copy from that same
   source FD, not a later path-based metadata lookup.
3. Retain the existing basename's extension and PID temporary-name convention
   (`<extension>.steerbox-tmp-<pid>`), including exclusive-create collisions.
   Create the temporary in the same pinned parent, requesting `0600` subject to
   umask; never truncate or remove a preexisting conflicting entry.
4. Write the complete replacement bytes, copy the source-FD permissions to the
   actual temporary FD, then `sync_all` that FD. This preserves permission-copy
   behavior, not source UID/GID or inode identity; it adds no permission policy.
5. Rename the temporary over the selected target entry using that same pinned
   parent, then synchronize the same parent before returning changed success.

Reuse shared exclusive-create, directory-relative rename/unlink and parent-sync
primitives where their contracts match. Keep editor regular-file reading
distinct from the stricter audit reader. Non-Unix actual edit IO fails closed;
do not introduce an unsafe path-based fallback. This is an explicit platform
boundary and a restriction relative to the former implementation, not a
cross-platform compatibility claim. An approved no-op also fails before reading
on non-Unix; earlier approval/path checks still apply. Non-Unix compilation and
execution were not validated in this slice. IO diagnostic strings may now use
the shared helper's stage/kind vocabulary; error variants remain unchanged.

## Failure and trust boundaries

- Before successful publication, the old target is not replaced. Only a
  temporary exclusively created and successfully FD-validated by this call is
  eligible for best-effort cleanup through its pinned parent. A validation
  failure after creation, cleanup failure or abrupt process exit can leave it
  behind; no orphan-cleanup guarantee is provided.
- After successful rename, a parent-sync error may leave the new contents
  already published. Do not roll back, automatically retry, or treat an error as
  proof that the old contents remain.
- The directory is trusted. Pinning avoids reopening a replaced parent path,
  but does not fence noncooperating writers changing entries in that directory.
  Expected-content comparison is not an atomic CAS; no writer lock, fencing,
  conflict arbitration, orphan recovery or power-loss recovery is promised.
- Safe parent traversal requires readable directories. Additional FD checks and
  directory synchronization are not benchmarked; W1 makes no performance claim.

Do not change controller authorization/audit flow, link verification, snapshot
capture/restore policy, or the shared checked-path API as an incidental part of
this IO slice. In particular, snapshot-specific hardlink/ownership checks do not
define the editor's source-file policy.

The executor propagates edit errors. The controller's ordinary edit failure
path may record a post-failure hash; it does not automatically restore the file.
Snapshot restore maps edit errors to `ReplacementFailed`, requiring inspection
of the current file before retry. Consumed Once approval is not refunded. These
caller behaviors were source-reviewed, not exercised by injected controller or
restore end-to-end faults. Audit failure can itself interrupt later records;
do not promise a complete set of terminal audit records after every error.

## Verification status

Commands used `--locked --offline`, one Cargo build job, and an external target.

| Evidence | Executed result |
|---|---|
| Existing edit/audit-gates/snapshot baseline | 70 passed |
| Deterministic parent replacement before the fix | 0 passed / 1 failed; original directory remained unchanged despite success |
| Same parent-replacement test after the fix | 1 passed |
| Real source/temp FD fault and success tests | 10 passed in the final full run |
| Parent replacement: publication/sync and failure cleanup | 2 passed in the final full run |
| New public ordinary-file compatibility tests | 7 passed |
| Final all-targets suite | 87 harnesses / 1201 passed / 0 failed or ignored |
| Strict all-targets Clippy, actual lib/bin build, fmt/diff | all exit 0 |

The FD tests cover read, partial/full write, permission-copy, file-sync,
prepublication rename and postpublication parent-sync failures. They also check
the same temporary FD/device/inode across stages, permission copying, no-op and
mismatch short-circuiting. Ordinary fixtures do not establish owner preservation.
The two directory-replacement tests check the old pinned directory, decoy
preservation and cleanup/sync routing without a timing-dependent race.
Production default delegates were reviewed separately from injected tests.

Logs and commit/integration status:
`/temp/codex/steerbox-validation/workspace-edit-path-20261122/RECEIPT.md`.
No temporary production mutant was used. No new special-path, FIFO,
privilege-drop, permission or link-specific matrix was added. No live provider,
real credentials, power-loss experiment or cross-platform validation was used.
