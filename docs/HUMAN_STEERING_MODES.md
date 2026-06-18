# Human Steering Modes Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 中人类介入、审批、观察、纠偏和接管的运行模式。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

它回答的问题是：

- `human-in-the-loop`、`human-steerable`、`manual takeover`、`autonomous loop` 分别是什么意思
- 自动化运行时，人类是否仍能观察、暂停、审批、纠偏和接管
- 不同风险任务应如何选择不同的人机协作模式
- 人类介入如何与 `GoalContract`、`LoopController`、`GoalAlignmentCheckpoint`、`RecoveryCheckpoint`、`PolicyGate` 和审计系统联动

## 核心判断

`steerBox` 不应把“人在环”作为唯一的人机协作模式。

更合理的设计是：

```text
HumanSteeringMode:
  - autonomous_loop
  - human_steerable_loop
  - human_in_the_loop
  - manual_takeover
```

其中：

- `autonomous_loop`：agent 在预设目标、权限、预算和安全边界内自主推进
- `human_steerable_loop`：agent 自主推进，但人类可随时观察、暂停、修改约束、插入指令或接管
- `human_in_the_loop`：关键节点必须等待人类审批或输入后才能继续
- `manual_takeover`：人类临时或长期接管执行权，agent 退到辅助、记录、建议或等待状态

默认情况下，`steerBox` 的 “auto” 不应表示不受控自动化，而应表示：

```text
autonomous execution inside a human-steerable, policy-gated, auditable loop
```

也就是说，低风险任务可以自动推进，但运行过程仍应可观察、可暂停、可审计、可纠偏、可接管。

## 与主流术语的关系

当前没有一个被所有智能体框架统一采用的标准枚举。

更常见的术语包括：

- `human-in-the-loop` / `HITL`
- `approval workflow`
- `interrupt` / `resume`
- `review and edit`
- `guardrails`
- `handoff`
- `autonomous agent`
- `agent loop`

这些术语覆盖的是局部能力，不一定天然组成完整运行模式。

为避免旧术语和正式 schema 混用，`steerBox` 建议采用以下映射：

| 旧描述 / 常见描述 | `steerBox` 正式枚举 | 含义 |
|---|---|---|
| `unattended` / 无人值守 | `autonomous_loop` | 在授权边界内自主循环执行 |
| `passive oversight` / 被动观察 | `human_steerable_loop` | agent 自主推进，人类可观察和随时介入 |
| `active approval` / 主动审批 | `human_in_the_loop` | 关键节点必须等待人类审批或输入 |
| `manual takeover` / 人工接管 | `manual_takeover` | 人类接管执行权，agent 退到辅助或等待状态 |

`steerBox` 建议保留自己的枚举，是因为它需要同时服务：

- 长时间自动化开发
- 长时间自动化安全运维
- 持续安全日志分析与事件响应
- 自动化攻防与漏洞挖掘分析
- 人类随时从前台接入观察、纠偏和接管

因此，`human-steerable` 更适合作为 `steerBox` 的上位概念：它不是某个固定审批点，而是整个运行系统始终允许人类驾驭。

## 四种模式定义

### 1. autonomous_loop

`autonomous_loop` 表示 agent 可以在已授权边界内自主循环执行。

适用场景：

- 低风险只读分析
- 日志初筛
- 文档整理
- 低影响代码检索
- 可重复执行的测试或格式检查
- 已有明确回滚点的低风险开发任务

关键约束：

- 必须绑定 `GoalContract`
- 必须受 `PolicyGate` 约束
- 必须记录 trace 和事件日志
- 必须有停止条件
- 必须有预算限制
- 必须允许外部控制面观察状态

不允许：

- 无限制运行
- 无审计运行
- 越权调用工具
- 在未授权范围内扩大目标
- 对不可逆外部动作自动执行

### 2. human_steerable_loop

`human_steerable_loop` 表示 agent 默认自主推进，但人类可以随时介入。

这是 `steerBox` 最重要的常规模式。

能力包括：

- 查看当前 goal
- 查看当前 loop 状态
- 查看计划、工具调用和验证结果
- 暂停 agent
- 修改约束或优先级
- 插入新指令
- 要求重新规划
- 标记 `GoalAlignmentCheckpoint`
- 标记 `RecoveryCheckpoint`
- 切换到 `human_in_the_loop`
- 切换到 `manual_takeover`

适用场景：

