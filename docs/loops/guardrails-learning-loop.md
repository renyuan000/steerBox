# Guardrails Learning Loop

- Slug: `guardrails-learning-loop`
- Category: Automation manual
- Install mode: Includes install files
- Hook status: Hook bundle required; extract the zip into the repo and restart the agent.
- Summary: When a check fails twice the same way, append a guardrail sign to `.ralph/guardrails.md` so the next iteration avoids repeating it.
- Guardrails:
  - Do not modify the check command or exit criteria to force success.
  - Do not skip, disable, or bypass checks to pass the exit condition.
  - If stuck after several iterations, stop and report blockers instead of gaming metrics.

- Source: https://loops.elorm.xyz/loops/guardrails-learning-loop
