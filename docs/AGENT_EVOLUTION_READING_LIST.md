# Agent Evolution Reading List and Notes

## 文档目的

本文件整理 `steerBox` 后续学习智能体进化工程时值得优先阅读的资料。

关注链路：

```text
prompt -> context -> memory -> harness -> goal -> loop engineering -> agent efficiency
```

本文件不是最终架构方案，也不代表所有资料都应直接采用。它的作用是给后续学习、设计和实现提供参考地图。

如果要看“这些资料具体怎么做、创新点是什么、`steerBox` 如何融合成开放功能”，请继续阅读：

- [Agent Evolution Innovation Synthesis](./AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md)

## 总体判断

当前 AI 智能体工程确实正在从 prompt 技巧演进到系统工程：

- `Prompt Engineering`: 让模型按预期表达和推理
- `Context Engineering`: 让模型在正确时刻看到正确信息
- `Memory Engineering`: 让经验、事实、状态跨任务复用
- `Harness Engineering`: 给 agent 工具、权限、状态、验证、恢复和运行环境
- `Goal Engineering`: 把目标、边界、成功标准、风险约束变成契约
- `Loop Engineering`: 控制 agent 如何观察、行动、验证、反思、更新和继续
- `Efficiency Engineering`: 用更低时间、token、工具调用、系统资源和人工介入达成更高质量结果
- `Evolution Engineering`: 基于运行证据持续改进 harness、tools、skills、memory、policy、model routing 和 evaluator

`推断，高置信`：`steerBox` 应继续坚持 Harness Engineering 总框架，但把 Goal、Loop、Efficiency 和 Evolution 作为核心控制层与优化层。

## 一、国际高价值资料

### 1. OpenAI: Harness Engineering

来源：

- https://openai.com/index/harness-engineering/

核心价值：

- Humans steer, agents execute
- repository knowledge as system of record
- AGENTS.md 应更像目录，而不是百科全书
- logs / metrics / traces / UI 都应对 agent 可见
- 机械化约束比文档口号更可靠
- entropy 需要持续 garbage collection

对 `steerBox` 的启发：

- 文档、规则、工具、观测、验证都应进入 repo / harness 体系
- 智能体效率不只是模型能力，而是环境和反馈闭环能力
- 需要把“人类掌舵”扩展为多模式：无人值守、被动观察、主动审批、人工接管

### 2. OpenAI: Unlocking the Codex Harness

来源：

- https://openai.com/index/unlocking-the-codex-harness/

核心价值：

- agent harness 应提供可运行环境
- agent 需要通过实际执行、日志、调试和验证闭环完成任务
- app/server/runtime 本身是 harness 的一部分

对 `steerBox` 的启发：

- 不能只做 prompt wrapper
- supervisor、runtime、storage、checkpoint、tooling、observability 都属于 harness

### 3. Anthropic: Effective Harnesses for Long-Running Agents

来源：

- https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents

核心价值：

- 长任务不能只靠上下文窗口
- structured handoff 很关键
- 长运行需要明确进度工件、初始化脚本、任务状态和交接机制

对 `steerBox` 的启发：

- 长时任务必须有 durable state
- handoff artifact、event log、checkpoint 是基础设施，不是后补功能

### 4. Anthropic: Harness Design for Long-Running Application Development

来源：

- https://www.anthropic.com/engineering/harness-design-long-running-apps

核心价值：

- sprint / contract / evaluator 可以提高长任务质量
- done 应提前定义
- generator 与 evaluator 职责应分离
- context reset / evaluator 是否启用取决于模型能力和任务边界

对 `steerBox` 的启发：

- `GoalContract` 和 `LoopController` 是一等对象
- evaluator 应按风险、成本、模型能力动态启用，不是永远开或永远关

### 5. LangSmith / LangGraph / LangChain 生态

来源：

- https://docs.langchain.com/langsmith/observability
- https://docs.langchain.com/langsmith/evaluation
- https://docs.langchain.com/oss/python/langgraph/persistence
- https://docs.langchain.com/oss/python/langgraph/interrupts

核心价值：

- tracing / observability
- offline / online evaluation
- experiment comparison
- persistence / checkpointer / store
- interrupt / resume
- human review / feedback loop

对 `steerBox` 的启发：

- 可追踪、可验证、可恢复、可对比应成为基础能力
- 不应把 observability 外包给 SaaS，但应吸收 LangSmith 的产品化思路
- 不应把 graph 作为唯一执行模型，但应吸收 LangGraph 的 persistence 和 interrupt 思路

