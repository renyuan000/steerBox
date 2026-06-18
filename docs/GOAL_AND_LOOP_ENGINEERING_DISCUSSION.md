# Goal and Loop Engineering Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 中 Goal Engineering 与 Loop Engineering 的设计方向。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

它补齐的是从 `harness` 继续向上的控制层：

```text
prompt -> context -> memory -> harness -> goal -> loop engineering -> agent efficiency
```

## 核心判断

`steerBox` 不应从 Harness Engineering 改名为 Loop Engineering。

更准确的定位是：

> `steerBox` 是一个以 Harness 为底座、以 Goal 和 Loop 为核心控制层、以 Efficiency 为优化目标的智能体学习与实践框架。

其中：

- `Harness` 提供工具、权限、状态、审计、恢复和运行环境
- `Goal` 定义目标、边界、成功标准、风险和验证契约
- `Loop` 决定 agent 如何观察、计划、行动、验证、反思、更新和继续
- `Efficiency` 衡量 agent 是否以更低总成本达成更高质量的可验证结果

## Goal Engineering

Goal Engineering 的目标是把自然语言目标转成可执行、可验证、可约束、可恢复的 `GoalContract`。

### 为什么 Goal 必须是一等对象

如果 goal 只是 prompt 里的自然语言描述，会出现几个问题：

- 目标边界容易在长任务中漂移
- 成功标准容易在最后才临时定义
- 工具权限与任务风险无法稳定绑定
- evaluator 不知道应该验证什么
- checkpoint 无法判断恢复后应继续哪个目标
- 多模型路由无法根据任务难度和风险做选择
- 人类介入点无法提前定义

因此，`GoalContract` 应该成为 harness 可读取、可审计、可更新的结构化对象。

### GoalContract 字段草案

建议至少包含：

```text
goal_id
objective
scope
out_of_scope
success_criteria
quality_bar
constraints
risk_level
allowed_tools
forbidden_tools
allowed_models
model_routing_policy
verification_plan
stop_conditions
human_steering_mode
human_intervention_points
checkpoint_policy
memory_policy
efficiency_budget
audit_requirements
side_effect_boundary
rollback_or_compensation_plan
```

### 字段说明

- `objective`：目标是什么
- `scope`：允许处理的范围
- `out_of_scope`：明确不处理的范围
- `success_criteria`：什么算完成
- `quality_bar`：质量要求，例如代码实践、安全标准、可维护性
- `constraints`：必须遵守的约束
- `risk_level`：低、中、高、关键风险
- `allowed_tools`：允许调用的工具
- `forbidden_tools`：明确禁止的工具
- `allowed_models`：可参与该任务的模型集合
- `model_routing_policy`：模型选择策略
- `verification_plan`：如何验证完成
- `stop_conditions`：何时停止、暂停或升级
- `human_steering_mode`：`autonomous_loop`、`human_steerable_loop`、`human_in_the_loop`、`manual_takeover`
- `human_intervention_points`：哪些节点需要人类介入
- `checkpoint_policy`：何时创建 soft / hard checkpoint
- `memory_policy`：哪些信息可以写入记忆
- `efficiency_budget`：时间、token、工具调用、系统资源预算
- `audit_requirements`：审计要求
- `side_effect_boundary`：允许产生哪些外部影响
- `rollback_or_compensation_plan`：回滚或补偿策略

## Loop Engineering

Loop Engineering 的目标是把 agent 执行过程设计成可观测、可调度、可验证、可恢复、可优化的循环。

### 基础循环

一个基础 loop 可以描述为：

```text
observe -> goal alignment checkpoint -> plan -> act -> record -> verify -> reflect -> update memory/context/goal -> recovery checkpoint decision -> next action / stop
```

这不是普通 while 循环，而是 harness 级控制对象。

### LoopController 职责

`LoopController` 至少负责：

- 读取当前 `GoalContract`
- 读取当前任务状态
- 选择下一步动作
- 控制 token 和上下文预算
- 决定是否调用工具
- 决定使用哪个模型或模型组合
- 决定是否启用 evaluator
- 决定是否写 memory
- 决定是否更新 goal
- 决定是否执行 goal alignment checkpoint
- 决定是否创建 recovery checkpoint
- 决定是否请求人类介入
- 决定失败后的重试、降级、中止或恢复

### Loop 阶段定义

#### 1. Observe

读取当前状态和必要上下文。

输入可能包括：

- GoalContract
- task state
- event log
- checkpoint metadata
- repo / log / security signal
- memory retrieval result
- previous evaluator result
- human instruction

关键要求：

- 上下文按需加载
- 原始证据与摘要分离
- retrieval 结果必须带来源和置信度

#### 2. Plan

根据目标、状态、约束和预算选择下一步。

输出可能包括：

- next action
- required tool
- selected model
- expected evidence
- risk level
- checkpoint need
- human approval need

关键要求：

- plan 不应过度冻结底层实现
- plan 必须桥接目标与验证
- 高风险动作必须经过 policy gate

#### 3. Act

执行具体动作。

动作类型可能包括：

- read file
- write file
- run command
- call tool
- call MCP server
- query memory
- ask human
- spawn subagent
- update checkpoint

关键要求：

- 工具调用必须可审计
- side effect 必须记录
- 高风险动作必须符合权限和审批策略

#### 4. Record

将关键事实写入事件流。

至少记录：

- action
- input summary
- output summary
- tool result
- model used
- token / time / resource cost
- policy decision
- side effect
- error
- trace id

关键要求：

