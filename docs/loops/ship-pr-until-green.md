# Ship PR Until Green

- Slug: `ship-pr-until-green`
- Category: CI manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Implement on a branch, run tests, push, open a PR, wait for CI, and loop until checks pass and the PR is ready to merge.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.
  - Do not weaken, delete, or skip tests to make the suite pass.
  - Do not replace real assertions with trivial always-pass tests.
  - Prefer fixing production code over patching tests to go green.

- Source: https://loops.elorm.xyz/loops/ship-pr-until-green
