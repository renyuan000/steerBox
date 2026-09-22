# Phase 1 Implementation Scope

## 文档目的

本文件用于把 `steerBox` 第一阶段实现范围收敛成可执行边界。

它不替代已有讨论稿。已有文档继续保存长期设计、参考资料和未来能力；本文件只回答第一阶段真正要做什么、暂不做什么、哪些只做占位，以及如何判断第一阶段成功。

## 核心判断

第一阶段不应该做成只能演示的 demo。

但第一阶段也不应该一次性实现完整的自适应多模型路由、插件市场、向量记忆、图记忆、dream learning、自动进化、四个专业方向的完整闭环和通用 Manus-like agent。

更合理的目标是：

```text
用最小但真实可扩展的 harness core，
跑通一个软件开发智能体垂直切片，
同时为持续运维、安全评估与漏洞研究、安全运营和长期演进能力保留清晰接口。
```

换句话说：

- 第一阶段要验证 `steerBox` 的核心控制模型是否成立
- 第一阶段优先服务软件开发智能体
- 其他三个方向先保留 track 边界、授权/policy 边界和 side-effect 边界
- 通用全功能智能体暂不作为第一阶段目标

### 长期运行目标与 Phase 1 实现边界

架构目标是四个专业方向都支持 7x24 常驻和多年可恢复生命周期；这不是 Phase 1 已实现能力的声明。Phase 1 只实现本地单机、顺序执行、持久化事件、任务状态和最小 `RecoveryCheckpoint`，用软件开发垂直切片验证恢复契约。

Phase 1 不实现分布式 worker、高可用故障转移、完整安全运营常驻服务、自动渗透测试执行链或自动安全处置闭环。后续方向必须复用共享 Harness 的任务、证据、授权、审批和恢复接口，而不能另起一套不可审计的常驻循环。

## 第一阶段必须实现

### 1. Harness Core 最小运行底座

第一阶段必须有一个通用核心，而不是直接写死某个开发 agent。

最小对象包括：

- `GoalContract`
- `LoopController`
- `ToolRegistry`
- `PolicyGate`
- `EventLog`
- `TraceRecord`
- `RecoveryCheckpoint`
- `SideEffectLedger`
- `HumanSteeringMode`

这些对象可以先是简单 schema 和本地实现，但边界必须清楚。

### 2. 软件开发智能体垂直切片

第一阶段主验证对象建议是软件开发智能体。

原因：

- 容易构造真实任务
- 容易验证结果
- 文件 diff、测试、lint、review、rollback 都有明确证据
- 比安全响应动作更适合作为早期闭环验证场景

最小能力包括：

- 读取仓库上下文
- 生成或维护 `GoalContract`
- 执行有限工具调用
- 修改文件
- 运行验证命令
- 记录每轮事件和工具调用
- 记录修改前后边界
- 失败后产生可审查的下一步建议

### 3. 持久化事件日志

第一阶段必须把关键执行过程落盘。

最小记录包括：

- `session_id`
- `loop_run_id`
- `step_id`
- `message_id`
- `tool_call_id`
- tool 输入输出摘要
- 文件修改摘要
- 验证命令与结果
- checkpoint 引用
- side effect 分类

事件日志是审计、恢复、轨迹检查、记忆候选和后续自动进化的基础。

### 4. Recovery Checkpoint 最小实现

第一阶段不需要 OS 级完整 checkpoint。

必须实现的是：

- 任务状态 checkpoint
- loop 状态 checkpoint
- 文件 diff 或 workspace 修改边界
- checkpoint metadata
- checkpoint 与 event log 的关联

第一阶段重点不是“能恢复一切”，而是明确知道：

- 哪些能恢复
- 哪些不能恢复
- 为什么不能恢复
- 哪些外部副作用需要人工处理或补偿

### 5. Goal Alignment Checkpoint 最小实现

第一阶段必须有方向检查。

最小实现可以是规则化检查或人工可读报告，而不是复杂 evaluator。

至少检查：

- 当前动作是否服务 root goal
- 当前阶段是否服务 stage goal
- 新增修改是否超出 scope
- 是否把辅助问题升级成主任务
- 是否需要人类确认

### 6. DriftGuard 最小实现

第一阶段必须防止 agent 把项目改乱。

最小实现包括：

