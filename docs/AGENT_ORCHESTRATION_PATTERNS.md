# Agent Orchestration Patterns

## 文档目的

本文件用于整理主流和未来更有价值的 AI Agent 编排模式，并说明 `steerBox` 应如何吸收这些模式。

本文件不替代 `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`。

更准确的关系是：

```text
Harness Core
  -> GoalContract
  -> LoopController
  -> PolicyGate / ToolRegistry / Memory / Checkpoint / Trace
  -> Orchestration Pattern
```

也就是说，编排模式决定 agent 如何组织推理、行动、分工和控制流；harness 决定这些行动在什么约束、状态、审计、恢复和人类驾驭边界中运行。

## 核心判断

视频里常见的分类是成立的：

- 单智能体
- ReAct
- 规划与执行
- 反思优化
- 思维树
- 多智能体协作
- 监督者架构
- 路由架构

但这些更像 `orchestration patterns`，不是完整 agent 工程架构。

`steerBox` 不应该只选择其中一种，而应该把它们抽象成可插拔的 `LoopPattern` / `OrchestrationPolicy`。

第一阶段建议只实现简单可控的模式，未来再扩展复杂模式。

## 外部资料精华

### 1. Anthropic Building Effective Agents

参考：https://www.anthropic.com/engineering/building-effective-agents

Anthropic 把常见 agentic systems 区分为 workflows 和 agents。

高价值模式包括：

- prompt chaining
- routing
- parallelization
- orchestrator-workers
- evaluator-optimizer
- autonomous agents

对 `steerBox` 的启发：

- 不要默认所有任务都上复杂 autonomous agent
- 简单任务用 workflow 更稳定、更便宜、更可控
- 复杂任务再用 agent loop
- evaluator-optimizer 适合代码、文档、安全分析的质量闭环
- orchestrator-workers 与 supervisor / subagent 体系高度相关

### 2. LangGraph Multi-Agent Systems

参考：https://langchain-ai.github.io/langgraph/concepts/multi_agent/

LangGraph 强调多智能体可以用不同拓扑组织，例如：

- network
- supervisor
- supervisor with tool-calling
- hierarchical teams
- custom multi-agent workflow

对 `steerBox` 的启发：

- 多智能体不等于自由群聊
- 每个 agent 应有明确能力边界、上下文边界和工具边界
- supervisor 可以集中控制 routing、policy、state 和 handoff
- hierarchical teams 适合大型任务，但第一阶段不应过早实现

### 3. OpenAI Agents SDK

参考：https://openai.github.io/openai-agents-python/

OpenAI Agents SDK 强调：

- agents
- handoffs
- guardrails
- tracing
- sessions
- tools

对 `steerBox` 的启发：

- handoff 应是一等能力，而不是 prompt 里的口头交接
- guardrails 和 tracing 应进入 runtime，而不是只靠提示词
- session state 与 trace 对长任务和恢复非常关键
- agent 编排必须和工具调用、状态、审计绑定

### 4. AutoGen

参考：https://microsoft.github.io/autogen/

AutoGen 强调多 agent conversation、agent teams、工具执行、人类参与和可扩展组件。

对 `steerBox` 的启发：

- 多智能体协作适合复杂任务，但容易失控和耗费 token
- agent 间通信必须被记录、压缩、审计和约束
- group chat 类模式适合探索，不适合第一阶段作为默认执行模型

### 5. ReAct

参考：https://arxiv.org/abs/2210.03629

ReAct 把 reasoning 和 acting 交替结合：模型一边思考，一边调用工具观察外部结果。

对 `steerBox` 的启发：

- ReAct 是工具型 agent 的基本模式
- 工具观察必须进入 event log
- 每次 action 前后都应经过 policy 和 side-effect 分类
- ReAct 如果没有 goal/checkpoint/drift guard，容易在长任务中逐步跑偏

### 6. Tree of Thoughts

参考：https://arxiv.org/abs/2305.10601

Tree of Thoughts 通过显式探索多条推理路径，提高复杂推理和搜索任务质量。

对 `steerBox` 的启发：

- 适合高不确定性规划、漏洞分析、架构方案比较
- 不适合作为所有任务默认模式，因为 token 和时间成本高
- 应作为 `deliberation_mode` 或 `branching_loop`，由风险和难度触发