### 6. Model Context Protocol

来源：

- https://modelcontextprotocol.io/specification/2025-06-18

核心价值：

- 标准化 tools / resources / prompts 的接入方式
- host / client / server 分层
- tool safety、user consent、data privacy 是协议级关注点

对 `steerBox` 的启发：

- MCP 是能力接入层，不是信任边界
- MCP tool 必须进入 capability、policy、audit、trace、evolution 体系

## 二、智能体自进化与 Harness 进化资料

### 1. Agentic Harness Engineering

来源：

- https://arxiv.org/abs/2604.25850

核心价值：

- observability-driven automatic evolution of coding-agent harnesses
- component observability：每个可编辑 harness 组件显式化、可回滚
- experience observability：把大量 trajectory token 压缩成可钻取证据
- decision observability：每次编辑带预测，后续用结果验证
- 论文摘要报告：收益主要来自 tools、middleware、long-term memory，而不是 system prompt

对 `steerBox` 的启发：

- 自动进化必须基于证据、预测、验证、回滚
- 进化对象应优先是 tool、middleware、memory、policy、loop，而不是只改 prompt

### 2. Self-Harness

来源：

- https://arxiv.org/abs/2606.09498

核心价值：

- harness 可以基于执行 traces 发现模型特定弱点
- 通过 Weakness Mining -> Harness Proposal -> Proposal Validation 形成改进循环
- 候选修改必须通过 regression testing 才接受

对 `steerBox` 的启发：

- 不同模型需要不同 harness 策略
- `ModelRouter` 和 `ModelProfile` 应记录模型弱点与适配策略
- 现场错误可以触发候选改进，但不能无验证直接全局生效

### 3. Continual Harness

来源：

- https://arxiv.org/abs/2605.09998

核心价值：

- agent 在单次长运行中持续改进 prompt、sub-agents、skills、memory
- online adaptation，不依赖每次 reset 后重新开始
- 从历史 trajectory 中改进当前 harness

对 `steerBox` 的启发：

- 进化不应只按天或按周批处理
- 运行中的现场反馈也应进入进化候选
- 但实时改动需要 policy gate、验证和回滚

### 4. SkillHone

来源：

- https://arxiv.org/abs/2606.08671

核心价值：

- skill 进化需要 persistent decision history
- 不只保留最终 skill，还要保留诊断、修订、证据、结果和被拒绝方案
- 用角色分离的 subagents 测试 skill 候选并提出修订

对 `steerBox` 的启发：

- `skill` 不是静态 prompt 文件
- 每次 skill 修订都应有 decision history
- 工具使用错误、人工纠偏和失败样例都可以沉淀成 skill 改进证据

### 5. Recursive Agent Harnesses

来源：

- https://arxiv.org/abs/2606.13643

核心价值：

- 递归单元不是模型调用，而是完整 agent harness
- parent agent 可以生成并运行脚本，调度 subagent harnesses 并行处理细粒度任务

对 `steerBox` 的启发：

- subagent 不是简单聊天角色
- 子 agent 也应运行在可审计、可恢复、可约束的 harness 中
- supervisor / AgentProcess / trace tree 需要从第一阶段考虑

### 6. Meta-Engineering Harnesses

来源：

- https://arxiv.org/abs/2605.25665

核心价值：

- 面向 AI-native software production 的 contract-driven verification architecture
- operational requirements -> explicit contracts
- role-specialized agents
- independent and adversarial verification
- structured failure classification and outer-loop calibration

对 `steerBox` 的启发：

- `GoalContract` 与 adversarial evaluator 很关键
- 失败要分类，不能只记“失败了”
- 外层校准循环应服务长期质量和效率

### 7. Reflexion

来源：

- https://arxiv.org/abs/2303.11366

核心价值：

- 不更新模型权重，而是用语言反馈和 episodic memory 改进后续决策
- 从 trial-and-error 中生成 reflective text

对 `steerBox` 的启发：

- 现场错误可以转成可复用经验
- 但反思文本必须绑定证据和适用边界，不能直接变成硬规则

### 8. Voyager

来源：

- https://arxiv.org/abs/2305.16291

核心价值：

- open-ended lifelong learning agent
- ever-growing skill library
- execution errors and self-verification drive program improvement
- skills are executable, interpretable, compositional

对 `steerBox` 的启发：

- skill library 是智能体进化的重要形式
- 工具错误、执行错误和自验证应进入 skill/tool/memory 改进闭环