- 检查修改文件范围
- 检查 diff 规模
- 检查是否触碰核心文档、配置或规则
- 检查是否出现大规模重命名、重构或风格 churn
- 高风险 diff 要求人工确认或 checkpoint

### 7. Human Steering Mode 最小实现

第一阶段至少支持以下模式语义：

- `human_steerable_loop`
- `human_in_the_loop`
- `manual_takeover`

可以暂不实现完整 UI，但事件和状态中必须记录当前模式。

默认建议：

```text
normal dev task -> human_steerable_loop
high-risk file/tool action -> human_in_the_loop
manual intervention -> manual_takeover
```

### 8. Context Hygiene 最小实现

第一阶段不要求完整自动 context pruning。

但必须从一开始记录：

- 哪些上下文被使用
- 哪些上下文被判断为关键
- 哪些上下文可能污染当前任务
- compact / summary 的来源和置信度

这为后续精准上下文清理和低 token 运行打基础。

### 9. Baseline Tool Recipes

第一阶段应预置少量高频工具正确用法。

目标不是做完整 skill 系统，而是避免 agent 在基础工具上反复犯低级错误。

第一批建议覆盖：

- shell 安全引用
- `rg` 固定字符串搜索
- git 状态检查
- 最小 diff 查看
- 测试命令记录
- Markdown 链接检查

### 10. 多模型接入最小垂直切片

第一阶段不能把多模型只写成未来愿景，至少要实现可验证的静态接入边界：

- ProviderProfile、EndpointProfile、ModelProfile、PromptProfile、AgentRolePrompt、TaskPromptPack、ResolvedPromptPack schema
- 静态 ModelRegistry 和 PromptRegistry，允许配置 cheap/fast、strong/slow、specialized、evaluator 或 local profile
- 至少一个真实 model adapter；其他模型可以是未连接 live endpoint 的可验证 fixture
- 基于任务类型、能力、风险、成本和延迟约束的规则式 RoutePolicy
- 每次路由和调用写入 RouteDecision、ModelCallRecord、ResolvedPromptPack 版本和验证状态
- fallback 必须能力兼容、显式记录并重新验证；高风险写操作不能静默降级
- registry、prompt、trace、event log 和 checkpoint 只保存 auth_ref，不保存 API key 或 token

### 11. Portfolio、Task 与 BoardProjection 最小切片

第一阶段只实现单 Portfolio、单 Project、本地顺序执行 fixture，但必须先固定未来可扩展的身份和投影边界：

- 定义 Portfolio、Project、Task、TaskRun、Attempt、TaskDependency schema
- 定义 Task 领域状态及合法迁移
- 从 EventLog + StateStore 重建 BoardProjection
- 提供只读 CLI / JSON board fixture
- 不实现真实多项目并行调度、worktree lease、分布式 worker 或交互式 TUI/Web

Phase 1 的对象、事件、状态迁移、幂等、版本冲突、投影和敏感数据规则以 PHASE_1_CONTRACTS.md 为准。

## 第一阶段只做占位

以下能力第一阶段必须在 schema 或接口上留位置，但不要求完整实现：

- 历史表现驱动的自适应多模型路由
- 动态插件安装
- MCP 自动发现与治理
- 向量记忆库
- 图记忆库
- dream learning
- 自动 skill 生成
- 自动 policy 进化
- 跨项目全局记忆
- 完整交互式 TUI/Web control plane
- 持续运维、安全评估与漏洞研究、安全运营的完整运营闭环
- 通用 Manus-like agent

占位的要求是：

- 不写死未来无法扩展的结构
- 不把未来能力伪装成当前已实现
- 不让占位阻塞第一阶段交付

## 第一阶段明确不实现

第一阶段不实现：

- 插件市场
- 企业级多租户
- 分布式调度
- 完整 sandbox / process checkpoint
- 自动渗透测试执行链
- 自动安全处置闭环
- 跨多个真实项目的自动联调
- 完整向量检索和图推理系统
- 完整 self-evolution runtime

这些能力保留在长期方向中，但不进入第一阶段交付标准。长期方向的正式划分和验收依赖见 `AGENT_RND_ROADMAP.md`、`CONTINUOUS_OPERATIONS_AGENT_DESIGN.md`、`SECURITY_ASSESSMENT_AGENT_DESIGN.md` 和 `SECURITY_OPERATIONS_AGENT_DESIGN.md`。