### 7. Reflexion

参考：https://arxiv.org/abs/2303.11366

Reflexion 通过 verbal feedback / self-reflection 改善后续尝试。

对 `steerBox` 的启发：

- 反思不能只停留在当前上下文里
- 有价值的反思应进入 memory candidate
- 反思写入长期记忆前必须经过验证或人工确认
- 工具误用、测试失败、错误假设都可以触发现场自进化候选

## 编排模式分类

### 1. Single Agent

一个 agent 独立处理 observe / plan / act / verify。

适合：

- 小任务
- 明确边界的代码修改
- 成本敏感任务
- 第一阶段 MVP

风险：

- 能力单点瓶颈
- 长任务容易上下文膨胀
- 自评容易偏宽

`steerBox` 用法：

- 第一阶段默认模式
- 配合 `GoalContract`、`EventLog`、`RecoveryCheckpoint`、`DriftGuard`

### 2. ReAct Loop

reasoning 和 tool action 交替进行。

适合：

- 代码阅读
- 调试
- 搜索资料
- 运行测试
- 安全日志分析中的调查步骤

风险：

- 工具调用过多
- 没有明确 stop condition 时会空转
- 容易把局部发现升级成主任务

`steerBox` 用法：

- 作为基础 tool loop
- 每次 action 必须记录 tool call、observation、side effect
- 配合 `LoopTrajectoryCheck` 检查过去 N 步是否合理

### 3. Plan and Execute

先生成计划，再按步骤执行。

适合：

- 多文件代码任务
- 架构改造
- 安全调查流程
- 长程任务

风险：

- 初始计划可能过早冻结错误路径
- 执行阶段可能机械执行过期计划
- 计划太细会浪费 token 并降低适应性

`steerBox` 用法：

- 计划必须绑定 `GoalContract`
- 每个阶段有 stage goal 和验证条件
- 计划可更新，但更新要记录原因

### 4. Reflection / Evaluator-Optimizer

执行后由自己、另一个模型或 evaluator 检查，再修正。

适合：

- 代码质量提升
- 文档质量提升
- 安全审查
- 测试失败修复

风险：

- 同一个模型自评可能偏宽
- evaluator 成本可能高
- 反复优化可能导致 churn

`steerBox` 用法：

- evaluator 不是每轮都开
- 由风险、失败次数、diff 规模和质量要求触发
- 反思结果先进入 memory candidate，不直接变成 policy

### 5. Tree / Graph of Thoughts

探索多个候选路径，再选择或合并。

适合：

- 高难度 bug 定位
- 安全漏洞成因分析
- 架构方案比较
- 多策略攻防推演的受控分析

风险：

- token 成本高
- 分支状态难管理
- 多路径可能引入更多错误假设

`steerBox` 用法：

- 只在高价值任务中开启
- 每个分支有 branch id、成本预算和停止条件
- 分支选择必须记录依据

### 6. Router Pattern

根据任务类型、风险、成本、模型能力或工具能力，把请求路由给不同 agent / model / workflow。

适合：

- 多模型协同
- 成本优化
- 开发 / 安全 / 文档 / 测试任务分流
- 不同风险级别策略切换

风险：

- 路由错误导致低能力模型处理高风险任务
- 过度路由增加延迟和复杂度
- 路由依据不可见会降低可审计性

`steerBox` 用法：

- 路由必须基于 `ModelProfile`、`TaskProfile`、`RiskLevel`、`CostBudget`
- 路由结果进入 trace
- 高风险任务不能只由成本最低模型决定

### 7. Supervisor / Orchestrator-Workers

一个 supervisor 负责拆解任务、分配 worker、收集结果、做最终决策。

适合：

- 大型代码任务
- 多领域安全分析
- 多工具长程任务
- 未来通用智能体

风险：

- supervisor 可能成为瓶颈
- worker 间上下文同步成本高
- 如果没有统一 event log，复盘困难

`steerBox` 用法：

- supervisor 不等于不受控的总指挥
- supervisor 本身也必须受 `PolicyGate`、`GoalContract`、`Trace` 约束
- worker 只能拿到最小必要上下文和工具权限

