# A11y Audit Until Clean

- Slug: `a11y-audit-until-clean`
- Category: Quality manual
- Install mode: Prompt only
- Hook status: No hooks required.
- Summary: Run automated accessibility checks on changed routes, fix violations, and repeat until the audit is clean.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.

- Source: https://loops.elorm.xyz/loops/a11y-audit-until-clean
