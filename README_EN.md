# steerBox

[README.md](./README.md) | [简体中文](./README_CN.md)

Practice Harness Engineering by building software development agents and security agents in a safe, steered environment.

## Positioning

`steerBox` is currently a `learning-first` Harness Engineering project.

Its core question is not simply whether an agent can do work, but how to combine the following into a durable harness:

- long-running execution
- constraint control
- feedback loops
- human takeover
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
- scenarios that require active response should be designed with policy, approval, and human-in-the-loop constraints in mind

## Core Principles

- `safe`
- `steered`
- `auditable`
- `traceable`
- `human-in-the-loop`
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