### 8. Multi-Agent Collaboration

多个 agent 以角色分工、对话、投票或互审方式协作。

适合：

- 代码作者 + reviewer
- 攻击模拟 + 防御分析的受控对照
- 架构设计 + 实现 + 测试分工
- 多视角审查

风险：

- token 消耗高
- 群聊容易发散
- 责任边界不清
- agent 之间可能互相强化错误假设

`steerBox` 用法：

- 多 agent 默认不是第一阶段核心
- 后续应优先实现 supervisor-controlled multi-agent，而不是自由 group chat
- 每个 agent 的输入、输出、工具、权限和结论都必须可审计

### 9. Hierarchical Teams

多层 supervisor / team 结构。

适合：

- 大型长期项目
- 跨项目联调
- 复杂安全运营
- 未来企业级部署

风险：

- 系统复杂度高
- 状态和记忆一致性难
- trace 和权限管理成本高

`steerBox` 用法：

- 第一阶段只保留数据结构占位
- 后续必须建立在稳定的 supervisor、event log、memory、policy 和 checkpoint 之上

### 10. Event-Triggered Loops

由事件触发 agent loop，例如文件变化、测试失败、告警输入、定时任务、用户介入。

适合：

- 长时间自动化开发
- 长时间安全日志分析
- 持续集成与回归检查
- 自动化观察和响应

风险：

- 事件风暴
- 重复处理
- 触发条件过宽导致资源浪费
- 响应动作越权

`steerBox` 用法：

- 事件触发必须进入 `EventLog`
- 需要去重、节流、优先级和 risk gate
- 高影响事件只能触发建议或审批，不能默认自动处置

## 推荐的 steerBox 抽象

### LoopPattern

```text
LoopPattern:
  id
  name
  pattern_type
  default_goal_contract_policy
  default_tool_policy
  default_checkpoint_policy
  default_memory_policy
  default_human_steering_mode
  cost_profile
  risk_profile
  stop_conditions
```

### OrchestrationPolicy

```text
OrchestrationPolicy:
  allowed_patterns
  default_pattern
  escalation_rules
  routing_rules
  evaluator_rules
  branch_budget
  max_parallel_agents
  max_tool_calls_per_step
  max_loop_steps
```

### AgentRole

```text
AgentRole:
  role_id
  capability_scope
  allowed_tools
  memory_access_scope
  write_permissions
  risk_limit
  handoff_contract
```

### HandoffContract

```text
HandoffContract:
  from_agent
  to_agent
  reason
  goal_state_summary
  completed_steps
  open_questions
  relevant_events
  checkpoint_ref
  side_effect_summary
  expected_next_action
```

## 第一阶段建议

第一阶段不建议实现全部模式。

建议只落地：

1. `single_agent_loop`
2. `react_tool_loop`
3. `plan_execute_loop`
4. `reflection_after_failure`
5. `manual_or_policy_router`

第一阶段只占位：

- tree / graph of thoughts
- full multi-agent collaboration
- supervisor-workers
- hierarchical teams
- event-triggered long-running loops
- historical-performance-driven adaptive routing across many models

原因：

- 第一阶段目标是验证 harness core，不是验证所有 agent 编排花样
- 多 agent 和 tree search 会显著增加 token、状态、trace 和恢复复杂度
- 没有稳定 event log / checkpoint / policy gate 时，复杂编排会放大风险

多项目、多任务、并行 worker、BoardProjection 和 ControlPlane 的一等对象见 `CONTROL_PLANE_DISCUSSION.md`。本文件只负责编排模式；Scheduler、TaskRun 生命周期、看板投影和控制面命令不在这里重复定义。

## 对 steerBox 的最终判断

`steerBox` 需要这个文档。

原因：

- 它把视频中提到的 agent 架构分类放到正确层级
- 它防止我们把 ReAct、多 agent、supervisor、router 混成一套不可控大系统
- 它为未来主流 agent 能力预留接口
- 它帮助第一阶段保持克制：先做可靠 core，再逐步增加编排能力

当前推荐定位：

```text
Orchestration patterns are pluggable strategies above LoopController,
not replacements for Harness Core.
```
