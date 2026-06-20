# Docs Sync After Edits

- Slug: `docs-sync-after-edits`
- Category: Maintenance manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: After code changes, find affected docs and update README, API references, and inline comments to match.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.

- Source: https://loops.elorm.xyz/loops/docs-sync-after-edits
