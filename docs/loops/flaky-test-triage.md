# Flaky Test Triage

- Slug: `flaky-test-triage`
- Category: Testing manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Run failing tests repeatedly, classify each failure as flaky or real, and fix only confirmed regressions.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.
  - Do not weaken, delete, or skip tests to make the suite pass.
  - Do not replace real assertions with trivial always-pass tests.
  - Prefer fixing production code over patching tests to go green.

- Source: https://loops.elorm.xyz/loops/flaky-test-triage
