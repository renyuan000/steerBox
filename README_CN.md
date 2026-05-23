# steerBox

[README.md](./README.md) | [English](./README_EN.md)

在一个安全、可控、可引导的环境中，通过构建软件开发智能体与安全智能体，练习 Harness Engineering。

## 项目定位

`steerBox` 当前首先是一个 `learning-first` 的 Harness Engineering 学习项目。

它关注的核心问题不是“让 agent 会不会做事”，而是如何把下面这些能力稳定组合成一个可持续运行的 harness：

- 长时执行
- 约束控制
- 反馈闭环
- 人在环接管
- 行为可审计
- 决策与执行流程可追踪

## 两条主线

### 1. 软件开发智能体

当前关注：

- 长时间自动化开发
- 代码管理
- 测试执行
- 重构
- 开发工作流控制
- 持续反馈与持续修正

### 2. 安全智能体

当前关注：

- 长时间自动化安全运维
- 长时间自动化安全日志分析
- 长时间自动化安全事件发现与响应
- 自动化攻防分析与漏洞发掘分析
- 安全开发与安全审查相关场景

说明：

- 与高影响安全动作相关的自动化，应默认建立在受控、可审计、可追踪和可介入的前提下
- 需要主动动作的场景，应优先假设存在审批、约束和人在环要求

## 核心原则

- `safe`
- `steered`
- `auditable`
- `traceable`
- `human-in-the-loop`
- `long-running`
- `feedback-driven`

## 当前状态

- 项目暂不预设最终目录结构
- 当前候选语言包括 `Rust`、`Python`、`C`、`Go`，但尚未定稿
- 当前重点是先收敛文档、问题定义和架构方向，再决定第一阶段原型

## 文档

- [Harness Principles](./docs/HARNESS_PRINCIPLES.md)
- [Harness Sources](./docs/HARNESS_SOURCES.md)
- [Architecture Direction](./docs/ARCHITECTURE_DIRECTION.md)
- [Architecture Questions](./docs/ARCHITECTURE_QUESTIONS.md)