- 长时间自动化开发
- 长时间自动化安全分析
- 持续日志分析
- 需要人类随时把控但不希望每一步都审批的任务
- 大部分真实生产级 agent 运行场景

设计重点：

- 人类介入必须是 runtime 能力，不是任务失败后的补救按钮
- 人类操作必须写入 audit log
- 人类指令必须进入后续 context，但不能绕过 policy
- 接管后应能恢复为自动推进或主动审批模式

### 3. human_in_the_loop

`human_in_the_loop` 表示某些节点必须等待人类审批、确认、输入或选择。

适用场景：

- 高风险文件修改
- 跨模块重构
- 修改安全策略
- 执行不可逆外部动作
- 改变 `GoalContract`
- 扩大 scope
- evaluator 判定不确定
- `GoalAlignmentCheckpoint` 判定偏离或不清楚
- `DriftGuard` 判定可能破坏成熟代码

典型节点：

- pre-action approval
- plan approval
- diff approval
- restore approval
- side-effect approval
- scope expansion approval
- final acceptance

设计重点：

- 审批必须结构化记录
- 审批应说明批准对象、范围、原因和有效期
- 审批不应被泛化成永久授权
- 人类审批不能替代后续验证

### 4. manual_takeover

`manual_takeover` 表示人类接管执行权。

此时 agent 可以进入以下状态之一：

- `paused`：暂停等待
- `assistant_only`：只提供建议，不执行动作
- `recording_only`：记录人类操作和上下文
- `observer`：观察外部状态并维护 trace
- `handoff_ready`：准备恢复给 agent 或交给另一个 agent

适用场景：

- agent 明显跑偏
- 任务涉及关键生产风险
- 人类需要临时手工处理复杂上下文
- 自动化工具连续失败
- 安全事件处置需要人工判断
- 需要人工完成外部系统操作

设计重点：

- 接管不是失败，而是受控运行的一部分
- 接管前应记录当前状态
- 接管期间应继续记录关键事件
- 交还给 agent 前应重新构造 `GoalContract`、状态摘要和恢复点

## 模式切换

模式不应固定在任务开始时。

推荐允许以下切换：

```text
autonomous_loop -> human_steerable_loop
autonomous_loop -> human_in_the_loop
human_steerable_loop -> human_in_the_loop
human_steerable_loop -> manual_takeover
human_in_the_loop -> human_steerable_loop
manual_takeover -> human_steerable_loop
manual_takeover -> human_in_the_loop
```

不建议直接允许：

```text
manual_takeover -> autonomous_loop
```

原因是人工接管后，任务状态、外部副作用、目标边界和上下文可能已经变化。恢复自动推进前，应至少经过一次 `GoalAlignmentCheckpoint` 和一次 policy 复核。

## 触发升级的条件

以下情况应从自主推进升级为更强的人类介入：

- 动作风险升高
- scope 扩大
- 连续工具失败
- 连续验证失败
- evaluator 不确定
- 需要不可逆外部动作
- 需要修改权限、策略或安全边界
- 需要跨模块大规模重构
- `GoalAlignmentCheckpoint` 判定 `drifting`、`off_goal` 或 `unclear`
- `DriftGuard` 判定可能发生过度重构、架构漂移或安全边界弱化
- `SideEffectLedger` 显示已有不可回滚副作用

升级动作可以是：

- 进入只读模式
- 请求审批
- 暂停 loop
- 切换强模型或 evaluator
- 创建 `RecoveryCheckpoint`
- 要求人类接管

## 与 GoalContract 的关系

`GoalContract` 应显式包含人机介入策略。

字段草案：

```text
human_steering_mode
allowed_mode_transitions
required_approval_points
optional_review_points
manual_takeover_policy
resume_policy
operator_visibility_level
operator_notification_policy
approval_expiration_policy
audit_requirements
```

关键规则：

- `human_steering_mode` 决定默认运行模式
- `allowed_mode_transitions` 决定哪些切换被允许
- `required_approval_points` 决定哪些动作必须审批
- `resume_policy` 决定人工介入后如何恢复
- 所有模式切换必须写入 trace 和 audit log

## 与 LoopController 的关系

`LoopController` 每一轮都应读取当前人机模式。

建议 loop 增加以下检查：

```text
observe
  -> read human steering mode
  -> goal alignment checkpoint
  -> policy and steering decision
  -> plan
  -> pre-action approval if required
  -> act
  -> record
  -> verify
  -> post-action alignment checkpoint
  -> reflect
  -> update memory/context/goal
  -> recovery checkpoint decision
  -> next action / wait / takeover / stop
```