- 关键事件必须 durable write
- trace 可以异步或批量写入
- 记录应服务恢复、审计和效率分析

#### 5. Verify

验证动作是否让任务向目标推进。

验证方式可能包括：

- tests
- lint
- static analysis
- security policy check
- replay / simulation
- log evidence check
- human review
- evaluator model

关键要求：

- generator 和 evaluator 职责应分离
- evaluator 是否开启由任务风险、模型能力和成本决定
- 不能接受 agent 的口头完成声明作为完成证据

#### 6. Reflect

分析本轮结果。

输出可能包括：

- 是否推进目标
- 是否偏离目标
- 是否需要调整计划
- 是否需要写入 memory
- 是否需要生成 evolution proposal
- 是否需要请求人类介入

关键要求：

- reflection 必须基于证据
- reflection 不应直接变成全局规则
- 高风险经验迁移必须验证或审批

#### 7. Update Memory / Context / Goal

根据本轮结果更新可持续状态。

可能更新：

- task state
- short-term context summary
- long-term memory candidate
- failure memory
- tool memory
- codebase memory
- GoalContract 子字段

关键要求：

- 原始证据不能被摘要覆盖
- memory candidate 不等于 accepted policy
- goal 更新必须可审计

#### 8. Goal Alignment Checkpoint

检查当前执行是否仍然对齐初始目标、当前阶段目标和已执行阶段链，必要时纠正方向、缩小 scope、请求人类确认或停止。

#### 9. Recovery Checkpoint Decision

根据 checkpoint policy 创建恢复点。

可能类型：

- soft checkpoint
- hard checkpoint
- operator-marked checkpoint
- pre-side-effect checkpoint
- post-verification checkpoint

关键要求：

- checkpoint 要能支持恢复到可继续状态
- 对外部 side effect 必须记录边界和补偿要求
- checkpoint 创建成本也应进入效率指标

#### 10. Next Action / Stop

决定继续、暂停、升级、回滚或结束。

可能结果：

- continue
- stop success
- stop failed
- wait for human
- downgrade model
- escalate model
- retry
- restore checkpoint
- spawn subagent
- create evolution proposal

## 必要控制门

LoopController 不应直接执行所有动作，应经过控制门。

### PolicyGate

决定动作是否允许。

关注：

- 权限
- 风险
- side effect
- 人机介入模式
- 是否需要审批

### ToolGate

决定工具是否可用和值得调用。

关注：

- 工具权限
- 工具成本
- 工具失败率
- 是否有替代工具
- 是否支持 dry-run 或 rollback

### ModelGate

决定使用哪个模型。

关注：

- 费用
- 特长
- 可访问性
- 限流
- 任务难度
- 质量要求
- 隐私要求
- 历史效率

### BudgetGate

决定是否继续消耗资源。

关注：

- token
- wall time
- tool call count
- memory / CPU / IO
- human intervention cost

### VerificationGate

决定验证是否足够。

关注：

- 是否满足 success criteria
- 是否满足 quality bar
- 是否存在未验证高风险改动
- evaluator 是否需要开启

### RecoveryGate

决定是否 checkpoint、restore 或补偿。

关注：

- 当前状态是否可恢复
- 外部副作用是否已记录
- 是否需要 hard checkpoint
- 是否需要人工确认恢复

## 与两条主线的关系

### 软件开发智能体

Goal 和 Loop 应重点控制：

- 修改范围
- 代码质量标准
- 测试与验证计划
- review 与 evaluator 策略
- 误改检测
- checkpoint 与代码改动绑定
- 多模型分工，例如低成本模型做检索，强模型做架构与疑难分析

### 安全智能体

Goal 和 Loop 应重点控制：

- 日志输入范围
- 事件判断标准
- 响应动作风险等级
- 自动化动作审批
- 证据链完整性
- side-effect ledger
- `autonomous_loop`、`human_steerable_loop`、`human_in_the_loop`、`manual_takeover` 模式切换

## 与效率工程的关系

Loop 是效率工程的主要观测对象。

每轮 loop 至少应记录：

- `loop_id`
- `goal_id`
- `selected_model`
- `tokens_used`
- `tool_calls`
- `wall_time`
- `verification_result`
- `policy_decision`
- `checkpoint_created`
- `human_intervention`
- `next_action`

后续才能计算：

- `tokens_per_solved_task`
- `loop_iterations_to_success`
- `failed_tool_call_rate`
- `irrelevant_context_ratio`
- `memory_hit_rate`
- `verification_pass_rate`
- `checkpoint_restore_success_rate`
- `wall_time_to_verified_result`
- `model_route_cost_saving`

## 第一阶段建议

第一阶段不需要完整实现所有控制门，但文档和接口设计应先保留位置。

建议优先定义：

- `GoalContract`
- `LoopController`
- `LoopEvent`
- `PolicyDecision`
- `VerificationResult`
- `MemoryUpdate`
- `GoalAlignmentCheckpoint`
- `CheckpointDecision`
- `ModelRoutingDecision`
- `EfficiencySnapshot`

最小可行闭环：

```text
GoalContract -> LoopController -> Tool/Model/Policy decision -> EventLog -> VerificationResult -> EfficiencySnapshot
```

## 当前结论

当前文档体系的方向是正确的：主线和横向能力应分开，由总纲串起来。

但在进入实现前，`GoalContract` 和 `LoopController` 必须成为一等设计对象，否则后续容易退回到“长 prompt + 工具调用 + 日志记录”的弱 harness。

本文件用于补齐这个缺口，并作为后续代码架构设计的直接输入。
