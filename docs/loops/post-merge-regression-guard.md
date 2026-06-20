# Post-Merge Regression Guard

- Slug: `post-merge-regression-guard`
- Category: Testing event
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: Run smoke tests after git merge or rebase to catch integration regressions immediately.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.
  - Do not weaken, delete, or skip tests to make the suite pass.
  - Do not replace real assertions with trivial always-pass tests.
  - Prefer fixing production code over patching tests to go green.

- Source: https://loops.elorm.xyz/loops/post-merge-regression-guard
