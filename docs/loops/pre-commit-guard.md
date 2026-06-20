# Pre-Commit Guard

- Slug: `pre-commit-guard`
- Category: Testing event
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: Run tests before git commit commands and block commits when the suite is red.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.
  - Do not weaken, delete, or skip tests to make the suite pass.
  - Do not replace real assertions with trivial always-pass tests.
  - Prefer fixing production code over patching tests to go green.

- Source: https://loops.elorm.xyz/loops/pre-commit-guard
