# loops.elorm.xyz Loop Index

## Scope

This index records the currently accessible public loop examples we could scrape from loops.elorm.xyz during this session.
The site reports 40 loops total, but the scrape consistently exposed 9 unique loop pages from the public browse listing.

## Learning Notes

- Chinese learning summary: [LEARNING_NOTES_CN.md](./LEARNING_NOTES_CN.md)

## Notes

- `Use loop` copies the kickoff prompt.
- `Open in Cursor` / `Open in Claude Code` pre-fill the prompt but do not install hook files.
- `Install files` means a zip bundle must be extracted into the repo for hook-based loops.
- Guardrails are anti-gaming rules: do not change the check command or exit criteria, do not bypass checks, and stop/report blockers if stuck.

## Loop Catalog

### [A11y Audit Until Clean](./a11y-audit-until-clean.md)

- `/a11y-audit-until-clean`
- Category: Quality manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Run automated accessibility checks on changed routes, fix violations, and repeat until the audit is clean.

### [Docs Sync After Edits](./docs-sync-after-edits.md)

- `/docs-sync-after-edits`
- Category: Maintenance manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: After code changes, find affected docs and update README, API references, and inline comments to match.

### [Flaky Test Triage](./flaky-test-triage.md)

- `/flaky-test-triage`
- Category: Testing manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Run failing tests repeatedly, classify each failure as flaky or real, and fix only confirmed regressions.

### [Guardrails Learning Loop](./guardrails-learning-loop.md)

- `/guardrails-learning-loop`
- Category: Automation manual
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: When a check fails twice the same way, append a guardrail sign to `.ralph/guardrails.md` so the next iteration avoids repeating it.

### [npm Audit Fix Loop](./npm-audit-fix-loop.md)

- `/npm-audit-fix-loop`
- Category: Security manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Fix high/critical npm audit findings one at a time with test verification, not a blind `npm audit fix --force`.

### [Post-Edit Test Guard](./post-edit-test-guard.md)

- `/post-edit-test-guard`
- Category: Testing event
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: Run related tests after file edits to catch regressions early.

### [Post-Merge Regression Guard](./post-merge-regression-guard.md)

- `/post-merge-regression-guard`
- Category: Testing event
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: Run smoke tests after git merge or rebase to catch integration regressions immediately.

### [Pre-Commit Guard](./pre-commit-guard.md)

- `/pre-commit-guard`
- Category: Testing event
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: Run tests before git commit commands and block commits when the suite is red.

### [Ship PR Until Green](./ship-pr-until-green.md)

- `/ship-pr-until-green`
- Category: CI manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Implement on a branch, run tests, push, open a PR, wait for CI, and loop until checks pass and the PR is ready to merge.
