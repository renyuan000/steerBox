# Loop Trajectory Checks Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 中 `LoopTrajectoryCheck` 的设计方向。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

它回答的问题是：

- 一个 loop 从 step 1 执行到 step N，中间路径是否合理
- agent 是否在多步执行中逐渐偏离目标、扩大范围或绕开约束
- 当前检查通过是否掩盖了过去步骤中的不当偏差
- 如何把轨迹审查和 `GoalAlignmentCheckpoint`、`DriftGuard`、`RecoveryCheckpoint` 区分开

## 为什么需要 LoopTrajectoryCheck

很多 loop 示例采用的是：

```text
do work -> run check -> fix -> repeat -> exit condition
```

这种结构有价值，但它主要验证当前结果是否通过检查，不一定验证从 step 1 到当前 step N 的执行路径是否合理。

问题是，长程 agent 可能出现这种情况：

- 当前 check 通过了，但中间删掉了重要边界条件
- 当前测试绿了，但 agent 改弱了测试
- 当前文档同步了，但文档写了未实现能力
- 当前 audit clean 了，但 agent 关闭了 audit 规则
- 当前目标看似完成，但 agent 多次扩大 scope
- 当前阶段看似合理，但过去 N 步建立在错误假设上
- 当前没有越权动作，但之前已经产生不可逆副作用

因此，仅有 exit condition 不够。

`steerBox` 需要一种面向执行轨迹的检查：

```text
LoopTrajectoryCheck = audit the path from step 1 to step N
```

## 和已有概念的区别

### RecoveryCheckpoint

`RecoveryCheckpoint` 负责：

- 保存可恢复状态
- 支持 restore / fork / replay
- 说明恢复前置条件和恢复保证

它不负责判断过去 N 步是否合理。

### GoalAlignmentCheckpoint

`GoalAlignmentCheckpoint` 负责：

- 当前动作是否仍对齐 root goal
- 当前阶段是否仍对齐 stage goal
- 当前阶段是否能被 stage chain 证明是必要步骤

它关注方向，但不一定审计完整历史路径。

### DriftGuard

`DriftGuard` 负责：

- 当前 diff 是否破坏代码、架构、安全边界或稳定 artifacts
- 是否发生过度重构、scope creep、style churn、安全弱化

它关注改动风险，但不一定判断多步路径中的累积偏差。

### LoopTrajectoryCheck

`LoopTrajectoryCheck` 负责：

- 从 step 1 到 step N 的执行路径是否合理
- 多轮决策是否逐渐偏离
- 是否存在投机取巧、绕过检查、错误假设延续
- 是否存在局部检查通过但整体路径不当

简单区分：

```text
RecoveryCheckpoint      -> 能不能恢复
GoalAlignmentCheckpoint -> 当前是否对齐目标
DriftGuard              -> 当前改动是否破坏系统
LoopTrajectoryCheck     -> 过去 N 步的路径是否合理
```

## 检查对象

`LoopTrajectoryCheck` 至少应读取：

```text
root_goal
stage_goal
GoalContract
loop_run_record
step_events
plans
actions
tool_calls
files_touched
diffs
checks_run
check_results
failures
retries
human_instructions
policy_decisions
checkpoint_events
side_effect_events
memory_writes
guardrail_triggers
assumptions
```

## 需要发现的问题

### 1. 目标漂移累积

表现：

- 每一步看似相关，但整体逐渐远离 root goal
- agent 用新的子目标替代原始目标
- stage chain 无法解释当前任务为什么必要

处理：

- 触发 `GoalAlignmentCheckpoint`
- 要求重新规划
- 必要时请求人类介入

### 2. Scope creep

表现：

- 触碰越来越多不相关文件
- 从局部修复扩展为架构重构
- 从文档整理扩展为功能设计

处理：

- 触发 `DriftGuard`
- 降级到 `human_in_the_loop`
- 要求 diff review

### 3. Check gaming

表现：

- 修改 check command
- 关闭 linter / audit / test
- 删除测试
- 放宽断言
- 改 CI 配置让失败项不再 required

处理：

- block
- restore 或 fork
- 写入 failure memory
- 触发人工审查

### 4. 错误假设延续

表现：

- 某一步引入未验证假设
- 后续多步都基于该假设推进
- 最终 check 失败或结果虽然通过但依据错误

处理：

- 标记 assumption invalidation
- 回退到引入假设前的 safe point
- 要求重新 observe / plan

### 5. 失败模式重复

表现：

- 同类工具错误重复出现
- 同类测试失败重复出现
- 同一错误解释被重复使用但未验证

处理：

- 生成 `FailurePattern`
- 生成 `GuardrailCandidate`
- 生成 `ToolRecipeCandidate`
- 必要时触发 `MemoryConsolidationQueue`

### 6. 副作用路径不清

表现：

- 外部动作发生了，但事件链不完整
- restore 后可能重复执行副作用
- 当前状态和外部系统状态不一致

处理：

- 检查 `SideEffectLedger`
- 降级到 `manual_takeover` 或 `human_in_the_loop`
- 禁止自动重放外部动作

## 触发时机

建议触发 `LoopTrajectoryCheck` 的时机：

```text
every N loop iterations
before stage transition
before final completion claim
before commit / PR / merge
before accepting checkpoint as stable
after repeated failures
after evaluator uncertainty
after large diff appears
after modifying tests or check config
after side effect event
when human requests review
```

其中最重要的是：

- before final completion claim
- after repeated failures
- after modifying tests or check config
- before accepting checkpoint as stable

## 输出状态

建议输出：

