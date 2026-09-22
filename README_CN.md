# steerBox

[README.md](./README.md) | [English](./README_EN.md)

在一个安全、可控、可引导的环境中，通过软件开发、持续运维、安全评估与漏洞研究、安全运营四个专业方向，练习 Harness Engineering。

## 项目定位

`steerBox` 当前首先是一个 `learning-first` 的 Harness Engineering 学习项目。

它关注的核心问题不是“让 agent 会不会做事”，而是如何把下面这些能力稳定组合成一个可持续运行的 harness：

- 长时执行
- 约束控制
- 反馈闭环
- 支持人在环、无人值守、被动观察、主动审批与人工接管
- 行为可审计
- 决策与执行流程可追踪

## 四个专业方向

### 1. 软件开发智能体

当前关注：

- 长时间自动化开发
- 代码管理
- 测试执行
- 重构
- 开发工作流控制
- 持续反馈与持续修正

### 2. 持续运维智能体

当前关注：

- 服务健康、可用性、性能、容量和成本治理
- 部署和变更观察
- 故障定位、恢复编排和恢复后验证
- 混合云资源治理

### 3. 安全评估与漏洞研究智能体

当前关注：

- 授权安全审查与攻击面分析
- 漏洞研究与受控 PoC 模拟验证
- 授权渗透测试与攻击路径验证
- 有证据的修复复测

### 4. 安全运营智能体

当前关注：

- 持续接入并关联日志、告警、资产和威胁情报
- 调查漏洞、后门、病毒和 APT 相关线索
- 编排受控验证和分级处置
- 允许用户随时查询、审批、暂停、恢复或接管长期调查

说明：

- 与高影响安全动作相关的自动化，应默认建立在受控、可审计、可追踪和可介入的前提下
- 需要主动动作的场景，应按风险等级决定无人值守、被动观察、主动审批或人工接管模式

## 核心原则

- `safe`
- `steered`
- `auditable`
- `traceable`
- `human-steerable`
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
- [Documentation Map](./docs/DOCUMENTATION_MAP.md)
- [Agent R&D Roadmap](./docs/AGENT_RND_ROADMAP.md)
- [Continuous Operations Agent Design](./docs/CONTINUOUS_OPERATIONS_AGENT_DESIGN.md)
- [Security Assessment Agent Design](./docs/SECURITY_ASSESSMENT_AGENT_DESIGN.md)
- [Security Operations Agent Design](./docs/SECURITY_OPERATIONS_AGENT_DESIGN.md)
- [Shared Harness Security Model](./docs/SECURITY_MODEL_DISCUSSION.md)
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