## 三、大陆 / 中文生态相关高价值资料

### 1. A Survey on Self-Evolution of Large Language Models

来源：

- https://arxiv.org/abs/2404.14387
- https://github.com/AlibabaResearch/DAMO-ConvAI/tree/main/Awesome-Self-Evolution-of-LLM

核心价值：

- 把 self-evolution 分为 experience acquisition、experience refinement、updating、evaluation
- 覆盖 LLM 和 LLM-based agents 的自进化目标与模块
- 提供系统性 taxonomy

对 `steerBox` 的启发：

- 我们的现场自进化也可以落到这四步：获取经验、提炼经验、更新候选、验证结果
- 但 `steerBox` 应更偏工程化：工具用法、MCP、skill、policy、loop、trace、checkpoint

### 2. Long Term Memory: The Foundation of AI Self-Evolution

来源：

- https://arxiv.org/abs/2410.15665

核心价值：

- 强调长期记忆是 AI self-evolution 的基础
- 关注 interaction data 的保存、管理、表征和复用
- 讨论 inference-time self-evolution，不只依赖大规模训练

对 `steerBox` 的启发：

- “梦境学习工程”应以长期记忆、经验压缩、冲突检测、检索和策略候选为核心
- 原始事件、episode summary、pattern memory、failure memory、tool memory 应分层

### 3. ModelScope-Agent

来源：

- https://arxiv.org/abs/2309.00986

核心价值：

- 面向真实应用的可定制 agent 框架
- 覆盖 tool-use data collection、tool retrieval、tool registration、memory control、customized model training、evaluation
- 连接开源大模型、API 和 ModelScope 社区模型

对 `steerBox` 的启发：

- tool registry、tool retrieval、memory control、evaluation 是工程核心
- 多模型、多工具接入需要统一 registry 和 policy

### 4. AgentScope

来源：

- https://arxiv.org/abs/2402.14034

核心价值：

- developer-centric multi-agent platform
- message exchange 作为通信核心
- 内置 agents、service functions、utility monitor、fault tolerance、tools、external knowledge
- actor-based distributed framework

对 `steerBox` 的启发：

- 多 agent 不只是角色配置，还需要监控、容错、分布式和运行管理
- `minimal supervisor` 与 `AgentProcess` 需要从第一阶段保留位置

### 5. Very Large-Scale Multi-Agent Simulation in AgentScope

来源：

- https://arxiv.org/abs/2407.17789

核心价值：

- 面向大规模 multi-agent simulation 的 actor-based distributed mechanism
- 支持并行执行、多 agent 管理和 web-based monitor

对 `steerBox` 的启发：

- 如果未来有大量 subagent，必须关注资源效率、监控、调度和隔离
- 不能默认常驻大量 sidecar 或 agent

### 6. AgentVerse

来源：

- https://arxiv.org/abs/2308.10848

核心价值：

- multi-agent collaboration
- dynamic group composition
- 讨论 emergent behaviors

对 `steerBox` 的启发：

- 多 agent 协作要可观测和可约束
- subagent 的组织方式应服务任务，而不是为了“多 agent”而多 agent

### 7. CValues

来源：

- https://arxiv.org/abs/2307.09705

核心价值：

- 面向中文语境的安全与责任价值评测
- 包含人工评估与自动评估

对 `steerBox` 的启发：

- 中文/大陆场景需要自己的安全、责任、合规评估集
- 安全智能体的 evaluator 不应只用英文 benchmark

## 四、你的“实时现场自进化”是否已有？

你的想法：

> 如果 agent 用错工具，或不熟悉工具却没先查帮助说明就乱用，应直接检测到错误，记录到教训里，并更新相关工具使用用法记录，下次直接查记录正确使用。

### 结论

这不是完全没人提出过的方向，但你的表述有明确工程价值，而且更偏“现场操作级自进化”。

更准确地说：

- `已有相近思想`：Reflexion、Voyager、AHE、Self-Harness、SkillHone、Continual Harness 都包含从错误、轨迹、反馈、工具执行结果中改进 agent/harness/skills/memory 的思想。
- `你的强化点`：把“工具使用错误”作为一等事件，在错误发生现场立即生成结构化教训，并更新工具使用 recipe / skill / policy / help-first rule，让下一次工具调用直接受益。
- `可能的创新空间`：把这种机制工程化为低延迟、可审计、可回滚、可验证的 `Real-time Operational Self-Evolution`，尤其针对 tool / MCP / shell / file-edit / model-routing / code-index 这些具体执行层。

