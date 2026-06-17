# Checkpoint Policy Algorithm Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 在自主推进 / loop 循环中的 recovery checkpoint 决策算法。它关注状态保存与恢复，不负责判断任务方向是否偏离；方向纠偏由 `GOAL_ALIGNMENT_CHECKS.md` 定义。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

它重点回答：

- 什么时候 checkpoint
- checkpoint 保存什么层级
- checkpoint 成本如何控制
- restore 后如何避免重复 side effect
- checkpoint 如何与 GoalAlignmentCheckpoint / DriftGuard 联动，辅助处理跑偏和破坏性改写

## 核心结论

`steerBox` 不应只采用固定周期 checkpoint。

推荐策略：

```text
fixed checkpoint
+ event-triggered checkpoint
+ risk-triggered checkpoint
+ operator-marked checkpoint
+ side-effect fence
```

原因：

- 固定 checkpoint 提供最低恢复保障，但成本可能高
- 事件触发 checkpoint 能覆盖关键动作前后
- 风险触发 checkpoint 能阻止自主 loop 越界扩大修改
- 用户标记 checkpoint 能保护人工认可的稳定状态
- side-effect fence 能避免 restore 后重复外部动作

## 外部资料精华与可用状态

### 1. Crab: Semantics-Aware Checkpoint/Restore Runtime for Agent Sandboxes

来源：

- https://arxiv.org/abs/2604.28138

对方怎么做：

- 研究 agent sandbox 中哪些 turn 真的需要 checkpoint
- 不是每轮都全量 checkpoint
- 根据 OS-visible side effects 和状态变化判断 checkpoint 必要性
- 目标是在恢复能力和 checkpoint 成本之间取得平衡

创新点：

- checkpoint 决策应语义感知，而不是固定频率
- 没有有效状态变化的 turn 不必昂贵保存
- agent 运行时应理解工具调用和系统副作用

steerBox 可用状态：

- 高价值，可直接吸收为 `CheckpointDecisionEngine`
- 第一阶段不需要 OS 级完整 sandbox checkpoint，但可以先用事件、文件 diff、tool side-effect 分类近似实现

### 2. DeltaBox: Scaling Stateful AI Agents with Millisecond-Level Sandbox Checkpoint/Rollback

来源：

- https://arxiv.org/abs/2605.22781

对方怎么做：

- 用增量 checkpoint / rollback 支持 stateful agent 大规模并发
- 避免每次复制完整 sandbox
- 面向多分支探索、快速回滚、低延迟恢复

创新点：

- checkpoint 不只是容灾，也可以服务探索、分支、回滚和并行验证
- 高性能 checkpoint 需要 delta / copy-on-write 思路

steerBox 可用状态：

- 中长期高价值
- 第一阶段先不实现 sandbox delta checkpoint，但在数据模型中保留 `checkpoint_parent_id`、`delta_from`、`forked_from`

### 3. ACRFence: Preventing Semantic Rollback Attacks in Agent Checkpoint-Restore

来源：

- https://arxiv.org/abs/2603.20625

对方怎么做：

- 关注 agent restore 后重新生成语义相近但不完全相同的外部请求
- 这可能导致重复支付、重复授权、凭证复活、重复运维动作等风险
- 提出对 checkpoint/restore 中外部动作进行防护

创新点：

- restore 风险不是只有技术状态不一致，还有语义层面的外部副作用重放
- LLM agent 的非确定性会放大 rollback 风险

steerBox 可用状态：

- 对安全智能体和运维智能体非常关键
- 必须设计 `SideEffectLedger`
- 高风险外部动作 restore 后默认不能自动重放

### 4. AEGIS: No Tool Call Left Unchecked

来源：

- https://arxiv.org/abs/2603.12621

对方怎么做：

- 在 agent tool call 前加入审计、防护和策略检查
- 不是事后记录，而是在调用前进行防护

创新点：

- tool call 应经过 firewall / policy gate
- checkpoint 应和 tool gate 结合，而不是孤立存在

steerBox 可用状态：

- 可直接转化为 `ToolGate` + `CheckpointGate`
- 高风险 tool call 前先 checkpoint、审批或阻断

### 5. SKILL.nb: Selective Formalization and Gated Execution for Durable Agent Workflows

来源：

- https://arxiv.org/abs/2606.08049

对方怎么做：

- 对 durable agent workflow 做选择性 formalization
- 关键步骤通过 gate 执行
- 可复用 skill 需要 evidence、validation 和 fallback

创新点：

- 不必形式化所有东西，但关键边界必须 gate
- skill/workflow 的复用需要验证和回退路径

steerBox 可用状态：

- 适合用于 skill/tool recipe 进化
- skill 更新前后应自动 checkpoint，并进入 evaluator / regression gate

### 6. LangGraph Persistence / Interrupts

来源：

- https://docs.langchain.com/oss/python/langgraph/persistence
- https://docs.langchain.com/oss/python/langgraph/interrupts

对方怎么做：

- checkpointer 保存 thread 的 graph state
- store 保存跨 thread 长期数据
- interrupt 支持执行中暂停、等待外部输入、恢复

创新点：

- persistence / interrupt / resume 是 runtime primitive
- thread state 和 long-term store 分离

steerBox 可用状态：