```text
trajectory_status:
  - clean
  - weakly_suspicious
  - drifting
  - gaming_detected
  - assumption_invalid
  - side_effect_unclear
  - needs_human_review
```

含义：

- `clean`: 轨迹合理，可以继续
- `weakly_suspicious`: 有轻微风险，收紧检查
- `drifting`: 累积偏离，要求重规划或目标对齐检查
- `gaming_detected`: 发现绕过检查或篡改指标，阻断
- `assumption_invalid`: 关键假设失效，需要回退或重规划
- `side_effect_unclear`: 外部副作用链不完整，暂停自动推进
- `needs_human_review`: 需要人工判断

## LoopTrajectoryCheck 输出草案

```text
trajectory_check_id
loop_run_id
goal_id
stage_id
step_range
status
findings
risk_level
confidence
root_goal_alignment_delta
stage_goal_alignment_delta
scope_delta
check_integrity_status
test_integrity_status
assumption_status
side_effect_status
recommended_action
related_events
related_checkpoints
created_at
```

## 推荐动作

```text
continue
tighten_checks
run_goal_alignment_checkpoint
run_drift_guard
run_evaluator
create_recovery_checkpoint
restore_or_fork
request_human_review
manual_takeover
block
```

## Per-Turn Traceability and Rollback Semantics

`steerBox` 应把每一轮对话和执行都记录成可追踪的 turn / step 对象。

### 每轮至少应有的标识

```text
conversation_id
session_id
loop_run_id
step_id
message_id
tool_call_id
checkpoint_id
trajectory_check_id
```

### 每轮应记录的内容

每一轮建议至少包含：

- 用户输入
- agent 输出
- 工具调用请求
- 工具调用结果
- 文件变更摘要
- policy decision
- human decision
- checkpoint decision
- side effect summary
- 关键置信度与来源

### 精确调出某轮

精确调出某轮不应只依赖聊天文本，而应依赖结构化记录：

```text
conversation_id + step_id
  -> turn record
  -> associated messages
  -> tool calls
  -> diffs
  -> check results
  -> checkpoint metadata
```

### 精确回滚语义

“回滚到某轮”不应理解为简单删除后续消息，而应理解为：

1. 选择一个稳定的 `RecoveryCheckpoint` 或 step boundary
2. 恢复到该轮对应的 state / workspace / policy 状态
3. 重新建立当前 context
4. 从该点 fork 或 replay 后续步骤
5. 若存在外部副作用，则根据 `SideEffectLedger` 决定是否重放、跳过或人工处理

### 回滚边界

不是所有轮次都能无损回滚：

- 只读分析轮次通常可回滚
- 文件修改轮次可在 checkpoint 覆盖范围内回滚
- 触发外部系统变化的轮次必须结合 side effect ledger
- 已经影响第三方系统的轮次，通常只能 fork 或人工补偿，而不能简单回退

### 设计要求

- 每轮 ID 必须稳定、可索引、可检索
- 每轮记录必须可审计
- 轨迹检查必须能按 step 范围查询
- 回滚点必须说明恢复保证
- 回滚后的 context 必须重新做 goal alignment check

## 和 loops.elorm.xyz 示例的关系

loops.elorm.xyz 的示例主要提供：

- 明确目标
- 明确 check command
- 明确 exit condition
- anti-gaming guardrails
- 失败后继续迭代

这些能力是好的，但还不等于完整高可靠长程 loop。

`steerBox` 应在这些 loop 之上补一层：

```text
LoopTrajectoryCheck
```

用于审查过去 N 步有没有累计偏差、投机取巧、错误假设延续或不当副作用。

## 与存储系统的关系

`LoopTrajectoryCheck` 依赖可审计事件链。

必须能够读取：

- event log
- current state
- trace summary
- loop run record
- tool call record
- check result record
- diff summary
- side effect ledger
- memory write record

如果没有这些记录，轨迹检查只能变成模型回忆，可靠性不够。

因此，`LoopTrajectoryCheck` 应和 `MEMORY_STORAGE_AND_CONSOLIDATION.md` 中的多层存储设计联动。

## 与记忆进化的关系

轨迹检查发现的问题不应只用于当前任务。

可沉淀为：

```text
FailurePattern
GuardrailCandidate
ToolRecipeCandidate
LoopPolicyCandidate
MemoryCorrectionCandidate
```

但这些 candidate 不应自动变成全局规则。

建议流程：

```text
trajectory finding
  -> candidate generated
  -> local validation
  -> risk classification
  -> human approval if high risk
  -> accepted memory / guardrail / recipe
```

## 第一阶段建议

第一阶段不需要实现完整 evaluator。

可以先做文档级 schema 和人工审查模板：

```text
LoopTrajectoryCheckTemplate:
  step_range:
  original_goal:
  current_stage:
  files_touched:
  checks_changed:
  tests_changed:
  assumptions_added:
  repeated_failures:
  side_effects:
  suspicious_actions:
  conclusion:
  recommended_action:
```

然后在以下节点手动使用：

- 大任务完成前
- 提交前
- 大量 diff 后
- 连续失败后
- 修改测试或检查配置后

## 当前结论

`LoopTrajectoryCheck` 是 `steerBox` 长程自主 loop 的必要安全层。

没有它，agent 可能每一步都看似合理，最终却通过一条不合理路径抵达结果。

`steerBox` 应同时具备：

```text
RecoveryCheckpoint      -> 状态恢复
GoalAlignmentCheckpoint -> 方向校准
DriftGuard              -> 改动防护
SideEffectLedger        -> 外部副作用治理
LoopTrajectoryCheck     -> Step 1-N 轨迹审查
```