### 为什么不是简单“每天/每周进化”

每天/每周进化更像 batch consolidation：

```text
collect traces -> summarize failures -> update skills/docs/policies -> evaluate -> promote
```

你的现场进化是 online micro-evolution：

```text
error detected -> classify -> write lesson candidate -> verify recipe -> update local tool usage record -> use next time
```

这两者都需要。

建议 `steerBox` 同时支持：

- `real-time micro-evolution`: 针对明确、低风险、可验证的局部错误，立即生成候选教训或更新本地 recipe
- `daily consolidation`: 汇总一天的错误、重复模式、低效路径，合并重复经验
- `weekly architecture evolution`: 处理更大范围的 harness、policy、plugin、memory、model routing 变化

### 现场自进化的边界

不能所有错误都自动改规则。

建议分级：

#### Level 0: Record Only

只记录，不改变行为。

适用：

- 原因不明
- 可能是偶发环境问题
- 证据不足

#### Level 1: Lesson Candidate

生成候选教训，下次可提醒，但不强制。

适用：

- 工具调用失败
- 参数格式错误
- 忘记先查 help
- 输出解析错误

#### Level 2: Verified Recipe Update

验证后更新工具使用记录。

适用：

- 正确用法已通过最小调用验证
- 错误模式明确
- 影响范围局部

#### Level 3: Skill / Policy Proposal

生成 skill 或 policy 修改提案，需要评估或人工批准。

适用：

- 反复发生的错误
- 涉及权限、安全、写文件、提交、网络、MCP 配置

#### Level 4: Harness Change

修改 harness 行为，需要 regression test 和回滚方案。

适用：

- loop policy
- model routing
- checkpoint policy
- tool registry
- memory policy

## 五、建议 steerBox 新增的能力

### 1. ToolUseErrorEvent

每次工具错误都结构化记录：

```text
event_id
tool_name
attempted_args
error_type
error_message
missing_precheck
help_was_checked
expected_usage
actual_usage
corrected_usage
verification_command
lesson_candidate
confidence
risk_level
```

### 2. LessonCandidate

现场生成候选教训：

```text
lesson_id
source_event_id
problem_pattern
correct_usage
when_to_apply
when_not_to_apply
evidence
verification_status
promotion_status
```

### 3. ToolUsageRecipe

验证后的工具用法记录：

```text
tool_name
recipe_version
valid_command_shape
required_precheck
common_errors
safe_examples
unsafe_examples
last_verified_at
rollback_version
```

### 4. Real-time Evolution Loop

建议最小闭环：

```text
observe tool failure
  -> classify failure
  -> check help/docs
  -> retry minimal correct usage
  -> record verified lesson
  -> update recipe candidate
  -> use recipe before next call
  -> aggregate daily
```

### 5. Promotion Gate

现场教训不能直接变全局规则。

建议：

- 单次错误 -> candidate
- 验证成功 -> local recipe
- 多次出现 -> skill update proposal
- 涉及安全/权限/全局行为 -> human approval
- 修改 harness -> regression evaluation

## 六、对 steerBox 的文档建议

建议后续新增：

- `docs/REAL_TIME_SELF_EVOLUTION_DISCUSSION.md`
- `docs/TOOL_USAGE_RECIPE_EVOLUTION.md`
- `docs/MEMORY_AND_DREAM_LEARNING_DISCUSSION.md`

其中最该优先补的是：

- `docs/REAL_TIME_SELF_EVOLUTION_DISCUSSION.md`

原因：它正好把你的现场自进化思想落到工程对象上，也能连接现有文档：

- `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`
- `AI_AGENT_EFFICIENCY_ENGINEERING.md`
- `PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md`
- `REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md`
- `STORAGE_AND_RECOVERY_DISCUSSION.md`

## 当前结论

你的“实时现场自进化”不是凭空孤立的新概念，它和 Reflexion、Voyager、AHE、Self-Harness、SkillHone、Continual Harness 等方向有明显交集。

但你的具体表述有独立工程价值：

> 把 agent 现场犯错，特别是工具使用错误，转化为可验证、可审计、可回滚的实时工具用法进化机制。

这比“每天/每周统一总结”更激进，也更适合 `steerBox` 的方向。

建议把它作为 `steerBox` 的一个明确横向能力：

> Real-time Operational Self-Evolution

即：现场错误立即结构化、局部验证、低风险局部生效、高风险进入审批和回归验证，最终通过日/周级 consolidation 合并进长期 memory、skill、tool recipe 和 harness policy。