- 可吸收 checkpointer/store/interrupt 思路
- 但需要补强 hard/soft checkpoint、side-effect ledger、operator-marked checkpoint 和风险触发 checkpoint

## checkpoint 层级

建议分层：

```text
L0: event log only
L1: GoalContract + LoopState checkpoint
L2: workspace / file diff checkpoint
L3: sandbox / process checkpoint
L4: side-effect ledger + replay/fork policy
```

### L0: event log only

保存：

- loop event
- tool call summary
- policy decision
- evaluator result
- model routing decision

适用：

- 低风险读取
- 检索
- 摘要
- 无状态工具调用

### L1: GoalContract + LoopState

保存：

- goal state
- loop cursor
- selected model
- context cursor
- memory cursor
- pending decisions

适用：

- 长任务每轮状态
- agent/subagent restart
- 上下文丢失恢复

### L2: workspace / file diff

保存：

- 文件改动前状态
- diff patch
- modified file list
- test/evaluator state

适用：

- 代码写入
- 文档修改
- 配置修改
- skill/tool/policy 文件修改

### L3: sandbox / process

保存：

- 进程状态
- 临时文件
- 环境变量
- 工作目录
- 运行中服务状态

适用：

- 长时运行服务
- 多进程 agent
- browser / sandbox / simulation

第一阶段可以先不实现完整 L3，但 schema 应保留。

### L4: side-effect ledger

保存：

- 外部动作
- 幂等键
- 请求摘要
- 响应摘要
- 是否可重放
- 是否可补偿
- restore 后处理策略

适用：

- 网络请求
- 安全响应动作
- 运维动作
- 提交/发布/推送
- 外部系统写入

## CheckpointDecisionEngine

建议每轮 loop 调用一次决策器：

```text
CheckpointDecisionEngine(goal, loop_state, action, risk, diff, tool, side_effect, evaluator_result)
  -> checkpoint_level
  -> checkpoint_timing
  -> reason
  -> required_approval
  -> restore_policy
```

### 输入

```text
goal_id
loop_id
action_type
action_risk
affected_paths
diff_size
tool_name
tool_side_effect_type
policy_decision
evaluator_result
current_checkpoint_age
operator_checkpoint_exists
side_effect_pending
```

### 输出

```text
checkpoint_required: true | false
checkpoint_level: L0 | L1 | L2 | L3 | L4
checkpoint_timing: before_action | after_action | before_and_after | periodic_only
reason: string
requires_human_approval: true | false
restore_policy: replay | no_replay | fork_only | manual_reconcile
```

## 触发策略

### 固定触发

```yaml
fixed:
  every_loop_iterations: 5
  every_minutes: 10
  every_tool_calls: 20
  every_file_changes: 5
```

适用：长期运行的最低保障。

### 事件触发

```yaml
event_triggered:
  before_file_write: L2
  before_delete: L2
  before_bulk_change: L2
  before_config_change: L2
  before_policy_change: L2
  before_skill_update: L2
  before_tool_registry_update: L2
  before_external_side_effect: L4
  after_evaluator_pass: L1
  after_goal_update: L1
```

适用：关键状态变化。

### 风险触发

```yaml
risk_triggered:
  scope_expansion: L2
  core_file_touched: L2
  security_boundary_touched: L4
  diff_too_large: L2
  repeated_evaluator_failure: L1
  loop_not_converged: L1
  irreversible_side_effect: L4
```

适用：防跑偏和高风险动作。

### 用户触发

```yaml
operator_marked:
  label_required: true
  reason_required: true
  default_level: L2
  allow_hard_checkpoint: true
```

适用：用户认可的稳定状态。

## restore 策略

### replay

适用：

- 纯读操作
- 幂等操作
- 可重复验证操作

### no_replay

适用：

- 不可逆外部动作
- 安全响应动作
- 生产系统写入
- 付款、授权、删除、封禁等动作

### fork_only

适用：

- 不确定是否可重放
- 需要保留原分支状态
- 探索性 refactor
- 多模型并行尝试

### manual_reconcile

适用：

- side effect 已发生但状态不确定
- 外部系统状态不可自动查询
- 需要人工判断是否补偿

## checkpoint 成本指标

checkpoint 本身也要进入效率工程。

建议记录：

- `checkpoint_count_per_task`
- `checkpoint_write_latency`
- `checkpoint_size_bytes`
- `restore_time_ms`
- `checkpoint_hit_rate`
- `checkpoint_restore_success_rate`
- `lost_work_after_restore`
- `side_effect_reconcile_count`
- `checkpoint_overhead_ratio`

## 第一阶段建议

先实现或预留：

- `CheckpointDecision`
- `CheckpointPolicy`
- `CheckpointEvent`
- `RestorePolicy`
- `CheckpointLevel`
- `SideEffectLedgerCursor`

第一阶段可以不实现完整 sandbox checkpoint，但必须实现：

- L0 event log
- L1 Goal/Loop checkpoint
- L2 workspace diff checkpoint
- L4 side-effect ledger schema

## 当前结论

自主 loop 的 checkpoint 不应靠单一固定间隔。

更稳的做法是语义感知的 `CheckpointDecisionEngine`：根据动作、风险、diff、工具、副作用、验证结果和用户标记综合决定 checkpoint 层级与 restore 策略。
