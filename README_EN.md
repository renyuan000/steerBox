# steerBox

[README.md](./README.md) | [简体中文](./README_CN.md)

Practice Harness Engineering by building software development agents and security agents in a safe, steered environment.

## Positioning

`steerBox` is currently a `learning-first` Harness Engineering project.

Its core question is not simply whether an agent can do work, but how to combine the following into a durable harness:

- long-running execution
- constraint control
- feedback loops
- human-steerable operation and optional takeover
- auditable behavior
- traceable decision and execution flows

## Two Tracks

### 1. Software Development Agents

Current focus:

- long-running automated software development
- code management
- test execution
- refactoring
- development workflow control
- continuous feedback and correction

### 2. Security Agents

Current focus:

- long-running security operations
- long-running security log analysis
- long-running security event detection and incident response
- automated offensive/defensive analysis and vulnerability research
- secure development and security review related scenarios

Notes:

- automation tied to high-impact security actions should default to controlled, auditable, traceable, and interruptible execution
- scenarios that require active response should choose between unattended, passive oversight, active approval, or manual takeover according to risk

## Core Principles

- `safe`
- `steered`
- `auditable`
- `traceable`
- `human-steerable`
- `long-running`
- `feedback-driven`

## Current Status

- the final codebase structure is intentionally not fixed yet
- current candidate languages include `Rust`, `Python`, `C`, and `Go`, but the split is not final
- the immediate focus is on documents, problem framing, and architecture direction before a phase-one prototype

## Documents

- [Harness Principles](./docs/HARNESS_PRINCIPLES.md)
- [Harness Sources](./docs/HARNESS_SOURCES.md)
- [Architecture Direction](./docs/ARCHITECTURE_DIRECTION.md)
- [Architecture Questions](./docs/ARCHITECTURE_QUESTIONS.md)
- [Documentation Map](./docs/DOCUMENTATION_MAP.md)
- [Tracks and Supporting Capabilities](./docs/TRACKS_AND_CAPABILITIES.md)
- [Goal and Loop Engineering Discussion](./docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md)
- [Goal Alignment Checkpoints](./docs/GOAL_ALIGNMENT_CHECKS.md)
- [AI Agent Efficiency Engineering](./docs/AI_AGENT_EFFICIENCY_ENGINEERING.md)
- [Plugin and Extension Architecture Discussion](./docs/PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md)
- [Model Prompt and Checkpoint Policy](./docs/MODEL_PROMPT_AND_CHECKPOINT_POLICY.md)
- [Prompt, Checkpoint, and DriftGuard Templates](./docs/PROMPT_CHECKPOINT_TEMPLATES.md)
- [Checkpoint Policy Algorithm](./docs/CHECKPOINT_POLICY_ALGORITHM.md)
- [Side Effect Ledger Discussion](./docs/SIDE_EFFECT_LEDGER_DISCUSSION.md)
- [Drift Guard Discussion](./docs/DRIFT_GUARD_DISCUSSION.md)
- [Reference Frameworks and Observability Notes](./docs/REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md)
- [Agent Evolution Reading List and Notes](./docs/AGENT_EVOLUTION_READING_LIST.md)
- [Agent Evolution Innovation Synthesis](./docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md)
- [Storage and Recovery Discussion](./docs/STORAGE_AND_RECOVERY_DISCUSSION.md)