## 第一阶段推荐技术边界

技术选择不在本文件中最终锁定。

但第一阶段实现应遵守以下边界：

- 本地优先
- 可审计优先
- SQLite / JSONL 足够时不引入复杂外部依赖
- 文件系统 artifact 可读可迁移
- schema 先稳定，优化后置
- 外部副作用默认保守处理

推荐最小存储组合：

```text
events.sqlite      -> durable event log
state.sqlite       -> current session / loop state
traces/*.jsonl     -> readable trace stream
checkpoints/       -> checkpoint metadata and artifacts
recipes/           -> baseline tool recipes
```

## 第一阶段成功标准

第一阶段成功不以“agent 看起来很智能”为标准。

成功标准应是：

1. 能运行一个真实软件开发任务闭环
2. 能明确记录目标、步骤、工具调用、修改、验证和结果
3. 能在关键点生成或引用 checkpoint
4. 能区分可恢复修改和不可恢复副作用
5. 能发现明显目标偏离或 scope creep
6. 能在高风险动作前要求人类介入
7. 能把失败工具调用沉淀为 tool recipe candidate
8. 能用事件日志复盘一次完整任务
9. 能说明哪些能力只是占位，哪些已经实现

如果这些成立，即使没有复杂 UI、向量记忆、自适应多模型路由和后三个专业方向的完整闭环，第一阶段也是成功的。

## 第一阶段建议实现顺序

建议顺序：

1. 定义核心 schema：goal、loop、event、tool call、checkpoint、side effect
2. 实现本地 event log 和 trace 写入
3. 实现最小 ToolRegistry 和 PolicyGate
4. 实现软件开发任务 loop skeleton
5. 接入文件修改和验证命令记录
6. 实现 RecoveryCheckpoint metadata 和 diff boundary
7. 实现 GoalAlignmentCheckpoint 报告
8. 实现 DriftGuard 最小检查
9. 实现 baseline tool recipes
10. 用一个真实开发任务做端到端验证

## 契约优先说明

实现顺序以 PHASE_1_CONTRACTS.md 为第一入口；如果本文件中的长期建议与契约冲突，先修正文档，再开始代码实现。

第一阶段的完成定义还必须包括：Task 状态迁移可验证、BoardProjection 可由 EventLog 重建、重复 mutating command 不重复产生副作用。

## 与其他文档的关系

- `ARCHITECTURE_DIRECTION.md` 定义项目方向
- `TRACKS_AND_CAPABILITIES.md` 定义四个专业方向与横向能力关系
- `AGENT_RND_ROADMAP.md` 定义四个方向的长期优先级、依赖和验收阶段
- `CONTINUOUS_OPERATIONS_AGENT_DESIGN.md` 定义持续运维方向
- `SECURITY_ASSESSMENT_AGENT_DESIGN.md` 定义安全评估与漏洞研究方向
- `SECURITY_OPERATIONS_AGENT_DESIGN.md` 定义安全运营方向
- `STORAGE_AND_RECOVERY_DISCUSSION.md` 定义长期运行和恢复底座
- `MEMORY_STORAGE_AND_CONSOLIDATION.md` 定义多层记忆和存储演进
- `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md` 定义 goal / loop 模型
- `GOAL_ALIGNMENT_CHECKS.md` 定义方向检查
- `DRIFT_GUARD_DISCUSSION.md` 定义防破坏性漂移
- `SIDE_EFFECT_LEDGER_DISCUSSION.md` 定义副作用边界
- `HUMAN_STEERING_MODES.md` 定义人类驾驭模式

本文件是这些讨论稿进入实现前的第一阶段收敛层。

## 当前结论

`steerBox` 当前文档已经足够支撑进入第一阶段开发。

该结论以 `PHASE_1_CONTRACTS.md`、Scope、Design、TODO、Validation、Documentation Map 和 Architecture Questions 已完成同步为前提；在本轮同步提交落地前，只能视为待闭合的设计结论。

但开工时必须以本文件为实现边界，不能直接按全部长期讨论稿展开。

第一阶段的正确姿势是：

```text
small core,
real loop,
durable evidence,
recoverable boundary,
human-steerable control,
future-ready interfaces.
```