其中 `policy and steering decision` 至少判断：

- 当前是否允许继续自动推进
- 当前动作是否需要审批
- 是否需要通知人类
- 是否需要暂停
- 是否需要切换模式
- 是否需要人工接管

## 与 GoalAlignmentCheckpoint 的关系

`GoalAlignmentCheckpoint` 负责判断任务是否偏离目标。

它不是恢复点，而是方向检查点。

当出现以下情况时，应触发人类介入：

- 与 root goal 的关系无法说明
- 与当前 stage goal 的关系无法说明
- 当前阶段无法被 stage chain 证明是必要步骤
- agent 多轮执行但无法产生推进证据
- agent 试图用新目标替代原始目标

推荐处理：

```text
aligned -> continue
weakly_aligned -> continue with tighter checks
drifting -> require human steering or evaluator
off_goal -> pause or manual takeover
unclear -> ask human or switch evaluator
```

## 与 RecoveryCheckpoint 的关系

`RecoveryCheckpoint` 负责保存可恢复状态。

它不是方向判断机制。

两者关系：

- `GoalAlignmentCheckpoint` 判断是否偏离目标
- `RecoveryCheckpoint` 提供偏离后可恢复的状态点
- `manual_takeover` 前应尽量创建或选择一个恢复点
- 从恢复点继续执行前，应重新执行目标对齐检查

示例：

```text
agent detects drift
  -> pause loop
  -> inspect latest RecoveryCheckpoint
  -> ask human whether to restore, fork, or continue
  -> run GoalAlignmentCheckpoint after resume
```

## 与 DriftGuard 的关系

`DriftGuard` 负责防止 agent 用“合理理由”破坏已成型项目。

它可以触发：

- 降级到 `human_in_the_loop`
- 要求 evaluator 审查
- 要求人工审批 diff
- 阻止大规模修改
- 标记稳定 artifact
- 要求创建 `RecoveryCheckpoint`

与 `GoalAlignmentCheckpoint` 的区别：

- `GoalAlignmentCheckpoint` 关注目标方向是否偏离
- `DriftGuard` 关注实现、架构、行为、质量和安全边界是否漂移

## 与 SideEffectLedger 的关系

`SideEffectLedger` 负责记录不可逆或外部副作用。

以下动作通常不应在纯 `autonomous_loop` 中直接执行：

- 生产环境变更
- 删除外部数据
- 修改安全策略
- 发送对外请求导致状态变化
- 执行攻击模拟或扫描动作
- 触发响应处置动作

这些动作应至少进入：

- `human_in_the_loop`
- `manual_takeover`
- 或带强 policy gate 的 `human_steerable_loop`

## 与多模型路由的关系

不同模型可以承担不同人机模式下的不同角色。

建议：

- 弱模型：只做低风险分类、摘要、检索、格式整理
- 中等模型：做常规执行，但高风险动作必须审批
- 强模型：做复杂规划、风险判断、目标对齐检查、evaluator
- 本地模型：做隐私敏感初筛和低风险批处理
- evaluator 模型：在切换模式、恢复、审批前做独立判断

不同模式下也应使用不同 prompt profile：

```text
autonomous_loop -> stricter scope and stop conditions
human_steerable_loop -> status visibility and steerability emphasis
human_in_the_loop -> approval request and evidence contract
manual_takeover -> handoff, state summary, and no-action contract
```

## 第一阶段建议

第一阶段不需要实现完整控制面，但应在设计中预留以下对象：

```text
HumanSteeringMode
HumanSteeringPolicy
HumanInterventionEvent
ApprovalRequest
ApprovalDecision
ManualTakeoverSession
ResumeRequest
ResumeDecision
```

最小字段建议：

```text
mode
reason
trigger
requested_by
approved_by
scope
expires_at
related_goal_id
related_trace_id
related_checkpoint_id
decision
decision_reason
resume_policy
audit_record_id
```

## 当前结论

`steerBox` 应同时支持人在环、人类可驾驭自主运行和人工接管。

最推荐的默认模式不是完全无人值守，也不是每一步都人在环，而是：

```text
human_steerable_loop
```

也就是 agent 在受控、可审计、可追踪、可恢复的 loop 中自主推进，同时人类始终可以通过控制面观察、审批、纠偏、暂停、恢复和接管。
